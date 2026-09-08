//! One plugin's ini file as the menu sees it: a [`Section`] read from that
//! plugin's schema file, plus one value per field.
//!
//! The overlay knows nothing about any particular mod. Everything it draws -
//! the keys, their labels, their defaults, their accepted ranges - arrives at
//! runtime in a `*.overlay.ini` schema written by the plugin itself, parsed by
//! [`desert_core::schema`]. This module is what turns that description into
//! something editable: [`DynModel::values`] holds one string per field, in the
//! field's **written spelling** (`1`/`0`, `40`, `6.5`, `F10`, the option's own
//! capitalisation), index-aligned with `section.fields`, so writing the file is
//! nothing more than pairing each field's key with its slot.
//!
//! Values stay in text for two reasons. It is the spelling the ini already
//! uses, so a value that came off disk unchanged is written back byte for
//! byte; and [`Kind::normalize`] is the single rule for what a field accepts,
//! shared with the plugins, so the menu can never show or write a value the
//! plugin would reject. An unacceptable value is not clamped or coerced: the
//! default stays, which is exactly what the plugin does with the same file.
//!
//! Keys the schema does not mention are ignored on read and never written -
//! [`crate::rewrite`] only touches the lines whose key it was handed, so hand
//! edits and keys a newer plugin added survive an edit from the menu.

use desert_core::ini::{self, Line};
use desert_core::schema::{Kind, Preset, Section};

/// A schema and its current values.
///
/// The two fields are public and deliberately separate: the render loop takes
/// `&section` and `&mut values` at the same time, which the borrow checker
/// allows for two fields of one struct but not for a `&Section` handed out by
/// a method on `&self`.
///
/// `values.len() == section.fields.len()` always holds; every constructor and
/// every mutator keeps it, and [`DynModel::value`] is total anyway.
#[derive(Debug, Clone, PartialEq)]
pub struct DynModel {
    pub section: Section,
    /// One value per field, in the field's written spelling.
    pub values: Vec<String>,
}

impl DynModel {
    /// A model at every field's default, which is what the menu shows for a
    /// file that does not exist yet.
    pub fn new(section: Section) -> Self {
        let values = section.fields.iter().map(|f| f.kind.default_text()).collect();
        DynModel { section, values }
    }

    /// The ini file this model edits, e.g. `DesertLooter.ini`.
    pub fn file_name(&self) -> &str {
        &self.section.ini
    }

    /// The value at `i`, or `""` for an index no field has (which cannot
    /// happen, but a menu must not be able to panic).
    pub fn value(&self, i: usize) -> &str {
        self.values.get(i).map_or("", String::as_str)
    }

    /// The value for an ini key, matched case-insensitively.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.index_of(key).map(|i| self.value(i))
    }

    /// Set the field at `i` from `raw`, if that field accepts it. `false` when
    /// it does not and the old value stays.
    pub fn set(&mut self, i: usize, raw: &str) -> bool {
        set_value(&self.section, &mut self.values, i, raw)
    }

    /// Every value back to its default.
    pub fn reset(&mut self) {
        self.values.clear();
        self.values.extend(self.section.fields.iter().map(|f| f.kind.default_text()));
    }

    /// Read ini text through the schema: a key the schema names takes the
    /// file's value when the field accepts it, and keeps its default when it
    /// does not (an out-of-range number, a choice that is not an option, a key
    /// name nothing maps to) - which is what the plugin itself does, so the
    /// menu shows the value in force rather than the value on disk.
    ///
    /// Every other key in the file is ignored. Values not mentioned at all
    /// fall back to their defaults, so a model is never left showing what a
    /// previous file said.
    pub fn parse_ini(&mut self, text: &str) {
        self.reset();
        for line in ini::lines(text) {
            let Line::Pair(key, value) = line else { continue };
            let Some(i) = self.index_of(key) else { continue };
            set_value(&self.section, &mut self.values, i, value);
        }
    }

    /// The keys this model owns with their current values, in field order.
    /// [`crate::rewrite::rewrite`] replaces exactly these and leaves the rest
    /// of the file alone.
    pub fn pairs(&self) -> Vec<(&str, String)> {
        self.section
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| (f.key.as_str(), self.value(i).to_string()))
            .collect()
    }

    /// Apply a preset button. Touches only the keys it names.
    pub fn apply_preset(&mut self, preset: &Preset) -> bool {
        apply_preset(&self.section, &mut self.values, preset)
    }

    fn index_of(&self, key: &str) -> Option<usize> {
        self.section.fields.iter().position(|f| f.key.eq_ignore_ascii_case(key))
    }
}

impl crate::store::IniModel for DynModel {
    fn file_name(&self) -> &str {
        DynModel::file_name(self)
    }

    /// The created file's starting text comes from the schema itself
    /// ([`desert_core::schema::render_ini_defaults`]), so a mod the overlay
    /// has never heard of still gets a file with a header and every key at the
    /// plugin's own default. The banner is the overlay's own: a plugin that
    /// seeds its own ini writes a different one, and whichever got there first
    /// says so.
    fn created_header(&self) -> String {
        let banner = format!(
            "{} was missing, so Desert Overlay created it.\n\
             Every key below is at the plugin's own default. Keys you add by hand are\n\
             kept: the overlay only ever rewrites the lines it owns.",
            self.section.ini
        );
        desert_core::schema::render_ini_defaults(&self.section, &banner)
    }

    fn parse_ini(&mut self, text: &str) {
        DynModel::parse_ini(self, text);
    }

    fn pairs(&self) -> Vec<(&str, String)> {
        DynModel::pairs(self)
    }
}

/// [`DynModel::set`] against a split borrow, for the render loop.
pub fn set_value(section: &Section, values: &mut [String], i: usize, raw: &str) -> bool {
    let Some(field) = section.fields.get(i) else { return false };
    let Some(text) = field.kind.normalize(raw) else { return false };
    match values.get_mut(i) {
        Some(slot) => {
            *slot = text;
            true
        }
        None => false,
    }
}

/// [`DynModel::apply_preset`] against a split borrow, for the render loop.
///
/// `true` if anything moved. Every assignment in a parsed preset names a field
/// of this schema and carries a value that field accepts
/// ([`desert_core::schema::parse`] refuses the preset otherwise), so this
/// normally sets all of them; it still checks, because a `Section` can also be
/// built by hand.
pub fn apply_preset(section: &Section, values: &mut [String], preset: &Preset) -> bool {
    let mut changed = false;
    for (key, value) in &preset.set {
        let Some(i) = section.fields.iter().position(|f| f.key.eq_ignore_ascii_case(key)) else {
            continue;
        };
        changed |= set_value(section, values, i, value);
    }
    changed
}

// ---------------------------------------------------------------------------
// What the widgets need: the stored text as a number, a flag or an index
// ---------------------------------------------------------------------------

/// The stored text as a flag. Every spelling that is not truthy is `false`,
/// the same rule the plugins read the file with.
pub fn as_bool(text: &str) -> bool {
    ini::parse_bool(text)
}

/// The stored text as a whole number, falling back to the field's default.
/// Only a `Kind::Int` has one; anything else answers 0.
pub fn as_int(kind: &Kind, text: &str) -> i64 {
    match kind {
        Kind::Int { default, min, max, .. } => match text.trim().parse::<i64>() {
            Ok(v) if (*min..=*max).contains(&v) => v,
            _ => *default,
        },
        _ => 0,
    }
}

/// The stored text as an `f32`, falling back to the field's default.
pub fn as_float(kind: &Kind, text: &str) -> f32 {
    match kind {
        Kind::Float { default, min, max, .. } => match text.trim().parse::<f32>() {
            Ok(v) if v.is_finite() && (*min..=*max).contains(&v) => v,
            _ => *default,
        },
        _ => 0.0,
    }
}

/// Where the stored text sits in a list of options, matched
/// case-insensitively. 0 (the first option) for anything not in the list, so a
/// combo always has something selected.
pub fn option_index(options: &[String], text: &str) -> usize {
    let t = text.trim();
    options.iter().position(|o| o.eq_ignore_ascii_case(t)).unwrap_or(0)
}

/// Where a key name sits in [`ini::key_names`], the list the key picker draws.
/// 0 for a name the list does not hold.
pub fn key_index(text: &str) -> usize {
    let t = text.trim();
    ini::key_names().iter().position(|n| n.eq_ignore_ascii_case(t)).unwrap_or(0)
}

/// An `i64` from the schema as the `i32` imgui's integer widgets take.
/// Saturating rather than wrapping: a bound that does not fit becomes the
/// widest bound that does, and the value is still checked against the schema's
/// own range by [`Kind::normalize`] before it is written.
pub fn i32_of(v: i64) -> i32 {
    match i32::try_from(v) {
        Ok(n) => n,
        Err(_) if v < 0 => i32::MIN,
        Err(_) => i32::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use desert_core::schema;

    /// A schema with one field of every kind, a heading, a same-line pair and
    /// two presets - the shapes the menu has to draw.
    const SCHEMA: &str = "\
[overlay]
Schema=1
Title=Test Mod
Ini=TestMod.ini
Module=TestMod.asi
Order=10

[Enabled]
Kind=bool
Label=Enabled
Default=1

[Interval]
Kind=int
Label=Interval (ms)
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

[ScanRange]
Kind=float
Label=Scan range
Default=40
Min=1
Max=200

[Theme]
Kind=choice
Label=Theme
Default=banner
Options=classic;parchment;banner

[KeyToggle]
Kind=key
Label=Toggle
Default=F10

[preset:Everything]
Hint=All of it.
Set=Enabled=1;Foraging=100

[preset:Off]
Set=Enabled=0
";

    fn model() -> DynModel {
        let (section, warnings) = schema::parse(SCHEMA).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        DynModel::new(section)
    }

    #[test]
    fn a_new_model_is_every_default_in_its_written_spelling() {
        let m = model();
        assert_eq!(m.values.len(), m.section.fields.len());
        assert_eq!(
            m.values,
            vec![
                "1".to_string(),
                "500".to_string(),
                "1".to_string(),
                "40".to_string(),
                "banner".to_string(),
                "F10".to_string(),
            ]
        );
        assert_eq!(m.file_name(), "TestMod.ini");
    }

    #[test]
    fn parse_ini_takes_the_keys_the_schema_names_and_ignores_the_rest() {
        let mut m = model();
        m.parse_ini(
            "; a comment\n[TestMod]\nEnabled=0\nInterval=250\nScanRange=25.5\n\
             Debug=1\nSomethingElse=hello\n",
        );
        assert_eq!(m.get("Enabled"), Some("0"));
        assert_eq!(m.get("Interval"), Some("250"));
        assert_eq!(m.get("ScanRange"), Some("25.5"));
        assert_eq!(m.get("Foraging"), Some("1"), "a key the file omits keeps its default");
        assert_eq!(m.get("Debug"), None, "a key the schema does not name is not modelled");
    }

    #[test]
    fn an_unacceptable_value_keeps_the_default() {
        let mut m = model();
        // Out of range, not a number, not an option, not a key name.
        m.parse_ini("Interval=5\nForaging=101\nScanRange=oops\nTheme=neon\nKeyToggle=F99\n");
        assert_eq!(m.get("Interval"), Some("500"));
        assert_eq!(m.get("Foraging"), Some("1"));
        assert_eq!(m.get("ScanRange"), Some("40"));
        assert_eq!(m.get("Theme"), Some("banner"));
        assert_eq!(m.get("KeyToggle"), Some("F10"));
    }

    #[test]
    fn a_choice_matches_case_insensitively_and_keeps_the_schemas_spelling() {
        let mut m = model();
        m.parse_ini("Theme=PARCHMENT\n");
        assert_eq!(m.get("Theme"), Some("parchment"));
    }

    #[test]
    fn a_key_name_is_stored_canonically() {
        let mut m = model();
        m.parse_ini("KeyToggle=pageup\n");
        assert_eq!(m.get("KeyToggle"), Some("PAGEUP"));
    }

    #[test]
    fn keys_are_matched_case_insensitively() {
        let mut m = model();
        m.parse_ini("enabled=0\nINTERVAL=1000\n");
        assert_eq!(m.get("Enabled"), Some("0"));
        assert_eq!(m.get("Interval"), Some("1000"));
    }

    #[test]
    fn a_float_is_written_without_a_pointless_decimal() {
        let mut m = model();
        m.parse_ini("ScanRange=40.0\n");
        assert_eq!(m.get("ScanRange"), Some("40"));
        m.parse_ini("ScanRange=6.50\n");
        assert_eq!(m.get("ScanRange"), Some("6.5"));
    }

    #[test]
    fn pairs_are_in_field_order_and_carry_the_current_values() {
        let mut m = model();
        m.parse_ini("Enabled=0\nTheme=classic\n");
        let keys: Vec<&str> = m.pairs().iter().map(|(k, _)| *k).collect();
        let fields: Vec<&str> = m.section.fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, fields);
        assert_eq!(
            m.pairs().iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(),
            vec!["0", "500", "1", "40", "classic", "F10"]
        );
    }

    #[test]
    fn pairs_round_trip_through_parse_ini() {
        let mut m = model();
        m.parse_ini("Enabled=0\nInterval=250\nForaging=7\nScanRange=12.5\nTheme=classic\nKeyToggle=END\n");
        let text: String = m.pairs().iter().map(|(k, v)| format!("{k}={v}\n")).collect();
        let mut back = DynModel::new(m.section.clone());
        back.parse_ini(&text);
        assert_eq!(back, m);
    }

    #[test]
    fn a_preset_sets_its_keys_and_nothing_else() {
        let mut m = model();
        m.parse_ini("Enabled=0\nInterval=250\nTheme=classic\n");
        let preset = m.section.presets.first().cloned().unwrap();
        assert_eq!(preset.label, "Everything");
        assert!(m.apply_preset(&preset));
        assert_eq!(m.get("Enabled"), Some("1"));
        assert_eq!(m.get("Foraging"), Some("100"));
        assert_eq!(m.get("Interval"), Some("250"), "a key the preset omits is untouched");
        assert_eq!(m.get("Theme"), Some("classic"));
    }

    #[test]
    fn set_refuses_a_value_the_field_would_not_accept() {
        let mut m = model();
        let i = m.section.fields.iter().position(|f| f.key == "Interval").unwrap();
        assert!(m.set(i, "1000"));
        assert_eq!(m.get("Interval"), Some("1000"));
        assert!(!m.set(i, "5"), "below Min");
        assert_eq!(m.get("Interval"), Some("1000"));
        assert!(!m.set(999, "1"), "an index no field has");
    }

    #[test]
    fn the_widget_readers_answer_with_the_current_value() {
        let mut m = model();
        m.parse_ini("Enabled=0\nInterval=250\nScanRange=12.5\nTheme=classic\nKeyToggle=END\n");
        let by = |key: &str| {
            let i = m.section.fields.iter().position(|f| f.key == key).unwrap();
            (m.section.fields.get(i).unwrap().kind.clone(), m.value(i).to_string())
        };
        let (kind, text) = by("Enabled");
        assert!(!as_bool(&text), "{kind:?}");
        let (kind, text) = by("Interval");
        assert_eq!(as_int(&kind, &text), 250);
        let (kind, text) = by("ScanRange");
        assert_eq!(as_float(&kind, &text), 12.5);
        let (kind, text) = by("Theme");
        let Kind::Choice { options, .. } = &kind else { panic!("not a choice") };
        assert_eq!(option_index(options, &text), 0);
        let (_, text) = by("KeyToggle");
        assert_eq!(desert_core::ini::key_names().get(key_index(&text)).copied(), Some("END"));
    }

    #[test]
    fn the_widget_readers_fall_back_rather_than_panicking() {
        let kind = Kind::Int { default: 5, min: 1, max: 10, step: 1, slider: false, format: None };
        assert_eq!(as_int(&kind, "nonsense"), 5);
        assert_eq!(as_int(&kind, "99"), 5);
        assert_eq!(as_float(&Kind::Float { default: 2.0, min: 0.0, max: 3.0, format: None }, ""), 2.0);
        assert_eq!(option_index(&[], "x"), 0);
        assert_eq!(key_index("not a key"), 0);
    }

    #[test]
    fn i32_of_saturates_instead_of_wrapping() {
        assert_eq!(i32_of(42), 42);
        assert_eq!(i32_of(i64::MAX), i32::MAX);
        assert_eq!(i32_of(i64::MIN), i32::MIN);
    }
}
