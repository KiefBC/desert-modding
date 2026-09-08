//! The schema a plugin writes so Desert Overlay can draw its settings.
//!
//! A **schema** is a small text file that says what a plugin's ini contains and
//! how the menu should draw it: one line per key, with its kind, its default,
//! its range and its label. The plugin **writes** it ([`write_beside`]) beside
//! its ini at every start, from a [`Section`] built in its own `config.rs`; the
//! overlay **reads** every such file it finds beside the game exe and renders
//! one collapsible section per file. Nothing else passes between them: the ini
//! stays the only channel the overlay writes, and the schema is a second,
//! read-only file in the same spirit. A new mod becomes visible in the menu by
//! dropping a schema file next to its ini - the overlay needs no change.
//!
//! **File naming.** For `DesertLooter.ini` the schema is
//! `DesertLooter.overlay.ini` in the same directory (the exe's directory,
//! `crate::log::exe_dir()`); the overlay discovers files by the
//! [`FILE_SUFFIX`] suffix, matched case-insensitively.
//!
//! **Format.** The workspace ini dialect ([`crate::ini`]): `Key=Value`, `;` and
//! `#` comments, `[Section]` headers, keys matched case-insensitively. One
//! `[overlay]` header section, then one `[<IniKey>]` section per field in
//! display order, then any `[preset:<Label>]` sections.
//!
//! **Forward compatibility.** The file is written by one program and read by
//! another, and the two are versioned and shipped separately, so the reader is
//! deliberately forgiving in one direction and strict in the other:
//!
//! - An **unknown key** inside a section is ignored, so a newer plugin may add
//!   decoration an older overlay never draws.
//! - An **unknown `Kind`**, or any field or preset that does not make sense
//!   (no `Default`, `Min` above `Max`, a choice default that is not one of the
//!   options...), is skipped with a warning; the rest of the section still
//!   draws. A field the overlay drops is simply not editable from the menu -
//!   the plugin still reads its own ini.
//! - A **`Schema` newer than [`VERSION`]** is a hard error and the whole file
//!   is skipped: the reader cannot know what the new format means, and drawing
//!   half of it would write ini values the plugin might not accept. A missing
//!   `[overlay]` section, `Title` or `Ini` is a hard error for the same reason.
//!
//! Everything here parses text a user can edit, so every path returns `Option`
//! or `Result`; nothing in this module can panic.

use std::path::Path;

use crate::ini::{self, Entry};

/// What the overlay looks for beside the game exe.
pub const FILE_SUFFIX: &str = ".overlay.ini";

/// The format version this crate writes and is the newest it can read.
pub const VERSION: u32 = 1;

/// `Order` when the header does not give one.
const DEFAULT_ORDER: i32 = 100;

/// What a field is and what it accepts, with its default. One variant per
/// `Kind=` spelling; the widget the overlay draws follows from it.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// A checkbox. Written as `1`/`0`.
    Bool { default: bool },
    /// A whole number in `min..=max`, drawn as an input (`step` per click) or
    /// as a slider.
    Int { default: i64, min: i64, max: i64, step: i64, slider: bool, format: Option<String> },
    /// A finite `f32` in `min..=max`, drawn as a slider.
    Float { default: f32, min: f32, max: f32, format: Option<String> },
    /// One of `options`, matched case-insensitively, written in the schema's
    /// own spelling.
    Choice { default: String, options: Vec<String> },
    /// A key name [`ini::vk_from_name`] accepts, written upper-case.
    Key { default: String },
}

/// One ini key, in display order.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The ini key. Matched case-insensitively everywhere.
    pub key: String,
    /// The widget label; defaults to the key.
    pub label: String,
    pub kind: Kind,
    /// Dim text drawn on its own line before this field.
    pub heading: Option<String>,
    /// Draw on the same row as the previous field.
    pub same_line: bool,
    /// Tooltip.
    pub help: Option<String>,
}

/// A button that writes several keys at once.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub label: String,
    pub hint: Option<String>,
    /// `(field key, value text)` in the field's written spelling.
    pub set: Vec<(String, String)>,
}

/// One plugin's menu section: the whole schema file, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: String,
    /// The ini file this describes: a bare file name, no path separators.
    pub ini: String,
    /// The `.asi` whose presence the overlay checks; `None` = always loaded.
    pub module: Option<String>,
    /// Sections sort by `(order, title)`.
    pub order: i32,
    /// Dim line drawn at the bottom of the section.
    pub notice: Option<String>,
    /// Dim line above the preset buttons.
    pub presets_label: Option<String>,
    pub presets: Vec<Preset>,
    pub fields: Vec<Field>,
}

/// What [`write_beside`] did, for the plugin's log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// The file was already byte-for-byte what we would have written.
    Unchanged,
    /// The file was created or replaced.
    Written,
    /// Nothing was written; the string says why.
    Failed(String),
}

impl Section {
    /// `DesertLooter.ini` -> `DesertLooter.overlay.ini`.
    pub fn schema_file_name(&self) -> String {
        let mut name = ini_stem(&self.ini).to_string();
        name.push_str(FILE_SUFFIX);
        name
    }

    /// The field with this ini key, matched case-insensitively.
    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.key.eq_ignore_ascii_case(key))
    }
}

impl Kind {
    /// The value text this kind would write for `text`, or `None` when `text`
    /// is not acceptable and the caller should keep the default.
    ///
    /// This is the plugins' own read rule, so that the menu shows the value the
    /// plugin is actually using: an int or float outside its range, a choice
    /// that is not an option and a key name nothing maps to are all rejected.
    /// `bool` is the one kind that accepts anything, because
    /// [`ini::parse_bool`] does: everything that is not a truthy spelling is
    /// `0`, in the menu exactly as in the plugin.
    pub fn normalize(&self, text: &str) -> Option<String> {
        let t = text.trim();
        match self {
            Kind::Bool { .. } => Some(flag(ini::parse_bool(t))),
            Kind::Int { min, max, .. } => {
                let v = t.parse::<i64>().ok()?;
                (*min..=*max).contains(&v).then(|| v.to_string())
            }
            Kind::Float { min, max, .. } => {
                let v = t.parse::<f32>().ok()?;
                (v.is_finite() && (*min..=*max).contains(&v)).then(|| num(v))
            }
            Kind::Choice { options, .. } => {
                options.iter().find(|o| o.eq_ignore_ascii_case(t)).cloned()
            }
            Kind::Key { .. } => ini::canonical_key_name(t),
        }
    }

    /// The default in its written spelling. Always accepted by
    /// [`normalize`](Kind::normalize): [`parse`] refuses a schema whose default
    /// is out of range or not one of the options.
    pub fn default_text(&self) -> String {
        match self {
            Kind::Bool { default } => flag(*default),
            Kind::Int { default, .. } => default.to_string(),
            Kind::Float { default, .. } => num(*default),
            Kind::Choice { default, .. } | Kind::Key { default } => default.clone(),
        }
    }

    /// The `Kind=` spelling.
    fn name(&self) -> &'static str {
        match self {
            Kind::Bool { .. } => "bool",
            Kind::Int { .. } => "int",
            Kind::Float { .. } => "float",
            Kind::Choice { .. } => "choice",
            Kind::Key { .. } => "key",
        }
    }
}

// ---------------------------------------------------------------------------
// Value spellings
// ---------------------------------------------------------------------------

/// `1` / `0`, the spelling the shipped ini templates use.
fn flag(b: bool) -> String {
    if b {
        "1".to_string()
    } else {
        "0".to_string()
    }
}

/// `40` rather than `40.0`, and `6.5` when it has to be - what the overlay has
/// always written into an ini. Formatting first and then dropping a `.0` (which
/// is the same as the old `fract() == 0.0` test for every value a widget can
/// produce) keeps it idempotent: `num(parse(num(x))) == num(x)`, which is what
/// lets a preset value survive a schema round trip.
fn num(f: f32) -> String {
    let s = format!("{f:.1}");
    match s.strip_suffix(".0") {
        Some(whole) => whole.to_string(),
        None => s,
    }
}

/// The full-precision spelling used for `Default`/`Min`/`Max` in the schema
/// file itself: `f32`'s `Display` is the shortest text that parses back to the
/// same value, so a rendered schema parses to an equal [`Section`]. (The ini
/// the player sees still gets [`num`].)
fn exact(f: f32) -> String {
    format!("{f}")
}

/// `DesertLooter.ini` -> `DesertLooter`; anything not ending in `.ini` is its
/// own stem.
fn ini_stem(ini: &str) -> &str {
    let cut = ini.len().saturating_sub(4);
    match (ini.get(cut..), ini.get(..cut)) {
        (Some(ext), Some(stem)) if ext.eq_ignore_ascii_case(".ini") && !stem.is_empty() => stem,
        _ => ini,
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// One `[name]` block with the pairs that followed it.
struct Block<'a> {
    name: &'a str,
    pairs: Vec<(&'a str, &'a str)>,
}

impl<'a> Block<'a> {
    /// The value of `key`, last occurrence wins, as the plugins' own parsers do.
    fn get(&self, key: &str) -> Option<&'a str> {
        self.pairs.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| *v)
    }

    /// [`Block::get`], but a present-and-empty value counts as absent.
    fn text(&self, key: &str) -> Option<&'a str> {
        self.get(key).map(str::trim).filter(|v| !v.is_empty())
    }

    /// [`Block::text`] as an owned optional field.
    fn opt(&self, key: &str) -> Option<String> {
        self.text(key).map(str::to_string)
    }

    /// [`Block::text`], or a "no `Key`" reason for the caller's warning.
    fn need(&self, key: &str) -> Result<&'a str, String> {
        self.text(key).ok_or_else(|| format!("no {key}"))
    }
}

/// Split the file into blocks. Pairs before the first header, and lines that
/// are neither a header nor a `key=value`, come back as warnings.
fn blocks(text: &str) -> (Vec<Block<'_>>, Vec<String>) {
    let mut out: Vec<Block<'_>> = Vec::new();
    let mut warnings = Vec::new();
    for entry in ini::entries(text) {
        match entry {
            Entry::Section(name) => out.push(Block { name: name.trim(), pairs: Vec::new() }),
            Entry::Pair(k, v) => match out.last_mut() {
                Some(block) => block.pairs.push((k, v)),
                None => warnings.push(format!("{k}={v} before any [section], ignored")),
            },
            Entry::Bad(why) => warnings.push(why),
        }
    }
    (out, warnings)
}

fn as_i64(v: &str, key: &str) -> Result<i64, String> {
    v.parse::<i64>().map_err(|_| format!("{key} {v:?} is not a whole number"))
}

fn as_f32(v: &str, key: &str) -> Result<f32, String> {
    match v.parse::<f32>() {
        Ok(f) if f.is_finite() => Ok(f),
        _ => Err(format!("{key} {v:?} is not a finite number")),
    }
}

/// The `Kind` a field block describes, or the reason the field is unusable.
fn parse_kind(b: &Block<'_>) -> Result<Kind, String> {
    let kind = b.need("Kind")?;
    match kind.to_ascii_lowercase().as_str() {
        "bool" => Ok(Kind::Bool { default: ini::parse_bool(b.need("Default")?) }),
        "int" => {
            let min = as_i64(b.need("Min")?, "Min")?;
            let max = as_i64(b.need("Max")?, "Max")?;
            if min > max {
                return Err(format!("Min {min} is above Max {max}"));
            }
            let step = match b.text("Step") {
                None => 1,
                Some(s) => match as_i64(s, "Step")? {
                    n if n < 1 => return Err(format!("Step {n} is below 1")),
                    n => n,
                },
            };
            let default = as_i64(b.need("Default")?, "Default")?;
            if !(min..=max).contains(&default) {
                return Err(format!("Default {default} is outside {min}..{max}"));
            }
            let slider = b.text("Widget").is_some_and(|w| w.eq_ignore_ascii_case("slider"));
            Ok(Kind::Int { default, min, max, step, slider, format: b.opt("Format") })
        }
        "float" => {
            let min = as_f32(b.need("Min")?, "Min")?;
            let max = as_f32(b.need("Max")?, "Max")?;
            if min > max {
                return Err(format!("Min {min} is above Max {max}"));
            }
            let default = as_f32(b.need("Default")?, "Default")?;
            if !(min..=max).contains(&default) {
                return Err(format!("Default {default} is outside {min}..{max}"));
            }
            Ok(Kind::Float { default, min, max, format: b.opt("Format") })
        }
        "choice" => {
            let options: Vec<String> = b
                .need("Options")?
                .split(';')
                .map(str::trim)
                .filter(|o| !o.is_empty())
                .map(str::to_string)
                .collect();
            if options.is_empty() {
                return Err("Options is empty".to_string());
            }
            let want = b.need("Default")?;
            let default = options
                .iter()
                .find(|o| o.eq_ignore_ascii_case(want))
                .cloned()
                .ok_or_else(|| format!("Default {want:?} is not one of the Options"))?;
            Ok(Kind::Choice { default, options })
        }
        "key" => {
            let want = b.need("Default")?;
            let default = ini::canonical_key_name(want)
                .ok_or_else(|| format!("Default {want:?} is not a key name"))?;
            Ok(Kind::Key { default })
        }
        other => Err(format!("unknown Kind {other:?}")),
    }
}

/// A whole field block, or the reason it is unusable.
fn parse_field(b: &Block<'_>) -> Result<Field, String> {
    let key = b.name;
    if key.is_empty() {
        return Err("empty section name".to_string());
    }
    // An ini key cannot contain these and stay readable: `=` ends the key, `;`
    // separates a preset's assignments, brackets open a section.
    if key.contains(['=', ';', '[', ']']) {
        return Err(format!("{key:?} is not a usable ini key"));
    }
    Ok(Field {
        kind: parse_kind(b)?,
        key: key.to_string(),
        label: b.opt("Label").unwrap_or_else(|| key.to_string()),
        heading: b.opt("Heading"),
        same_line: b.get("SameLine").is_some_and(ini::parse_bool),
        help: b.opt("Help"),
    })
}

/// A whole preset block, or the reason it is unusable. Needs the fields: a
/// preset that names a key this schema does not have, or a value that key would
/// not accept, is refused rather than written to the ini.
fn parse_preset(label: &str, b: &Block<'_>, fields: &[Field]) -> Result<Preset, String> {
    if label.is_empty() {
        return Err("empty preset label".to_string());
    }
    let mut set = Vec::new();
    for item in b.need("Set")?.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| format!("Set item {item:?} is not Key=Value"))?;
        let (key, value) = (key.trim(), value.trim());
        let field = fields
            .iter()
            .find(|f| f.key.eq_ignore_ascii_case(key))
            .ok_or_else(|| format!("Set names unknown field {key:?}"))?;
        let text = field
            .kind
            .normalize(value)
            .ok_or_else(|| format!("Set value {value:?} is not valid for {}", field.key))?;
        set.push((field.key.clone(), text));
    }
    if set.is_empty() {
        return Err("Set is empty".to_string());
    }
    Ok(Preset { label: label.to_string(), hint: b.opt("Hint"), set })
}

/// Parse schema text.
///
/// `Err` is an unusable file the caller should skip, the string saying why.
/// `Ok` carries the section and one warning per skipped field, skipped preset
/// or malformed line, for the caller's log.
pub fn parse(text: &str) -> Result<(Section, Vec<String>), String> {
    let (blocks, mut warnings) = blocks(text);

    let header = blocks
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case("overlay"))
        .ok_or_else(|| "no [overlay] section".to_string())?;

    let schema = header.need("Schema").map_err(|_| "[overlay]: no Schema".to_string())?;
    let schema = schema
        .parse::<u32>()
        .map_err(|_| format!("[overlay]: Schema {schema:?} is not a number"))?;
    if schema > VERSION {
        return Err(format!("[overlay]: Schema {schema} is newer than {VERSION}"));
    }

    let title = header.need("Title").map_err(|_| "[overlay]: no Title".to_string())?;
    let ini_name = header.need("Ini").map_err(|_| "[overlay]: no Ini".to_string())?;
    if ini_name.contains('/') || ini_name.contains('\\') || ini_name.contains("..") {
        return Err(format!("[overlay]: Ini {ini_name:?} must be a bare file name"));
    }

    let order = match header.text("Order") {
        None => DEFAULT_ORDER,
        Some(o) => o.parse::<i32>().unwrap_or_else(|_| {
            warnings.push(format!("[overlay]: Order {o:?} is not a number, using {DEFAULT_ORDER}"));
            DEFAULT_ORDER
        }),
    };

    // Fields first: a preset may name a field the file declares below it.
    let mut fields: Vec<Field> = Vec::new();
    let mut seen_header = false;
    let mut presets_todo: Vec<(&str, &Block<'_>)> = Vec::new();
    for b in &blocks {
        if b.name.eq_ignore_ascii_case("overlay") {
            if seen_header {
                warnings.push("[overlay]: repeated, the later one is ignored".to_string());
            }
            seen_header = true;
            continue;
        }
        if let Some(label) = strip_preset(b.name) {
            presets_todo.push((label, b));
            continue;
        }
        match parse_field(b) {
            Ok(f) => match fields.iter().find(|o| o.key.eq_ignore_ascii_case(&f.key)) {
                Some(_) => warnings.push(format!("[{}]: repeated key, skipped", f.key)),
                None => fields.push(f),
            },
            Err(why) => warnings.push(format!("[{}]: {why}, field skipped", b.name)),
        }
    }

    let mut presets = Vec::new();
    for (label, b) in presets_todo {
        match parse_preset(label, b, &fields) {
            Ok(p) => presets.push(p),
            Err(why) => warnings.push(format!("[{}]: {why}, preset skipped", b.name)),
        }
    }

    let section = Section {
        title: title.to_string(),
        ini: ini_name.to_string(),
        module: header.opt("Module"),
        order,
        notice: header.opt("Notice"),
        presets_label: header.opt("PresetsLabel"),
        presets,
        fields,
    };
    Ok((section, warnings))
}

/// `preset:Plants only` -> `Plants only`, case-insensitively on the prefix.
fn strip_preset(name: &str) -> Option<&str> {
    let (head, rest) = name.split_at_checked(7)?;
    head.eq_ignore_ascii_case("preset:").then(|| rest.trim())
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn kv(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push('=');
    out.push_str(value);
    out.push('\n');
}

/// Render a section as schema text. [`parse`] reads it back to an equal
/// `Section` (a tested property).
///
/// `banner` is a comment block put at the top so the file says who wrote it and
/// that editing it is pointless; give it one or more lines **without** a
/// leading `;`, which this adds.
pub fn render(section: &Section, banner: &str) -> String {
    let mut out = String::with_capacity(512);
    for line in banner.lines() {
        out.push_str("; ");
        out.push_str(line);
        out.push('\n');
    }

    out.push_str("[overlay]\n");
    kv(&mut out, "Schema", &VERSION.to_string());
    kv(&mut out, "Title", &section.title);
    kv(&mut out, "Ini", &section.ini);
    if let Some(m) = &section.module {
        kv(&mut out, "Module", m);
    }
    kv(&mut out, "Order", &section.order.to_string());
    if let Some(n) = &section.notice {
        kv(&mut out, "Notice", n);
    }
    if let Some(p) = &section.presets_label {
        kv(&mut out, "PresetsLabel", p);
    }

    for f in &section.fields {
        out.push_str("\n[");
        out.push_str(&f.key);
        out.push_str("]\n");
        kv(&mut out, "Kind", f.kind.name());
        if let Kind::Int { slider: true, .. } = f.kind {
            kv(&mut out, "Widget", "slider");
        }
        kv(&mut out, "Label", &f.label);
        if let Some(h) = &f.heading {
            kv(&mut out, "Heading", h);
        }
        if f.same_line {
            kv(&mut out, "SameLine", "1");
        }
        match &f.kind {
            Kind::Bool { default } => kv(&mut out, "Default", &flag(*default)),
            Kind::Int { default, min, max, step, format, .. } => {
                kv(&mut out, "Default", &default.to_string());
                kv(&mut out, "Min", &min.to_string());
                kv(&mut out, "Max", &max.to_string());
                if *step != 1 {
                    kv(&mut out, "Step", &step.to_string());
                }
                if let Some(fmt) = format {
                    kv(&mut out, "Format", fmt);
                }
            }
            Kind::Float { default, min, max, format } => {
                kv(&mut out, "Default", &exact(*default));
                kv(&mut out, "Min", &exact(*min));
                kv(&mut out, "Max", &exact(*max));
                if let Some(fmt) = format {
                    kv(&mut out, "Format", fmt);
                }
            }
            Kind::Choice { default, options } => {
                kv(&mut out, "Default", default);
                kv(&mut out, "Options", &options.join(";"));
            }
            Kind::Key { default } => kv(&mut out, "Default", default),
        }
        if let Some(h) = &f.help {
            kv(&mut out, "Help", h);
        }
    }

    for p in &section.presets {
        out.push_str("\n[preset:");
        out.push_str(&p.label);
        out.push_str("]\n");
        if let Some(h) = &p.hint {
            kv(&mut out, "Hint", h);
        }
        let set: Vec<String> = p.set.iter().map(|(k, v)| format!("{k}={v}")).collect();
        kv(&mut out, "Set", &set.join(";"));
    }
    out
}

/// The ini text the overlay writes when the ini file is missing: a short
/// comment header, `[<stem>]`, then every field at its default.
pub fn render_ini_defaults(section: &Section) -> String {
    let mut out = String::with_capacity(256);
    out.push_str("; ");
    out.push_str(&section.ini);
    out.push_str(" was missing, so Desert Overlay created it.\n");
    out.push_str("; Every key below is at the plugin's own default. Keys you add by hand are\n");
    out.push_str("; kept: the overlay only ever rewrites the lines it owns.\n\n");
    out.push('[');
    out.push_str(ini_stem(&section.ini));
    out.push_str("]\n");
    for f in &section.fields {
        kv(&mut out, &f.key, &f.kind.default_text());
    }
    out
}

/// Write `render(section, banner)` to `dir/<schema_file_name>` if the file is
/// missing or its content differs: `<name>.tmp` first, then a rename over the
/// target, so a reader never sees a half-written file. Never panics.
pub fn write_beside(dir: &Path, section: &Section, banner: &str) -> Written {
    let name = section.schema_file_name();
    let target = dir.join(&name);
    let text = render(section, banner);
    if std::fs::read_to_string(&target).is_ok_and(|old| old == text) {
        return Written::Unchanged;
    }
    let tmp = dir.join(format!("{name}.tmp"));
    if let Err(e) = std::fs::write(&tmp, text.as_bytes()) {
        return Written::Failed(format!("{}: {e}", tmp.display()));
    }
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Written::Failed(format!("{}: {e}", target.display()));
    }
    Written::Written
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A schema that uses every kind and every optional key.
    const SAMPLE: &str = "\
; written by the test
[overlay]
Schema=1
Title=Desert Looter
Ini=DesertLooter.ini
Module=DesertLooter.asi
Order=10
Notice=Takes effect on the next load.
PresetsLabel=Presets (what auto-loot picks up):

[Enabled]
Kind=bool
Label=Enabled
Default=1
Help=Master switch. 0 = idle.

[ScanRange]
Kind=float
Label=Scan range
Heading=Ranges:
Default=40
Min=1
Max=200
Format=%.0f m

[GatherInterval]
Kind=int
Label=Gather interval (ms)
SameLine=1
Default=500
Min=100
Max=60000
Step=50

[Foraging]
Kind=int
Widget=slider
Label=Foraging
Default=1
Min=1
Max=100
Format=%dx

[Theme]
Kind=choice
Label=Theme
Default=BANNER
Options=classic;parchment;banner

[KeyToggle]
Kind=key
Label=Toggle auto gather
Default=f10

[preset:Plants only]
Hint=Foraging only: plants, fruit, berries.
Set=Enabled=1;Foraging=4
";

    /// The header of `SAMPLE`, for tests that vary one field block.
    const HEAD: &str = "[overlay]\nSchema=1\nTitle=T\nIni=T.ini\n\n";

    fn ok(text: &str) -> (Section, Vec<String>) {
        parse(text).expect("the sample parses")
    }

    /// `HEAD` plus one block, parsed.
    fn one(block: &str) -> (Section, Vec<String>) {
        ok(&format!("{HEAD}{block}"))
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("desert-core-schema-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    // -----------------------------------------------------------------------
    // A whole file
    // -----------------------------------------------------------------------

    #[test]
    fn parses_every_kind_and_option() {
        let (s, warnings) = ok(SAMPLE);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(s.title, "Desert Looter");
        assert_eq!(s.ini, "DesertLooter.ini");
        assert_eq!(s.module.as_deref(), Some("DesertLooter.asi"));
        assert_eq!(s.order, 10);
        assert_eq!(s.notice.as_deref(), Some("Takes effect on the next load."));
        assert_eq!(s.presets_label.as_deref(), Some("Presets (what auto-loot picks up):"));
        assert_eq!(s.fields.len(), 6);

        let f = &s.fields[0];
        assert_eq!((f.key.as_str(), f.label.as_str()), ("Enabled", "Enabled"));
        assert_eq!(f.kind, Kind::Bool { default: true });
        assert_eq!(f.help.as_deref(), Some("Master switch. 0 = idle."));
        assert!(!f.same_line && f.heading.is_none());

        let f = &s.fields[1];
        assert_eq!(f.heading.as_deref(), Some("Ranges:"));
        assert_eq!(
            f.kind,
            Kind::Float { default: 40.0, min: 1.0, max: 200.0, format: Some("%.0f m".into()) }
        );

        let f = &s.fields[2];
        assert!(f.same_line);
        assert_eq!(
            f.kind,
            Kind::Int {
                default: 500,
                min: 100,
                max: 60_000,
                step: 50,
                slider: false,
                format: None
            }
        );

        assert!(matches!(s.fields[3].kind, Kind::Int { slider: true, step: 1, .. }));
        assert_eq!(
            s.fields[4].kind,
            Kind::Choice {
                // the schema's own spelling wins over the Default's
                default: "banner".to_string(),
                options: vec!["classic".into(), "parchment".into(), "banner".into()],
            },
        );
        assert_eq!(s.fields[5].kind, Kind::Key { default: "F10".to_string() });

        assert_eq!(s.presets.len(), 1);
        assert_eq!(s.presets[0].label, "Plants only");
        assert_eq!(s.presets[0].hint.as_deref(), Some("Foraging only: plants, fruit, berries."));
        assert_eq!(
            s.presets[0].set,
            vec![("Enabled".to_string(), "1".to_string()), ("Foraging".into(), "4".into())]
        );
    }

    /// The round trip the plugin/overlay pair depends on: what `render` writes,
    /// `parse` reads back unchanged.
    #[test]
    fn render_round_trips() {
        let (s, _) = ok(SAMPLE);
        let text = render(&s, "line one\nline two");
        assert!(text.starts_with("; line one\n; line two\n[overlay]\n"), "{text}");
        let (again, warnings) = ok(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again, s);
    }

    /// The same, for a section with none of the optional keys.
    #[test]
    fn render_round_trips_without_the_optional_keys() {
        let (mut s, _) = ok(SAMPLE);
        s.module = None;
        s.notice = None;
        s.presets_label = None;
        s.presets.clear();
        for f in &mut s.fields {
            f.heading = None;
            f.help = None;
            f.same_line = false;
        }
        let (again, warnings) = ok(&render(&s, ""));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again, s);
    }

    #[test]
    fn field_lookup_is_case_insensitive() {
        let (s, _) = ok(SAMPLE);
        assert_eq!(s.field("scanrange").map(|f| f.key.as_str()), Some("ScanRange"));
        assert_eq!(s.field("SCANRANGE").map(|f| f.key.as_str()), Some("ScanRange"));
        assert!(s.field("nope").is_none());
    }

    #[test]
    fn schema_file_names() {
        let mut s = ok(SAMPLE).0;
        assert_eq!(s.schema_file_name(), "DesertLooter.overlay.ini");
        s.ini = "DesertGatherer.INI".to_string();
        assert_eq!(s.schema_file_name(), "DesertGatherer.overlay.ini");
        s.ini = "noext".to_string();
        assert_eq!(s.schema_file_name(), "noext.overlay.ini");
        s.ini = ".ini".to_string();
        assert_eq!(s.schema_file_name(), ".ini.overlay.ini", "an empty stem is left alone");
    }

    // -----------------------------------------------------------------------
    // Hard errors: the whole file is unusable
    // -----------------------------------------------------------------------

    #[test]
    fn rejects_unusable_files() {
        let cases = [
            ("", "no [overlay] section"),
            ("[Enabled]\nKind=bool\nDefault=1\n", "no [overlay] section"),
            ("[overlay]\nTitle=T\nIni=T.ini\n", "no Schema"),
            ("[overlay]\nSchema=\nTitle=T\nIni=T.ini\n", "no Schema"),
            ("[overlay]\nSchema=x\nTitle=T\nIni=T.ini\n", "is not a number"),
            ("[overlay]\nSchema=-1\nTitle=T\nIni=T.ini\n", "is not a number"),
            ("[overlay]\nSchema=2\nTitle=T\nIni=T.ini\n", "newer than 1"),
            ("[overlay]\nSchema=1\nIni=T.ini\n", "no Title"),
            ("[overlay]\nSchema=1\nTitle=\nIni=T.ini\n", "no Title"),
            ("[overlay]\nSchema=1\nTitle=T\n", "no Ini"),
            ("[overlay]\nSchema=1\nTitle=T\nIni=\n", "no Ini"),
            ("[overlay]\nSchema=1\nTitle=T\nIni=sub/T.ini\n", "bare file name"),
            ("[overlay]\nSchema=1\nTitle=T\nIni=sub\\T.ini\n", "bare file name"),
            ("[overlay]\nSchema=1\nTitle=T\nIni=../T.ini\n", "bare file name"),
        ];
        for (text, want) in cases {
            let got = parse(text).map(|(s, _)| s.title).unwrap_err();
            assert!(got.contains(want), "{text:?}: {got:?} does not mention {want:?}");
        }
    }

    #[test]
    fn schema_at_the_version_is_accepted() {
        let text = format!("[overlay]\nSchema={VERSION}\nTitle=T\nIni=T.ini\n");
        let (s, warnings) = ok(&text);
        assert_eq!((s.order, s.fields.len()), (DEFAULT_ORDER, 0));
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    // -----------------------------------------------------------------------
    // Warnings: one field or preset is skipped, the file still loads
    // -----------------------------------------------------------------------

    #[test]
    fn skips_unusable_fields_with_a_warning() {
        let cases = [
            ("[A]\nDefault=1\n", "no Kind"),
            ("[A]\nKind=\nDefault=1\n", "no Kind"),
            ("[A]\nKind=colour\nDefault=1\n", "unknown Kind \"colour\""),
            ("[A]\nKind=bool\n", "no Default"),
            ("[A]\nKind=int\nDefault=1\nMax=9\n", "no Min"),
            ("[A]\nKind=int\nDefault=1\nMin=0\n", "no Max"),
            ("[A]\nKind=int\nDefault=x\nMin=0\nMax=9\n", "Default \"x\" is not a whole number"),
            ("[A]\nKind=int\nDefault=1\nMin=9\nMax=0\n", "Min 9 is above Max 0"),
            ("[A]\nKind=int\nDefault=1\nMin=0\nMax=9\nStep=0\n", "Step 0 is below 1"),
            ("[A]\nKind=int\nDefault=1\nMin=0\nMax=9\nStep=x\n", "Step \"x\" is not a whole"),
            ("[A]\nKind=int\nDefault=99\nMin=0\nMax=9\n", "Default 99 is outside 0..9"),
            ("[A]\nKind=float\nDefault=x\nMin=0\nMax=9\n", "Default \"x\" is not a finite"),
            ("[A]\nKind=float\nDefault=inf\nMin=0\nMax=9\n", "not a finite"),
            ("[A]\nKind=float\nDefault=99\nMin=0\nMax=9\n", "is outside"),
            ("[A]\nKind=float\nDefault=1\nMax=9\n", "no Min"),
            ("[A]\nKind=choice\nDefault=a\n", "no Options"),
            ("[A]\nKind=choice\nDefault=a\nOptions=;;\n", "Options is empty"),
            ("[A]\nKind=choice\nDefault=z\nOptions=a;b\n", "is not one of the Options"),
            ("[A]\nKind=key\nDefault=nope\n", "is not a key name"),
            ("[A=B]\nKind=bool\nDefault=1\n", "is not a usable ini key"),
            ("[]\nKind=bool\nDefault=1\n", "empty section name"),
        ];
        for (block, want) in cases {
            let (s, warnings) = one(block);
            assert!(s.fields.is_empty(), "{block:?} produced a field");
            assert_eq!(warnings.len(), 1, "{block:?}: {warnings:?}");
            let got = warnings.first().map(String::as_str).unwrap_or_default();
            assert!(got.contains(want), "{block:?}: {got:?} does not mention {want:?}");
            assert!(got.ends_with("field skipped"), "{got:?}");
        }
    }

    #[test]
    fn skips_a_repeated_key_with_a_warning() {
        let (s, warnings) = one("[A]\nKind=bool\nDefault=1\n\n[a]\nKind=bool\nDefault=0\n");
        assert_eq!(s.fields.len(), 1);
        assert_eq!(s.fields[0].kind, Kind::Bool { default: true }, "the first one wins");
        assert_eq!(warnings, vec!["[a]: repeated key, skipped".to_string()]);
    }

    #[test]
    fn skips_unusable_presets_with_a_warning() {
        let field = "[A]\nKind=int\nDefault=1\nMin=0\nMax=9\n\n";
        let cases = [
            ("[preset:P]\nHint=h\n", "no Set"),
            ("[preset:P]\nSet=;;\n", "Set is empty"),
            ("[preset:P]\nSet=A\n", "is not Key=Value"),
            ("[preset:P]\nSet=Z=1\n", "Set names unknown field \"Z\""),
            ("[preset:P]\nSet=A=99\n", "Set value \"99\" is not valid for A"),
            ("[preset:P]\nSet=A=x\n", "is not valid for A"),
            ("[preset:]\nSet=A=1\n", "empty preset label"),
        ];
        for (block, want) in cases {
            let (s, warnings) = one(&format!("{field}{block}"));
            assert_eq!(s.fields.len(), 1);
            assert!(s.presets.is_empty(), "{block:?} produced a preset");
            let got = warnings.first().map(String::as_str).unwrap_or_default();
            assert!(got.contains(want), "{block:?}: {got:?} does not mention {want:?}");
            assert!(got.ends_with("preset skipped"), "{got:?}");
        }
    }

    /// A preset may name a field the file declares after it.
    #[test]
    fn presets_see_every_field_whatever_the_order() {
        let (s, warnings) =
            one("[preset:P]\nSet=A=on\n\n[A]\nKind=bool\nDefault=0\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(s.presets[0].set, vec![("A".to_string(), "1".to_string())]);
    }

    #[test]
    fn ignores_unknown_keys_and_reports_junk_lines() {
        let (s, warnings) = one("[A]\nKind=bool\nDefault=1\nFuture=whatever\nJunk\n");
        assert_eq!(s.fields.len(), 1, "an unknown key does not skip the field");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no '='"), "{warnings:?}");
    }

    #[test]
    fn a_bad_order_warns_and_falls_back() {
        let (s, warnings) = ok("[overlay]\nSchema=1\nTitle=T\nIni=T.ini\nOrder=soon\n");
        assert_eq!(s.order, DEFAULT_ORDER);
        assert!(warnings[0].contains("Order \"soon\""), "{warnings:?}");
    }

    #[test]
    fn a_repeated_header_warns() {
        let (s, warnings) =
            ok("[overlay]\nSchema=1\nTitle=T\nIni=T.ini\n[overlay]\nTitle=Other\n");
        assert_eq!(s.title, "T");
        assert_eq!(warnings, vec!["[overlay]: repeated, the later one is ignored".to_string()]);
    }

    // -----------------------------------------------------------------------
    // Value spellings
    // -----------------------------------------------------------------------

    #[test]
    fn bool_values() {
        let k = Kind::Bool { default: true };
        assert_eq!(k.default_text(), "1");
        assert_eq!(Kind::Bool { default: false }.default_text(), "0");
        for truthy in ["1", "true", "yes", "on", " on "] {
            assert_eq!(k.normalize(truthy).as_deref(), Some("1"), "{truthy:?}");
        }
        // `parse_bool` is the plugins' rule and it is case-sensitive: anything
        // that is not a truthy spelling is `0`, in the menu as in the plugin.
        for falsy in ["0", "no", "TRUE", "", "banana"] {
            assert_eq!(k.normalize(falsy).as_deref(), Some("0"), "{falsy:?}");
        }
    }

    #[test]
    fn int_values() {
        let k = Kind::Int { default: 500, min: 100, max: 60_000, step: 50, slider: false, format: None };
        assert_eq!(k.default_text(), "500");
        assert_eq!(k.normalize(" 120 ").as_deref(), Some("120"));
        assert_eq!(k.normalize("+120").as_deref(), Some("120"));
        assert_eq!(k.normalize("99"), None, "below Min");
        assert_eq!(k.normalize("60001"), None, "above Max");
        assert_eq!(k.normalize("12.0"), None);
        assert_eq!(k.normalize(""), None);
    }

    #[test]
    fn float_values() {
        let k = Kind::Float { default: 40.0, min: 1.0, max: 200.0, format: None };
        assert_eq!(k.default_text(), "40", "no pointless decimal");
        let half = Kind::Float { default: 6.5, min: 1.0, max: 200.0, format: None };
        assert_eq!(half.default_text(), "6.5");
        assert_eq!(k.normalize("6.5").as_deref(), Some("6.5"));
        assert_eq!(k.normalize("40.0").as_deref(), Some("40"));
        assert_eq!(k.normalize("1e2").as_deref(), Some("100"));
        assert_eq!(k.normalize("0.5"), None, "below Min");
        assert_eq!(k.normalize("inf"), None);
        assert_eq!(k.normalize("NaN"), None);
        assert_eq!(k.normalize("x"), None);
    }

    /// A written value read back and written again is the same text; a preset
    /// value stored in a schema depends on it.
    #[test]
    fn float_spelling_is_idempotent() {
        let k = Kind::Float { default: 0.0, min: -1e30, max: 1e30, format: None };
        for text in ["0", "40", "6.5", "6.25", "1e-30", "-0.04", "1e20", "0.0"] {
            let once = k.normalize(text).unwrap();
            assert_eq!(k.normalize(&once).as_deref(), Some(once.as_str()), "{text:?}");
        }
    }

    #[test]
    fn choice_values() {
        let k = Kind::Choice {
            default: "banner".to_string(),
            options: vec!["classic".into(), "banner".into()],
        };
        assert_eq!(k.default_text(), "banner");
        assert_eq!(k.normalize("CLASSIC").as_deref(), Some("classic"), "the schema spelling");
        assert_eq!(k.normalize(" banner ").as_deref(), Some("banner"));
        assert_eq!(k.normalize("gilded"), None);
        assert_eq!(k.normalize(""), None);
    }

    #[test]
    fn key_values() {
        let k = Kind::Key { default: "F10".to_string() };
        assert_eq!(k.default_text(), "F10");
        assert_eq!(k.normalize("f10").as_deref(), Some("F10"));
        assert_eq!(k.normalize(" num5 ").as_deref(), Some("NUM5"));
        assert_eq!(k.normalize("F99"), None);
        assert_eq!(k.normalize(""), None);
    }

    /// Whatever a schema declares, its own default is a value it accepts.
    #[test]
    fn every_default_normalises() {
        let (s, _) = ok(SAMPLE);
        for f in &s.fields {
            let text = f.kind.default_text();
            assert_eq!(f.kind.normalize(&text).as_deref(), Some(text.as_str()), "{}", f.key);
        }
    }

    // -----------------------------------------------------------------------
    // The ini the overlay creates
    // -----------------------------------------------------------------------

    #[test]
    fn renders_a_default_ini() {
        let (s, _) = ok(SAMPLE);
        let text = render_ini_defaults(&s);
        assert_eq!(
            text,
            "; DesertLooter.ini was missing, so Desert Overlay created it.\n\
             ; Every key below is at the plugin's own default. Keys you add by hand are\n\
             ; kept: the overlay only ever rewrites the lines it owns.\n\
             \n\
             [DesertLooter]\n\
             Enabled=1\n\
             ScanRange=40\n\
             GatherInterval=500\n\
             Foraging=1\n\
             Theme=banner\n\
             KeyToggle=F10\n"
        );
        // and it reads back as the same values through the schema
        for line in ini::lines(&text) {
            let ini::Line::Pair(k, v) = line else { panic!("{text}") };
            let f = s.field(k).unwrap();
            assert_eq!(f.kind.normalize(v).as_deref(), Some(v), "{k}");
        }
    }

    // -----------------------------------------------------------------------
    // Writing beside the ini
    // -----------------------------------------------------------------------

    #[test]
    fn write_beside_writes_once_then_says_unchanged() {
        let dir = temp_dir("write");
        let (s, _) = ok(SAMPLE);
        let banner = "DesertLooter.overlay.ini - written by the test.";
        let target = dir.join("DesertLooter.overlay.ini");

        assert_eq!(write_beside(&dir, &s, banner), Written::Written);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), render(&s, banner));
        assert_eq!(write_beside(&dir, &s, banner), Written::Unchanged);

        std::fs::write(&target, "; someone edited it\n").unwrap();
        assert_eq!(write_beside(&dir, &s, banner), Written::Written);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), render(&s, banner));

        // a changed schema is a changed file
        let mut s2 = s.clone();
        s2.order = 42;
        assert_eq!(write_beside(&dir, &s2, banner), Written::Written);
        assert_eq!(write_beside(&dir, &s2, banner), Written::Unchanged);
        assert!(!dir.join("DesertLooter.overlay.ini.tmp").exists(), "the tmp file is renamed away");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_beside_reports_a_failure_instead_of_panicking() {
        let dir = temp_dir("fail").join("does-not-exist");
        let (s, _) = ok(SAMPLE);
        assert!(matches!(write_beside(&dir, &s, "b"), Written::Failed(_)));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
