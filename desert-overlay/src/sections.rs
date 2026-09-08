//! Discovery: which mods the menu is drawing this second.
//!
//! The overlay has no list of mods. Every plugin writes a small schema file
//! beside its ini at startup (`DesertLooter.ini` ->
//! `DesertLooter.overlay.ini`, see [`desert_core::schema`]), and this module
//! is the directory scan that finds them: one [`SectionEntry`] per readable
//! schema file, each owning a [`Store`] on the ini that schema describes,
//! sorted by the schema's own `Order` and then its title. A mod nobody wrote
//! an overlay for is simply not in the menu; a new mod appears in it within a
//! second of dropping its files into `bin64`, with no change here at all.
//!
//! The scan is deliberately dull. It runs once a second off the render loop
//! (same budget as the existing ini poll: a `read_dir` and one `stat` per
//! file), keeps each file's modified time, and re-parses only what changed.
//! A file that will not parse is complained about **once per modified time**,
//! not once per second, so a mod that ships a broken schema costs one log line
//! and not a growing file.
//!
//! Nothing here is Windows-only: it is `std::fs` and text, and it unit-tests
//! natively against a temporary directory. The one thing it cannot answer on
//! its own is whether a section's `Module` is loaded in the game process -
//! that needs `GetModuleHandleW`, so [`SectionEntry::loaded`] is a plain flag
//! the Windows side sets.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use desert_core::schema::{self, Section, FILE_SUFFIX};

use crate::dynmodel::DynModel;
use crate::store::Store;

/// How often the directory is listed. The same 1 Hz as everything else the
/// overlay watches.
pub const SCAN_INTERVAL: Duration = Duration::from_secs(1);

/// One mod's section of the menu: its schema, its ini, and whether its plugin
/// is actually in the process.
pub struct SectionEntry {
    /// The schema file this came from, e.g. `DesertLooter.overlay.ini`. It is
    /// the identity of the entry: a file with this name reappearing with a new
    /// modified time replaces it.
    pub file: String,
    /// Modified time of that schema file when it was parsed, so an unchanged
    /// file is never re-read.
    mtime: Option<SystemTime>,
    /// Whether the section's `Module` is loaded in the game process. A section
    /// that names no module is always true; for the rest the Windows side
    /// refreshes this once a second, and a false one is drawn disabled.
    pub loaded: bool,
    /// The ini the schema describes, watched and written as before.
    pub store: Store<DynModel>,
}

impl SectionEntry {
    pub fn section(&self) -> &Section {
        &self.store.model.section
    }

    pub fn title(&self) -> &str {
        &self.store.model.section.title
    }

    /// The `.asi` whose presence decides [`SectionEntry::loaded`], if the
    /// schema names one.
    pub fn module(&self) -> Option<&str> {
        self.store.model.section.module.as_deref()
    }
}

/// Every section the exe directory currently describes, in display order.
pub struct Sections {
    dir: PathBuf,
    entries: Vec<SectionEntry>,
    /// Schema files that would not parse, with the modified time the warning
    /// was written for, so the same broken file is not reported every second.
    rejected: Vec<(String, Option<SystemTime>)>,
    last_scan: Option<Instant>,
}

impl Sections {
    /// An empty registry over `dir`. Nothing is read until the first
    /// [`Sections::scan`], which the caller makes right away so its log lines
    /// land with the rest of the startup output.
    pub fn new(dir: &Path) -> Self {
        Sections { dir: dir.to_path_buf(), entries: Vec::new(), rejected: Vec::new(), last_scan: None }
    }

    pub fn entries(&self) -> &[SectionEntry] {
        &self.entries
    }

    pub fn entries_mut(&mut self) -> &mut [SectionEntry] {
        &mut self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// List the directory and bring the sections in line with it: parse new
    /// and changed schema files, drop the ones whose file is gone, leave
    /// everything else exactly as it was (a re-parse would throw away a
    /// pending edit and reset the store's watermarks).
    ///
    /// Does nothing until [`SCAN_INTERVAL`] has passed. Returns the lines the
    /// caller should log; returning them rather than writing them is what
    /// keeps this module testable off a real game.
    pub fn scan(&mut self, now: Instant) -> Vec<String> {
        let mut log = Vec::new();
        if self.last_scan.is_some_and(|t| now.duration_since(t) < SCAN_INTERVAL) {
            return log;
        }
        self.last_scan = Some(now);

        // A directory that cannot be listed leaves the menu exactly as it is.
        // It is not worth a log line every second, and the only way it happens
        // is a directory that has gone away underneath a running game.
        let Ok(listing) = std::fs::read_dir(&self.dir) else { return log };
        let mut found: Vec<(String, Option<SystemTime>)> = Vec::new();
        for item in listing.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            if !is_schema_file(&name) {
                continue;
            }
            let mtime = item.metadata().and_then(|m| m.modified()).ok();
            found.push((name, mtime));
        }
        // read_dir's order is the filesystem's; sorting makes the log lines of
        // a first scan deterministic, which is what a bug report quotes.
        found.sort();

        let mut kept: Vec<SectionEntry> = Vec::new();
        for (name, mtime) in &found {
            let at = self.entries.iter().position(|e| e.file == *name);
            let previous = at.map(|i| self.entries.remove(i));
            if let Some(entry) = previous {
                if entry.mtime == *mtime {
                    kept.push(entry);
                    continue;
                }
                // Changed on disk: the old section is replaced wholesale
                // below, keeping only whether its plugin was loaded so the
                // section does not flicker to "(not installed)" for a second.
                if let Some(fresh) = self.load(name, *mtime, entry.loaded, now, &mut log) {
                    kept.push(fresh);
                }
                continue;
            }
            if let Some(fresh) = self.load(name, *mtime, false, now, &mut log) {
                kept.push(fresh);
            }
        }

        // Whatever is still in `self.entries` was not in the listing.
        for gone in self.entries.drain(..) {
            log.push(format!("[schema] {} is gone; {} left the menu", gone.file, gone.title()));
        }
        // A broken file that has been deleted should complain again if it
        // comes back, so only remember the ones still present.
        self.rejected.retain(|(name, _)| found.iter().any(|(f, _)| f == name));

        kept.sort_by(|a, b| {
            let (x, y) = (a.section(), b.section());
            x.order.cmp(&y.order).then_with(|| x.title.cmp(&y.title))
        });
        self.entries = kept;
        log
    }

    /// Read and parse one schema file into an entry, or report why not.
    ///
    /// `loaded` seeds [`SectionEntry::loaded`]; the plugin poll corrects it
    /// within a second either way.
    fn load(
        &mut self,
        name: &str,
        mtime: Option<SystemTime>,
        loaded: bool,
        now: Instant,
        log: &mut Vec<String>,
    ) -> Option<SectionEntry> {
        let text = match std::fs::read_to_string(self.dir.join(name)) {
            Ok(t) => t,
            Err(e) => {
                // The file was listed a moment ago, so this is a permission
                // problem or a file being rewritten right now; either way the
                // next scan tries again.
                self.reject(name, mtime, &e.to_string(), log);
                return None;
            }
        };
        let (section, warnings) = match schema::parse(&text) {
            Ok(ok) => ok,
            Err(why) => {
                self.reject(name, mtime, &why, log);
                return None;
            }
        };
        log.push(format!(
            "[schema] {name}: {}, {} fields, {} presets",
            section.title,
            section.fields.len(),
            section.presets.len()
        ));
        // Skipped fields and presets: the section still draws, so these are
        // notes rather than a refusal, but they are how a plugin author finds
        // out a key never appeared in the menu.
        for w in warnings {
            log.push(format!("[schema] WARN {name}: {w}"));
        }
        let loaded = loaded || section.module.is_none();
        let store = Store::new(&self.dir, DynModel::new(section), now);
        Some(SectionEntry { file: name.to_string(), mtime, loaded, store })
    }

    /// Log a file the overlay will not draw, once per modified time.
    fn reject(&mut self, name: &str, mtime: Option<SystemTime>, why: &str, log: &mut Vec<String>) {
        if self.rejected.iter().any(|(n, m)| n == name && *m == mtime) {
            return;
        }
        self.rejected.retain(|(n, _)| n != name);
        self.rejected.push((name.to_string(), mtime));
        log.push(format!("[schema] WARN {name}: {why}"));
    }
}

/// Is this the name of a schema file? The suffix is matched
/// case-insensitively, and a file called nothing but `.overlay.ini` is not
/// one: there would be no ini stem in front of it.
pub fn is_schema_file(name: &str) -> bool {
    name.len() > FILE_SUFFIX.len() && name.to_ascii_lowercase().ends_with(FILE_SUFFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    fn tmpdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "desert-overlay-sections-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn schema(title: &str, ini: &str, order: i32, extra: &str) -> String {
        format!(
            "[overlay]\nSchema=1\nTitle={title}\nIni={ini}\nModule=Test.asi\nOrder={order}\n\n\
             [Enabled]\nKind=bool\nLabel=Enabled\nDefault=1\n{extra}"
        )
    }

    /// Each scan needs a later instant than the last, or the interval gate
    /// swallows it.
    fn later(n: u32) -> Instant {
        Instant::now() + SCAN_INTERVAL * n
    }

    #[test]
    fn a_schema_file_is_recognised_by_its_suffix() {
        assert!(is_schema_file("DesertLooter.overlay.ini"));
        assert!(is_schema_file("DESERTLOOTER.OVERLAY.INI"), "matched case-insensitively");
        assert!(!is_schema_file("DesertLooter.ini"));
        assert!(!is_schema_file(".overlay.ini"), "no ini stem in front of it");
        assert!(!is_schema_file("overlay.ini"));
    }

    #[test]
    fn a_scan_finds_a_new_file_and_a_second_scan_says_nothing() {
        let d = tmpdir();
        std::fs::write(d.join("TestMod.overlay.ini"), schema("Test Mod", "TestMod.ini", 10, ""))
            .unwrap();
        let mut s = Sections::new(&d);
        let log = s.scan(later(1));
        assert_eq!(log, vec!["[schema] TestMod.overlay.ini: Test Mod, 1 fields, 0 presets"]);
        assert_eq!(s.len(), 1);
        assert_eq!(s.entries().first().unwrap().title(), "Test Mod");
        assert_eq!(s.entries().first().unwrap().module(), Some("Test.asi"));
        assert_eq!(s.entries().first().unwrap().store.file_name(), "TestMod.ini");

        assert!(s.scan(later(2)).is_empty(), "an unchanged file is not re-read or re-logged");
        assert_eq!(s.len(), 1);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_scan_interval_gates_the_directory_listing() {
        let d = tmpdir();
        let mut s = Sections::new(&d);
        let t0 = Instant::now();
        assert!(s.scan(t0).is_empty());
        std::fs::write(d.join("TestMod.overlay.ini"), schema("Test Mod", "TestMod.ini", 10, ""))
            .unwrap();
        assert!(s.scan(t0).is_empty(), "same instant, no second listing");
        assert!(s.is_empty());
        assert!(!s.scan(t0 + SCAN_INTERVAL).is_empty());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_changed_file_replaces_the_store() {
        let d = tmpdir();
        let path = d.join("TestMod.overlay.ini");
        std::fs::write(&path, schema("Test Mod", "TestMod.ini", 10, "")).unwrap();
        let mut s = Sections::new(&d);
        s.scan(later(1));
        assert_eq!(s.entries().first().unwrap().section().fields.len(), 1);

        std::fs::write(
            &path,
            schema(
                "Test Mod",
                "TestMod.ini",
                10,
                "\n[Ore]\nKind=int\nLabel=Ore\nDefault=1\nMin=1\nMax=100\n",
            ),
        )
        .unwrap();
        bump_mtime(&path);
        let log = s.scan(later(2));
        assert_eq!(log, vec!["[schema] TestMod.overlay.ini: Test Mod, 2 fields, 0 presets"]);
        assert_eq!(s.len(), 1);
        assert_eq!(s.entries().first().unwrap().section().fields.len(), 2);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_deleted_file_drops_its_section() {
        let d = tmpdir();
        let path = d.join("TestMod.overlay.ini");
        std::fs::write(&path, schema("Test Mod", "TestMod.ini", 10, "")).unwrap();
        let mut s = Sections::new(&d);
        s.scan(later(1));
        std::fs::remove_file(&path).unwrap();
        let log = s.scan(later(2));
        assert_eq!(log, vec!["[schema] TestMod.overlay.ini is gone; Test Mod left the menu"]);
        assert!(s.is_empty());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_rejected_file_is_logged_once_and_again_when_it_changes() {
        let d = tmpdir();
        let path = d.join("Broken.overlay.ini");
        std::fs::write(&path, "Title=No header section\n").unwrap();
        let mut s = Sections::new(&d);
        let log = s.scan(later(1));
        assert_eq!(log, vec!["[schema] WARN Broken.overlay.ini: no [overlay] section"]);
        assert!(s.is_empty());
        assert!(s.scan(later(2)).is_empty(), "the same broken file is not reported again");

        // Still broken, but different: worth saying once more.
        std::fs::write(&path, "[overlay]\nSchema=99\nTitle=T\nIni=T.ini\n").unwrap();
        bump_mtime(&path);
        let log = s.scan(later(3));
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(log.first().unwrap().contains("newer than"), "{log:?}");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_skipped_field_is_a_warning_and_the_section_still_draws() {
        let d = tmpdir();
        std::fs::write(
            d.join("TestMod.overlay.ini"),
            schema("Test Mod", "TestMod.ini", 10, "\n[Odd]\nKind=colour\nDefault=red\n"),
        )
        .unwrap();
        let mut s = Sections::new(&d);
        let log = s.scan(later(1));
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(log.get(1).unwrap().starts_with("[schema] WARN TestMod.overlay.ini:"), "{log:?}");
        assert_eq!(s.len(), 1);
        assert_eq!(s.entries().first().unwrap().section().fields.len(), 1);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn sections_are_sorted_by_order_then_title() {
        let d = tmpdir();
        for (file, title, ini, order) in [
            ("C.overlay.ini", "Cc", "Cc.ini", 10),
            ("A.overlay.ini", "Aa", "Aa.ini", 20),
            ("B.overlay.ini", "Bb", "Bb.ini", 10),
        ] {
            std::fs::write(d.join(file), schema(title, ini, order, "")).unwrap();
        }
        let mut s = Sections::new(&d);
        s.scan(later(1));
        let titles: Vec<&str> = s.entries().iter().map(SectionEntry::title).collect();
        assert_eq!(titles, vec!["Bb", "Cc", "Aa"]);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn files_that_are_not_schemas_are_ignored() {
        let d = tmpdir();
        std::fs::write(d.join("DesertLooter.ini"), "Enabled=1\n").unwrap();
        std::fs::write(d.join("notes.txt"), "hello\n").unwrap();
        let mut s = Sections::new(&d);
        assert!(s.scan(later(1)).is_empty());
        assert!(s.is_empty());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_schema_file_goes_in_and_an_ini_file_comes_out() {
        // The whole path in one test, on a schema of the shape a plugin
        // writes: discovered, drawn from (a preset is what the buttons do),
        // then written to an ini that did not exist.
        let d = tmpdir();
        std::fs::write(
            d.join("TestMod.overlay.ini"),
            "; written by Test Mod 1.0\n[overlay]\nSchema=1\nTitle=Test Mod\nIni=TestMod.ini\n\
             Module=TestMod.asi\nOrder=10\nNotice=Takes effect on the next load.\n\
             PresetsLabel=Presets:\n\n\
             [Enabled]\nKind=bool\nLabel=Enabled\nDefault=1\nHelp=Master switch.\n\n\
             [GatherForaging]\nKind=bool\nLabel=Foraging\nHeading=Gather families:\nDefault=1\n\n\
             [GatherLogging]\nKind=bool\nLabel=Logging\nSameLine=1\nDefault=1\n\n\
             [ScanRange]\nKind=float\nLabel=Scan range\nDefault=40\nMin=1\nMax=200\n\
             Format=%.0f m\n\n\
             [KeyToggle]\nKind=key\nLabel=Toggle\nDefault=F10\n\n\
             [preset:Plants only]\nHint=Foraging only.\nSet=GatherForaging=1;GatherLogging=0\n",
        )
        .unwrap();

        let now = Instant::now();
        let mut s = Sections::new(&d);
        assert_eq!(s.scan(now).len(), 1);
        let entry = s.entries_mut().first_mut().unwrap();
        assert_eq!(entry.section().notice.as_deref(), Some("Takes effect on the next load."));
        assert_eq!(entry.section().presets.len(), 1);
        assert!(!entry.loaded, "TestMod.asi is not a loaded module in a unit test");

        let preset = entry.section().presets.first().cloned().unwrap();
        assert!(entry.store.model.apply_preset(&preset));
        entry.store.touch();
        assert_eq!(entry.store.flush(now, true), Some(crate::store::Flushed::Created));

        let text = std::fs::read_to_string(d.join("TestMod.ini")).unwrap();
        assert!(text.contains("[TestMod]"), "{text}");
        assert!(text.contains("GatherForaging=1"), "{text}");
        assert!(text.contains("GatherLogging=0"), "{text}");
        assert!(text.contains("ScanRange=40"), "{text}");
        assert!(text.contains("KeyToggle=F10"), "{text}");
        std::fs::remove_dir_all(&d).ok();
    }

    /// Move a file's modified time into the future, so a change is visible on
    /// filesystems with coarse timestamps.
    fn bump_mtime(path: &Path) {
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(2)).unwrap();
    }
}
