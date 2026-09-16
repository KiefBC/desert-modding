//! Inferring an item's name from the names of the records that yield it.
//!
//! **Item names are inferred, never read.** The game's own item names live in
//! the `.paz` archives, which nothing extracts; what this does instead is read
//! the name of the record that yields the item - `gimmick_item_trade_salt_02`
//! -> `1000648` is salt. An inferred name is a hypothesis, and `confidence`
//! says how much of one.

use std::collections::{BTreeMap, HashMap, HashSet};

use regex::Regex;

/// Tokens that name the container, the prop kind or the variant rather than
/// the goods. Stripped before a record name is read as an item name.
const NOISE: &[&str] = &[
    "gimmick", "item", "items", "cd", "in", "ex", "dpf", "dpfo", "common",
    "basic", "trade", "dropset", "tableset", "equip", "backpack", "shops",
    "shop", "collect", "rare", "catched", "attach", "background", "bg",
    "breakable", "docking", "target", "core", "mine", "ore", "cave", "part",
    "parts", "index", "off", "on", "obj", "prop", "deco",
    "tool", "craft", "battle", "log",
    // bare container nouns and size adjectives: `docking_core_big` is not an
    // item called "big", and `..._egg_bucket_01` is an egg
    "box", "bucket", "basket", "sack", "crate", "barrel", "bag", "pack",
    "big", "small", "large", "mid", "middle",
];

/// Identities established by evidence the name inference cannot see. Each needs
/// a reason, and the reason is what makes it not a guess.
pub const CURATED: &[(u64, &str, &str)] = &[
    (
        22008,
        "water",
        "the yielding record carries GIMMICK_WATER_PICKUP and lives under \
         /well/; the other source is a water pot \
         (findings-water-wells-2026-09-12.md section 1)",
    ),
    (
        1,
        "money",
        "gimmick_item_common_coin_0001 gives 10..15 and \
         gimmick_item_common_silverbar_0001 gives 2500..2500 of the same id: \
         this is currency, not an item. The bag names it Money_Copper; silver \
         and gold are denominations of the one count, not items (there is no \
         silver bar in the world, whatever silverbar_0001 is named). A \
         currency multiplier is planned separately and this id must stay out \
         of any gather family",
    ),
    (
        53,
        "goldbar",
        "the bag names it GoldBar (every 2026-09-13 survey, `GoldBar key=53 \
         x1`). The name inference said itembox, because the only records \
         paying it are itembox_07, itembox_11 and itembox_Field_Space - chests \
         that contain a gold bar, so the inference named the container. Not to \
         be confused with 10021720, the fake/prop gold bar",
    ),
];

/// The confidence ladder, in the order the JSON and the doc table print it.
pub const CONFIDENCE_DOC: &[(&str, &str)] = &[
    ("curated", "identity established by evidence outside the name inference; see the note"),
    ("strong", "two or more independent records agree on the subject word, and they are the majority of this item's records"),
    ("single", "one record names it and nothing contradicts"),
    ("split", "two or more records agree, but more records of this item name something else - read the source records before trusting it"),
    ("weak", "one record names it and other records of this item disagree"),
    ("guess", "named only by a generic container, which legitimately holds varied goods"),
];

/// The regexes, compiled once. They are the honesty rules in miniature.
pub struct Res {
    /// A token that is itself the container (`mushroombasket`, `itembox`,
    /// `cerealstall`) names the furniture, not the goods. It is not dropped -
    /// for a few items it is all there is - but it never wins while a goods
    /// word is on offer.
    container_token: Regex,
    noise: Regex,
    /// A record whose name says "container": it legitimately holds varied
    /// goods, so an item named only by one of these is a guess however clean
    /// the token is.
    container: Regex,
}

impl Default for Res {
    fn default() -> Self {
        Res {
            container_token: Regex::new(r"basket|box|sack|stall|bucket|crate|barrel|shelf|stand")
                .unwrap(),
            noise: Regex::new(r"^(?:\d+|index\d*|parts?\d*|\d{2,}[a-z]?)$").unwrap(),
            container: Regex::new(
                r"(?i)dropset|tableset|basket|itembox|_box|box_|sack|stall|bucket|shops_|breakable|backpack|noticeboard|docking|warehouse",
            )
            .unwrap(),
        }
    }
}

/// A record name reduced to the tokens that might name the goods.
pub fn phrase_of(res: &Res, name: &str) -> Vec<String> {
    let lower = name.to_lowercase();
    let toks: Vec<String> = lower
        .split(['_', '-'])
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();
    let keep: Vec<String> = toks
        .iter()
        .filter(|t| !NOISE.contains(&t.as_str()) && !res.noise.is_match(t))
        .cloned()
        .collect();
    if keep.is_empty() {
        toks
    } else {
        keep
    }
}

/// `len(re.split(r"[_\-]+", n))` - the *undecorated* piece count, empties
/// included, so that a leading or doubled separator still counts. It is the
/// third rank key: the least decorated record name wins, which is what makes
/// the game's own bare `peony_01` beat
/// `gimmick_equip_gimmick_Ssari_Mushroom_backpack`.
fn split_pieces(n: &str) -> usize {
    let b = n.as_bytes();
    let mut count = 1;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'_' || b[i] == b'-' {
            count += 1;
            while i < b.len() && (b[i] == b'_' || b[i] == b'-') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    count
}

/// One item's inferred identity, before the dataset adds anything to it.
#[derive(Clone, Debug)]
pub struct Named {
    pub name: String,
    pub confidence: &'static str,
    pub named_by: String,
    pub agreeing: Vec<String>,
    pub conflicting: Vec<String>,
    pub note: Option<&'static str>,
}

/// Infer one name per item from the names of the records that yield them.
///
/// A token that shows up in the records of many *different* items names the
/// container, not the goods (`food` across a dozen `dropset_food_*` props), so
/// the winning record is the one whose most generic token is least generic -
/// plain inverse document frequency over items. Ties go to the shorter phrase,
/// then to the phrase more records share, which is what makes `stalactite`
/// (two records) beat `stalactites` (one).
pub fn infer_names(res: &Res, sources: &BTreeMap<u64, Vec<String>>) -> HashMap<u64, Named> {
    let mut df: HashMap<&str, usize> = HashMap::new();
    let mut phrase_cache: HashMap<&str, Vec<String>> = HashMap::new();
    for names in sources.values() {
        for n in names {
            phrase_cache
                .entry(n.as_str())
                .or_insert_with(|| phrase_of(res, n));
        }
    }
    for names in sources.values() {
        let mut seen: HashSet<&str> = HashSet::new();
        for n in names {
            for t in &phrase_cache[n.as_str()] {
                seen.insert(t.as_str());
            }
        }
        for t in seen {
            *df.entry(t).or_insert(0) += 1;
        }
    }
    // Worse than any real token can score, so a container word never wins
    // while a goods word is on offer.
    let generic = sources.len() + 1;
    let container_tokens: Vec<&str> = df
        .keys()
        .copied()
        .filter(|t| res.container_token.is_match(t))
        .collect();
    for t in container_tokens {
        df.insert(t, generic);
    }

    let mut out = HashMap::new();
    for (&item, srcs) in sources {
        let mut names: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        let mut share: HashMap<&str, usize> = HashMap::new();
        let joined: HashMap<&str, String> =
            names.iter().map(|n| (*n, phrase_cache[*n].join("_"))).collect();
        for n in &names {
            *share.entry(joined[*n].as_str()).or_insert(0) += 1;
        }

        // Least generic token first; then the shortest phrase; then the least
        // decorated record name, so the game's own bare `peony_01` beats
        // `gimmick_equip_gimmick_Ssari_Mushroom_backpack`; then the phrase more
        // records share, which is what makes `stalactite` (two records) beat
        // `stalactites` (one). The record name itself is the last key, so the
        // order is total and the winner never depends on iteration order.
        let rank = |n: &str| -> (usize, usize, i64, i64, usize, String) {
            let p = &phrase_cache[n];
            let j = joined[n].as_str();
            let dropped = split_pieces(n) as i64 - p.len() as i64;
            (
                p.iter()
                    .map(|t| *df.get(t.as_str()).unwrap_or(&0))
                    .max()
                    .unwrap_or(0),
                p.len(),
                dropped,
                -(share[j] as i64),
                j.len(),
                n.to_string(),
            )
        };

        let best: &str = names.iter().copied().min_by_key(|n| rank(n)).unwrap();
        let subject: HashSet<&str> = phrase_cache[best].iter().map(|s| s.as_str()).collect();
        let mut agreeing: Vec<String> = Vec::new();
        let mut conflicting: Vec<String> = Vec::new();
        for n in &names {
            if phrase_cache[*n].iter().any(|t| subject.contains(t.as_str())) {
                agreeing.push((*n).to_string());
            } else {
                conflicting.push((*n).to_string());
            }
        }
        let mut inferred = joined[best].clone();

        // Honesty rules, in order. A container that names the goods after
        // itself is a guess however many copies of it there are, and a subject
        // the minority of records agree on is not "strong" just because two of
        // them said it.
        let mut conf = if res.container.is_match(best) {
            "guess"
        } else if agreeing.len() >= 2 && conflicting.len() <= agreeing.len() {
            "strong"
        } else if agreeing.len() >= 2 {
            "split"
        } else if !conflicting.is_empty() {
            "weak"
        } else {
            "single"
        };
        let mut note = None;
        if let Some((_, n, why)) = CURATED.iter().find(|(i, _, _)| *i == item) {
            inferred = (*n).to_string();
            note = Some(*why);
            conf = "curated";
        }
        // `names` was already sorted, so `agreeing`/`conflicting` are too; the
        // Python sorts them again and so does this, to keep the port literal.
        agreeing.sort();
        conflicting.sort();
        out.insert(
            item,
            Named {
                name: inferred,
                confidence: conf,
                named_by: best.to_string(),
                agreeing,
                conflicting,
                note,
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(rows: &[(u64, &[&str])]) -> BTreeMap<u64, Vec<String>> {
        rows.iter()
            .map(|(id, names)| (*id, names.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    fn named(rows: &[(u64, &[&str])]) -> HashMap<u64, Named> {
        infer_names(&Res::default(), &sources(rows))
    }

    #[test]
    fn phrase_of_drops_the_container_and_variant_noise() {
        let res = Res::default();
        assert_eq!(phrase_of(&res, "gimmick_item_trade_salt_02"), vec!["salt"]);
        assert_eq!(phrase_of(&res, "peony_01"), vec!["peony"]);
        // Every token is noise, so the unfiltered tokens are all there is -
        // a name with nothing left is worse than a generic one.
        assert_eq!(phrase_of(&res, "gimmick_item_box"), vec!["gimmick", "item", "box"]);
    }

    /// The third rank key counts the *undecorated* pieces, empties included, so
    /// that the game's own bare `peony_01` beats a name with extra separators.
    #[test]
    fn split_pieces_counts_empty_pieces() {
        assert_eq!(split_pieces("a"), 1);
        assert_eq!(split_pieces("a_b"), 2);
        assert_eq!(split_pieces("a__b"), 2);
        assert_eq!(split_pieces("_a"), 2);
        assert_eq!(split_pieces("a_"), 2);
        assert_eq!(split_pieces("a-b_c"), 3);
    }

    #[test]
    fn one_record_and_nothing_contradicting_is_single() {
        let n = named(&[(1001, &["peony_01"])]);
        assert_eq!(n[&1001].name, "peony");
        assert_eq!(n[&1001].confidence, "single");
    }

    #[test]
    fn two_agreeing_records_in_the_majority_are_strong() {
        let n = named(&[(1001, &["salt_01", "salt_02"])]);
        assert_eq!(n[&1001].name, "salt");
        assert_eq!(n[&1001].confidence, "strong");
        assert_eq!(n[&1001].agreeing.len(), 2);
        assert!(n[&1001].conflicting.is_empty());
    }

    #[test]
    fn one_record_with_others_disagreeing_is_weak() {
        let n = named(&[(1001, &["salt_01", "amber_02", "copperwire_03"])]);
        assert_eq!(n[&1001].confidence, "weak");
        assert_eq!(n[&1001].conflicting.len(), 2);
    }

    /// Two records agreeing is not "strong" when more records of the same item
    /// name something else. The honesty rule exists because two agreeing
    /// sources out of five is a minority, not a consensus.
    #[test]
    fn a_minority_consensus_is_split_not_strong() {
        let n = named(&[(
            1001,
            &["salt_01", "salt_02", "amber_03", "quartz_04", "resin_05"],
        )]);
        assert_eq!(n[&1001].confidence, "split");
        assert_eq!(n[&1001].agreeing.len(), 2);
        assert_eq!(n[&1001].conflicting.len(), 3);
    }

    /// A container legitimately holds varied goods, so being named by one is a
    /// guess however many copies of it there are.
    #[test]
    fn a_container_name_is_always_a_guess() {
        let n = named(&[(1001, &["gimmick_item_dropset_food_01", "gimmick_item_dropset_food_02"])]);
        assert_eq!(n[&1001].confidence, "guess");
    }

    /// Curated ids carry an identity established outside the inference, and the
    /// reason is what makes it not a guess. Item 1 is money; nothing the name
    /// inference can see would ever say so.
    #[test]
    fn curated_ids_override_the_inference_and_keep_their_reason() {
        let n = named(&[(1, &["gimmick_item_common_coin_0001"]), (22008, &["gimmick_well_0001_parts01"])]);
        assert_eq!(n[&1].name, "money");
        assert_eq!(n[&1].confidence, "curated");
        assert!(n[&1].note.unwrap().contains("Money_Copper"));
        assert_eq!(n[&22008].name, "water");
        assert!(n[&22008].note.unwrap().contains("GIMMICK_WATER_PICKUP"));
    }

    /// Inverse document frequency over *items*, and it beats every other rank
    /// key. `hub` shows up in the records of four different items, so it names
    /// the container rather than the goods and the record carrying it loses -
    /// even though its phrase is the shorter one, which is the key idf has to
    /// outrank. Without this, every `dropset_food_*` item would be called
    /// "food".
    #[test]
    fn a_token_many_items_share_loses_however_short_its_phrase_is() {
        let n = named(&[
            (1001, &["hub_zeta", "quartz_vein_deep"]),
            (1002, &["hub_alpha"]),
            (1003, &["hub_beta"]),
            (1004, &["hub_gamma"]),
        ]);
        assert_eq!(n[&1001].named_by, "quartz_vein_deep");
        assert_eq!(n[&1001].name, "quartz_vein_deep");
    }

    /// Ties go to the least decorated record name, which is what makes the
    /// game's own bare `peony_01` win over the backpack prop.
    #[test]
    fn the_least_decorated_record_name_wins_a_tie() {
        let n = named(&[(757006, &["peony_01", "gimmick_equip_gimmick_Peony_backpack"])]);
        assert_eq!(n[&757006].named_by, "peony_01");
        assert_eq!(n[&757006].name, "peony");
    }
}
