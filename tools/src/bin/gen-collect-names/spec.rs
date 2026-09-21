//! Source 2: `tools/extra-families.json`, and the two-way money rule.
//!
//! FORMAT of tools/extra-families.json: a JSON object mapping family name ->
//! {note, records, records_not_enabled?, candidates_not_enabled?}, where
//!
//!   note                     free text: what the entry adds and how sure we
//!                            are. Emitted as a doc comment on the enum
//!                            variant.
//!   records                  {"<record key>": {name, items, why}} - the
//!                            SOURCE OF TRUTH: the gimmickinfo records this
//!                            entry adds, by key. `name` is the record's name,
//!                            `items` the item ids its resource-output blocks
//!                            carry (derived data, verified in `table`), `why`
//!                            the evidence that a table edit reaches the
//!                            player through this record.
//!   records_not_enabled      optional, same shape: records measured or argued
//!                            to be unreachable by a table edit and
//!                            deliberately NOT rows. Verified against the body
//!                            the same way, and emitted only as a test
//!                            asserting they stay out of the table, so the
//!                            trace is machine-checked rather than a paragraph
//!                            somebody has to find.
//!   candidates_not_enabled   optional {"<item id>": "why not"} - items
//!                            considered and deferred, recorded so the
//!                            decision is visible. Never emitted.
//!
//! Entries are record-keyed, not item-keyed, and that is a finding rather than
//! a convenience. The same item (water, 22008) comes from two records, and only
//! one of them - the well - ever reads its record's output block; the other,
//! the breakable pot, hands the player a pre-built item instance and the table
//! is never consulted (docs/findings/2026-09-12-water-wells.md section 11). So
//! which RECORD a row names is what decides whether the multiplier reaches
//! anything, and the item is only what it pays. An earlier item-keyed form of
//! this file expanded 22008 to both records and shipped a pot row that did
//! nothing.

use anyhow::{bail, Result};
use serde_json::Value;

/// Item `1` is money (coin_0001 10..15, silverbar_0001 2500..2500). Paid by no
/// row outside `MONEY_FAMILY`, and by every row inside it; see
/// [`check_money_rule`].
pub const MONEY_ITEM: u32 = 1;
pub const MONEY_FAMILY: &str = "Money";

#[derive(Debug)]
/// One record of one family: the name and `why` are quoted straight out of the
/// json, `items` is derived data the clean-body walk re-derives and checks.
pub struct Record {
    pub key: u32,
    pub name: String,
    pub items: Vec<u32>,
    #[allow(dead_code)] // Read by a human out of the json, never emitted.
    pub why: String,
}

#[derive(Debug)]
/// One `extra-families.json` entry, in the json's own order. Order is
/// load-bearing: it is the order the extra tests are emitted in, and
/// `serde_json`'s `preserve_order` feature is what keeps it.
pub struct FamilySpec {
    pub name: String,
    pub note: String,
    pub records: Vec<Record>,
    pub records_not_enabled: Vec<Record>,
    pub candidates_not_enabled: usize,
}

impl FamilySpec {
    /// The two record maps, paired with whether a record in them must own an
    /// output block. Both are verified against the clean body the same way.
    pub fn fields(&self) -> [(&'static str, &[Record], bool); 2] {
        [
            ("records", &self.records, true),
            ("records_not_enabled", &self.records_not_enabled, false),
        ]
    }
}

/// Python's `repr` of a string: the error messages here were written against
/// the python generator and are compared with it when this is changed, so they
/// quote the same way it did.
pub fn q(s: &str) -> String {
    format!("'{s}'")
}

/// Python's `repr` of a list of ints, for the same reason as [`q`].
pub fn py_ints(v: &[u32]) -> String {
    let inner: Vec<String> = v.iter().map(u32::to_string).collect();
    format!("[{}]", inner.join(", "))
}

/// Python's `repr` of a list of strings, for the same reason as [`q`].
pub fn py_strs(v: &[String]) -> String {
    let inner: Vec<String> = v.iter().map(|s| q(s)).collect();
    format!("[{}]", inner.join(", "))
}

/// The two-way money rule, applied to one record's item list.
///
/// A record paying item 1 inside `Foraging` or any other gathering family would
/// turn a yield slider into an economy lever by accident, so every family but
/// one refuses it outright. The one exception is `Money`, which exists for
/// exactly those records and has the opposite rule: every record in it, enabled
/// or not, must pay item 1 and NOTHING ELSE. So money can never ride into a
/// gathering family and a gathering item can never ride into Money; the two
/// refusals are each other's mirror.
///
/// Checked twice for every record: once on the `items` stored in the json and
/// again on what the clean body says the record pays. `source` names which of
/// the two the list came from ("stored" for the json, "the clean body" for the
/// walk) so the message says which one disagreed.
pub fn check_money_rule(
    location: &str,
    name: &str,
    family: &str,
    items: &[u32],
    source: &str,
) -> Result<()> {
    if family == MONEY_FAMILY {
        if items != [MONEY_ITEM] {
            bail!(
                "{location}: {} is a {MONEY_FAMILY} record but {source} says it \
                 pays {}; a {MONEY_FAMILY} record must pay item {MONEY_ITEM} \
                 (money) and nothing else",
                q(name),
                py_ints(items)
            );
        }
    } else if items.contains(&MONEY_ITEM) {
        bail!(
            "{location}: {} pays item {MONEY_ITEM} ({source}), which is MONEY \
             (coin_0001 10..15, silverbar_0001 2500..2500). Money never rides into \
             a gathering family; the {MONEY_FAMILY} family is the only place for it.",
            q(name)
        );
    }
    Ok(())
}

/// `{key: {name, items, why}}` from one of the two record maps, validated.
fn read_record_map(path: &str, family: &str, field: &str, raw: Option<&Value>) -> Result<Vec<Record>> {
    let raw = match raw {
        None => return Ok(Vec::new()),
        Some(Value::Object(map)) => map,
        Some(_) => bail!("{path}: {family}.{field} must be an object keyed by record key"),
    };
    let mut out = Vec::new();
    for (key_text, rec) in raw {
        let location = format!("{path}: {family}.{field}[{}]", q(key_text));
        let Ok(key) = key_text.parse::<i64>() else {
            bail!("{location}: the key is not an integer");
        };
        if key <= 0 {
            bail!("{location}: non-positive record key");
        }
        let Some(rec) = rec.as_object() else {
            bail!("{location}: must be {{name, items, why}}");
        };
        let name = match rec.get("name").and_then(Value::as_str) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => bail!("{location}: `name` must be the record's name"),
        };
        let items = rec
            .get("items")
            .and_then(Value::as_array)
            .and_then(|a| {
                a.iter()
                    .map(|v| v.as_u64().filter(|i| *i > 0).map(|i| i as u32))
                    .collect::<Option<Vec<u32>>>()
            })
            .filter(|v| !v.is_empty());
        let Some(mut items) = items else {
            bail!("{location}: `items` must be a non-empty list of item ids");
        };
        // Stored as a set, sorted: the walk derives its own the same way, and
        // `items != rec.items` below has to be a comparison of like with like.
        items.sort_unstable();
        items.dedup();
        check_money_rule(&location, &name, family, &items, "stored")?;
        let why = match rec.get("why") {
            None => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(_) => bail!("{location}: `why` must be text"),
        };
        out.push(Record { key: key as u32, name, items, why });
    }
    Ok(out)
}

/// The whole of `extra-families.json`, validated, in the json's order.
pub fn read_extras(path: &str, text: &str) -> Result<Vec<FamilySpec>> {
    let spec: Value = serde_json::from_str(text)?;
    let Some(spec) = spec.as_object() else {
        bail!("{path}: the top level must be an object mapping family name -> entry");
    };
    let variant = regex::Regex::new(r"^[A-Z][A-Za-z0-9]*$").expect("static regex");
    let mut out = Vec::new();
    for (family, body) in spec {
        if !variant.is_match(family) {
            bail!("{path}: {} is not a usable Rust enum variant name", q(family));
        }
        let Some(body) = body.as_object() else {
            bail!("{path}: {family} must be an object");
        };
        let records = read_record_map(path, family, "records", body.get("records"))?;
        let records_not_enabled =
            read_record_map(path, family, "records_not_enabled", body.get("records_not_enabled"))?;
        if records.is_empty() {
            bail!("{path}: {family} has no records; it is the source of truth");
        }
        let mut both: Vec<u32> = records
            .iter()
            .filter(|r| records_not_enabled.iter().any(|o| o.key == r.key))
            .map(|r| r.key)
            .collect();
        both.sort_unstable();
        both.dedup();
        if !both.is_empty() {
            bail!(
                "{path}: {family}: {} are in both records and records_not_enabled",
                py_ints(&both)
            );
        }
        // A name in both maps would make the "is present" and the "stays out"
        // test contradict each other, and only one of them would fail.
        let mut seen: Vec<&str> = Vec::new();
        for rec in records.iter().chain(records_not_enabled.iter()) {
            if seen.contains(&rec.name.as_str()) {
                bail!("{path}: {family}: record name {} appears twice", q(&rec.name));
            }
            seen.push(&rec.name);
        }
        let candidates = match body.get("candidates_not_enabled") {
            None => 0,
            Some(Value::Object(map)) => {
                for raw in map.keys() {
                    let Ok(item) = raw.parse::<i64>() else {
                        bail!("{path}: {family}.candidates_not_enabled[{}]: the key is not an item id", q(raw));
                    };
                    if item == i64::from(MONEY_ITEM) {
                        bail!("{path}: {family} lists item {MONEY_ITEM} as a candidate; never.");
                    }
                }
                map.len()
            }
            Some(_) => bail!("{path}: {family}.candidates_not_enabled must be an object"),
        };
        let note = match body.get("note") {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        out.push(FamilySpec {
            name: family.clone(),
            note,
            records,
            records_not_enabled,
            candidates_not_enabled: candidates,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_rule_refuses_money_in_a_gathering_family() {
        let err = check_money_rule("where", "bush", "Foraging", &[1, 22008], "stored")
            .unwrap_err()
            .to_string();
        assert!(err.contains("which is MONEY"), "{err}");
        assert!(check_money_rule("where", "bush", "Foraging", &[22008], "stored").is_ok());
    }

    #[test]
    fn money_rule_refuses_anything_but_money_in_money() {
        // The mirror half: a material riding into the economy lever.
        let err = check_money_rule("where", "coin", "Money", &[1, 22008], "the clean body")
            .unwrap_err()
            .to_string();
        assert!(err.contains("and nothing else"), "{err}");
        assert!(check_money_rule("where", "coin", "Money", &[22008], "stored").is_err());
        assert!(check_money_rule("where", "coin", "Money", &[1], "stored").is_ok());
    }

    #[test]
    fn a_family_needs_records() {
        let err = read_extras("p.json", r#"{"Foo": {"note": "n"}}"#).unwrap_err().to_string();
        assert!(err.contains("has no records"), "{err}");
    }

    #[test]
    fn a_family_name_must_be_a_variant() {
        let err = read_extras("p.json", r#"{"foo bar": {}}"#).unwrap_err().to_string();
        assert!(err.contains("not a usable Rust enum variant name"), "{err}");
    }

    #[test]
    fn a_record_cannot_be_in_both_maps() {
        let json = r#"{"Foo": {
            "records": {"7": {"name": "a", "items": [2], "why": ""}},
            "records_not_enabled": {"7": {"name": "b", "items": [2], "why": ""}}
        }}"#;
        let err = read_extras("p.json", json).unwrap_err().to_string();
        assert!(err.contains("are in both records and records_not_enabled"), "{err}");
    }

    #[test]
    fn items_are_stored_as_a_sorted_set() {
        let json = r#"{"Foo": {
            "records": {"7": {"name": "a", "items": [9, 2, 9], "why": "w"}}
        }}"#;
        let spec = read_extras("p.json", json).unwrap();
        assert_eq!(spec[0].records[0].items, vec![2, 9]);
        assert_eq!(spec[0].candidates_not_enabled, 0);
    }

    #[test]
    fn money_is_never_a_deferred_candidate() {
        let json = r#"{"Foo": {
            "records": {"7": {"name": "a", "items": [2], "why": ""}},
            "candidates_not_enabled": {"1": "no"}
        }}"#;
        let err = read_extras("p.json", json).unwrap_err().to_string();
        assert!(err.contains("as a candidate; never."), "{err}");
    }
}
