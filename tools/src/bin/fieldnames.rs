//! Field-name inventory for the game's static-info record classes.
//!
//!     fieldnames                          # rebuild the inventory
//!     fieldnames GimmickInfo              # one class's fields
//!     fieldnames --field dropTagNameHash  # who has this field?
//!     fieldnames --grep drop              # fuzzy over class+field
//!     fieldnames --list                   # one line per class
//!     fieldnames --rescan                 # force the walk
//!
//! Output, gitignored and machine-generated:
//!     analysis/fieldnames.json    one entry per (class, field), with the file
//!                                 offset and the RVA of the error message that
//!                                 names it
//!
//! Where the names come from, because it is not obvious and nothing else in
//! `docs/` used to mention it. Every static-info record deserializer reports a
//! per-field read failure with a UTF-8 **Korean** message of the form
//!
//!     <ClassName>의 _<fieldName>를 읽어들이는데 실패했다.
//!     ("failed to read <Class>'s <field>")
//!
//! so the class name and the field name are sitting in the string pool of the
//! shipped exe, in plain sight, for every field of every record type. `strings`
//! misses them because they are not ASCII and Ghidra has not typed them. One
//! regex over the image recovers the lot:
//!
//!     [A-Za-z0-9_:\-]{2,80}\xec\x9d\x98 _[A-Za-z0-9_]{1,80}\xeb\xa5\xbc
//!
//! The two escapes are the only Korean this needs: `\xec\x9d\x98` is 의 (the
//! possessive particle) and `\xeb\xa5\xbc` is 를 (the object marker). Nothing
//! here decodes Korean beyond matching those two literals. It is also why the
//! scan runs over raw bytes with `regex::bytes` rather than `regex`: the exe is
//! 363 MB of mostly non-UTF-8 and will not decode as a `str` at all.
//!
//! **Scope: this tool answers "what are the fields called", not "where are the
//! fields".** Recovering a field's *offset* needs a second, mechanical step
//! that is deliberately not implemented here: find the RIP-relative
//! `48 8D 05 disp32` that loads the message (its RVA is in the output, and
//! `xrefs` takes an RVA), then read the offset out of the surrounding
//! `lea rdx,[rec+OFF]; mov rcx,rdi; call <reader>; test al,al; jne ok;
//! lea rax,[msg]` shape, where OFF sits 0x12-0x20 bytes above the message load.
//! That procedure is written down in
//! `docs/findings-water-wells-2026-09-12.md` section 7 and the method itself in
//! `docs/reference-internals.md` section 19.9; it is how that investigation's
//! `ItemInfo` offsets were obtained.
//!
//! What the names are and are not:
//!
//!   * The class names are the **logical** CamelCase names (`GimmickInfo`,
//!     `DropSetInfo`), not the lowercase table names the accessor census keys
//!     on (`gimmickinfo`, `dropsetinfo`). Section 19.2's inventory carries both.
//!   * 536 classes appear here against 149 static-info **tables**, because
//!     nested record types (`DropInfoData` inside `GimmickInfo`) get their own
//!     messages and are not tables of their own.
//!   * A name is evidence of a field the deserializer reads, nothing more. It
//!     says nothing about the field's type, width, order or offset, and read
//!     order is not string-pool order.
//!
//! Offline, no network, about a second. Run from the workspace; the output
//! lives with everything else the repo builds.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use desert_tools::{paths, pe::Pe};
use regex::bytes::Regex;
use serde::{Deserialize, Serialize};

/// 의 then a space then `_`, and 를 at the end. The full message is
/// "<Class>의 _<field>를 읽어들이는데 실패했다."; the trailing verb is not
/// matched, because a couple of variants word it differently and the two
/// particles are what actually delimit the two names.
const MSG_RE: &str = r"(?-u)([A-Za-z0-9_:\-]{2,80})\xec\x9d\x98 _([A-Za-z0-9_]{1,80})\xeb\xa5\xbc";

/// Anchors re-checked on every rebuild. The two counts are this walk's own
/// numbers on build 25246367; the four relations under them are the ones that
/// made the inventory worth keeping, and each is independently checkable.
const EXPECT_PAIRS: usize = 4675;
const EXPECT_CLASSES: usize = 536;

/// (description, class, fields that must be present)
const ANCHORS: &[(&str, &str, &[&str])] = &[
    (
        "the two GimmickInfo yield lists (sections 16, 19.6.1)",
        "GimmickInfo",
        &["dropInfoDataList", "dropSetInfoList"],
    ),
    (
        "DropInfoData is the output block the gatherer edits (section 16)",
        "DropInfoData",
        &["minValue", "maxValue", "keyRaw", "dropTagNameHash"],
    ),
    (
        "DropSetInfo is the reward table of section 20.22",
        "DropSetInfo",
        &["key", "dropRollType", "totalDropRate", "dropTagNameHash"],
    ),
];

/// Exactly two classes carry a drop tag hash, which is what makes section
/// 16.1's "are these dropsetinfo keys?" question a sharp one rather than a
/// vague one.
const TAG_HASH_CLASSES: &[&str] = &["DropInfoData", "DropSetInfo"];

/// One recovered `(class, field)`, with both addresses of the message.
///
/// `message_off` is a FILE offset and `message_rva` is where the loader puts
/// that byte. They are different numbers and `tools/README.md` warns about
/// exactly that confusion: `sigscan` prints offsets, `xrefs` takes an RVA.
/// Carrying both is what lets a row here be handed straight to `xrefs` with no
/// conversion step for anyone to get wrong.
#[derive(Serialize, Deserialize, Clone)]
struct Pair {
    class: String,
    field: String,
    message_off: u64,
    message_rva: Option<u32>,
}

/// The whole of `analysis/fieldnames.json`, in the key order it is written.
#[derive(Serialize)]
struct Doc<'a> {
    source: String,
    count: usize,
    classes: usize,
    method: &'a str,
    pairs: &'a [Pair],
}

/// What is read back. Only `pairs` is required, so an older file with different
/// bookkeeping keys still answers a lookup instead of forcing a re-walk.
#[derive(Deserialize)]
struct DocIn {
    pairs: Vec<Pair>,
}

#[derive(Parser)]
#[command(
    name = "fieldnames",
    about = "Field-name inventory for the game's static-info record classes"
)]
struct Args {
    /// Class name to look up (substring ok).
    name: Option<String>,
    /// Which classes carry this field name.
    #[arg(long)]
    field: Option<String>,
    /// Substring over both class and field names.
    #[arg(long)]
    grep: Option<String>,
    /// One line per class.
    #[arg(long)]
    list: bool,
    /// Re-walk the exe.
    #[arg(long)]
    rescan: bool,
    /// The exe to walk.
    #[arg(long)]
    exe: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let exe = args.exe.clone().unwrap_or_else(paths::game_exe);
    let json_out = paths::repo_root()?.join("analysis").join("fieldnames.json");

    if args.name.is_some() || args.field.is_some() || args.grep.is_some() || args.list {
        let pairs = load(&exe, &json_out, args.rescan)?;
        if args.list {
            show_list(&pairs);
        }
        if let Some(name) = &args.field {
            show_field(&pairs, name);
        }
        if let Some(needle) = &args.grep {
            show_grep(&pairs, needle);
        }
        if let Some(name) = &args.name {
            show_class(&pairs, name);
        }
        return Ok(());
    }
    rebuild(&exe, &json_out, false)?;
    Ok(())
}

/// Every (class, field) the deserializers name, in file order.
fn scan(exe: &Path) -> Result<Vec<Pair>> {
    let data = std::fs::read(exe).with_context(|| format!("cannot read {}", exe.display()))?;
    // Only the headers are needed to map offsets to RVAs, but the shared parser
    // wants the whole image and it is already in hand.
    let pe = Pe::parse(&data).with_context(|| format!("{} is not a PE", exe.display()))?;
    let re = Regex::new(MSG_RE).expect("the message regex is a literal");

    let mut pairs = Vec::with_capacity(EXPECT_PAIRS);
    for caps in re.captures_iter(&data) {
        let whole = caps.get(0).expect("group 0 always participates");
        let text = |i: usize| {
            String::from_utf8(
                caps.get(i)
                    .expect("both groups always participate")
                    .as_bytes()
                    .to_vec(),
            )
            .expect("the character classes admit ASCII only")
        };
        pairs.push(Pair {
            class: text(1),
            field: text(2),
            message_off: whole.start() as u64,
            message_rva: pe.offset_to_rva(whole.start()),
        });
    }
    Ok(pairs)
}

/// Pairs grouped by class, in the order each class first appears in the file.
///
/// First-appearance order, not sorted order: the string pool's layout is the
/// only evidence of grouping this tool has, and the lookups that want an
/// alphabetical answer sort for themselves. The map alongside is only so that
/// an anchor check does not scan 536 entries per class it asks about.
struct Groups<'a> {
    order: Vec<(&'a str, Vec<&'a Pair>)>,
    index: HashMap<&'a str, usize>,
}

impl<'a> Groups<'a> {
    fn len(&self) -> usize {
        self.order.len()
    }

    fn get(&self, class: &str) -> Option<&[&'a Pair]> {
        self.index.get(class).map(|&i| self.order[i].1.as_slice())
    }
}

fn by_class(pairs: &[Pair]) -> Groups<'_> {
    let mut g = Groups {
        order: Vec::new(),
        index: HashMap::new(),
    };
    for p in pairs {
        match g.index.get(p.class.as_str()) {
            Some(&i) => g.order[i].1.push(p),
            None => {
                g.index.insert(p.class.as_str(), g.order.len());
                g.order.push((p.class.as_str(), vec![p]));
            }
        }
    }
    g
}

fn check_line(ok: &mut bool, good: bool, text: &str) {
    *ok &= good;
    println!("  {}  {}", if good { "ok  " } else { "FAIL" }, text);
}

/// Print the self-checks. Any FAIL means the exe moved; explain it first.
///
/// These are the tool's whole trustworthiness story. The inventory is recovered
/// by regex from an undocumented string pool, so nothing downstream can tell a
/// good walk from a walk that silently found half the messages - except these
/// counts and relations, which were true when the method was validated and are
/// re-asserted on every rebuild.
fn checks(pairs: &[Pair]) -> bool {
    let groups = by_class(pairs);
    let mut ok = true;

    check_line(
        &mut ok,
        pairs.len() == EXPECT_PAIRS,
        &format!(
            "{} (class, field) pairs, expected {EXPECT_PAIRS}",
            pairs.len()
        ),
    );
    check_line(
        &mut ok,
        groups.len() == EXPECT_CLASSES,
        &format!(
            "{} distinct classes, expected {EXPECT_CLASSES}",
            groups.len()
        ),
    );
    // A class naming a field twice would mean the regex is matching something
    // that is not a field message, or that two classes share a name.
    let dupes: Vec<&str> = groups
        .order
        .iter()
        .filter(|(_, ps)| {
            let mut names: Vec<&str> = ps.iter().map(|p| p.field.as_str()).collect();
            names.sort_unstable();
            let before = names.len();
            names.dedup();
            names.len() != before
        })
        .map(|(c, _)| *c)
        .collect();
    check_line(
        &mut ok,
        dupes.is_empty(),
        &format!(
            "no class names a field twice{}",
            if dupes.is_empty() {
                String::new()
            } else {
                format!(
                    ": {}",
                    dupes.iter().take(5).copied().collect::<Vec<_>>().join(", ")
                )
            }
        ),
    );
    for (why, cls, want) in ANCHORS {
        let have: Vec<&str> = groups
            .get(cls)
            .map(|ps| ps.iter().map(|p| p.field.as_str()).collect())
            .unwrap_or_default();
        let missing: Vec<&str> = want.iter().copied().filter(|w| !have.contains(w)).collect();
        let mut text = format!("{cls}: {} - {why}", want.join(", "));
        if !missing.is_empty() {
            text.push_str(&format!("  MISSING {}", missing.join(", ")));
        }
        check_line(&mut ok, missing.is_empty(), &text);
    }
    let mut holders: Vec<&str> = groups
        .order
        .iter()
        .filter(|(_, ps)| ps.iter().any(|p| p.field == "dropTagNameHash"))
        .map(|(c, _)| *c)
        .collect();
    holders.sort_unstable();
    check_line(
        &mut ok,
        holders == TAG_HASH_CLASSES,
        &format!(
            "dropTagNameHash on exactly {} (got: {})",
            TAG_HASH_CLASSES.join(", "),
            if holders.is_empty() {
                "none".to_string()
            } else {
                holders.join(", ")
            }
        ),
    );
    // An offset with no RVA is a message outside every section's raw bytes,
    // which cannot happen for a string in the pool and would mean the section
    // walk disagrees with the scan.
    let unmapped = pairs.iter().filter(|p| p.message_rva.is_none()).count();
    check_line(
        &mut ok,
        unmapped == 0,
        &format!("every message offset maps to an RVA ({unmapped} did not)"),
    );
    ok
}

fn load(exe: &Path, json_out: &Path, rescan: bool) -> Result<Vec<Pair>> {
    if !rescan {
        if let Ok(text) = std::fs::read_to_string(json_out) {
            // A truncated or older file: rebuild rather than half-answer.
            if let Ok(doc) = serde_json::from_str::<DocIn>(&text) {
                return Ok(doc.pairs);
            }
        }
    }
    rebuild(exe, json_out, true)
}

fn rebuild(exe: &Path, json_out: &Path, quiet: bool) -> Result<Vec<Pair>> {
    if !exe.exists() {
        // Not an anyhow error: the Python printed this bare line and exited 1,
        // and the message is the whole of what a user needs.
        eprintln!("{} not found (pass --exe)", exe.display());
        std::process::exit(1);
    }
    let pairs = scan(exe)?;
    let groups = by_class(&pairs);
    if let Some(dir) = json_out.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    write_json(json_out, exe, &pairs, groups.len())?;
    if !quiet {
        let root = paths::repo_root()?;
        let rel = json_out.strip_prefix(&root).unwrap_or(json_out);
        println!(
            "{}\n  -> {}  {} pairs, {} classes\n",
            exe.display(),
            rel.display(),
            pairs.len(),
            groups.len()
        );
        println!("--- self-checks ---");
        if !checks(&pairs) {
            println!("\na FAIL means the exe's field messages moved. Do not trust anything");
            println!("downstream of this inventory until it is explained.");
        }
    }
    Ok(pairs)
}

/// Write the inventory with a ONE-space indent.
///
/// Not cosmetic: the file is 600 kB of four-key objects and the width is the
/// difference between a diffable artifact and one nobody opens. `serde_json`'s
/// pretty printer defaults to two, so the formatter is set explicitly.
fn write_json(json_out: &Path, exe: &Path, pairs: &[Pair], classes: usize) -> Result<()> {
    let doc = Doc {
        source: exe.display().to_string(),
        count: pairs.len(),
        classes,
        method: "docs/reference-internals.md section 19.9",
        pairs,
    };
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(
        &mut buf,
        serde_json::ser::PrettyFormatter::with_indent(b" "),
    );
    doc.serialize(&mut ser)
        .context("cannot encode the inventory")?;
    buf.push(b'\n');
    std::fs::write(json_out, buf)
        .with_context(|| format!("cannot write {}", json_out.display()))?;
    Ok(())
}

fn show_list(pairs: &[Pair]) {
    let mut order = by_class(pairs).order;
    // Fattest class first, ties alphabetical: the interesting records are the
    // ones with the most fields and they should not need scrolling to.
    order.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));
    for (c, ps) in order {
        println!("{:4}  {c}", ps.len());
    }
}

fn show_class(pairs: &[Pair], name: &str) {
    let groups = by_class(pairs);
    let needle = name.to_lowercase();
    // An exact name wins outright; only when nothing matches exactly does the
    // substring search run, so `GimmickInfo` does not also print
    // `GimmickInfoExtra` and bury the answer.
    let mut hits: Vec<&str> = groups
        .order
        .iter()
        .filter(|(c, _)| c.to_lowercase() == needle)
        .map(|(c, _)| *c)
        .collect();
    if hits.is_empty() {
        hits = groups
            .order
            .iter()
            .filter(|(c, _)| c.to_lowercase().contains(&needle))
            .map(|(c, _)| *c)
            .collect();
        hits.sort_unstable();
    }
    if hits.is_empty() {
        println!("no class matching '{name}'");
        return;
    }
    for c in hits.iter().take(20) {
        let ps = groups.get(c).expect("every hit came out of the groups");
        println!("\n{c}  ({} fields)", ps.len());
        let mut sorted: Vec<&&Pair> = ps.iter().collect();
        sorted.sort_by(|a, b| a.field.cmp(&b.field));
        for p in sorted {
            let rva = match p.message_rva {
                Some(r) => format!("  rva 0x{r:X}"),
                None => String::new(),
            };
            println!("  _{:<48} msg @ file 0x{:X}{rva}", p.field, p.message_off);
        }
    }
    if hits.len() > 20 {
        println!("\n... and {} more classes matched", hits.len() - 20);
    }
}

fn show_field(pairs: &[Pair], name: &str) {
    let needle = name.to_lowercase();
    let mut hits: Vec<&Pair> = pairs
        .iter()
        .filter(|p| p.field.to_lowercase() == needle)
        .collect();
    if hits.is_empty() {
        println!("no field named '{name}' (try --grep)");
        return;
    }
    let mut classes: Vec<&str> = hits.iter().map(|p| p.class.as_str()).collect();
    classes.sort_unstable();
    classes.dedup();
    println!("_{name} appears on {} class(es):", classes.len());
    hits.sort_by(|a, b| a.class.cmp(&b.class));
    for p in hits {
        println!("  {:<48} msg @ file 0x{:X}", p.class, p.message_off);
    }
}

fn show_grep(pairs: &[Pair], needle: &str) {
    let n = needle.to_lowercase();
    let mut hits: Vec<&Pair> = pairs
        .iter()
        .filter(|p| p.class.to_lowercase().contains(&n) || p.field.to_lowercase().contains(&n))
        .collect();
    hits.sort_by(|a, b| a.class.cmp(&b.class).then_with(|| a.field.cmp(&b.field)));
    for p in hits.iter().take(400) {
        println!("{}._{}", p.class, p.field);
    }
    println!(
        "{} match(es){}",
        hits.len(),
        if hits.len() > 400 {
            ", first 400 shown"
        } else {
            ""
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regex must find the two names in a real message and nothing else.
    #[test]
    fn the_message_regex_splits_a_real_message() {
        let msg = "GimmickInfo의 _dropInfoDataList를 읽어들이는데 실패했다.".as_bytes();
        let re = Regex::new(MSG_RE).unwrap();
        let caps = re.captures(msg).expect("the message should match");
        assert_eq!(&caps[1], b"GimmickInfo");
        assert_eq!(&caps[2], b"dropInfoDataList");
    }

    /// A one-character class name is below the {2,80} floor, and a bare
    /// particle with no leading name must not match at all.
    #[test]
    fn the_message_regex_rejects_near_misses() {
        let re = Regex::new(MSG_RE).unwrap();
        assert!(re.captures("A의 _x를".as_bytes()).is_none());
        assert!(re.captures("의 _x를".as_bytes()).is_none());
        // No `_` before the field name: that underscore is in the game's own
        // message and is what keeps ordinary Korean prose out of the results.
        assert!(re.captures("Info의 x를".as_bytes()).is_none());
    }

    #[test]
    fn grouping_keeps_first_appearance_order() {
        let mk = |c: &str, f: &str| Pair {
            class: c.into(),
            field: f.into(),
            message_off: 0,
            message_rva: None,
        };
        let pairs = vec![mk("B", "x"), mk("A", "y"), mk("B", "z")];
        let groups = by_class(&pairs);
        assert_eq!(
            groups.order.iter().map(|g| g.0).collect::<Vec<_>>(),
            ["B", "A"]
        );
        assert_eq!(groups.get("B").unwrap().len(), 2);
        assert!(groups.get("C").is_none());
    }

    /// The JSON is an artifact other tools read; its shape is part of the
    /// contract, one-space indent included.
    #[test]
    fn the_json_is_written_with_a_one_space_indent() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fieldnames.json");
        let pairs = vec![Pair {
            class: "DropSetInfo".into(),
            field: "dropTagNameHash".into(),
            message_off: 0x568_B150,
            message_rva: Some(0x568_BD50),
        }];
        write_json(&out, Path::new("/tmp/CrimsonDesert.exe"), &pairs, 1).unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(
            text.starts_with("{\n \"source\": \"/tmp/CrimsonDesert.exe\",\n"),
            "{text}"
        );
        assert!(
            text.contains("\n \"pairs\": [\n  {\n   \"class\": \"DropSetInfo\","),
            "{text}"
        );
        assert!(text.ends_with("}\n"), "{text}");
    }

    #[test]
    fn a_null_rva_round_trips_as_json_null() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fieldnames.json");
        let pairs = vec![Pair {
            class: "X".into(),
            field: "y".into(),
            message_off: 1,
            message_rva: None,
        }];
        write_json(&out, Path::new("/x"), &pairs, 1).unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.contains("\"message_rva\": null"), "{text}");
        let back: DocIn = serde_json::from_str(&text).unwrap();
        assert!(back.pairs[0].message_rva.is_none());
    }
}
