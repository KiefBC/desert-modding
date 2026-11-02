//! One subsystem's slice of the ini as the menu sees it: the
//! [`Section`] that subsystem declared, plus one value per field.
//!
//! The overlay knows nothing about any particular mod. Everything it draws -
//! the keys, their labels, their defaults, their accepted ranges - arrives at
//! startup as a [`desert_core::schema::Section`] the subsystem built in its own
//! `config.rs` and `desert-tooling` handed to [`crate::start`]. This module is
//! what turns that description into something editable: [`DynModel::values`]
//! holds one string per field, in the field's **written spelling** (`1`/`0`,
//! `40`, `6.5`, `F10`, the option's own capitalisation), index-aligned with
//! `section.fields`, so writing the file is nothing more than pairing each
//! field's key with its slot.
//!
//! All three models sit on the **same** file and are told apart by the
//! `[Section]` header each owns ([`Section::ini_section`]): reads go through
//! [`ini::lines_in_section`] and writes through [`crate::rewrite::rewrite`],
//! both scoped to that header, because `Enabled` exists under all three of
//! them.
//!
//! Values stay in text for two reasons. It is the spelling the ini already
//! uses, so a value that came off disk unchanged is written back byte for
//! byte; and [`Kind::normalize`] is the single rule for what a field accepts,
//! shared with the plugins, so the menu can never show or write a value the
//! plugin would reject. An unacceptable value is not clamped or coerced: the
//! default stays, which is exactly what the plugin does with the same file.
//!
//! Keys the section does not mention are ignored on read and never written -
//! [`crate::rewrite`] only touches the lines whose key it was handed, so hand
//! edits and keys a newer plugin added survive an edit from the menu.
//!
//! The menu's own arithmetic lives here too, at the bottom: which of a
//! section's fields belong on which tab, whether a section gets a tab of its
//! own, whether its `Enabled` is off, and what the footer says about the file.
//! None of it draws anything, which is the point - [`crate::ui`] is
//! `#[cfg(windows)]` and cannot run on Linux, so every rule about what the menu
//! shows is decided by a plain function that is unit-tested natively.

use std::path::Path;
use std::time::Instant;

use desert_core::ini::{self, Line};
use desert_core::schema::{Kind, Preset, Section, Tab};

use crate::store::Store;

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

    /// The ini file this model edits: `DesertTooling.ini`, the same one for
    /// every model.
    pub fn file_name(&self) -> &str {
        &self.section.ini
    }

    /// The `[Section]` header inside that file this model owns.
    pub fn ini_section(&self) -> &str {
        &self.section.ini_section
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

    /// Read ini text through the section: a key the section names takes the
    /// file's value when the field accepts it, and keeps its default when it
    /// does not (an out-of-range number, a choice that is not an option, a key
    /// name nothing maps to) - which is what the subsystem itself does, so the
    /// menu shows the value in force rather than the value on disk.
    ///
    /// **Only the model's own `[Section]` is read**, exactly as the subsystem
    /// reads it, so the looter's `Enabled` cannot be shown in the gatherer's
    /// row. Every other key in the file is ignored, and values not mentioned at
    /// all fall back to their defaults, so a model is never left showing what a
    /// previous file said.
    pub fn parse_ini(&mut self, text: &str) {
        self.reset();
        // Cloned, because the iterator borrows `self.section` otherwise and the
        // loop writes `self.values`. One short string per read.
        let section = self.section.ini_section.clone();
        for line in ini::lines_in_section(text, &section) {
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

    fn ini_section(&self) -> &str {
        DynModel::ini_section(self)
    }

    /// The created file's starting text comes from the section itself
    /// ([`desert_core::schema::render_ini_defaults`]), so a menu edit made
    /// against a file somebody deleted still produces a file with a banner, the
    /// `[Section]` header and every key at the subsystem's own default. The
    /// banner is the overlay's own: `desert-tooling` seeds the file with a
    /// different one at startup, and whichever got there first says so.
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

// ---------------------------------------------------------------------------
// One section, which is one tab of the menu plus a share of the two shared ones
// ---------------------------------------------------------------------------

/// One subsystem's section of the menu: its model over the shared ini, and
/// whether its module is actually in the process.
///
/// There is nothing to discover any more - the sections arrive in
/// [`crate::start`] and this list is fixed for the life of the process - so an
/// entry is only the pairing of a [`Store`] with the "is it loaded" flag the
/// Windows side refreshes once a second.
pub struct SectionEntry {
    /// Whether the section's `Module` is loaded in the game process. A section
    /// that names no module (which is all of them now that the subsystems ship
    /// in one `.asi`) is always true; for the rest the Windows side refreshes
    /// this once a second, and a false one is drawn disabled.
    pub loaded: bool,
    /// The ini the section describes, watched and written as before.
    pub store: Store<DynModel>,
}

impl SectionEntry {
    pub fn section(&self) -> &Section {
        &self.store.model.section
    }

    pub fn title(&self) -> &str {
        &self.store.model.section.title
    }

    /// The `[Section]` header this entry owns in the ini. Also the imgui id
    /// that keeps two sections' identically named widgets apart.
    pub fn ini_section(&self) -> &str {
        &self.store.model.section.ini_section
    }

    /// The `.asi` whose presence decides [`SectionEntry::loaded`], if the
    /// section names one.
    pub fn module(&self) -> Option<&str> {
        self.store.model.section.module.as_deref()
    }
}

/// Build one entry per section, in display order: the section's own `order`,
/// then its title, exactly as the discovery scan used to sort them. Each store
/// reads the file once here, on the caller's thread and never on a render
/// thread.
pub fn entries(dir: &Path, sections: Vec<Section>, now: Instant) -> Vec<SectionEntry> {
    let mut entries: Vec<SectionEntry> = sections
        .into_iter()
        .map(|section| {
            let loaded = section.module.is_none();
            SectionEntry { loaded, store: Store::new(dir, DynModel::new(section), now) }
        })
        .collect();
    entries.sort_by(|a, b| {
        let (x, y) = (a.section(), b.section());
        x.order.cmp(&y.order).then_with(|| x.title.cmp(&y.title))
    });
    entries
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

// ---------------------------------------------------------------------------
// Which fields go on which tab, and what the footer says
// ---------------------------------------------------------------------------

/// Whether `section` has anything at all on `tab`, which is what decides
/// whether the shared tab draws a group header for it.
///
/// A yes/no rather than the list: the renderer walks `section.fields` itself
/// and pairs each field with its slot by index, and this is asked once per
/// section per frame on a render thread, where collecting a list only to test
/// it for emptiness is an allocation for nothing.
pub fn has_fields_on(section: &Section, tab: Tab) -> bool {
    section.fields.iter().any(|f| f.tab == tab)
}

/// The fields of `section` that belong on `tab`, each with its index into
/// [`DynModel::values`]. Test-only: it spells out the pairing the renderer
/// relies on, that the index is into the full list and not the filtered one.
#[cfg(test)]
pub fn fields_on(section: &Section, tab: Tab) -> Vec<(usize, &desert_core::schema::Field)> {
    section.fields.iter().enumerate().filter(|(_, f)| f.tab == tab).collect()
}

/// The `Theme` value the overlay's own section currently holds, by the
/// section's ini `[Header]`, so the window can follow the ini rather than only
/// the picker: the store rewrites every slot from disk once a second, and a
/// `Theme=` edited by hand while the game runs has to repaint the menu the way
/// a pick in the menu does.
pub fn ini_theme_name<'a>(sections: &'a [SectionEntry], overlay_section: &str) -> Option<&'a str> {
    sections
        .iter()
        .find(|e| e.ini_section().eq_ignore_ascii_case(overlay_section))
        .and_then(|e| e.store.model.get("Theme"))
}

/// Whether this section gets a tab of its own, which it does exactly when it
/// has a field that stayed on [`Tab::Section`].
///
/// The overlay's own section is the one that does not: every key it declares is
/// marked for Settings or Debug, so a tab for it would be a second copy of what
/// the Settings tab already shows.
pub fn has_own_tab(section: &Section) -> bool {
    section.fields.iter().any(|f| f.tab == Tab::Section)
}

/// Whether any section has anything to show on `tab`, which is what decides
/// whether that tab is in the bar at all.
pub fn any_fields_on(sections: &[SectionEntry], tab: Tab) -> bool {
    sections.iter().any(|e| e.section().fields.iter().any(|f| f.tab == tab))
}

/// Whether a section's own master switch is on.
///
/// **This is the one key name the menu relies on.** Every section declares a
/// `Kind::Bool` called `Enabled` as its master switch (the ini's header says so
/// in as many words), and the tab bar dims a subsystem's tab while that switch
/// reads false, so a player can see which subsystems are live without opening
/// each page. A section that declares no such field is treated as on: the tab
/// is then never dimmed, which is the harmless answer, and so is a section whose
/// values have not been filled in - "on" is the answer that claims the least.
pub fn section_enabled(section: &Section, values: &[String]) -> bool {
    let found = section
        .fields
        .iter()
        .enumerate()
        .find(|(_, f)| f.key.eq_ignore_ascii_case("Enabled") && matches!(f.kind, Kind::Bool { .. }));
    match found.and_then(|(i, _)| values.get(i)) {
        Some(value) => as_bool(value),
        None => true,
    }
}

/// Whether a section's `notice` is drawn under its group on `tab`.
///
/// A notice belongs at the foot of the page that section owns, and a section
/// with a page of its own gets it there. A section with **no** page of its own -
/// the overlay - has nowhere else to put it, so it goes under its group on the
/// Settings tab, which is the first shared tab and the one that holds most of
/// its keys. Never on both: a line about when a change takes effect said twice
/// reads as two different claims.
pub fn shows_notice_on(section: &Section, tab: Tab) -> bool {
    section.notice.is_some() && tab == Tab::Settings && !has_own_tab(section)
}

/// What the footer says about the one ini every section shares.
///
/// The stores each watch and write the same file, so this is one answer for all
/// of them rather than a line under each section: a failure anybody hit is the
/// failure, and "saving" is true while anybody is still holding an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    /// A read or a write failed. The string is the store's own status line,
    /// which names the file and the reason.
    Failed(String),
    /// An edit is waiting for the debounce to run out.
    Saving,
    /// Everything a widget changed is on disk.
    Saved,
}

/// The widest the footer draws a failure before it becomes a tooltip. A status
/// line carries an OS error message and there is no useful width to give it, so
/// the footer shows the front of it and hovering shows all of it.
const STATUS_SHOWN: usize = 44;

impl FileState {
    /// The footer's own words, given the file name every section shares.
    ///
    /// A failure is shown in the store's own wording (`cannot write
    /// DesertTooling.ini: ...`), because that already names the file and says
    /// which half failed, cut to [`STATUS_SHOWN`] characters with an ellipsis.
    pub fn label(&self, file_name: &str) -> String {
        match self {
            FileState::Failed(status) => ellipsis(status, STATUS_SHOWN),
            // A middle dot rather than a colon: the file name is not a label
            // for what follows, the two are one phrase.
            FileState::Saving => format!("{file_name} \u{b7} saving\u{2026}"),
            FileState::Saved => format!("{file_name} \u{b7} saved"),
        }
    }

    /// The full status text, for the tooltip, when there is more to say than
    /// [`FileState::label`] shows.
    pub fn tooltip(&self) -> Option<&str> {
        match self {
            FileState::Failed(status) => Some(status.as_str()),
            _ => None,
        }
    }
}

/// `text` cut to at most `max` characters, with an ellipsis when anything was
/// cut. Counted in characters and cut on a character boundary, because a status
/// line carries an OS message and nothing promises it is ASCII.
fn ellipsis(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}\u{2026}", &text[..at]),
        None => text.to_string(),
    }
}

/// The one state the footer draws, from every section's store: the first
/// failure if there is one, otherwise saving while anybody is dirty, otherwise
/// saved.
pub fn file_state(sections: &[SectionEntry]) -> FileState {
    let failure = sections.iter().find_map(|e| e.store.status.clone());
    match failure {
        Some(status) => FileState::Failed(status),
        None if sections.iter().any(|e| e.store.dirty()) => FileState::Saving,
        None => FileState::Saved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use desert_core::schema::{Field, Preset};

    fn f(key: &str, label: &str, kind: Kind) -> Field {
        Field {
            key: key.to_string(),
            label: label.to_string(),
            kind,
            heading: None,
            same_line: false,
            help: None,
            tab: Tab::Section,
        }
    }

    /// A section with one field of every kind and two presets - the shapes the
    /// menu has to draw. Built here exactly as a subsystem's `config.rs`
    /// builds its own: there is no schema file format any more.
    fn section() -> Section {
        Section {
            title: "Test Mod".to_string(),
            ini: "DesertTooling.ini".to_string(),
            ini_section: "TestMod".to_string(),
            module: Some("TestMod.asi".to_string()),
            order: 10,
            notice: None,
            presets_label: None,
            presets: vec![
                Preset {
                    label: "Everything".to_string(),
                    hint: Some("All of it.".to_string()),
                    set: vec![
                        ("Enabled".to_string(), "1".to_string()),
                        ("Foraging".to_string(), "100".to_string()),
                    ],
                },
                Preset {
                    label: "Off".to_string(),
                    hint: None,
                    set: vec![("Enabled".to_string(), "0".to_string())],
                },
            ],
            fields: vec![
                f("Enabled", "Enabled", Kind::Bool { default: true }),
                f(
                    "Interval",
                    "Interval (ms)",
                    Kind::Int {
                        default: 500,
                        min: 100,
                        max: 60000,
                        step: 50,
                        slider: false,
                        format: None,
                    },
                ),
                f(
                    "Foraging",
                    "Foraging",
                    Kind::Int { default: 1, min: 1, max: 100, step: 1, slider: true, format: None },
                ),
                f(
                    "ScanRange",
                    "Scan range",
                    Kind::Float { default: 40.0, min: 1.0, max: 200.0, format: None },
                ),
                f(
                    "Theme",
                    "Theme",
                    Kind::Choice {
                        default: "banner".to_string(),
                        options: vec![
                            "classic".to_string(),
                            "parchment".to_string(),
                            "banner".to_string(),
                        ],
                    },
                ),
                f("KeyToggle", "Toggle", Kind::Key { default: "F10".to_string() }),
            ],
        }
    }

    fn model() -> DynModel {
        DynModel::new(section())
    }

    /// Ini text under this model's own header, which is the only place
    /// `parse_ini` reads.
    fn under(body: &str) -> String {
        format!("[TestMod]\n{body}")
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
        assert_eq!(m.file_name(), "DesertTooling.ini");
        assert_eq!(m.ini_section(), "TestMod");
    }

    #[test]
    fn parse_ini_takes_the_keys_the_section_names_and_ignores_the_rest() {
        let mut m = model();
        m.parse_ini(&under(
            "Enabled=0\nInterval=250\nScanRange=25.5\nDebug=1\nSomethingElse=hello\n",
        ));
        assert_eq!(m.get("Enabled"), Some("0"));
        assert_eq!(m.get("Interval"), Some("250"));
        assert_eq!(m.get("ScanRange"), Some("25.5"));
        assert_eq!(m.get("Foraging"), Some("1"), "a key the file omits keeps its default");
        assert_eq!(m.get("Debug"), None, "a key the section does not name is not modelled");
    }

    #[test]
    fn only_the_models_own_section_is_read() {
        // The whole reason `parse_ini` is section-scoped: `Enabled` exists
        // under every header of the shared ini.
        let mut m = model();
        m.parse_ini(
            "Enabled=0\nInterval=100\n\n[Other]\nEnabled=0\nInterval=200\n\n\
             [TestMod]\nEnabled=1\nInterval=300\n\n[Later]\nInterval=400\n",
        );
        assert_eq!(m.get("Enabled"), Some("1"));
        assert_eq!(m.get("Interval"), Some("300"), "not 100, 200 or 400");
    }

    #[test]
    fn a_file_without_our_header_leaves_every_default_in_place() {
        let mut m = model();
        m.parse_ini("[Other]\nEnabled=0\nInterval=250\n");
        assert_eq!(m, model());
    }

    #[test]
    fn the_header_is_matched_case_insensitively() {
        let mut m = model();
        m.parse_ini("[testmod]\nEnabled=0\n");
        assert_eq!(m.get("Enabled"), Some("0"));
    }

    #[test]
    fn an_unacceptable_value_keeps_the_default() {
        let mut m = model();
        // Out of range, not a number, not an option, not a key name.
        m.parse_ini(&under(
            "Interval=5\nForaging=101\nScanRange=oops\nTheme=neon\nKeyToggle=F99\n",
        ));
        assert_eq!(m.get("Interval"), Some("500"));
        assert_eq!(m.get("Foraging"), Some("1"));
        assert_eq!(m.get("ScanRange"), Some("40"));
        assert_eq!(m.get("Theme"), Some("banner"));
        assert_eq!(m.get("KeyToggle"), Some("F10"));
    }

    #[test]
    fn a_choice_matches_case_insensitively_and_keeps_the_sections_spelling() {
        let mut m = model();
        m.parse_ini(&under("Theme=PARCHMENT\n"));
        assert_eq!(m.get("Theme"), Some("parchment"));
    }

    #[test]
    fn a_key_name_is_stored_canonically() {
        let mut m = model();
        m.parse_ini(&under("KeyToggle=pageup\n"));
        assert_eq!(m.get("KeyToggle"), Some("PAGEUP"));
    }

    #[test]
    fn keys_are_matched_case_insensitively() {
        let mut m = model();
        m.parse_ini(&under("enabled=0\nINTERVAL=1000\n"));
        assert_eq!(m.get("Enabled"), Some("0"));
        assert_eq!(m.get("Interval"), Some("1000"));
    }

    #[test]
    fn a_float_is_written_without_a_pointless_decimal() {
        let mut m = model();
        m.parse_ini(&under("ScanRange=40.0\n"));
        assert_eq!(m.get("ScanRange"), Some("40"));
        m.parse_ini(&under("ScanRange=6.50\n"));
        assert_eq!(m.get("ScanRange"), Some("6.5"));
    }

    #[test]
    fn pairs_are_in_field_order_and_carry_the_current_values() {
        let mut m = model();
        m.parse_ini(&under("Enabled=0\nTheme=classic\n"));
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
        m.parse_ini(&under(
            "Enabled=0\nInterval=250\nForaging=7\nScanRange=12.5\nTheme=classic\nKeyToggle=END\n",
        ));
        let text: String = m.pairs().iter().map(|(k, v)| format!("{k}={v}\n")).collect();
        let mut back = DynModel::new(m.section.clone());
        back.parse_ini(&under(&text));
        assert_eq!(back, m);
    }

    #[test]
    fn a_preset_sets_its_keys_and_nothing_else() {
        let mut m = model();
        m.parse_ini(&under("Enabled=0\nInterval=250\nTheme=classic\n"));
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
        m.parse_ini(&under(
            "Enabled=0\nInterval=250\nScanRange=12.5\nTheme=classic\nKeyToggle=END\n",
        ));
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

    // -----------------------------------------------------------------------
    // Which fields go on which tab, and what the footer says
    // -----------------------------------------------------------------------

    /// A section like the shipped ones: a page of its own, two keys pulled onto
    /// the shared Settings tab and two switches onto the shared Debug tab.
    fn tabbed() -> Section {
        let mut s = section();
        s.notice = Some("Takes effect on the next load.".to_string());
        for field in &mut s.fields {
            field.tab = match field.key.as_str() {
                "KeyToggle" | "Theme" => Tab::Settings,
                "ScanRange" => Tab::Debug,
                _ => Tab::Section,
            };
        }
        s
    }

    #[test]
    fn fields_on_answers_with_each_fields_own_index() {
        let s = tabbed();
        let keys = |tab| {
            fields_on(&s, tab).into_iter().map(|(i, f)| (i, f.key.as_str())).collect::<Vec<_>>()
        };
        assert_eq!(keys(Tab::Settings), vec![(4, "Theme"), (5, "KeyToggle")]);
        assert_eq!(keys(Tab::Debug), vec![(3, "ScanRange")]);
        assert_eq!(
            keys(Tab::Section),
            vec![(0, "Enabled"), (1, "Interval"), (2, "Foraging")],
            "the index is into the values, not into the filtered list"
        );
        // Which is the whole reason the index is carried: it still selects the
        // right slot after the list has been filtered.
        let m = DynModel::new(s.clone());
        for (i, field) in fields_on(&s, Tab::Settings) {
            assert_eq!(m.value(i), field.kind.default_text(), "{}", field.key);
        }
    }

    #[test]
    fn has_fields_on_agrees_with_the_list() {
        let s = tabbed();
        for tab in [Tab::Section, Tab::Settings, Tab::Debug] {
            assert_eq!(has_fields_on(&s, tab), !fields_on(&s, tab).is_empty(), "{tab:?}");
        }
        let mut none = s.clone();
        none.fields.retain(|f| f.tab != Tab::Debug);
        assert!(!has_fields_on(&none, Tab::Debug));
    }

    #[test]
    fn the_ini_theme_name_is_read_off_the_overlays_own_section() {
        let dir = std::env::temp_dir().join(format!("desert-overlay-theme-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("DesertTooling.ini"), "[TestMod]\nTheme=parchment\n").unwrap();
        let built = entries(&dir, vec![tabbed()], std::time::Instant::now());
        assert_eq!(ini_theme_name(&built, "testmod"), Some("parchment"), "header matched case-insensitively");
        assert_eq!(ini_theme_name(&built, "Overlay"), None, "no such section, nothing to follow");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_section_with_nothing_left_on_its_own_tab_gets_no_tab() {
        let mut s = tabbed();
        assert!(has_own_tab(&s));
        for field in &mut s.fields {
            field.tab = Tab::Settings;
        }
        assert!(!has_own_tab(&s), "every field is on a shared tab: there is no page to draw");
        assert!(fields_on(&s, Tab::Section).is_empty());
    }

    #[test]
    fn section_enabled_reads_the_sections_own_master_switch() {
        let s = tabbed();
        let mut m = DynModel::new(s.clone());
        assert!(section_enabled(&s, &m.values), "the fixture defaults to Enabled=1");
        m.parse_ini(&under("Enabled=0\n"));
        assert!(!section_enabled(&s, &m.values));
        // Anything that is not a truthy spelling is off, the same rule the
        // plugins read the file with.
        m.parse_ini(&under("Enabled=nonsense\n"));
        assert!(!section_enabled(&s, &m.values));

        // A section with no `Enabled` at all is treated as on, so its tab is
        // never dimmed; so is one whose `Enabled` is not a checkbox.
        let mut without = s.clone();
        without.fields.retain(|f| f.key != "Enabled");
        assert!(section_enabled(&without, &DynModel::new(without.clone()).values));
        assert!(section_enabled(&s, &[]), "nor one whose values were never filled in");
    }

    #[test]
    fn a_notice_is_drawn_on_the_settings_tab_only_when_the_section_has_no_page() {
        let s = tabbed();
        // It has a page of its own, so the notice goes there, not on a shared tab.
        for tab in [Tab::Section, Tab::Settings, Tab::Debug] {
            assert!(!shows_notice_on(&s, tab), "{tab:?}");
        }
        let mut homeless = s.clone();
        for field in &mut homeless.fields {
            field.tab = if field.key == "Interval" { Tab::Debug } else { Tab::Settings };
        }
        assert!(shows_notice_on(&homeless, Tab::Settings));
        assert!(!shows_notice_on(&homeless, Tab::Debug), "said once, not twice");
        homeless.notice = None;
        assert!(!shows_notice_on(&homeless, Tab::Settings), "there is no notice to draw");
    }

    #[test]
    fn the_footer_says_saved_then_saving_then_why_it_failed() {
        let dir = std::env::temp_dir().join(format!("desert-overlay-footer-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let mut built = entries(&dir, vec![tabbed()], std::time::Instant::now());

        assert_eq!(file_state(&built), FileState::Saved);
        assert_eq!(
            file_state(&built).label("DesertTooling.ini"),
            "DesertTooling.ini \u{b7} saved"
        );
        assert_eq!(file_state(&built).tooltip(), None);
        assert!(any_fields_on(&built, Tab::Settings));
        assert!(any_fields_on(&built, Tab::Debug));

        built[0].store.touch();
        assert_eq!(file_state(&built), FileState::Saving);
        assert_eq!(
            file_state(&built).label("DesertTooling.ini"),
            "DesertTooling.ini \u{b7} saving\u{2026}"
        );

        // A failure wins over a pending edit: it is the reason the edit is still
        // pending.
        let status = "cannot write DesertTooling.ini: Permission denied (os error 13)";
        built[0].store.status = Some(status.to_string());
        assert_eq!(file_state(&built), FileState::Failed(status.to_string()));
        let label = file_state(&built).label("DesertTooling.ini");
        assert!(label.starts_with("cannot write DesertTooling.ini"), "{label}");
        assert!(label.ends_with('\u{2026}'), "the rest is in the tooltip: {label}");
        assert_eq!(label.chars().count(), 45, "44 characters and the ellipsis");
        assert_eq!(file_state(&built).tooltip(), Some(status));

        // A short status is shown whole, ellipsis and all.
        let short = FileState::Failed("cannot read it".to_string());
        assert_eq!(short.label("DesertTooling.ini"), "cannot read it");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn entries_are_built_in_order_and_each_reads_its_own_section() {
        let dir = std::env::temp_dir().join(format!("desert-overlay-entries-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("DesertTooling.ini"),
            "[TestMod]\nEnabled=0\n\n[Bb]\nEnabled=1\n",
        )
        .unwrap();

        let mut first = section();
        first.title = "Bb".to_string();
        first.ini_section = "Bb".to_string();
        first.order = 20;
        let mut third = section();
        third.title = "Aa".to_string();
        third.ini_section = "Aa".to_string();
        third.order = 10;

        let built = entries(&dir, vec![first, section(), third], std::time::Instant::now());
        let titles: Vec<&str> = built.iter().map(SectionEntry::title).collect();
        assert_eq!(titles, vec!["Aa", "Test Mod", "Bb"], "order, then title");
        assert_eq!(built.first().unwrap().store.file_name(), "DesertTooling.ini");
        // Each store read the file, and each read only its own header.
        let by = |name: &str| {
            built
                .iter()
                .find(|e| e.ini_section() == name)
                .map(|e| e.store.model.get("Enabled").unwrap_or("").to_string())
        };
        assert_eq!(by("TestMod").as_deref(), Some("0"));
        assert_eq!(by("Bb").as_deref(), Some("1"));
        assert_eq!(by("Aa").as_deref(), Some("1"), "no header of its own: the default");
        assert!(built.iter().all(|e| !e.loaded), "every fixture names a module");
        std::fs::remove_dir_all(&dir).ok();
    }
}
