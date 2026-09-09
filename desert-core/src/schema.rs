//! The settings model the menu is drawn from, and the ini it seeds.
//!
//! A [`Section`] is one subsystem's settings described in data: one [`Field`]
//! per ini key, with its [`Kind`] (which picks the widget), its default, its
//! range and its label, plus the [`Preset`] buttons that write several keys at
//! once. Each subsystem builds its own in its `config.rs`; `desert-tooling`
//! collects them and hands them to the overlay at startup, and the overlay
//! draws one collapsible menu section per [`Section`].
//!
//! None of this is a file format. The sections travel in process; the ini is
//! the only thing on disk. Every section names the file it describes
//! ([`Section::ini`] - one shared file for all of them) and the `[Header]` it
//! owns inside it ([`Section::ini_section`]). The overlay writes that file and
//! each subsystem reads its own section back on its own poll, through
//! [`crate::ini::lines_in_section`].
//!
//! This module also seeds that file when it is missing:
//! [`render_ini_defaults_all`] renders every section at its default and
//! [`create_ini_if_missing_all`] writes it, atomically and only if absent.
//!
//! [`Kind::normalize`] takes text a player can edit, so nothing here can
//! panic: a value path returns `Option` and a writer returns [`Written`].

use std::path::Path;

use crate::ini;

/// What a field is and what it accepts, with its default. The widget the
/// overlay draws follows from the variant.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// A checkbox. Written as `1`/`0`.
    Bool { default: bool },
    /// A whole number in `min..=max`, drawn as an input (`step` per click) or
    /// as a slider.
    Int { default: i64, min: i64, max: i64, step: i64, slider: bool, format: Option<String> },
    /// A finite `f32` in `min..=max`, drawn as a slider.
    Float { default: f32, min: f32, max: f32, format: Option<String> },
    /// One of `options`, matched case-insensitively, written back in the
    /// section's own spelling of that option.
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

/// One subsystem's settings: one menu section, and one `[Header]` of the
/// shared ini.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: String,
    /// The ini file this describes: a bare file name, no path separators.
    /// Every section names the same one now.
    pub ini: String,
    /// The `[Header]` this section owns inside [`ini`](Section::ini):
    /// `Looter`, `Gatherer`, `Overlay`. One file holds all three and `Enabled`
    /// means something different in each of them, so the header is what keeps
    /// them apart - when the file is seeded and when the overlay writes a key
    /// back.
    pub ini_section: String,
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

/// What a write did, for the caller's log line. See
/// [`create_ini_if_missing_all`] for what each variant means there - it is the
/// only thing that returns one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// Nothing was written.
    Unchanged,
    /// The file was created.
    Written,
    /// Nothing was written; the string says why.
    Failed(String),
}

impl Section {
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

    /// The default in its written spelling. A section is expected to keep it
    /// inside its own range and among its own options, so that
    /// [`normalize`](Kind::normalize) accepts it - the
    /// `every_default_normalises` test holds the shipped ones to that.
    pub fn default_text(&self) -> String {
        match self {
            Kind::Bool { default } => flag(*default),
            Kind::Int { default, .. } => default.to_string(),
            Kind::Float { default, .. } => num(*default),
            Kind::Choice { default, .. } | Kind::Key { default } => default.clone(),
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
/// lets a value survive being read back out of the ini and written again.
fn num(f: f32) -> String {
    let s = format!("{f:.1}");
    match s.strip_suffix(".0") {
        Some(whole) => whole.to_string(),
        None => s,
    }
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

/// Emit `banner` as a comment block, one `;` line per line of `banner` (which
/// is given **without** the `;`). A blank line becomes a bare `;` rather than
/// `"; "`, so no line carries trailing whitespace - the shipped inis use the
/// bare form for paragraph breaks and a generated file should read the same.
///
/// Returns whether anything was emitted, which is how the callers know an
/// empty banner must not be followed by a blank separator line.
fn banner_block(out: &mut String, banner: &str) -> bool {
    let mut any = false;
    for line in banner.lines() {
        if line.is_empty() {
            out.push_str(";\n");
        } else {
            out.push_str("; ");
            out.push_str(line);
            out.push('\n');
        }
        any = true;
    }
    any
}

/// Render `sections` as a default ini: the `banner` comment block, then each
/// section as `[<ini_section>]` followed by every one of its fields at its
/// default, in the order given and with a blank line between them.
///
/// This is not the overlay's alone. It is how the one ini gets written when it
/// is missing (see [`create_ini_if_missing_all`]), which is why the header is
/// the caller's and not baked in here: only the caller knows which mod is
/// writing and why. `banner` is one or more lines **without** a leading `;`,
/// which this adds. An empty banner emits no comment lines and no leading blank
/// line, so the text starts straight at the first header.
pub fn render_ini_defaults_all(sections: &[Section], banner: &str) -> String {
    let mut out = String::with_capacity(256 * sections.len().max(1));
    if banner_block(&mut out, banner) {
        out.push('\n');
    }
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push('[');
        out.push_str(&section.ini_section);
        out.push_str("]\n");
        for f in &section.fields {
            kv(&mut out, &f.key, &f.kind.default_text());
        }
    }
    out
}

/// [`render_ini_defaults_all`] for a single section - the same text, so a
/// subsystem's own "my parser agrees with my defaults" test reads exactly what
/// the seeded file would give it.
pub fn render_ini_defaults(section: &Section, banner: &str) -> String {
    render_ini_defaults_all(std::slice::from_ref(section), banner)
}

/// Create `dir/file_name` from [`render_ini_defaults_all`], but only when the
/// file does not already exist. An existing file is never read, rewritten or
/// replaced.
///
/// This deliberately does **not** use a tmp-file-then-rename dance. The ini
/// belongs to the player, who edits it by hand, and clobbering it is the one
/// outcome that is not allowed here. A rename would overwrite a file that
/// appeared in the window between an `exists()` check and the write, so the
/// check and the create have to be a single operation: `create_new` is the OS's
/// own atomic "only if absent", and it cannot clobber.
///
/// [`Written`] here means:
///
/// - [`Written::Unchanged`] - **the file already existed**, whatever is in it;
///   nothing was compared, because nothing was read.
/// - [`Written::Written`] - the file was absent and has been created.
/// - [`Written::Failed`] - nothing was written; the string says why.
///
/// A failure is not fatal for the caller. A missing ini is already covered by
/// each subsystem's own `Config::default()`, so seeding the file is a
/// convenience for the player and not a load-bearing step: log a WARN and carry
/// on. Never panics.
pub fn create_ini_if_missing_all(
    dir: &Path,
    file_name: &str,
    sections: &[Section],
    banner: &str,
) -> Written {
    use std::io::Write as _;

    let target = dir.join(file_name);
    let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Written::Unchanged,
        Err(e) => return Written::Failed(format!("{}: {e}", target.display())),
    };
    if let Err(e) = file.write_all(render_ini_defaults_all(sections, banner).as_bytes()) {
        return Written::Failed(format!("{}: {e}", target.display()));
    }
    Written::Written
}

/// [`create_ini_if_missing_all`] for a single section, into the file that
/// section names.
pub fn create_ini_if_missing(dir: &Path, section: &Section, banner: &str) -> Written {
    create_ini_if_missing_all(dir, &section.ini, std::slice::from_ref(section), banner)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A field with nothing but the essentials; the decoration is the overlay's
    /// business and none of it reaches the ini.
    fn field(key: &str, kind: Kind) -> Field {
        Field {
            key: key.to_string(),
            label: key.to_string(),
            kind,
            heading: None,
            same_line: false,
            help: None,
        }
    }

    /// A section using every kind, in the shape a subsystem's `config.rs`
    /// builds one. There is no schema text to parse any more, so the tests
    /// build the model directly, exactly as the shipped code does.
    fn looter() -> Section {
        Section {
            title: "Desert Looter".to_string(),
            ini: "DesertTooling.ini".to_string(),
            ini_section: "Looter".to_string(),
            module: None,
            order: 10,
            notice: Some("Takes effect on the next load.".to_string()),
            presets_label: Some("Presets (what auto-loot picks up):".to_string()),
            presets: vec![Preset {
                label: "Plants only".to_string(),
                hint: Some("Foraging only: plants, fruit, berries.".to_string()),
                set: vec![
                    ("Enabled".to_string(), "1".to_string()),
                    ("Foraging".to_string(), "4".to_string()),
                ],
            }],
            fields: vec![
                field("Enabled", Kind::Bool { default: true }),
                field(
                    "ScanRange",
                    Kind::Float {
                        default: 40.0,
                        min: 1.0,
                        max: 200.0,
                        format: Some("%.0f m".into()),
                    },
                ),
                field(
                    "GatherInterval",
                    Kind::Int {
                        default: 500,
                        min: 100,
                        max: 60_000,
                        step: 50,
                        slider: false,
                        format: None,
                    },
                ),
                field(
                    "Foraging",
                    Kind::Int {
                        default: 1,
                        min: 1,
                        max: 100,
                        step: 1,
                        slider: true,
                        format: Some("%dx".into()),
                    },
                ),
                field(
                    "Theme",
                    Kind::Choice {
                        default: "banner".to_string(),
                        options: vec!["classic".into(), "parchment".into(), "banner".into()],
                    },
                ),
                field("KeyToggle", Kind::Key { default: "F10".to_string() }),
            ],
        }
    }

    /// A second section over the same file, with an `Enabled` of its own: the
    /// collision the `[Header]` exists to keep apart.
    fn gatherer() -> Section {
        Section {
            title: "Desert Gatherer".to_string(),
            ini: "DesertTooling.ini".to_string(),
            ini_section: "Gatherer".to_string(),
            module: None,
            order: 20,
            notice: None,
            presets_label: None,
            presets: Vec::new(),
            fields: vec![
                field("Enabled", Kind::Bool { default: false }),
                field(
                    "Foraging",
                    Kind::Int { default: 2, min: 1, max: 99, step: 1, slider: false, format: None },
                ),
            ],
        }
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("desert-core-schema-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    // -----------------------------------------------------------------------
    // The model
    // -----------------------------------------------------------------------

    #[test]
    fn field_lookup_is_case_insensitive() {
        let s = looter();
        assert_eq!(s.field("scanrange").map(|f| f.key.as_str()), Some("ScanRange"));
        assert_eq!(s.field("SCANRANGE").map(|f| f.key.as_str()), Some("ScanRange"));
        assert!(s.field("nope").is_none());
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
        let k = Kind::Int {
            default: 500,
            min: 100,
            max: 60_000,
            step: 50,
            slider: false,
            format: None,
        };
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
    /// value carried in a section depends on it.
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
        assert_eq!(k.normalize("CLASSIC").as_deref(), Some("classic"), "the section's spelling");
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

    /// Whatever a section declares, its own default is a value it accepts.
    #[test]
    fn every_default_normalises() {
        for s in [looter(), gatherer()] {
            for f in &s.fields {
                let text = f.kind.default_text();
                assert_eq!(f.kind.normalize(&text).as_deref(), Some(text.as_str()), "{}", f.key);
            }
        }
    }

    // -----------------------------------------------------------------------
    // The ini that gets seeded
    // -----------------------------------------------------------------------

    /// The body is the same either way; the banner is the caller's, and an
    /// empty one leaves the file starting at the section header.
    #[test]
    fn renders_a_default_ini() {
        let s = looter();
        const BODY: &str = "\
[Looter]
Enabled=1
ScanRange=40
GatherInterval=500
Foraging=1
Theme=banner
KeyToggle=F10
";

        let banner = "DesertTooling.ini was missing, so Desert Tooling created it.\n\
                      Every key below is at the plugin's own default.";
        let text = render_ini_defaults(&s, banner);
        assert_eq!(
            text,
            format!(
                "; DesertTooling.ini was missing, so Desert Tooling created it.\n\
                 ; Every key below is at the plugin's own default.\n\
                 \n\
                 {BODY}"
            )
        );

        // no banner: no comment lines and no leading blank line
        let bare = render_ini_defaults(&s, "");
        assert_eq!(bare, BODY);

        // and either way every emitted line reads back through the section
        for text in [&text, &bare] {
            for line in ini::lines(text) {
                let ini::Line::Pair(k, v) = line else { panic!("{text}") };
                let f = s.field(k).unwrap();
                assert_eq!(f.kind.normalize(v).as_deref(), Some(v), "{k}");
            }
        }
    }

    /// The whole point of the shared file: each section gets its own header, in
    /// the order given, and a subsystem reading through
    /// [`ini::lines_in_section`] sees only its own keys - including the
    /// `Enabled` and `Foraging` both of these declare.
    #[test]
    fn renders_every_section_under_its_own_header() {
        let sections = [looter(), gatherer()];
        let text = render_ini_defaults_all(&sections, "one file");
        assert_eq!(
            text,
            "\
; one file

[Looter]
Enabled=1
ScanRange=40
GatherInterval=500
Foraging=1
Theme=banner
KeyToggle=F10

[Gatherer]
Enabled=0
Foraging=2
"
        );

        for s in &sections {
            let got: Vec<_> = ini::lines_in_section(&text, &s.ini_section)
                .map(|l| match l {
                    ini::Line::Pair(k, v) => (k.to_string(), v.to_string()),
                    ini::Line::Bad(w) => panic!("{w}"),
                })
                .collect();
            let want: Vec<_> =
                s.fields.iter().map(|f| (f.key.clone(), f.kind.default_text())).collect();
            assert_eq!(got, want, "[{}]", s.ini_section);
        }
    }

    /// One section renders the same through either door, so a subsystem's own
    /// "my parser agrees with my defaults" test reads what the seeded file
    /// would actually give it.
    #[test]
    fn one_section_renders_the_same_either_way() {
        let s = looter();
        assert_eq!(
            render_ini_defaults(&s, "b"),
            render_ini_defaults_all(std::slice::from_ref(&s), "b")
        );
        assert_eq!(render_ini_defaults_all(&[], "b"), "; b\n\n", "no sections is just the banner");
        assert_eq!(render_ini_defaults_all(&[], ""), "");
    }

    /// A paragraph break in a banner is a bare `;`, not `"; "`. The shipped
    /// inis are written that way and a generated one should not differ by a
    /// trailing space no editor shows.
    #[test]
    fn a_blank_banner_line_carries_no_trailing_space() {
        let text = render_ini_defaults_all(&[looter(), gatherer()], "one\n\ntwo");
        assert!(text.starts_with("; one\n;\n; two\n\n["), "{text}");
        for line in text.lines() {
            assert_eq!(line.trim_end(), line, "trailing space in {line:?}");
        }
    }

    // -----------------------------------------------------------------------
    // Seeding a missing ini
    // -----------------------------------------------------------------------

    #[test]
    fn creates_a_missing_ini() {
        let dir = temp_dir("create");
        let sections = [looter(), gatherer()];
        let banner = "DesertTooling.ini was missing, so the test created it.";
        let target = dir.join("DesertTooling.ini");

        assert_eq!(
            create_ini_if_missing_all(&dir, "DesertTooling.ini", &sections, banner),
            Written::Written
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            render_ini_defaults_all(&sections, banner)
        );

        // the single-section door writes the file that section names
        let one = temp_dir("create-one");
        assert_eq!(create_ini_if_missing(&one, &sections[0], banner), Written::Written);
        assert_eq!(
            std::fs::read_to_string(one.join("DesertTooling.ini")).unwrap(),
            render_ini_defaults(&sections[0], banner)
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&one);
    }

    /// The guarantee that matters: whatever the player has in their ini, a
    /// second call leaves it byte-for-byte alone.
    #[test]
    fn never_clobbers_an_existing_ini() {
        let dir = temp_dir("keep");
        let sections = [looter(), gatherer()];
        let target = dir.join("DesertTooling.ini");
        let mine = "; hand written\n[Looter]\nEnabled=0\nnot even a pair\n";
        std::fs::write(&target, mine).unwrap();

        assert_eq!(
            create_ini_if_missing_all(&dir, "DesertTooling.ini", &sections, "a banner"),
            Written::Unchanged
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), mine, "the file was left alone");

        // and again, to show Unchanged is not a one-shot
        assert_eq!(
            create_ini_if_missing_all(&dir, "DesertTooling.ini", &sections, ""),
            Written::Unchanged
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), mine);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_ini_reports_a_failure_instead_of_panicking() {
        let dir = temp_dir("create-fail").join("does-not-exist");
        let sections = [looter()];
        assert!(matches!(
            create_ini_if_missing_all(&dir, "DesertTooling.ini", &sections, "b"),
            Written::Failed(_)
        ));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
