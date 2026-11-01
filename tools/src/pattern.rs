//! IDA-style byte patterns (`"48 8B ?? 20 01 00 00"`) and the matcher every
//! scanning tool here shares.
//!
//! This was `sigscan`'s private matcher until `retarget` needed the same one.
//! It moved with no change in behaviour: `sigscan`'s output is still produced
//! by [`Pattern::find_all`], and its tests moved here with it.
//!
//! Two match semantics, and which one a caller wants is a real choice:
//!
//! * [`Pattern::find_all`] is **non-overlapping**, left to right, the way
//!   Python's `re.finditer` reported matches. `sigscan` uses it, and its
//!   NUL-delimited name check depends on it.
//! * [`Pattern::find_all_overlapping`] reports every start, overlapping or
//!   not. `retarget triage` uses it, because "does this signature identify
//!   exactly one place" is the question the plugin asks at load, and the
//!   plugin's matcher overlaps: `desert-core/src/pattern.rs::find_all` resumes
//!   its search one byte past the anchor it just tested, not past the end of
//!   the match, so two overlapping copies are two hits to it. For a signature
//!   the two semantics give the same count in practice; for a uniqueness
//!   check the overlapping one is the one that cannot flatter.

use std::fmt;

use memchr::memmem;

/// One IDA-style signature: the bytes, plus which of them are wildcards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    pub bytes: Vec<u8>,
    /// `false` at a `??` position. Kept as a parallel vector rather than an
    /// `Option<u8>` per byte so the verify loop stays a pair of slice reads.
    pub fixed: Vec<bool>,
}

impl Pattern {
    /// Parse `"48 89 5C 24 ?? 4C"`. `??` and `?` are both a wildcard byte.
    /// Returns `None` for anything that is not a whole line of two-hex-digit
    /// tokens and wildcards, which is how comment and prose lines in
    /// `reference-signatures.txt` are skipped.
    pub fn parse(line: &str) -> Option<Pattern> {
        let mut bytes = Vec::new();
        let mut fixed = Vec::new();
        for tok in line.split(' ') {
            if tok == "??" || tok == "?" {
                bytes.push(0);
                fixed.push(false);
            } else if tok.len() == 2 && tok.bytes().all(|b| b.is_ascii_hexdigit()) {
                bytes.push(u8::from_str_radix(tok, 16).ok()?);
                fixed.push(true);
            } else {
                return None;
            }
        }
        // The Python's SIG_RE needs at least two tokens, so a bare `48` on a
        // line is prose, not a signature. Keep that: a one-byte "signature"
        // would hit millions of times and drown the report.
        (bytes.len() >= 2).then_some(Pattern { bytes, fixed })
    }

    /// A plain literal, for the name lists.
    pub fn literal(bytes: &[u8]) -> Pattern {
        Pattern { bytes: bytes.to_vec(), fixed: vec![true; bytes.len()] }
    }

    /// Bytes and a wildcard mask built elsewhere (the disassembler's
    /// auto-wildcarding). The two must be the same length.
    pub fn from_parts(bytes: Vec<u8>, fixed: Vec<bool>) -> Pattern {
        assert_eq!(bytes.len(), fixed.len(), "a pattern's mask must cover every byte");
        Pattern { bytes, fixed }
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// `true` when at least one byte is not a wildcard, i.e. the pattern can
    /// be anchored at all. An all-wildcard pattern "matches" everywhere, and
    /// both finders return nothing for it rather than every offset.
    pub fn has_fixed(&self) -> bool {
        self.fixed.iter().any(|&f| f)
    }

    /// The first `n` bytes (all of them if `n` is past the end).
    pub fn prefix(&self, n: usize) -> Pattern {
        let n = n.min(self.len());
        Pattern { bytes: self.bytes[..n].to_vec(), fixed: self.fixed[..n].to_vec() }
    }

    /// The last `n` bytes (all of them if `n` is past the end).
    pub fn suffix(&self, n: usize) -> Pattern {
        let at = self.len() - n.min(self.len());
        Pattern { bytes: self.bytes[at..].to_vec(), fixed: self.fixed[at..].to_vec() }
    }

    /// The longest run of non-wildcard bytes, as (start, bytes).
    ///
    /// This is the run handed to `memmem`, and picking the LONGEST one rather
    /// than the first matters: several of these signatures start `48 89 5C 24`,
    /// which occurs by the hundred thousand in 375 MB, while the six-byte run
    /// in their middle occurs a handful of times. Scanning for the first run
    /// works and is thirty times slower.
    pub fn anchor(&self) -> (usize, &[u8]) {
        let (mut best_at, mut best_len) = (0usize, 0usize);
        let mut i = 0;
        while i < self.fixed.len() {
            if !self.fixed[i] {
                i += 1;
                continue;
            }
            let start = i;
            while i < self.fixed.len() && self.fixed[i] {
                i += 1;
            }
            if i - start > best_len {
                best_at = start;
                best_len = i - start;
            }
        }
        (best_at, &self.bytes[best_at..best_at + best_len])
    }

    /// Whether the pattern matches `hay` starting at `at`.
    pub fn matches_at(&self, hay: &[u8], at: usize) -> bool {
        let Some(win) = at.checked_add(self.bytes.len()).and_then(|end| hay.get(at..end)) else {
            return false;
        };
        win.iter()
            .zip(self.bytes.iter().zip(self.fixed.iter()))
            .all(|(&h, (&b, &f))| !f || h == b)
    }

    /// Every offset the pattern matches at, NON-OVERLAPPING and left to right.
    ///
    /// Non-overlapping because the Python used `re.finditer`, which skips past
    /// a match before looking for the next one. For these signatures it makes
    /// no difference; for the NUL-delimited name check it very much does, and
    /// the two matchers share this function so the rule cannot drift apart
    /// between them.
    pub fn find_all(&self, hay: &[u8]) -> Vec<usize> {
        self.scan(hay, false, usize::MAX)
    }

    /// Every offset the pattern matches at, overlapping matches included.
    /// What a uniqueness check wants; see the module docs.
    pub fn find_all_overlapping(&self, hay: &[u8]) -> Vec<usize> {
        self.scan(hay, true, usize::MAX)
    }

    /// [`Pattern::find_all_overlapping`], stopping after `cap` hits. A short
    /// prefix of a signature can match millions of times; a search that only
    /// asks "is there one" or "how many, up to a point" should not collect
    /// them all.
    pub fn find_overlapping_capped(&self, hay: &[u8], cap: usize) -> Vec<usize> {
        self.scan(hay, true, cap)
    }

    fn scan(&self, hay: &[u8], overlapping: bool, cap: usize) -> Vec<usize> {
        let (anchor_at, anchor) = self.anchor();
        if anchor.is_empty() {
            // An all-wildcard pattern matches everywhere; the data files have
            // no such line and a scan of 375 MB of hits helps nobody.
            return Vec::new();
        }
        let finder = memmem::Finder::new(anchor);
        let mut out = Vec::new();
        let mut next = 0usize; // first offset a match is still allowed to start at
        let mut from = 0usize;
        while let Some(rel) = finder.find(&hay[from..]) {
            let pos = from + rel;
            // Step the anchor search by one, not by the anchor length: two
            // occurrences of the anchor can overlap even when two matches of
            // the whole pattern cannot, and `find_iter` would skip the second.
            from = pos + 1;
            let Some(start) = pos.checked_sub(anchor_at) else { continue };
            if start < next {
                continue;
            }
            if self.matches_at(hay, start) {
                out.push(start);
                if out.len() >= cap {
                    break;
                }
                if !overlapping {
                    next = start + self.bytes.len();
                }
            }
        }
        out
    }
}

/// `48 89 ?? 24`, the same spelling [`Pattern::parse`] reads.
impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (&b, &fx)) in self.bytes.iter().zip(self.fixed.iter()).enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            if fx {
                write!(f, "{b:02X}")?;
            } else {
                f.write_str("??")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ida_style_signatures_and_rejects_prose() {
        let p = Pattern::parse("48 89 ?? 24").unwrap();
        assert_eq!(p.bytes, vec![0x48, 0x89, 0x00, 0x24]);
        assert_eq!(p.fixed, vec![true, true, false, true]);
        assert_eq!(Pattern::parse("48 ? 24").unwrap().fixed, vec![true, false, true]);

        // Everything in reference-signatures.txt that is not a signature.
        assert!(Pattern::parse("alloc_event - function start").is_none());
        assert!(Pattern::parse("48").is_none(), "one token is not a signature");
        assert!(Pattern::parse("4 8 89").is_none(), "single hex digits are not bytes");
        assert!(Pattern::parse("48 8G").is_none());
        assert!(Pattern::parse("48  89").is_none(), "a double space is an empty token");
    }

    #[test]
    fn display_round_trips_through_parse() {
        let text = "48 89 ?? 24 0A FF";
        let p = Pattern::parse(text).unwrap();
        assert_eq!(p.to_string(), text);
        assert_eq!(Pattern::parse(&p.to_string()), Some(p));
    }

    #[test]
    fn prefix_and_suffix_cut_both_vectors() {
        let p = Pattern::parse("01 ?? 03 04").unwrap();
        assert_eq!(p.prefix(2).to_string(), "01 ??");
        assert_eq!(p.suffix(3).to_string(), "?? 03 04");
        assert_eq!(p.prefix(99), p);
        assert_eq!(p.suffix(99), p);
        assert!(p.prefix(0).is_empty());
        assert!(!Pattern::parse("?? ??").unwrap().has_fixed());
    }

    #[test]
    fn anchor_is_the_longest_literal_run_not_the_first() {
        let p = Pattern::parse("48 89 ?? 57 48 83 EC 20 ?? BA").unwrap();
        let (at, a) = p.anchor();
        assert_eq!(at, 3);
        assert_eq!(a, &[0x57, 0x48, 0x83, 0xEC, 0x20]);

        // Leading wildcards must not push the anchor offset negative when the
        // match start is computed back from an anchor hit.
        let p = Pattern::parse("?? ?? 90 90 90").unwrap();
        assert_eq!(p.anchor(), (2, &[0x90, 0x90, 0x90][..]));
        let hay = [0x90u8, 0x90, 0x90, 0x00, 0x11, 0x90, 0x90, 0x90];
        // The run at offset 0 has no room for the two wildcard bytes in front
        // of it, so it is NOT a match; the run at 5 is, starting at 3.
        assert_eq!(p.find_all(&hay), vec![3]);
    }

    #[test]
    fn wildcards_match_any_byte_and_fixed_bytes_do_not() {
        let p = Pattern::parse("AA ?? CC").unwrap();
        let hay = [0xAA, 0x00, 0xCC, 0xAA, 0xFF, 0xCC, 0xAA, 0xFF, 0xCD];
        assert_eq!(p.find_all(&hay), vec![0, 3]);
    }

    #[test]
    fn a_match_running_off_the_end_is_not_a_match() {
        // The off-by-one that hides: the anchor is present, the rest of the
        // pattern is past the end of the buffer.
        let p = Pattern::parse("AA BB CC DD").unwrap();
        assert_eq!(p.find_all(&[0xAA, 0xBB, 0xCC]), Vec::<usize>::new());
        assert_eq!(p.find_all(&[0xAA, 0xBB, 0xCC, 0xDD]), vec![0]);
        // ... and at the very last possible offset it IS a match.
        assert_eq!(p.find_all(&[0x00, 0xAA, 0xBB, 0xCC, 0xDD]), vec![1]);
    }

    #[test]
    fn matches_do_not_overlap() {
        // `re.finditer` semantics: after a match, the search resumes at its
        // end. `ABAB` contains `ABA` twice if overlaps count and once if not.
        let p = Pattern::parse("41 42 41").unwrap();
        assert_eq!(p.find_all(b"ABABA"), vec![0]);
        assert_eq!(p.find_all(b"ABABABABA"), vec![0, 4]);
    }

    #[test]
    fn the_overlapping_finder_reports_every_start() {
        let p = Pattern::parse("41 42 41").unwrap();
        assert_eq!(p.find_all_overlapping(b"ABABA"), vec![0, 2]);
        assert_eq!(p.find_all_overlapping(b"ABABABABA"), vec![0, 2, 4, 6]);
        // With nothing overlapping the two agree.
        assert_eq!(p.find_all_overlapping(b"ABAxABA"), p.find_all(b"ABAxABA"));
        assert_eq!(p.find_overlapping_capped(b"ABABABABA", 3), vec![0, 2, 4]);
        assert!(p.find_overlapping_capped(b"ABABABABA", 0).len() <= 1);
    }

    #[test]
    fn overlapping_anchor_runs_do_not_hide_a_match() {
        // The anchor `AA AA` occurs at offsets 3 AND 4 - overlapping - and only
        // the second one has the pattern's `01` two bytes in front of it.
        // Stepping the anchor search by the anchor's length would resume at 5,
        // never see the occurrence at 4, and report no match at all.
        let p = Pattern::parse("01 ?? AA AA").unwrap();
        assert_eq!(p.anchor(), (2, &[0xAA, 0xAA][..]));
        assert_eq!(p.find_all(&[0x00, 0x00, 0x01, 0xAA, 0xAA, 0xAA, 0x00]), vec![2]);
    }

    #[test]
    fn nul_delimited_is_not_the_same_as_substring() {
        // The `iteminfo` case from tools/README.md, in miniature: a string
        // table holding `trademarketiteminfo` and `iteminfo` back to back,
        // sharing the terminator between them.
        let table = b"\0trademarketiteminfo\0iteminfo\0";
        let substring = Pattern::literal(b"iteminfo").find_all(table);
        assert_eq!(substring.len(), 2, "substring finds it inside the longer name too");

        let mut delim = b"iteminfo".to_vec();
        delim.push(0);
        let nul: Vec<usize> = Pattern::literal(&delim)
            .find_all(table)
            .into_iter()
            .filter(|&s| s > 0 && table[s - 1] == 0)
            .collect();
        assert_eq!(nul.len(), 1, "NUL-delimited finds only the standalone name");
        assert_eq!(nul[0], 21);
    }

    #[test]
    fn a_name_at_offset_zero_is_not_counted() {
        // There is no leading NUL to test, and the check must not read
        // `data[-1]`. The Python guards with `x.start() > 0` for exactly this.
        let table = b"iteminfo\0";
        let mut delim = b"iteminfo".to_vec();
        delim.push(0);
        let nul: Vec<usize> = Pattern::literal(&delim)
            .find_all(table)
            .into_iter()
            .filter(|&s| s > 0 && table[s - 1] == 0)
            .collect();
        assert!(nul.is_empty());
    }
}
