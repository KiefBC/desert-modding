//! The buff/stat census: the guarded reads and the log calls behind
//! `[Dispatch] DumpBuffs`.
//!
//! **Nothing in this file writes to the game.** There is no `safe::write`, no
//! hook, no code patch and no call into a game function; every foreign read
//! goes through [`crate::safe`], which turns an unmapped page into `None`
//! rather than into a crash to desktop. A wrong offset here prints a wrong
//! number and nothing else. That is not a promise about the future either: the
//! whole file is reads and `format!`, and the one funnel that writes anything
//! in this subsystem is `apply::Ctx::put`, which this module cannot reach.
//!
//! It answers one question, and it is a question about *money*: which of the
//! game's buffs touch a drop rate, a sell price, a crime price or a dispatch
//! reward rate, and which `statusinfo` row the name `AddMoneyDropRate`
//! actually resolves to. `buff.rs` is the offsets, the decoders, the line
//! rendering and the verdict - all natively tested - and this is the walk.
//!
//! # Nothing is found by an address
//!
//! The two managers come from the accessor template naming the table
//! (`sites.manager_slot(img, gimmick::BUFF_TABLE)`), the way every table in this
//! workspace is reached since the 0.2 milestone rule in `VERSIONING.md`. The
//! **class** of a `buffDataList` entry is read out of the object's own RTTI at
//! runtime, because the kind byte the game's constructor switch
//! (`FUN_141e7a1a0`) dispatches on comes off the stream and is not known to be
//! stored in the object at all.
//!
//! [`class_name`] is the **live counterpart of [`desert_core::rtti`]**. That
//! module documents the same MSVC layout and searches a file image for the
//! vtable of a named class; this walks the chain in the other direction -
//! object to vtable to CompleteObjectLocator to TypeDescriptor to name - in the
//! running process. It stays here while it has one caller, and
//! [`crate::buff::class_from_mangled`] is the pure half of it, so the affix
//! rules are tested natively.
//!
//! # Why it polls, and why the status half goes first
//!
//! Both tables are parsed into objects that live for the session, so a plain
//! pass over `manager+0x58` on this subsystem's own thread sees everything the
//! loader ever produced - the same reason `scan.rs` needs no hook. They load
//! lazily and independently, so each half gets its own settle-then-dump clock,
//! exactly like `scan::Watch`.
//!
//! The order inside a tick is **statusinfo first, then buffinfo**, and it is
//! load-bearing rather than tidy: a `VaryStatBuffData` entry names a
//! `statusinfo` **row index**, and whether that row is the money drop rate can
//! only be decided against the row the name `AddMoneyDropRate` resolved to. Run
//! the buff half first and every money line of that pass is missing its
//! `MONEY ` prefix and its stat name.

use std::collections::{BTreeMap, BTreeSet};

use crate::buff::{self, parsed, DataFields, Interest};
use crate::config::Config;
use crate::manager::{self, Slot};
use crate::module::MainModule;
use crate::node;
use crate::safe;
use crate::scan::{Budget, STABLE_POLLS};

/// The tag every line of the census carries.
///
/// **Its own tag, not `[dispatch]`.** The census is a one-launch diagnostic
/// whose output has nothing to do with the mission levers, and one `grep` for
/// `[buffs]` has to find all of it and none of the rest. `crate::log!` is
/// deliberately not used anywhere below for that reason.
const TAG: &str = "buffs";

/// One census line. The counterpart of `crate::log!`, tagged [`TAG`].
macro_rules! blog {
    ($($arg:tt)*) => { desert_core::log::write_tagged(TAG, &format!($($arg)*)) };
}

/// How many more records must have loaded before a table is dumped again.
///
/// The same floor `scan::REDUMP_GROWTH` uses and for the same reason: the
/// observed behaviour is one burst filling every slot, but a floor costs nothing
/// and stops a table that did trickle in from writing one dump per record.
const GROWTH: usize = 8;

/// How many class names the `classes:` line names before it summarises the
/// tail. Every class is still counted.
const CLASS_KEYS: usize = 40;

/// Read a string field: the pointer at `field_ptr_addr`, then the string object
/// behind it.
///
/// The layout is [`parsed::string`]. Every step is checked before the next one
/// is taken - field pointer plausible, `chars` plausible, `len` inside
/// [`buff::MAX_NAME`], every byte printable ASCII - because this is called on
/// an address computed from a record offset that might be wrong, and a name is
/// the one thing in the log a human will trust without checking. A garbled
/// name would be worse than none, so anything that fails answers `None` and the
/// line prints `?`.
///
/// `len == 0` is `""`, not a failure: an empty string points at a static
/// sentinel object the whole game shares.
fn read_string(field_ptr_addr: usize) -> Option<String> {
    if !manager::plausible(field_ptr_addr) {
        return None;
    }
    let obj = safe::read_ptr(field_ptr_addr).filter(|p| manager::plausible(*p))?;
    let len = safe::read::<i32>(obj.checked_add(parsed::string::LEN)?)?;
    if len == parsed::string::EMPTY_LEN {
        return Some(String::new());
    }
    let len = usize::try_from(len).ok().filter(|n| *n <= buff::MAX_NAME)?;
    let chars = safe::read_ptr(obj.checked_add(parsed::string::CHARS)?)
        .filter(|p| manager::plausible(*p))?;
    let mut bytes = vec![0u8; len];
    if !safe::read_into(chars, &mut bytes) {
        return None;
    }
    if !bytes.iter().all(|b| (0x20..0x7F).contains(b)) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// The class name of the object at `obj`, read out of its own RTTI.
///
/// `vtable = [obj]`, and it has to be **inside the main module** and at least 8
/// bytes above its base: the CompleteObjectLocator pointer lives at
/// `vtable - 8`, so a vtable at the very base of the image would have this
/// reading a pointer out of the DOS header. From there
/// `col = [vtable - 8]`, also in the module, `td_rva = [col + 12]`, and the
/// mangled name is the NUL-terminated string at `base + td_rva + 0x10`. The
/// affixes are checked by [`buff::class_from_mangled`], which is what makes a
/// wrong chain answer `None` instead of a plausible-looking string.
///
/// `cache` is per pass and keyed on the **vtable**, which is the address that
/// repeats: a table of thousands of entries holds a few dozen distinct classes,
/// so without it this would be four guarded reads per entry to learn what it
/// already knew. A `None` is cached too - a vtable that did not resolve will not
/// resolve on the next entry either. Past [`buff::MAX_CLASS_CACHE`] entries
/// nothing more is cached and the reads are simply made again, so the cap costs
/// time and never correctness.
fn class_name(
    module: &MainModule,
    obj: usize,
    cache: &mut BTreeMap<usize, Option<String>>,
) -> Option<String> {
    let vtable = safe::read_ptr(obj)?;
    if !module.contains(vtable) || vtable < module.base.checked_add(8)? {
        return None;
    }
    if let Some(hit) = cache.get(&vtable) {
        return hit.clone();
    }
    let name = read_class_name(module, vtable);
    if cache.len() < buff::MAX_CLASS_CACHE {
        cache.insert(vtable, name.clone());
    }
    name
}

/// The uncached half of [`class_name`]: the three chained reads and the affix
/// check. `vtable` has already been checked to be in the module and at least 8
/// bytes above its base.
fn read_class_name(module: &MainModule, vtable: usize) -> Option<String> {
    let col = safe::read_ptr(vtable.checked_sub(8)?)?;
    if !module.contains(col) {
        return None;
    }
    let td_rva = safe::read::<u32>(col.checked_add(12)?)?;
    let at = module.base.checked_add(td_rva as usize)?.checked_add(0x10)?;
    let mangled = safe::read_cstr(at, buff::MAX_NAME)?;
    buff::class_from_mangled(&mangled).map(str::to_string)
}

/// How many of `count` record slots the game has actually parsed into.
///
/// Deliberately cheap - one pointer read per slot, no field reads - because it
/// runs on every watch tick for the life of the process. `manager::record`
/// rather than `manager::slot` for the same reason `scan::count_loaded` uses it:
/// this counts non-null slots and never uses one as a base.
fn count_loaded(count: usize, records: usize) -> usize {
    (0..count).filter(|&idx| manager::record(records, idx).is_some()).count()
}

/// One settle-then-dump clock, the same shape `scan::Watch` keeps two of.
struct Clock {
    /// Loaded count at the previous poll; `usize::MAX` = no poll yet.
    last: usize,
    stable: u32,
    /// Loaded count at the last dump.
    dumped_at: usize,
}

impl Clock {
    fn new() -> Self {
        Clock { last: usize::MAX, stable: 0, dumped_at: 0 }
    }

    /// Fold one poll in and say whether a dump is owed: the count has held
    /// still for [`STABLE_POLLS`] polls and has grown by at least [`GROWTH`]
    /// since the last dump.
    fn owed(&mut self, loaded: usize) -> bool {
        if loaded == self.last {
            self.stable = self.stable.saturating_add(1);
        } else {
            self.last = loaded;
            self.stable = 0;
        }
        loaded > 0
            && self.stable >= STABLE_POLLS
            && loaded >= self.dumped_at.saturating_add(GROWTH)
    }
}

/// The census's state between ticks.
///
/// The two `logged` sets are why a pass that finds nothing new writes nothing:
/// the log is one file shared with the looter and the gatherer's hook, every
/// write takes the process-wide lock, and re-printing a census every time eight
/// more records appeared would be the loudest thing in the process for no new
/// information. The summaries still cover everything that was read.
pub struct Census {
    /// The `buffinfo` manager slot, or `None` if it did not resolve. A failure
    /// costs the census and nothing else.
    buff_slot: Option<usize>,
    /// The `statusinfo` manager slot, same.
    status_slot: Option<usize>,
    buff_clock: Clock,
    status_clock: Clock,
    /// `(record index, entry index)` of every interest line already written.
    logged_buffs: BTreeSet<(u32, u32)>,
    /// Record index of every `stat` line already written.
    logged_stats: BTreeSet<u32>,
    /// The row [`buff::MONEY_STAT`] resolved to, once the status half has seen
    /// it. **This is the answer the census exists to get**, and every `MONEY `
    /// prefix on a `VaryStat` line is a comparison against it.
    money_row: Option<u16>,
    /// The same for [`buff::EQUIP_DROP_STAT`].
    equip_row: Option<u16>,
    /// Row index -> name, for the well-known rows found so far, so a
    /// `VaryStat` line can print the name of the stat it names.
    well_known: BTreeMap<u16, &'static str>,
}

impl Census {
    pub fn new(buff_slot: Option<usize>, status_slot: Option<usize>) -> Self {
        Census {
            buff_slot,
            status_slot,
            buff_clock: Clock::new(),
            status_clock: Clock::new(),
            logged_buffs: BTreeSet::new(),
            logged_stats: BTreeSet::new(),
            money_row: None,
            equip_row: None,
            well_known: BTreeMap::new(),
        }
    }

    /// One poll of both tables. Called on the watch loop's own clock while
    /// `DumpBuffs=1`, and never otherwise.
    ///
    /// The status half runs first so the money row is known before any buff
    /// line is judged - see the module header. One [`Budget`] covers the whole
    /// tick, because `MaxLines` is documented as a ceiling on what **one pass**
    /// writes and the two halves are one pass.
    pub fn tick(&mut self, module: &MainModule, cfg: &Config) {
        let mut budget = Budget::new(cfg.max_lines);
        self.status_tick(cfg, &mut budget);
        self.buff_tick(module, cfg, &mut budget);
        if budget.capped() {
            blog!(
                "[cap] census lines stopped at MaxLines={} ({} written this pass); the summaries \
                 below cover everything that was read",
                cfg.max_lines,
                budget.used()
            );
        }
    }

    /// The `statusinfo` half: every loaded row's `_statType`, and a line for
    /// each row whose name the game itself looks up at startup.
    ///
    /// The 19 names are record names resolved by hash (`FUN_14250b680`), so
    /// this is a name-to-row lookup and not a guess about an enum. `_statType`
    /// is a byte whose meaning is unknown, which is why it gets a histogram: if
    /// it takes 19 values it may be the same enum the names are, and if it
    /// takes three it is something else entirely.
    fn status_tick(&mut self, cfg: &Config, budget: &mut Budget) {
        let Some(slot) = self.status_slot else {
            return;
        };
        let Some((declared, records)) = manager::view(slot) else {
            return;
        };
        let count = (declared as usize).min(buff::MAX_STATUS_RECORDS);
        let loaded = count_loaded(count, records);
        if !self.status_clock.owed(loaded) {
            return;
        }

        let t0 = std::time::Instant::now();
        let mut sum = buff::Summary::new();
        for idx in 0..count {
            let obj = match manager::slot(records, idx) {
                Slot::Loaded(p) => p,
                Slot::Empty => {
                    sum.record(false);
                    sum.skips.unloaded += 1;
                    continue;
                }
                Slot::Unreadable => {
                    sum.record(false);
                    sum.skips.record_read += 1;
                    continue;
                }
            };
            // One block read, decoded by the pure, natively tested decoder:
            // the offsets are then exercised on this machine by the same code
            // that runs in the game process.
            let mut raw = [0u8; parsed::status::RAW];
            if !safe::read_into(obj, &mut raw) {
                sum.record(false);
                sum.skips.head_read += 1;
                continue;
            }
            let Some(f) = buff::decode_status(&raw) else {
                sum.record(false);
                sum.skips.head_read += 1;
                continue;
            };
            sum.record(true);
            sum.stat_type(f.stat_type);

            let Some(name) = read_string(obj.wrapping_add(parsed::status::STRING_KEY)) else {
                sum.skips.string_read += 1;
                continue;
            };
            let Some(&known) = buff::WELL_KNOWN_STATS.iter().find(|w| **w == name) else {
                continue;
            };
            // The row index is the record index, and it is a `u16` everywhere
            // a buff names one. A table longer than 65535 rows would be a
            // misread count, so such a row is counted and not named.
            let Ok(row) = u16::try_from(idx) else {
                sum.skips.record_read += 1;
                continue;
            };
            sum.found_stat(known, row);
            self.well_known.insert(row, known);
            if known == buff::MONEY_STAT {
                self.money_row = Some(row);
            } else if known == buff::EQUIP_DROP_STAT {
                self.equip_row = Some(row);
            }

            let key = u32::try_from(idx).unwrap_or(u32::MAX);
            let fresh = !self.logged_stats.contains(&key);
            if fresh && budget.take() {
                blog!("{}", buff::stat_line(idx, Some(known), &f));
                if cfg.dump_raw && budget.take() {
                    blog!("rawstat idx={idx} {}", node::hex_line(&raw));
                }
            }
            // Remembered as logged only while the budget has refused nothing,
            // so a row the cap ate is printed on the next pass rather than
            // being marked done and never printed at all. Same rule `scan.rs`
            // applies to a mission and a reward row.
            if fresh && !budget.capped() {
                self.logged_stats.insert(key);
            }
        }

        blog!(
            "statusinfo: {count} records, {loaded} loaded, well-known rows found {}/{}: {}",
            sum.well_known_count(),
            buff::WELL_KNOWN_STATS.len(),
            sum.well_known_line()
        );
        blog!("statType histogram: {}", sum.stat_type_line());
        let missing = sum.missing_stats();
        if !missing.is_empty() {
            // Normal early in a session - rows load lazily - and a finding
            // later on: a name the game resolves at startup that no loaded row
            // carries is either a row this build renamed or a table that is
            // still filling.
            blog!("statusinfo missing: {}", missing.join(" "));
        }
        if !sum.skips.is_empty() {
            blog!("statusinfo skipped: {}", sum.skips.summary());
        }
        blog!(
            "stat dump finished in {:.0} ms ({loaded} of {count} rows parsed so far); nothing was \
             written to the game",
            t0.elapsed().as_secs_f64() * 1000.0
        );
        self.status_clock.dumped_at = loaded;
    }

    /// The `buffinfo` half: every record's `_buffDataList`, one line per entry
    /// whose class is one of the seven.
    fn buff_tick(&mut self, module: &MainModule, cfg: &Config, budget: &mut Budget) {
        let Some(slot) = self.buff_slot else {
            return;
        };
        let Some((declared, records)) = manager::view(slot) else {
            return;
        };
        let count = (declared as usize).min(buff::MAX_BUFF_RECORDS);
        let loaded = count_loaded(count, records);
        if !self.buff_clock.owed(loaded) {
            return;
        }

        let t0 = std::time::Instant::now();
        let mut sum = buff::Summary::new();
        let mut cache: BTreeMap<usize, Option<String>> = BTreeMap::new();
        let mut walked = 0usize;
        'records: for idx in 0..count {
            let obj = match manager::slot(records, idx) {
                Slot::Loaded(p) => p,
                Slot::Empty => {
                    sum.record(false);
                    sum.skips.unloaded += 1;
                    continue;
                }
                Slot::Unreadable => {
                    sum.record(false);
                    sum.skips.record_read += 1;
                    continue;
                }
            };
            let mut raw = [0u8; parsed::buff::RAW];
            if !safe::read_into(obj, &mut raw) {
                sum.record(false);
                sum.skips.head_read += 1;
                continue;
            }
            let Some(head) = buff::decode_buff_head(&raw) else {
                sum.record(false);
                sum.skips.head_read += 1;
                continue;
            };
            sum.record(true);
            let name = read_string(obj.wrapping_add(parsed::buff::STRING_KEY));
            if name.is_none() {
                sum.skips.string_read += 1;
            }

            let declared_entries = head.count as usize;
            if declared_entries > buff::MAX_ENTRIES_PER_RECORD {
                sum.skips.entries_capped += 1;
                // Named, so the log can say which records the census could not
                // read whole rather than only how many.
                sum.capped_record(name.as_deref().unwrap_or("?"), head.count);
            }
            let n = declared_entries.min(buff::MAX_ENTRIES_PER_RECORD);
            if n == 0 {
                continue;
            }
            // The entry array is a base for field reads, so it passes
            // `plausible` before anything is computed from it - and `n` is
            // capped, so `list + i * STRIDE` cannot wrap.
            let Some(list) = usize::try_from(head.list).ok().filter(|p| manager::plausible(*p))
            else {
                sum.skips.head_read += 1;
                continue;
            };

            for i in 0..n {
                // The bound that actually holds. The per-record cap multiplies
                // out against a misread record count; one misread `u32` would
                // otherwise wedge this thread for the rest of the session with
                // nothing in the log to say why. Same argument as
                // `node::MAX_TOTAL_OPS`.
                if walked >= buff::MAX_TOTAL_ENTRIES {
                    sum.skips.entry_budget += 1;
                    blog!(
                        "[cap] stopped after {} buffDataList entries (MAX_TOTAL_ENTRIES); a count \
                         somewhere is being misread - the summary below covers only what was \
                         walked",
                        buff::MAX_TOTAL_ENTRIES
                    );
                    break 'records;
                }
                walked += 1;

                let at = list + i * parsed::entry::STRIDE;
                let mut eraw = [0u8; parsed::entry::STRIDE];
                if !safe::read_into(at, &mut eraw) {
                    sum.skips.entry_read += 1;
                    continue;
                }
                let Some((entry_u32, data_ptr)) = buff::decode_entry(&eraw) else {
                    sum.skips.entry_read += 1;
                    continue;
                };
                if data_ptr == 0 {
                    // The normal reading for an absent BuffData: the factory
                    // `FUN_141e7e4c0` writes null when the stream's presence
                    // byte is 1.
                    sum.skips.null_entry += 1;
                    continue;
                }
                let Some(data) = usize::try_from(data_ptr).ok().filter(|p| manager::plausible(*p))
                else {
                    sum.skips.entry_read += 1;
                    continue;
                };
                // Counted as walked from here on, class or no class: the
                // verdict is about what share of walked entries resolved to a
                // `*BuffData`, so an entry whose RTTI would not read has to be
                // in the denominator.
                sum.entry();
                let Some(class) = class_name(module, data, &mut cache) else {
                    sum.skips.no_rtti += 1;
                    continue;
                };
                sum.class(&class);
                let Some(what) = Interest::from_class(&class) else {
                    continue;
                };
                sum.interest(what);

                let mut draw = [0u8; parsed::data::RAW];
                if !safe::read_into(data, &mut draw) {
                    sum.skips.entry_read += 1;
                    continue;
                }
                let (Some(base), Some(fields)) =
                    (buff::decode_base(&draw), buff::decode_fields(what, &draw))
                else {
                    sum.skips.entry_read += 1;
                    continue;
                };

                let mut refs = buff::Refs {
                    money_row: self.money_row,
                    equip_row: self.equip_row,
                    stat_name: None,
                };
                // Which lines are written, and why the two stat classes are
                // treated differently from the other five: a vanilla table
                // holds thousands of plain `VaryStat` entries, and printing all
                // of them would bury the handful that matter under the shared
                // log lock. Both are still counted in full, seen against
                // logged, so the log says what it left out.
                let write = match &fields {
                    // The four classes that name a statusinfo row: kind 5, and
                    // the three static-stat kinds 3, 4 and 7 the first launch
                    // showed the money stat belongs to.
                    DataFields::VaryStat { row, .. }
                    | DataFields::VaryStaticStat { row, .. }
                    | DataFields::VaryStaticStatLevel { row, .. }
                    | DataFields::VaryStaticStatRate { row, .. } => {
                        refs.stat_name = self.well_known.get(row).copied();
                        let money = self.money_row == Some(*row);
                        let equip = self.equip_row == Some(*row);
                        if money {
                            sum.money_ref();
                        }
                        if equip {
                            sum.equip_ref();
                        }
                        let write = money || equip;
                        sum.vary_stat(write);
                        write
                    }
                    DataFields::VaryStatRate { b90, .. } => {
                        let money = *b90 == buff::MONEY_STAT_POS;
                        let equip = *b90 == buff::EQUIP_DROP_STAT_POS;
                        if money {
                            sum.money_ref();
                        }
                        if equip {
                            sum.equip_ref();
                        }
                        let write = money || equip;
                        sum.vary_stat_rate(*b90, write);
                        write
                    }
                    // The other five are rare enough to print whole.
                    _ => true,
                };
                if !write {
                    continue;
                }

                // Both indices are inside caps far below `u32::MAX`, so the
                // fallback is unreachable arithmetic rather than a real case -
                // it is here because a `try_from` is the only spelling of this
                // that cannot panic.
                let seen = (
                    u32::try_from(idx).unwrap_or(u32::MAX),
                    u32::try_from(i).unwrap_or(u32::MAX),
                );
                let fresh = !self.logged_buffs.contains(&seen);
                if fresh && budget.take() {
                    blog!(
                        "{}",
                        buff::buff_line(&buff::BuffLine {
                            record: idx,
                            key: head.key,
                            name: name.as_deref(),
                            entry: i,
                            entry_u32,
                            what,
                            base: &base,
                            fields: &fields,
                            refs,
                        })
                    );
                    if cfg.dump_raw && budget.take() {
                        // The address is part of the dump, not decoration: two
                        // entries printing the same `at` is how a shared
                        // BuffData object would show itself.
                        blog!(
                            "rawbuff idx={idx} entry={i} at=0x{data:X} {}",
                            node::hex_line(&draw)
                        );
                    }
                }
                if fresh && !budget.capped() {
                    self.logged_buffs.insert(seen);
                }
            }
        }

        blog!("sum: {}", sum.counts_line());
        blog!("classes: {}", sum.classes_line(CLASS_KEYS));
        blog!("vsr90: {}", sum.vsr90_line());
        if let Some(line) = sum.capped_line() {
            blog!("capped records: {line}");
        }
        if !sum.skips.is_empty() {
            blog!("skipped: {}", sum.skips.summary());
        }
        blog!("{}", sum.verdict());
        blog!(
            "buff dump finished in {:.0} ms ({loaded} of {count} records parsed so far); nothing \
             was written to the game",
            t0.elapsed().as_secs_f64() * 1000.0
        );
        self.buff_clock.dumped_at = loaded;
    }
}
