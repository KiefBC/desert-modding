//! Desert Tooling - the one ASI plugin this workspace ships.
//!
//! It holds no mod policy at all. Everything the player sees is in one of the
//! three subsystem crates, each an rlib linked in here:
//!
//! | subsystem | crate | ini section | log tag |
//! | --- | --- | --- | --- |
//! | auto-loot | `desert-looter` | `[Looter]` | `[looter]` |
//! | gathering yields | `desert-gatherer` | `[Gatherer]` | `[gatherer]` |
//! | the in-game menu | `desert-overlay` | `[Overlay]` | `[overlay]` |
//!
//! What is left here is the plumbing that used to exist three times over, once
//! per shipped `.asi`: the single `DllMain`, the host-exe gate, `log::init` for
//! the one shared `DesertTooling.log`, seeding the one shared
//! `DesertTooling.ini` from all three schemas, the guard against a pre-merge
//! `.asi` still being loaded beside this one, and one thread per subsystem.
//!
//! The workspace's rules are hardest here, because this is the file the loader
//! calls: **no file I/O in `DllMain`** (the loader lock is held), **never
//! panic** (`panic = "abort"` makes one a crash to desktop), and a `// SAFETY:`
//! comment on every `unsafe` block.

// The shared logger, re-exported under the name every crate in this workspace
// uses, so `crate::log!` resolves here too. One `use` brings in both the module
// and the exported macro.
pub use desert_core::log;

/// The tag `crate::log!` puts on this crate's own lines. `desert_core`'s macro
/// expands to `write_tagged(crate::LOG_TAG, ...)` and resolves `crate::` at the
/// call site, so every crate that logs declares one of these; ours marks the
/// lines that belong to the merge plumbing rather than to a subsystem.
pub const LOG_TAG: &str = "tooling";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The one log, beside the game exe. Every subsystem writes into it, tagged.
pub const LOG_NAME: &str = "DesertTooling.log";

/// The one ini, beside the game exe. Every subsystem reads its own `[Section]`
/// of it and the overlay writes it.
pub const INI_NAME: &str = "DesertTooling.ini";

/// Only this process is the game. The ASI loader (`winmm.dll`) is also pulled
/// into helper processes started from `bin64` - `crashpad_handler.exe` - and
/// each of those would otherwise run its own copy of all three subsystems.
pub const GAME_EXE: &str = "CrimsonDesert.exe";

/// Every subsystem's settings, in the order the menu shows them and the order
/// the seeded ini writes them: Looter, Gatherer, Overlay.
///
/// This is the only place the three are named together. Adding a fourth
/// subsystem means adding its `config::schema()` here and starting its thread
/// in `entry::main_thread`; nothing in `desert-overlay` changes.
///
/// Always compiled, not `#[cfg(windows)]`: it is pure data, and the test at the
/// bottom of this file reads it on the native target to check the shipped
/// `DesertTooling.ini` against every subsystem's own parser.
pub fn sections() -> Vec<desert_core::schema::Section> {
    vec![
        desert_looter::config::schema(),
        desert_gatherer::config::schema(),
        desert_overlay::config::schema(),
    ]
}

#[cfg(windows)]
mod entry {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::{BOOL, HMODULE, TRUE};
    use windows_sys::Win32::System::LibraryLoader::{DisableThreadLibraryCalls, GetModuleHandleA};
    use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
    use windows_sys::Win32::System::Threading::{CreateThread, GetCurrentProcessId};

    use desert_core::{log, schema};

    /// The `HMODULE` the loader handed `DllMain`, on its way to the overlay's
    /// thread. hudhook wants it to register its own window hooks against.
    ///
    /// A raw pointer is not `Send`, so it cannot cross a thread boundary in a
    /// closure without this wrapper.
    #[derive(Clone, Copy)]
    struct ModuleHandle(*mut c_void);

    // SAFETY: the value is this very module's load address, which the loader
    // keeps mapped for the life of the process - an `.asi` is never unloaded -
    // and it is never dereferenced: it is only handed back to Win32 through
    // hudhook. A load address means the same thing on every thread, so moving
    // it between them cannot race with anything.
    unsafe impl Send for ModuleHandle {}

    impl ModuleHandle {
        /// The handle itself. A method rather than a field read at the call
        /// site on purpose: under edition 2021's disjoint closure capture,
        /// `move || ... hmodule.0 ...` captures the `*mut c_void` field alone
        /// rather than the wrapper, and `spawn`'s `F: Send` bound then rejects
        /// the closure. Going through a method captures `self`, which is the
        /// `Send` type. Nothing is silently defeated either way - the field
        /// version does not compile - but the error points at the closure, not
        /// at the field, so it is worth not writing.
        fn get(self) -> *mut c_void {
            self.0
        }
    }

    fn host_exe_name() -> String {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default()
    }

    /// Which of the three pre-merge plugins are loaded in this process right
    /// now, by the names the ASI loader gives them.
    ///
    /// `GetModuleHandleA` is an exact, case-insensitive match on a loaded
    /// module's base file name, and it neither loads anything nor takes a
    /// reference. The names passed are the full file names including the
    /// extension, so the "append `.dll` when there is no extension" rule never
    /// fires and nothing but a module actually called `DesertLooter.asi` can
    /// answer for `DesertLooter.asi`. `DesertTooling.asi` - this module - is not
    /// one of the three names, so we cannot find ourselves. A non-null handle
    /// therefore means a genuine second plugin is mapped in the process, and a
    /// null one means it is not: the check has no false positive to give.
    ///
    /// It matters that this is exact, because the consequence of the two hooks
    /// meeting is not a degraded plugin, it is a crash to desktop: an old
    /// `DesertLooter.asi` writes its trampoline over the same `area_sweep`
    /// prologue this one does, and the second patch overwrites the first one's
    /// stolen bytes.
    fn stale_plugins() -> Vec<&'static str> {
        // Byte literals with their own NUL: `GetModuleHandleA` wants a C
        // string, and building one at runtime would be an allocation that can
        // fail in a function whose whole job is to be trustworthy.
        const NAMES: [(&str, &[u8]); 3] = [
            ("DesertLooter.asi", b"DesertLooter.asi\0"),
            ("DesertGatherer.asi", b"DesertGatherer.asi\0"),
            ("DesertOverlay.asi", b"DesertOverlay.asi\0"),
        ];
        let mut found = Vec::new();
        for (name, c_name) in NAMES {
            // SAFETY: `c_name` is a `'static` byte literal ending in a NUL, so
            // the pointer is valid, aligned and NUL-terminated for the whole
            // call. `GetModuleHandleA` only looks the name up in this process's
            // own loaded-module list; it borrows nothing of ours, allocates
            // nothing, takes no reference on what it finds and may be called
            // from any thread.
            let handle = unsafe { GetModuleHandleA(c_name.as_ptr()) };
            if !handle.is_null() {
                found.push(name);
            }
        }
        found
    }

    /// Create `DesertTooling.ini` beside the exe, with every key of every
    /// section at its default, when the file is not there at all.
    ///
    /// This is the fallback for someone who dropped only the `.asi` into
    /// `bin64` without the commented template the release zip ships: instead of
    /// having nothing to edit, they get a bare file with every key present. An
    /// existing ini is never read, rewritten or replaced - the create is
    /// `create_new`, the OS's own atomic "only if absent", so a file that
    /// appears in the race window wins and the player's settings can never be
    /// clobbered.
    ///
    /// A failure is a WARN and nothing else: every subsystem's
    /// `Config::default()` already covers a missing ini, so the plugin behaves
    /// identically either way and the only thing lost is the file to edit.
    ///
    /// **This runs before any subsystem thread is started**, and that ordering
    /// is load bearing rather than tidy: the looter stamps its ini watermark
    /// from the file's modified time as it reads it, and a file that did not
    /// exist yet at that moment but appeared a millisecond later would make its
    /// once-a-second poll fire a spurious reload on its very first tick.
    ///
    /// On our own thread, after `log::init`, never in `DllMain`.
    fn seed_ini(sections: &[schema::Section]) {
        let banner = format!(
            "{} was not found, so Desert Tooling {} created it with every key at\n\
             its default. Edit it here or from the in-game menu (Insert); it is re-read\n\
             while the game runs, once a second.\n\
             \n\
             One file, one section per subsystem. Enabled, Debug and DryRun each exist under\n\
             more than one header and mean different things there, so a key belongs to the\n\
             [Section] it is under.\n\
             \n\
             The copy that ships in the release zip has a comment explaining every key. This\n\
             one is bare. Nothing here is regenerated: your edits survive, and a key you add\n\
             by hand is left alone.",
            crate::INI_NAME,
            crate::VERSION
        );
        match schema::create_ini_if_missing_all(
            &log::exe_dir(),
            crate::INI_NAME,
            sections,
            &banner,
        ) {
            schema::Written::Written => crate::log!(
                "[ini] {} was missing, so it was created with every key at its default",
                crate::INI_NAME
            ),
            // The overwhelmingly common case: the file is there. Each subsystem
            // is about to log the values it read out of it, so a line saying so
            // here is pure noise.
            schema::Written::Unchanged => {}
            schema::Written::Failed(why) => crate::log!(
                "[ini] WARN could not create {}: {why}; the defaults are in effect",
                crate::INI_NAME
            ),
        }
    }

    /// Runs on its own thread for the life of the process. `param` is the
    /// `HMODULE` `DllMain` was handed.
    unsafe extern "system" fn main_thread(param: *mut c_void) -> u32 {
        log::init(crate::LOG_NAME);
        // SAFETY: `GetCurrentProcessId` takes no arguments and only reads this
        // process's own PEB; it is sound to call from any thread.
        let pid = unsafe { GetCurrentProcessId() };
        crate::log!(
            "Desert Tooling {} loaded, pid {} (looter {}, gatherer {}, overlay {})",
            crate::VERSION,
            pid,
            desert_looter::VERSION,
            desert_gatherer::VERSION,
            desert_overlay::VERSION
        );

        // Before anything is installed, and before any subsystem thread exists.
        let stale = stale_plugins();
        if !stale.is_empty() {
            crate::log!(
                "REFUSING TO START: {} still loaded in this process alongside DesertTooling.asi",
                stale.join(" and ")
            );
            crate::log!(
                "Delete {} from bin64 and restart the game. Desert Tooling replaces all three \
                 of them, and two copies of one plugin patch the same game function: the second \
                 trampoline overwrites the first one's stolen bytes and the game crashes to \
                 desktop.",
                stale.join(", ")
            );
            crate::log!(
                "Nothing was hooked and no subsystem was started, so this session runs on the \
                 old plugin(s) alone - it is safe to play, it is just not this one."
            );
            return 0;
        }

        let sections = crate::sections();
        seed_ini(&sections);

        // One thread per subsystem, exactly reproducing the three-DLL timing
        // that was verified in game: each of the three used to get its own
        // thread out of its own DllMain, and they raced each other from there.
        //
        // The gatherer goes first and is never made to wait: it hooks the
        // gimmickinfo record loader, which the game reads while the level is
        // still loading, and the only safe moment to patch that prologue is
        // before any thread is executing it. The looter's own 20-second boot
        // grace is inside its start(), which is exactly why it must not be
        // ahead of the gatherer on one shared thread.
        spawn("gatherer", desert_gatherer::start);
        spawn("looter", desert_looter::start);
        let hmodule = ModuleHandle(param);
        spawn("overlay", move || {
            // The overlay is handed every section, its own included, and
            // returns once hudhook's DX12 hooks are applied.
            desert_overlay::start(hmodule.get(), sections);
        });

        // Nothing is joined. The three threads own the rest of the process's
        // life (two of them never return at all), an `.asi` is never unloaded,
        // so their code cannot go away underneath them, and this thread has
        // nothing left to do.
        0
    }

    /// Start one subsystem on a thread of its own. A thread that cannot be
    /// created costs that one subsystem and nothing else, so it is logged and
    /// stepped over rather than being allowed to take the others down with it.
    fn spawn<F: FnOnce() + Send + 'static>(name: &str, body: F) {
        match std::thread::Builder::new().name(format!("desert-{name}")).spawn(body) {
            // Deliberately dropped, which detaches: see main_thread.
            Ok(_handle) => {}
            Err(e) => crate::log!("[thread] WARN the {name} subsystem could not be started: {e}"),
        }
    }

    #[no_mangle]
    pub extern "system" fn DllMain(hinst: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
        if reason == DLL_PROCESS_ATTACH {
            // SAFETY: `hinst` is the handle the loader passed for this very
            // module, so it is a live HMODULE; the call only clears this
            // module's thread-attach notifications and is the documented thing
            // to do first under the loader lock.
            unsafe {
                DisableThreadLibraryCalls(hinst);
            }
            // No file I/O inside DllMain (loader lock), and none at all in
            // helper processes such as crashpad_handler.exe.
            let host = host_exe_name();
            if !host.eq_ignore_ascii_case(crate::GAME_EXE) {
                log::disable();
                return TRUE;
            }
            // SAFETY: every pointer argument is null except the entry point,
            // which is a `'static` function in this module, and the parameter,
            // which is `hinst` - this module's own load address, valid for the
            // life of the process and only ever passed back to Win32.
            // `main_thread` is what carries it to the overlay. An .asi is never
            // unloaded, so the thread cannot outlive its own code, and creating
            // a thread is one of the few things permitted while the loader lock
            // is held - it does not run until DllMain returns.
            unsafe {
                // Never do real work inside DllMain itself; hand off to a thread.
                CreateThread(
                    core::ptr::null(),
                    0,
                    Some(main_thread),
                    hinst.cast(),
                    0,
                    core::ptr::null_mut(),
                );
            }
        }
        TRUE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `DesertTooling.ini` that goes into the release zip, as bytes rather
    /// than as a claim about them.
    const SHIPPED_INI: &str = include_str!("../DesertTooling.ini");

    /// Every subsystem's parser must read the shipped template back as its own
    /// `Config::default()`, with nothing to warn about.
    ///
    /// Each subsystem used to carry this test against its own template, where
    /// it repeatedly caught a default changed in `config.rs` and not in the ini
    /// beside it. There is one shipped file now, so the test belongs to the
    /// crate that ships it - and it is stronger here, because the same text has
    /// to satisfy three independent parsers at once. An `Enabled` that landed
    /// under the wrong `[Section]` fails it twice over.
    #[test]
    fn the_shipped_ini_is_every_subsystem_at_its_defaults() {
        let (looter, warnings) = desert_looter::config::parse(SHIPPED_INI);
        assert!(warnings.is_empty(), "[Looter] warnings: {warnings:?}");
        assert_eq!(looter, desert_looter::config::Config::default());

        let (gatherer, warnings) = desert_gatherer::config::parse(SHIPPED_INI);
        assert!(warnings.is_empty(), "[Gatherer] warnings: {warnings:?}");
        assert_eq!(gatherer, desert_gatherer::config::Config::default());

        let (overlay, warnings) = desert_overlay::config::parse(SHIPPED_INI);
        assert!(warnings.is_empty(), "[Overlay] warnings: {warnings:?}");
        assert_eq!(overlay, desert_overlay::config::Config::default());
    }

    /// The template has to actually carry a header and every key for each
    /// section, or the test above would pass on an empty file: every parser
    /// falls back to its defaults when it finds nothing of its own.
    #[test]
    fn the_shipped_ini_has_a_header_and_every_key_for_each_section() {
        use desert_core::ini::{self, Line};

        for section in sections() {
            let header = format!("[{}]", section.ini_section);
            assert!(
                SHIPPED_INI.lines().any(|l| l.trim() == header),
                "{INI_NAME} has no {header} header"
            );
            let keys: Vec<&str> = ini::lines_in_section(SHIPPED_INI, &section.ini_section)
                .map(|line| match line {
                    Line::Pair(k, _) => k,
                    Line::Bad(why) => panic!("{INI_NAME} under {header}: {why}"),
                })
                .collect();
            for field in &section.fields {
                assert!(
                    keys.iter().any(|k| k.eq_ignore_ascii_case(&field.key)),
                    "{INI_NAME} has no {} under {header}",
                    field.key
                );
            }
        }
    }

    /// The menu order and the seeded ini's order are the same list, and it is
    /// the one the schemas' own `Order` asks for.
    #[test]
    fn the_sections_are_in_menu_order() {
        let names: Vec<String> = sections().into_iter().map(|s| s.ini_section).collect();
        assert_eq!(names, ["Looter", "Gatherer", "Overlay"]);
        let orders: Vec<i32> = sections().iter().map(|s| s.order).collect();
        assert!(orders.windows(2).all(|w| w[0] < w[1]), "orders: {orders:?}");
    }

    /// Every section names the one shared ini, and nothing names a file of its
    /// own any more.
    #[test]
    fn every_section_names_the_one_ini() {
        for section in sections() {
            assert_eq!(section.ini, INI_NAME);
        }
    }
}
