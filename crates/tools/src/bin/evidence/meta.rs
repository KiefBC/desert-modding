//! The `<!-- evidence 1 {...} -->` line: the graph facts about one function,
//! carried in the file itself.
//!
//! This is what makes the export resumable. A second run reads the edges back
//! out of the files it already has instead of asking Ghidra again, which turns
//! a re-run after an interruption from twenty-five minutes into two seconds.
//! It is also why the addresses in it are hex strings: the line has to stay
//! greppable, and a decimal `5372401328` matches nothing anyone would search
//! for.

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{json, Value};

use super::parse::Caller;

fn meta_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)^<!-- evidence 1 (\{.*\}) -->$").unwrap())
}

/// One function's place in the call graph, as the metadata line records it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub va: u64,
    pub name: String,
    /// `None` only for a hand-edited or corrupt line: everything this tool
    /// writes carries a depth. Rendered as the literal `None` in `index.tsv`,
    /// which is what the Python's `str(m.get("depth", ""))` did with it.
    pub depth: Option<i64>,
    pub callees: Vec<u64>,
    pub tailcalls: Vec<u64>,
    pub callers: Vec<Caller>,
    pub callers_truncated: bool,
    pub oversized: u64,
}

impl Meta {
    /// The metadata line's JSON object, addresses as hex strings.
    pub fn to_json(&self) -> String {
        let v = json!({
            "va": format!("{:x}", self.va),
            "name": self.name,
            "depth": self.depth,
            "callees": self.callees.iter().map(|c| format!("{c:x}")).collect::<Vec<_>>(),
            "tailcalls": self.tailcalls.iter().map(|c| format!("{c:x}")).collect::<Vec<_>>(),
            "callers": self.callers.iter()
                .map(|c| json!([format!("{:x}", c.site), c.name, c.kind]))
                .collect::<Vec<_>>(),
            "callers_truncated": self.callers_truncated,
            "oversized": self.oversized,
        });
        ensure_ascii(&serde_json::to_string(&v).expect("a Meta always serialises"))
    }

    /// Inverse of `to_json`: hex strings back to integers for the walk.
    ///
    /// Every malformation answers `None`, exactly as the Python's blanket
    /// `except (JSONDecodeError, ValueError, TypeError, KeyError)` did, and
    /// `None` means two different useful things: the walk re-fetches the
    /// function, and the index rebuild leaves the file out. A half-written
    /// file therefore heals on the next run instead of poisoning the tree.
    pub fn from_json(v: &Value) -> Option<Meta> {
        let obj = v.as_object()?;
        Some(Meta {
            va: hex_field(obj.get("va")?)?,
            name: obj.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            depth: obj.get("depth").and_then(Value::as_i64),
            callees: hex_list(obj.get("callees"))?,
            tailcalls: hex_list(obj.get("tailcalls"))?,
            callers: match obj.get("callers") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) => a
                    .iter()
                    .map(|e| {
                        let t = e.as_array()?;
                        // Python unpacked `for site, n, k in ...`, so an entry
                        // of any other arity raised and lost the whole file.
                        if t.len() != 3 {
                            return None;
                        }
                        Some(Caller {
                            site: hex_field(&t[0])?,
                            name: t[1].as_str().unwrap_or("").to_string(),
                            kind: t[2].as_str().unwrap_or("").to_string(),
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
                Some(_) => return None,
            },
            callers_truncated: truthy(obj.get("callers_truncated")),
            oversized: obj.get("oversized").map_or(Some(0), |v| match v {
                Value::Null => Some(0),
                Value::Number(n) => n.as_u64(),
                _ => None,
            })?,
        })
    }
}

fn hex_field(v: &Value) -> Option<u64> {
    super::parse::py_hex(v.as_str()?)
}

fn hex_list(v: Option<&Value>) -> Option<Vec<u64>> {
    match v {
        None | Some(Value::Null) => Some(Vec::new()),
        Some(Value::Array(a)) => a.iter().map(hex_field).collect(),
        Some(_) => None,
    }
}

/// Python's `bool(x)` over a JSON value: 0, "", [], {} and null are all false.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// Read one file's metadata line, or `None` if it has none we can use.
pub fn read_meta(path: &Path) -> Option<Meta> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    // The Python looked at the first 65536 characters only. Keep the bound: it
    // is what stops a 2 MiB disassembly being regex-scanned once per file on
    // every index rebuild, and the line this is looking for is line 1.
    let head: String = text.chars().take(65536).collect();
    let caps = meta_re().captures(&head)?;
    Meta::from_json(&serde_json::from_str(&caps[1]).ok()?)
}

/// Python's `json.dumps` defaults to `ensure_ascii=True` and serde_json has no
/// such mode. Without this a Ghidra symbol carrying one non-ASCII character
/// makes the two implementations' files differ byte-for-byte - and worse, the
/// metadata line is read back by the next run, so the two would disagree about
/// what is already exported. Non-ASCII can only appear inside a JSON string,
/// so escaping the serialised text wholesale is safe.
pub fn ensure_ascii(s: &str) -> String {
    if s.is_ascii() {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            for unit in c.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}
