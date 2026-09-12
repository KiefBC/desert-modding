//! IDA-style byte patterns: `"48 8B ?? 20 01 00 00"`, `??` = any byte.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    bytes: Vec<Option<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Unique(usize),
    None,
    Ambiguous(usize),
}

impl Pattern {
    /// Parse a space-separated pattern. Returns `None` on a malformed token or
    /// an empty / all-wildcard pattern.
    pub fn parse(text: &str) -> Option<Pattern> {
        let mut bytes = Vec::new();
        for tok in text.split_whitespace() {
            match tok {
                "??" | "?" => bytes.push(None),
                t if t.len() == 2 => bytes.push(Some(u8::from_str_radix(t, 16).ok()?)),
                _ => return None,
            }
        }
        if bytes.is_empty() || bytes.iter().all(Option::is_none) {
            return None;
        }
        Some(Pattern { bytes })
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn matches_at(&self, hay: &[u8], at: usize) -> bool {
        // One bounds check per candidate offset, not per byte: the window is
        // taken once and the compare below runs over a slice of known length.
        match hay.get(at..at + self.bytes.len()) {
            Some(win) => self.bytes.iter().zip(win).all(|(p, b)| p.is_none_or(|p| p == *b)),
            None => false,
        }
    }

    /// The first literal byte and its index: what [`Pattern::find_all`] anchors
    /// on, so that the inner loop is a byte compare instead of a full pattern
    /// compare at every offset. `parse` is the only constructor and it rejects
    /// an all-wildcard pattern, so this always answers; `None` is a pattern no
    /// scan can anchor, and finding nothing is the safe answer for it.
    fn anchor(&self) -> Option<(usize, u8)> {
        self.bytes.iter().enumerate().find_map(|(i, b)| b.map(|b| (i, b)))
    }

    /// The literal byte that `counts` says is rarest, and its index: what
    /// [`find_all_multi`] anchors on instead.
    ///
    /// Any literal position anchors the pattern correctly — the candidate start
    /// is just `i - idx` — so this is free to pick whichever one costs the
    /// fewest `matches_at` calls. Which matters enormously: four of Desert
    /// Looter's eight signatures begin with `48`, the REX.W prefix, which
    /// occurs 24,079,962 times in the 363 MB image. Anchored on their first
    /// literal the eight together raise **101,982,014** candidates; anchored on
    /// their rarest, **4,494,400**, a 23x cut of the ~0.33 s the pass spends
    /// checking candidates on top of its ~0.19 s of bare walking.
    ///
    /// Ties keep the lower index, so the choice is deterministic and an
    /// uninformative sample (a haystack shorter than [`SAMPLE_STRIDE`], or an
    /// empty one) degrades to exactly [`Pattern::anchor`] rather than to
    /// something arbitrary.
    fn rarest_anchor(&self, counts: &[u32; 256]) -> Option<(usize, u8)> {
        let mut best: Option<(usize, u8, u32)> = None;
        for (i, byte) in self.bytes.iter().enumerate() {
            let Some(byte) = *byte else { continue };
            let count = counts.get(usize::from(byte)).copied().unwrap_or(0);
            let keep = matches!(best, Some((_, _, b)) if b <= count);
            if !keep {
                best = Some((i, byte, count));
            }
        }
        best.map(|(i, byte, _)| (i, byte))
    }

    /// Every offset where the pattern matches, stopping after `limit` hits.
    pub fn find_all(&self, hay: &[u8], limit: usize) -> Vec<usize> {
        let mut hits = Vec::new();
        if hay.len() < self.bytes.len() || limit == 0 {
            return hits;
        }
        let Some((anchor_idx, anchor)) = self.anchor() else {
            return hits;
        };
        let last_start = hay.len() - self.bytes.len();
        let mut i = anchor_idx;
        while i <= last_start + anchor_idx {
            let Some(rest) = hay.get(i..) else { break };
            match rest.iter().position(|&b| b == anchor) {
                None => break,
                Some(p) => {
                    let start = i + p - anchor_idx;
                    if start <= last_start && self.matches_at(hay, start) {
                        hits.push(start);
                        if hits.len() >= limit {
                            break;
                        }
                    }
                    i += p + 1;
                }
            }
        }
        hits
    }

    /// A signature is only useful if it identifies one place.
    pub fn find_unique(&self, hay: &[u8]) -> Found {
        verdict(&self.find_all(hay, 2))
    }
}

/// What a capped hit list says about a signature. The cap is why
/// `Ambiguous` counts "at least": two hits is enough to know.
fn verdict(hits: &[usize]) -> Found {
    match hits {
        [] => Found::None,
        [one] => Found::Unique(*one),
        many => Found::Ambiguous(many.len()),
    }
}

/// How many patterns one walk of the haystack can carry: one bit of the mask
/// table's word each. More than that is chunked into passes of this size, so
/// there is no cap a caller has to know about.
const LANES: usize = 64;

/// Every this many bytes is sampled to rank byte rarity. Prime, so it cannot
/// alias with the 16-byte alignment compiled code is padded to and read only
/// the same slot of every instruction group.
///
/// Counting all 363 MB costs about 250 ms — more than the whole walk the
/// ranking is meant to speed up. This reads ~1.5 M bytes in a few ms, and a
/// sample that size is far more than enough to *rank*: a byte occurring 24 M
/// times in the image lands ~94,000 times in it, a rare one ~1,400. Only the
/// order matters, never the values.
const SAMPLE_STRIDE: usize = 251;

/// Sampled frequency of each byte value in `hay`, for [`Pattern::rarest_anchor`].
///
/// A haystack shorter than [`SAMPLE_STRIDE`] contributes its first byte alone
/// and an empty one contributes nothing; both leave a table too flat to rank,
/// which is not a failure — the anchor choice falls back to the lowest-index
/// literal, which is what the scan used before and is correct, just not faster.
fn sample_byte_counts(hay: &[u8]) -> [u32; 256] {
    let mut counts = [0u32; 256];
    for &b in hay.iter().step_by(SAMPLE_STRIDE) {
        if let Some(slot) = counts.get_mut(usize::from(b)) {
            *slot = slot.saturating_add(1);
        }
    }
    counts
}

/// Every offset where each of `pats` matches, from **one** walk of `hay`,
/// aligned with `pats` by index. `find_all_multi(pats, hay, n)[i]` is
/// `pats[i].find_all(hay, n)` byte for byte, limit truncation included.
///
/// The image is 363 MB and a pass over it costs about 190 ms, so the count of
/// passes is the whole cost: Desert Looter's eight signatures were eight passes
/// and 1.49 s, some 80% of its startup, all of it before the inline hooks it
/// must install while the game is still loading.
///
/// The walk is [`Pattern::find_all`]'s anchor trick widened to a set: a
/// 256-entry table of bit masks says which patterns this byte anchors, so a
/// byte that anchors nothing — nearly all of them — costs a load and a branch,
/// and only a byte that anchors something pays for a compare.
///
/// Which byte anchors a pattern is decided by [`Pattern::rarest_anchor`] over a
/// sample of `hay`, not by taking the first literal: the first literal of half
/// Desert Looter's signatures is `48`, and the difference is 102 M candidate
/// compares against 4.5 M.
pub fn find_all_multi(pats: &[Pattern], hay: &[u8], limit: usize) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = pats.iter().map(|_| Vec::new()).collect();
    if limit == 0 || pats.is_empty() {
        return out;
    }
    // Once for the whole call, not once per pass: the ranking does not depend
    // on which patterns a pass carries.
    let counts = sample_byte_counts(hay);
    for (pass, group) in pats.chunks(LANES).enumerate() {
        let first = pass * LANES;
        let mut table = [0u64; 256];
        let mut anchors = [0usize; LANES];
        // Cleared as a pattern reaches `limit`, so a satisfied pattern stops
        // costing compares and an all-satisfied pass stops walking.
        let mut live: u64 = 0;
        for (lane, p) in group.iter().enumerate() {
            let Some((idx, byte)) = p.rarest_anchor(&counts) else { continue };
            if hay.len() < p.len() {
                continue;
            }
            let bit = 1u64 << lane;
            if let Some(slot) = table.get_mut(byte as usize) {
                *slot |= bit;
            }
            if let Some(slot) = anchors.get_mut(lane) {
                *slot = idx;
            }
            live |= bit;
        }
        if live == 0 {
            continue;
        }
        for (i, &b) in hay.iter().enumerate() {
            // `b` is a byte and the table has 256 entries, so this is always a
            // hit; going through `get` is what keeps the bounds check out of
            // the hot loop's source and LLVM elides it anyway.
            let mut mask = table.get(usize::from(b)).copied().unwrap_or(0) & live;
            while mask != 0 {
                let lane = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                let (Some(&anchor_idx), Some(p)) = (anchors.get(lane), group.get(lane)) else {
                    continue;
                };
                // The anchor is at `i`, so the match would start this far back;
                // an underflow is a candidate that begins before the haystack.
                let Some(start) = i.checked_sub(anchor_idx) else { continue };
                if !p.matches_at(hay, start) {
                    continue;
                }
                let Some(hits) = out.get_mut(first + lane) else { continue };
                hits.push(start);
                if hits.len() >= limit {
                    live &= !(1u64 << lane);
                }
            }
            if live == 0 {
                break;
            }
        }
    }
    out
}

/// Per-pattern [`Found`], from one walk of `hay`: [`Pattern::find_unique`] for
/// a whole signature table at the price of a single pass.
pub fn find_unique_multi(pats: &[Pattern], hay: &[u8]) -> Vec<Found> {
    find_all_multi(pats, hay, 2).into_iter().map(|hits| verdict(&hits)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wildcards() {
        let p = Pattern::parse("48 8B ?? 20").unwrap();
        assert_eq!(p.len(), 4);
        assert!(Pattern::parse("?? ??").is_none());
        assert!(Pattern::parse("4G").is_none());
        assert!(Pattern::parse("").is_none());
    }

    #[test]
    fn finds_with_wildcard_prefix() {
        let hay = [0x00, 0xE8, 0x11, 0x22, 0x44, 0xE8, 0x11, 0x22, 0x44, 0xE8];
        let p = Pattern::parse("?? E8 ?? 22 44").unwrap();
        assert_eq!(p.find_all(&hay, 10), vec![0, 4]);

        assert_eq!(Pattern::parse("E8").unwrap().find_unique(&hay), Found::Ambiguous(2));
        assert_eq!(Pattern::parse("FF").unwrap().find_unique(&hay), Found::None);
    }

    /// The whole contract of the one-pass scan: what it reports for a pattern
    /// is what that pattern reports on its own, whatever it finds.
    #[test]
    fn one_pass_agrees_with_one_pattern_at_a_time() {
        let hay = [0x00, 0xE8, 0x11, 0x22, 0x44, 0xE8, 0x11, 0x22, 0x44, 0xE8];
        let texts = ["?? E8 ?? 22 44", "E8", "FF", "22 44", "11 22 44 E8 11 22 44 E8 11 22 44"];
        let pats: Vec<Pattern> = texts.iter().filter_map(|t| Pattern::parse(t)).collect();
        assert_eq!(pats.len(), texts.len());

        let all = find_all_multi(&pats, &hay, 10);
        let unique = find_unique_multi(&pats, &hay);
        assert_eq!(all.len(), pats.len());
        assert_eq!(unique.len(), pats.len());
        for (i, p) in pats.iter().enumerate() {
            assert_eq!(all.get(i), Some(&p.find_all(&hay, 10)), "{}", texts[i]);
            assert_eq!(unique.get(i), Some(&p.find_unique(&hay)), "{}", texts[i]);
        }
        assert_eq!(unique.first(), Some(&Found::Ambiguous(2))); // the wildcard-prefix one
        assert_eq!(unique.get(2), Some(&Found::None)); // "FF", absent
    }

    /// Two patterns anchored on the same byte share a lane mask bit-for-bit;
    /// neither may swallow the other's hits.
    #[test]
    fn patterns_sharing_an_anchor_byte_both_report() {
        let hay = [0xE8, 0x01, 0xE8, 0x02, 0xE8, 0x01];
        let pats: Vec<Pattern> = ["E8 01", "E8 02", "E8"]
            .iter()
            .filter_map(|t| Pattern::parse(t))
            .collect();
        let all = find_all_multi(&pats, &hay, 8);
        assert_eq!(all, vec![vec![0, 4], vec![2], vec![0, 2, 4]]);
    }

    /// The truncation is per pattern, not a budget shared across the pass.
    #[test]
    fn the_limit_is_per_pattern() {
        let hay = [0xE8, 0xE8, 0xE8, 0x90, 0x90];
        let pats: Vec<Pattern> = ["E8", "90"].iter().filter_map(|t| Pattern::parse(t)).collect();
        assert_eq!(find_all_multi(&pats, &hay, 2), vec![vec![0, 1], vec![3, 4]]);
        assert_eq!(find_unique_multi(&pats, &hay), vec![Found::Ambiguous(2), Found::Ambiguous(2)]);
    }

    /// Nothing to scan, nothing to scan for, and no room for a hit: three ways
    /// in that must all leave with an empty answer of the right shape.
    #[test]
    fn degenerate_inputs_answer_empty() {
        let hay = [0xE8, 0x11];
        let pats: Vec<Pattern> =
            ["E8 11", "E8 11 22 33"].iter().filter_map(|t| Pattern::parse(t)).collect();
        assert!(find_all_multi(&[], &hay, 4).is_empty());
        assert!(find_unique_multi(&[], &hay).is_empty());
        assert_eq!(find_all_multi(&pats, &hay, 0), vec![Vec::new(), Vec::new()]);
        assert_eq!(find_all_multi(&pats, &[], 4), vec![Vec::new(), Vec::new()]);
        // Longer than the haystack: no match, and no arithmetic to get wrong.
        assert_eq!(find_all_multi(&pats, &hay, 4), vec![vec![0], Vec::new()]);
    }

    /// A haystack in the shape the real image has: one byte everywhere (`48`,
    /// the REX.W prefix, 24 M of them in 363 MB) and the interesting one rare.
    /// Long enough that the stride samples it hundreds of times.
    fn skewed_hay() -> Vec<u8> {
        let mut hay = vec![0x48u8; 4000];
        for at in [300usize, 2600] {
            for (i, b) in [0x48u8, 0xBA, 0x01].iter().enumerate() {
                if let Some(slot) = hay.get_mut(at + i) {
                    *slot = *b;
                }
            }
        }
        hay
    }

    /// The anchor the one-pass scan picks is the rarest literal, not the first,
    /// and picking it changes nothing about what is found.
    #[test]
    fn the_rarest_literal_anchors_not_the_first() {
        let hay = skewed_hay();
        let counts = sample_byte_counts(&hay);
        let p = Pattern::parse("48 BA 01").unwrap();
        assert_eq!(p.anchor(), Some((0, 0x48)));
        assert_eq!(p.rarest_anchor(&counts), Some((1, 0xBA)));
        assert!(counts[0xBA] < counts[0x48]);

        let pats = vec![p.clone(), Pattern::parse("48 48 48 48").unwrap()];
        let all = find_all_multi(&pats, &hay, 8);
        assert_eq!(all.first(), Some(&vec![300, 2600]));
        for (i, p) in pats.iter().enumerate() {
            assert_eq!(all.get(i), Some(&p.find_all(&hay, 8)));
        }
    }

    /// The rarest literal may be the last token, which makes `anchor_idx` as
    /// large as the pattern: the candidate start is still `i - anchor_idx` and
    /// `checked_sub` is what covers a match that would begin before the start.
    #[test]
    fn the_rarest_literal_may_be_the_last_token() {
        let hay = skewed_hay();
        let counts = sample_byte_counts(&hay);
        let p = Pattern::parse("48 48 ?? 01").unwrap();
        assert_eq!(p.rarest_anchor(&counts), Some((3, 0x01)));
        assert_eq!(find_all_multi(std::slice::from_ref(&p), &hay, 8), vec![p.find_all(&hay, 8)]);

        // The same anchor at the front of a haystack, where the match it would
        // start is off the left edge: the lane must drop that candidate, not
        // wrap around. Only `hay[0]` is sampled here, so `48` is the seen byte
        // and `01` is again the rarest.
        let short = [0x48u8, 0x01, 0x48, 0x48, 0x99, 0x01];
        let q = Pattern::parse("48 48 ?? 01").unwrap();
        assert_eq!(q.rarest_anchor(&sample_byte_counts(&short)), Some((3, 0x01)));
        let one = find_all_multi(std::slice::from_ref(&q), &short, 8);
        assert_eq!(one, vec![q.find_all(&short, 8)]);
        assert_eq!(q.find_all(&short, 8), vec![2]);
    }

    /// A haystack shorter than the sampling stride yields at most one sampled
    /// byte, so the ranking says nothing and the lowest-index literal — the old
    /// anchor — is what is used. Still correct, just not faster.
    #[test]
    fn a_haystack_shorter_than_the_stride_falls_back_to_the_first_literal() {
        let hay: Vec<u8> = (0..200u8).map(|b| b % 7).collect();
        assert!(hay.len() < SAMPLE_STRIDE);
        let counts = sample_byte_counts(&hay);
        assert_eq!(counts.iter().map(|&c| c as usize).sum::<usize>(), 1);

        let empty = sample_byte_counts(&[]);
        assert!(empty.iter().all(|&c| c == 0));

        for text in ["03 04 05", "?? 06 00 01", "05"] {
            let p = Pattern::parse(text).unwrap();
            assert_eq!(p.rarest_anchor(&counts), p.anchor(), "{text}");
            assert_eq!(p.rarest_anchor(&empty), p.anchor(), "{text}");
            let one = find_all_multi(std::slice::from_ref(&p), &hay, 9);
            assert_eq!(one, vec![p.find_all(&hay, 9)], "{text}");
        }
    }

    /// More patterns than there are lanes: the chunking is invisible from the
    /// outside, including the index alignment across the seam.
    #[test]
    fn more_patterns_than_lanes_stay_aligned() {
        let hay: Vec<u8> = (0..=255u8).collect();
        let pats: Vec<Pattern> =
            (0..=255u8).filter_map(|b| Pattern::parse(&format!("{b:02X}"))).collect();
        assert_eq!(pats.len(), 256);
        let all = find_all_multi(&pats, &hay, 4);
        assert_eq!(all.len(), 256);
        for (i, hits) in all.iter().enumerate() {
            assert_eq!(hits, &vec![i], "byte {i:02X}");
        }
    }
}
