//! One ini file's live state: what is on disk, what the menu is showing, and
//! when the difference gets written back.
//!
//! Three jobs, all of them driven from the render loop so nothing here needs a
//! thread of its own:
//!
//! * **Read.** [`Store::poll`] stats the file at most once a second and
//!   re-reads it only when the modified time moved, so a hand edit (or another
//!   copy of the game's launcher rewriting it) shows up in the menu. A `stat`
//!   per file per second is nothing next to a frame.
//! * **Write.** A widget change calls [`Store::touch`], which only marks the
//!   model dirty. [`Store::flush`] does the actual write, at most once every
//!   [`FLUSH_INTERVAL`] while a slider is still being dragged, and immediately
//!   on release. Dragging a slider across its range therefore costs four
//!   writes a second, not one per frame.
//! * **Report.** Every failure is caught, turned into a one-line message in
//!   [`Store::status`] for the red line in the window, and left alone. Nothing
//!   retries in a tight loop: the next poll or the next edit tries again.
//!
//! While the model is dirty, [`Store::poll`] does not reload. Otherwise the
//! overlay's own pending edit would be overwritten by the file it is about to
//! replace. After a successful write the watermark is set from the file the
//! write produced, so the overlay never reloads its own change either.
//!
//! The store knows nothing about any particular ini. It is handed a model -
//! in the shipped overlay always a [`crate::dynmodel::DynModel`] built from a
//! plugin's schema file - and asks it for the file's name, how to read it, and
//! what to write.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::rewrite;

/// Longest a pending edit waits before it reaches disk. Four writes a second
/// while a slider is being dragged; the plugins poll at about 1 Hz, so this is
/// well inside "the game picks it up within a second".
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(250);

/// How often the file is stat'd for an outside change.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How long a failed write waits before it is tried again. A file that cannot
/// be written (open in an editor, read-only, a full disk) stays dirty, and
/// without this the "flush on release" path would retry it on every single
/// frame - the tight loop the failure policy exists to avoid.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// What the store needs of the thing it is editing.
///
/// A trait rather than a concrete type so the store stays what it was before
/// the schema work - file watching, debouncing and failure reporting, with no
/// opinion about the contents of any ini.
pub trait IniModel {
    /// The file's name beside the game exe.
    fn file_name(&self) -> &str;

    /// The text used as the starting point when the overlay has to CREATE the
    /// file, because the user deleted it or never unzipped it: a short comment
    /// header and every key at the plugin's own default. The model's current
    /// values are then written over it in the usual way, so a created file is
    /// exactly what the menu is showing. The overlay never rewrites this
    /// header on a file that already exists.
    fn created_header(&self) -> String;

    /// Read the model out of ini text. Unknown keys are ignored; a value
    /// outside the plugin's accepted range keeps the default, exactly as the
    /// plugin itself would.
    fn parse_ini(&mut self, text: &str);

    /// The keys this model owns, with their values formatted for the file.
    /// [`crate::rewrite::rewrite`] replaces exactly these keys and leaves the
    /// rest of the file alone.
    fn pairs(&self) -> Vec<(&str, String)>;
}

/// What a flush did, for the caller's log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flushed {
    /// The file was rewritten in place, preserving its comments.
    Wrote,
    /// The file did not exist and was created from the model's header.
    Created,
    /// Nothing was written; the message is already in [`Store::status`].
    Failed,
}

pub struct Store<M: IniModel> {
    path: PathBuf,
    /// The file's own name, taken from the model once so a log line or a
    /// status message never has to borrow the model to name it.
    name: String,
    /// What the menu edits. Public so the widgets can bind straight to it.
    pub model: M,
    /// Modified time of the file as the store last read or wrote it.
    mtime: Option<SystemTime>,
    dirty: bool,
    /// The last read or write failed, so back off to [`RETRY_INTERVAL`].
    failed: bool,
    last_flush: Instant,
    last_poll: Instant,
    /// Set on any read or write failure, cleared by the next success. Drawn as
    /// a red line under the section it belongs to.
    pub status: Option<String>,
}

impl<M: IniModel> Store<M> {
    /// Open the store on `dir/<model's file name>` and read the file once. A
    /// missing file is not an error: the model stays at the plugin's defaults
    /// and the file is created on the first edit.
    pub fn new(dir: &Path, model: M, now: Instant) -> Self {
        let name = model.file_name().to_string();
        let mut s = Store {
            path: dir.join(&name),
            name,
            model,
            mtime: None,
            dirty: false,
            failed: false,
            last_flush: now,
            last_poll: now,
            status: None,
        };
        s.reload();
        s
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// The file's name, for log lines and the window's section labels.
    pub fn file_name(&self) -> &str {
        &self.name
    }

    /// A widget changed the model. Cheap: no I/O, just a flag.
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    /// Stat the file, and re-read it if somebody else changed it. Call once a
    /// frame; it does nothing at all until [`POLL_INTERVAL`] has passed.
    ///
    /// Returns `true` if the model was replaced from disk, so the caller can
    /// log it.
    pub fn poll(&mut self, now: Instant) -> bool {
        if now.duration_since(self.last_poll) < POLL_INTERVAL {
            return false;
        }
        self.last_poll = now;
        // A pending edit wins over the file: reloading here would throw away
        // what the user is in the middle of doing.
        if self.dirty {
            return false;
        }
        let seen = file_mtime(&self.path);
        if seen.is_none() || seen == self.mtime {
            return false;
        }
        self.reload()
    }

    /// Read and parse the file, replacing the model. `true` if it worked.
    fn reload(&mut self) -> bool {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                // Take the modified time from AFTER the read: if the file is
                // rewritten between the stat and the read, the next poll sees
                // a newer stamp and reads again rather than missing the change.
                self.mtime = file_mtime(&self.path);
                self.model.parse_ini(&text);
                self.status = None;
                self.failed = false;
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Not an error worth a red line: the file appears the moment
                // anything is edited.
                self.mtime = None;
                false
            }
            Err(e) => {
                self.status = Some(format!("cannot read {}: {e}", self.name));
                self.failed = true;
                false
            }
        }
    }

    /// Write the model back if it is dirty and it is time.
    ///
    /// `released` should be true when no widget is being held (imgui's
    /// `is_any_item_active`), which is what makes the end of a slider drag
    /// land immediately instead of up to [`FLUSH_INTERVAL`] later.
    pub fn flush(&mut self, now: Instant, released: bool) -> Option<Flushed> {
        if !self.dirty {
            return None;
        }
        // Normally: write when the widget is let go, or every FLUSH_INTERVAL
        // while it is still held. After a failure the "on release" shortcut is
        // withdrawn, because `released` is true on every idle frame and would
        // otherwise turn a locked file into a per-frame retry.
        let wait = if self.failed { RETRY_INTERVAL } else { FLUSH_INTERVAL };
        let due = now.duration_since(self.last_flush) >= wait;
        if !due && !(released && !self.failed) {
            return None;
        }
        self.last_flush = now;

        let (text, outcome) = match std::fs::read_to_string(&self.path) {
            Ok(existing) => (rewrite::rewrite(&existing, &self.model.pairs()), Flushed::Wrote),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let header = self.model.created_header();
                (rewrite::rewrite(&header, &self.model.pairs()), Flushed::Created)
            }
            Err(e) => {
                // Leave it dirty: the next flush tries again, no sooner than
                // RETRY_INTERVAL from now.
                self.status = Some(format!("cannot read {}: {e}", self.name));
                self.failed = true;
                return Some(Flushed::Failed);
            }
        };

        match rewrite::write_atomically(&self.path, &text) {
            Ok(()) => {
                self.dirty = false;
                // Our own write, so remember its stamp; otherwise the next
                // poll would see a change and reload what we just wrote.
                self.mtime = file_mtime(&self.path);
                self.status = None;
                self.failed = false;
                Some(outcome)
            }
            Err(e) => {
                self.status = Some(format!("cannot write {}: {e}", self.name));
                self.failed = true;
                Some(Flushed::Failed)
            }
        }
    }
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dynmodel::DynModel;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    /// A schema of the same shape the plugins write, small enough to read.
    const SCHEMA: &str = "\
[overlay]
Schema=1
Title=Test Mod
Ini=TestMod.ini

[Enabled]
Kind=bool
Label=Enabled
Default=1

[Foraging]
Kind=int
Label=Foraging
Default=1
Min=1
Max=100

[Ore]
Kind=int
Label=Ore
Default=1
Min=1
Max=100
";

    fn model() -> DynModel {
        let (section, _) = desert_core::schema::parse(SCHEMA).unwrap();
        DynModel::new(section)
    }

    /// Set one key, the way a widget would.
    fn set(m: &mut DynModel, key: &str, value: &str) {
        let i = m.section.fields.iter().position(|f| f.key == key).unwrap();
        assert!(m.set(i, value), "{key}={value}");
    }

    fn get(m: &DynModel, key: &str) -> String {
        m.get(key).unwrap().to_string()
    }

    /// A private directory per test, so they can run in parallel.
    fn tmpdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "desert-overlay-store-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_missing_file_gives_the_defaults_and_no_error() {
        let d = tmpdir();
        let s = Store::new(&d, model(), Instant::now());
        assert_eq!(s.model, model());
        assert_eq!(s.file_name(), "TestMod.ini");
        assert!(s.status.is_none(), "a missing ini is normal, not an error");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn new_reads_the_file() {
        let d = tmpdir();
        std::fs::write(d.join("TestMod.ini"), "Foraging=7\n").unwrap();
        let s = Store::new(&d, model(), Instant::now());
        assert_eq!(get(&s.model, "Foraging"), "7");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn flush_creates_a_missing_file_from_the_header() {
        let d = tmpdir();
        let now = Instant::now();
        let mut s = Store::new(&d, model(), now);
        set(&mut s.model, "Ore", "5");
        s.touch();
        assert_eq!(s.flush(now, true), Some(Flushed::Created));
        let text = std::fs::read_to_string(d.join("TestMod.ini")).unwrap();
        assert!(
            text.starts_with("; TestMod.ini was missing, so Desert Overlay created it."),
            "the header comes from the schema: {text}"
        );
        assert!(text.contains("[TestMod]"));
        assert!(text.contains("Ore=5"));
        let mut back = model();
        back.parse_ini(&text);
        assert_eq!(back, s.model);
        assert!(!s.dirty());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn flush_rewrites_in_place_and_keeps_the_comments() {
        let d = tmpdir();
        let path = d.join("TestMod.ini");
        std::fs::write(&path, "; keep me\n[TestMod]\nForaging=2\nDebug=1\n").unwrap();
        let now = Instant::now();
        let mut s = Store::new(&d, model(), now);
        assert_eq!(get(&s.model, "Foraging"), "2");
        set(&mut s.model, "Foraging", "9");
        s.touch();
        assert_eq!(s.flush(now, true), Some(Flushed::Wrote));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("; keep me\n[TestMod]\nForaging=9\n"));
        assert!(text.contains("Debug=1"), "a key the overlay does not own survives");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn flush_is_debounced_while_a_widget_is_held_and_immediate_on_release() {
        let d = tmpdir();
        let t0 = Instant::now();
        let mut s = Store::new(&d, model(), t0);
        set(&mut s.model, "Foraging", "3");
        s.touch();
        assert_eq!(s.flush(t0, false), None, "a held slider does not write on the same frame");
        assert!(s.dirty());
        assert_eq!(s.flush(t0 + FLUSH_INTERVAL, false), Some(Flushed::Created));

        set(&mut s.model, "Foraging", "4");
        s.touch();
        assert_eq!(s.flush(t0 + FLUSH_INTERVAL, true), Some(Flushed::Wrote), "release writes now");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_failed_write_backs_off_instead_of_retrying_every_frame() {
        // The target is a directory, so every write fails no matter what.
        let d = tmpdir();
        std::fs::create_dir_all(d.join("TestMod.ini")).unwrap();
        let t0 = Instant::now();
        let mut s = Store::new(&d, model(), t0);
        assert!(s.status.is_some(), "the failed read is reported");
        set(&mut s.model, "Ore", "2");
        s.touch();
        // The failed read in `new` already put the store in backoff, so the
        // first attempt has to wait it out too.
        let t1 = t0 + RETRY_INTERVAL;
        assert_eq!(s.flush(t1, true), Some(Flushed::Failed));
        assert!(s.dirty(), "a failed write stays pending");
        assert_eq!(s.flush(t1, true), None, "released does not shortcut the backoff");
        assert_eq!(s.flush(t1 + FLUSH_INTERVAL, true), None, "nor does the normal interval");
        assert_eq!(s.flush(t1 + RETRY_INTERVAL, true), Some(Flushed::Failed));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn nothing_is_written_when_nothing_changed() {
        let d = tmpdir();
        let now = Instant::now();
        let mut s = Store::new(&d, model(), now);
        assert_eq!(s.flush(now, true), None);
        assert!(!d.join("TestMod.ini").exists());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn poll_picks_up_an_outside_edit() {
        let d = tmpdir();
        let path = d.join("TestMod.ini");
        std::fs::write(&path, "Foraging=2\n").unwrap();
        let t0 = Instant::now();
        let mut s = Store::new(&d, model(), t0);
        assert_eq!(get(&s.model, "Foraging"), "2");

        assert!(!s.poll(t0), "no stat before the poll interval is up");
        // A filesystem whose mtime has one-second resolution needs the stamp
        // to actually differ, so set it explicitly rather than racing it.
        std::fs::write(&path, "Foraging=8\n").unwrap();
        bump_mtime(&path);
        assert!(s.poll(t0 + POLL_INTERVAL));
        assert_eq!(get(&s.model, "Foraging"), "8");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn poll_does_not_clobber_a_pending_edit() {
        let d = tmpdir();
        let path = d.join("TestMod.ini");
        std::fs::write(&path, "Foraging=2\n").unwrap();
        let t0 = Instant::now();
        let mut s = Store::new(&d, model(), t0);
        set(&mut s.model, "Foraging", "5");
        s.touch();
        std::fs::write(&path, "Foraging=8\n").unwrap();
        bump_mtime(&path);
        assert!(!s.poll(t0 + POLL_INTERVAL), "a dirty model is not overwritten from disk");
        assert_eq!(get(&s.model, "Foraging"), "5");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn our_own_write_is_not_read_back_as_an_outside_change() {
        let d = tmpdir();
        let t0 = Instant::now();
        let mut s = Store::new(&d, model(), t0);
        set(&mut s.model, "Ore", "6");
        s.touch();
        s.flush(t0, true);
        assert!(!s.poll(t0 + POLL_INTERVAL), "the write's own mtime is already the watermark");
        assert_eq!(get(&s.model, "Ore"), "6");
        std::fs::remove_dir_all(&d).ok();
    }

    /// Move a file's modified time a second into the future, so a change is
    /// visible on filesystems with coarse timestamps.
    fn bump_mtime(path: &Path) {
        let f = std::fs::File::options().write(true).open(path).unwrap();
        let t = SystemTime::now() + Duration::from_secs(2);
        f.set_modified(t).unwrap();
    }
}
