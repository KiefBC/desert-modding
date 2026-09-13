//! Append-only text log beside the game exe:
//! `[+seconds] [tid] [tag] message`.
//!
//! Rules that keep the game launching no matter what state the file is in:
//! - never panic: a log that cannot be opened is silently dropped;
//! - open-append-close per write, with full share flags, so no handle is held
//!   while Defender, an editor or a crash handler has the file;
//! - nothing here is called from DllMain (loader lock) or from helper processes.
//!
//! One process writes one file, so the name is chosen once by the caller:
//! `log::init("DesertTooling.log")` before the first `log!`. A `log!` that
//! somehow beats `init` lands in `DEFAULT_LOG_NAME` rather than panicking.
//!
//! Every subsystem shares that one file, so every line carries the tag of the
//! crate that wrote it (`looter`, `gatherer`, `overlay`, `tooling`). The
//! [`log!`](crate::log!) macro takes it from the calling crate's own `LOG_TAG`;
//! see the macro for why that costs no call-site changes.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Used only if a plugin logs before calling [`init`].
pub const DEFAULT_LOG_NAME: &str = "DesertMods.log";

/// A line whose writer waited at least this long on [`WRITE_LOCK`] carries the
/// wait in its own text. Two milliseconds is below anything a healthy write
/// costs (open-append-close on a local file) and far below anything a human
/// would call a stall, so in normal running the suffix never appears; when the
/// game hangs, the first thread to be held up says so on the line it was
/// already writing. It is deliberately not a second log line: [`WRITE_LOCK`] is
/// a plain non-reentrant `Mutex` and reporting the wait through `emit` again
/// would deadlock the very thread the report is about.
const WAIT_REPORT: Duration = Duration::from_millis(2);

static START: OnceLock<Instant> = OnceLock::new();
static NAME: OnceLock<String> = OnceLock::new();
static PATH: OnceLock<PathBuf> = OnceLock::new();
static DISABLED: AtomicBool = AtomicBool::new(false);
/// Serialises writers within this process so lines never interleave.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Directory of the running exe (bin64), falling back to the cwd.
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Name the log file and start the clock. First caller wins; later calls with
/// a different name are ignored (one process only ever writes one log).
pub fn init(name: &str) {
    NAME.get_or_init(|| name.to_string());
    ensure();
}

/// Everything [`init`] does, with the name defaulted. Never fails, never panics.
fn ensure() {
    START.get_or_init(Instant::now);
    PATH.get_or_init(|| {
        exe_dir().join(NAME.get().map(String::as_str).unwrap_or(DEFAULT_LOG_NAME))
    });
}

/// Stop writing for the rest of the process (used when the host is not the game).
pub fn disable() {
    DISABLED.store(true, Ordering::Relaxed);
}

#[cfg(windows)]
fn tid() -> u32 {
    // SAFETY: GetCurrentThreadId takes no arguments and reads the id out of the
    // calling thread's own TEB. It touches no memory of ours and cannot fail.
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}
#[cfg(not(windows))]
fn tid() -> u32 {
    0
}

fn open_for_append(path: &std::path::Path) -> Option<std::fs::File> {
    let mut o = OpenOptions::new();
    o.create(true).append(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
        o.share_mode(0x1 | 0x2 | 0x4);
    }
    o.open(path).ok()
}

/// One formatted line, without the trailing newline. Split out so the exact
/// shape of a line is unit-testable without touching the file system: `write`
/// emits the untagged form the logger has always written, `write_tagged` puts
/// the subsystem's own tag between the thread id and the message.
fn line(secs: f64, tid: u32, tag: Option<&str>, msg: &str) -> String {
    match tag {
        Some(tag) => format!("[{secs:9.3}] [tid {tid:5}] [{tag}] {msg}"),
        None => format!("[{secs:9.3}] [tid {tid:5}] {msg}"),
    }
}

/// The one place that touches the file. Never panics: a log that cannot be
/// opened is silently dropped, and a poisoned lock is taken anyway (the only
/// thing it guards is the ordering of writes).
fn emit(tag: Option<&str>, msg: &str) {
    if DISABLED.load(Ordering::Relaxed) {
        return;
    }
    ensure();
    let (Some(start), Some(path)) = (START.get(), PATH.get()) else { return };
    // The clock and the thread id are read here, *before* the lock is taken, so
    // that a line's timestamp is the moment its caller asked to log rather than
    // the moment it finally won the mutex. Sampling them inside the critical
    // section - which is what this did - gave a thread that had queued for a
    // quarter of a second a perfectly on-time stamp, which made the log
    // structurally incapable of recording a stall: the one thing a hang capture
    // has to show was the one thing it hid.
    let called = start.elapsed();
    let tid = tid();
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Whatever the clock moved on by while we were queuing is the wait.
    let waited = start.elapsed().saturating_sub(called);
    if let Some(mut f) = open_for_append(path) {
        let l = line(called.as_secs_f64(), tid, tag, msg);
        if waited >= WAIT_REPORT {
            let _ = writeln!(f, "{l} [log-lock wait {:.0} ms]", waited.as_secs_f64() * 1000.0);
        } else {
            let _ = writeln!(f, "{l}");
        }
        // dropped here: handle closed immediately
    }
}

/// `[+seconds] [tid] message`.
pub fn write(msg: &str) {
    emit(None, msg);
}

/// `[+seconds] [tid] [tag] message`, the form every subsystem writes now that
/// they share one file. Same guarantees as [`write()`]: never panics,
/// open-append-close, silent when [`disable`] has been called.
pub fn write_tagged(tag: &str, msg: &str) {
    emit(Some(tag), msg);
}

/// Write one line to the shared log, tagged with the **calling crate's**
/// `LOG_TAG`.
///
/// `crate::` inside a `macro_rules!` body is resolved where the macro is
/// *invoked*, not where it is defined, so this expands to the invoking crate's
/// own `LOG_TAG` constant - which every crate that logs must declare at its
/// root (`pub const LOG_TAG: &str = "looter";`). That is deliberate: it tags
/// every line by subsystem without touching a single one of the hundreds of
/// existing `crate::log!` call sites. A crate that forgets the constant does
/// not log untagged lines, it fails to compile.
// `clippy::crate_in_macro_def` warns about exactly the `crate::` below, on the
// assumption that it is a typo for `$crate::`. Here it is the point: `$crate`
// would resolve to `desert-core` and tag every line `core`, while `crate`
// resolves in the crate that invoked the macro and tags each line with the
// subsystem that wrote it. `tests/props.rs` is a separate crate and asserts
// which of the two this is.
#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write_tagged(crate::LOG_TAG, &format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `desert-core` invokes the macro too, so `crate::LOG_TAG` has to resolve
    /// here as well - this test is what makes the crate root's constant load
    /// bearing rather than decorative. It disables writing first (a one-way
    /// switch for the whole test binary, which is fine: nothing else in this
    /// crate logs), so it touches no file.
    #[test]
    fn the_macro_resolves_this_crates_tag() {
        disable();
        crate::log!("tagged {}", crate::LOG_TAG);
    }

    #[test]
    fn line_shapes() {
        assert_eq!(line(1.5, 42, None, "hello"), "[    1.500] [tid    42] hello");
        assert_eq!(
            line(1.5, 42, Some("looter"), "hello"),
            "[    1.500] [tid    42] [looter] hello",
            "the tag sits between the thread id and the message"
        );
        // the two columns are the widths the log has always used, so a tagged
        // and an untagged line still line up
        assert_eq!(
            line(1234.5678, 123_456, Some("g"), ""),
            "[ 1234.568] [tid 123456] [g] ",
            "an over-wide field grows rather than being cut"
        );
    }

    /// The lock-wait note is appended by `emit`, never by `line`: the line
    /// shape above is what `tests/props.rs` also pins down, and a diagnostic
    /// suffix must not move it. This holds both halves honest without touching
    /// the file system - the threshold the suffix triggers at, and the exact
    /// text it adds to a line that is otherwise unchanged.
    #[test]
    fn the_lock_wait_note_is_appended_after_the_line() {
        assert_eq!(WAIT_REPORT, Duration::from_millis(2));
        let l = line(1.5, 42, Some("core"), "hello");
        assert!(
            !l.contains("log-lock wait"),
            "a line carries no wait of its own"
        );
        let waited = Duration::from_millis(250);
        assert_eq!(
            format!("{l} [log-lock wait {:.0} ms]", waited.as_secs_f64() * 1000.0),
            "[    1.500] [tid    42] [core] hello [log-lock wait 250 ms]",
            "whole milliseconds, appended to the line the caller would have got"
        );
    }
}
