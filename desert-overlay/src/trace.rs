//! Forward hudhook's `tracing` output into `DesertOverlay.log`.
//!
//! hudhook reports everything through `tracing` - which swapchain it found,
//! which vtable index it hooked, and, crucially, *why* it gave up. With no
//! subscriber installed those records go nowhere, and a hook that silently
//! did nothing is the hardest kind of failure to diagnose from a user's bug
//! report. This is the smallest thing that fixes that: a `Subscriber` that
//! keeps no spans and does nothing but format an event's fields into one line
//! and hand it to `desert_core::log`.
//!
//! WARN and ERROR always; `Debug=1` in `DesertOverlay.ini` forwards *every*
//! level, INFO, DEBUG and TRACE included. hudhook's DEBUG and TRACE records are
//! the only place the order of the hook calls shows up - which swapchain each
//! Present ran on, when a pipeline was reset, which trampoline was entered -
//! and that sequence is what a launch failure has to be read from. It is a
//! firehose: hudhook traces at least once per presented frame, and the log is
//! an open-append-close file per write, so [`MAX_LINES`] caps how much of it
//! can ever reach the disk.
//!
//! `tracing` is not a dependency of this crate: hudhook re-exports it
//! (`pub use tracing;`), so the version is the one hudhook itself uses and
//! cannot drift.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};

use hudhook::tracing::field::{Field, Visit};
use hudhook::tracing::{span, Event, Level, Metadata, Subscriber};

use desert_core::log;

/// How many lines the bridge will ever forward in one game session.
///
/// At `Debug=1` hudhook writes several TRACE records per frame, so an hour of
/// play would otherwise be gigabytes. 20000 lines is minutes of frames, far
/// more than the launch sequence a bug report needs, and once they are spent
/// the bridge stops writing for the rest of the process. Restarting the game
/// is what resets it; the counter is per process, like the log itself.
const MAX_LINES: usize = 20_000;

/// Lines handed to [`log::write`] so far, including the ones dropped after the
/// cap (the count only ever grows, and one `usize` cannot wrap in a session).
static FORWARDED: AtomicUsize = AtomicUsize::new(0);

struct HudhookLog {
    /// `Debug=1`: forward every level, not just WARN and ERROR.
    verbose: bool,
}

impl Subscriber for HudhookLog {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        match *metadata.level() {
            Level::ERROR | Level::WARN => true,
            _ => self.verbose,
        }
    }

    /// No span storage: hudhook's spans carry no information the log needs,
    /// and every span gets the same id so nothing has to be allocated or
    /// reference-counted on a render thread.
    fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}

    fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

    /// The cap is enforced here rather than in [`Self::enabled`] because
    /// `tracing` caches a callsite's enabled-ness the first time it sees it and
    /// then stops asking; this runs for every record that gets through.
    fn event(&self, event: &Event<'_>) {
        let meta = event.metadata();
        // WARN and ERROR are never capped: they are the lines a bug report
        // needs, and hudhook emits them rarely. The cap exists for the
        // Debug=1 firehose of INFO/DEBUG/TRACE, several lines per frame.
        let capped = !matches!(*meta.level(), Level::ERROR | Level::WARN);
        let forwarded = if capped { FORWARDED.fetch_add(1, Ordering::Relaxed) } else { 0 };
        if capped && forwarded >= MAX_LINES {
            return;
        }

        let mut fields = Fields(String::new());
        event.record(&mut fields);
        log::write(&format!("[hudhook] {} {}: {}", meta.level(), meta.target(), fields.0));

        if capped && forwarded + 1 == MAX_LINES {
            log::write(&format!(
                "[hudhook] {MAX_LINES} lines forwarded; only warnings and errors from hudhook \
                 will be logged from here on"
            ));
        }
    }

    fn enter(&self, _span: &span::Id) {}

    fn exit(&self, _span: &span::Id) {}
}

/// Flattens an event's fields into `message key=value key=value`.
struct Fields(String);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        // The `message` field is the format string of `error!("...")`, so it
        // reads best bare; everything else is named.
        let _ = if field.name() == "message" {
            write!(self.0, "{value:?}")
        } else {
            write!(self.0, "{}={value:?}", field.name())
        };
    }
}

/// Install the bridge as the process-wide default subscriber.
///
/// Returns false if something else got there first, which is not worth
/// worrying about: nothing else in this process installs one, and if a future
/// plugin does, the loser simply logs nothing.
pub fn install(verbose: bool) -> bool {
    hudhook::tracing::subscriber::set_global_default(HudhookLog { verbose }).is_ok()
}
