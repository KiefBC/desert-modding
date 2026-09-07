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
        self.bytes
            .iter()
            .zip(&hay[at..at + self.bytes.len()])
            .all(|(p, b)| p.is_none_or(|p| p == *b))
    }

    /// Every offset where the pattern matches, stopping after `limit` hits.
    pub fn find_all(&self, hay: &[u8], limit: usize) -> Vec<usize> {
        let mut hits = Vec::new();
        if hay.len() < self.bytes.len() || limit == 0 {
            return hits;
        }
        // Anchor on the first literal byte so the inner loop is a memchr-style
        // byte compare instead of a full pattern compare at every offset.
        let (anchor_idx, anchor) = self
            .bytes
            .iter()
            .enumerate()
            .find_map(|(i, b)| b.map(|b| (i, b)))
            .expect("pattern has a literal byte");
        let last_start = hay.len() - self.bytes.len();
        let mut i = anchor_idx;
        while i <= last_start + anchor_idx {
            match hay[i..].iter().position(|&b| b == anchor) {
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
        let hits = self.find_all(hay, 2);
        match hits.len() {
            0 => Found::None,
            1 => Found::Unique(hits[0]),
            n => Found::Ambiguous(n),
        }
    }
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
}
