//! Append-only text log beside the game exe: `[+seconds] [tid] message`.
//!
//! Rules that keep the game launching no matter what state the file is in:
//! - never panic: a log that cannot be opened is silently dropped;
//! - open-append-close per write, with full share flags, so no handle is held
//!   while Defender, an editor or a crash handler has the file;
//! - nothing here is called from DllMain (loader lock) or from helper processes.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub const LOG_NAME: &str = "DesertLooter.log";

static START: OnceLock<Instant> = OnceLock::new();
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

pub fn init() {
    START.get_or_init(Instant::now);
    PATH.get_or_init(|| exe_dir().join(LOG_NAME));
}

/// Stop writing for the rest of the process (used when the host is not the game).
pub fn disable() {
    DISABLED.store(true, Ordering::Relaxed);
}

#[cfg(windows)]
fn tid() -> u32 {
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

pub fn write(msg: &str) {
    if DISABLED.load(Ordering::Relaxed) {
        return;
    }
    init();
    let (Some(start), Some(path)) = (START.get(), PATH.get()) else { return };
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(mut f) = open_for_append(path) {
        let t = start.elapsed().as_secs_f64();
        let _ = writeln!(f, "[{t:9.3}] [tid {:5}] {msg}", tid());
        // dropped here: handle closed immediately
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}
