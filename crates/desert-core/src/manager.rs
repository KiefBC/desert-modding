//! The static-info record manager: the offsets every parsed game table is
//! reached through, the pointer check in front of every use of one, and the
//! walk over its record entries.
//!
//! ```text
//! Manager (mgr), build 26636972
//!   +0x08  ptr   file context (handed to the accessor's file-load call)
//!   +0x20  ptr   entries: record_count entries of 24 bytes
//!   +0x28  u32   record_count
//!
//! Entry (entries + idx * 24)
//!   +0x04  u32   file offset of record idx in the table body
//!   +0x08  ptr   the parsed record object (null = not loaded yet)
//! ```
//!
//! Read off the getters and the loader on 26636972 (`cmp r8d,[rcx+0x28]`,
//! `mov rax,[rcx+0x20]; lea rdx,[r8+r8*2]; cmp qword [rax+rdx*8+8],0`, and the
//! accessor's `mov eax,[rdx+0x4]` that parks the stream cursor; see
//! `crate::gimmick`'s module docs). **Through 26480954 the layout was
//! different**: the count was a `u32` at `+0x08`, the file offsets a separate
//! table at `+0x28` (8-byte entries, the offset at `+4`), and the objects a
//! dense pointer array at `+0x58`. Every caller now goes through [`view_of`] /
//! [`slot`] / [`record`] / [`file_offset`], so no caller computes an entry
//! address itself and the next move is one file.
//!
//! Every static-info table the game parses - `gimmickinfo`, `iteminfo`,
//! `characterinfo`, `FactionNode`, `dropsetinfo` and the rest - is reached
//! through this one layout, and three subsystems walk it:
//! `desert_harvest::gatherer::hook` (the record loader's guard and `reapply`),
//! the looter's table reads, and `desert_dispatch`'s scan and apply passes.
//! What each subsystem *does* with a record stays in that subsystem - this is
//! plumbing only, and nothing here knows what a record means.
//!
//! [`plausible`] and [`entry_at`] are pure and always compiled, so they are
//! unit tested natively on Linux. The reads are `#[cfg(windows)]` because they
//! need a live process, and every one of them goes through [`crate::safe`], so
//! an unmapped page is a `None` rather than a fault.

/// Pointer to the entries array, at `manager+0x20`.
pub const MGR_ENTRIES: usize = 0x20;
/// `u32` record count, at `manager+0x28`.
pub const MGR_COUNT: usize = 0x28;
/// Bytes per entry.
pub const ENTRY_STRIDE: usize = 24;
/// `u32` file offset of the record in the table body, at `entry+0x04`.
pub const ENTRY_FILE_OFFSET: usize = 0x04;
/// The parsed record object, at `entry+0x08`; null = not loaded yet.
pub const ENTRY_OBJECT: usize = 0x08;

/// Is `p` an address that could be a live heap or image pointer?
///
/// Non-null, inside user address space, and far enough below the top of it that
/// adding a struct offset cannot wrap a `usize`. Everything read out of the
/// game and then used as a **base** goes through this first: a `safe` read of a
/// wild value is harmless but slow, and a value this rejects is never a pointer
/// the game wrote.
pub fn plausible(p: usize) -> bool {
    (0x10000..0x7FFF_FFFF_0000).contains(&p)
}

/// Address of entry `idx` of the entries array at `entries`, or `None` if
/// computing it would wrap. `entries + idx * ENTRY_STRIDE`, with `checked_*`
/// rather than `+`, which no caller could promise for itself.
pub fn entry_at(entries: usize, idx: usize) -> Option<usize> {
    idx.checked_mul(ENTRY_STRIDE).and_then(|off| entries.checked_add(off))
}

/// `(record count, entries array)` of the manager pointed to by `slot`, or
/// `None` while the table is not there yet.
///
/// `slot` is the image address that holds the manager pointer, so this is the
/// reading that starts from a static-info slot rather than from a manager
/// already in hand. The manager pointer and the entries array both have to be
/// [`plausible`]; the count is handed back raw, because what a caller may cap
/// it at is the caller's policy and not this module's.
#[cfg(windows)]
pub fn view(slot: usize) -> Option<(u32, usize)> {
    view_of(crate::safe::read_ptr(slot).filter(|m| plausible(*m))?)
}

/// The same pair, read out of a manager address the caller already holds.
///
/// Read **together and on every pass**, never cached: the count published when
/// a table first appears is not the count for the rest of the session - records
/// get registered later - and a cached array pointer beside a fresh count is
/// the worst of both.
#[cfg(windows)]
pub fn view_of(mgr: usize) -> Option<(u32, usize)> {
    let count = count(mgr)?;
    let entries = crate::safe::read_ptr(mgr.checked_add(MGR_ENTRIES)?).filter(|r| plausible(*r))?;
    Some((count, entries))
}

/// The record count alone, for the one caller (the record loader's hook) that
/// must read it before anything else.
#[cfg(windows)]
pub fn count(mgr: usize) -> Option<u32> {
    crate::safe::read::<u32>(mgr.checked_add(MGR_COUNT)?)
}

/// The three readings of one record entry's object pointer, kept apart
/// because every caller counts them differently.
///
/// A null pointer is the **normal** reading for a record the game has not
/// parsed yet - both tables `desert_dispatch` walks are lazy, and `dropsetinfo`
/// rows appear one at a time over a whole session - so flattening it into the
/// same answer as a failed read would turn an ordinary state into a reported
/// fault.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// The pointer would not read, or it holds something that is not a
    /// pointer the game wrote ([`plausible`] said no). Callers charge both to
    /// the same counter: either way the array is not what this build thinks
    /// it is.
    Unreadable,
    /// The pointer is null: nothing has been parsed into it yet.
    Empty,
    /// A record object, safe to use as a base for field reads.
    Loaded(usize),
}

/// Read record `idx`'s object pointer out of the entries array at `entries`.
///
/// The order of the three checks is read, then null, then [`plausible`]. An
/// entry address that would wrap answers [`Slot::Unreadable`] instead of
/// wrapping to some other address and reading that.
#[cfg(windows)]
pub fn slot(entries: usize, idx: usize) -> Slot {
    let Some(at) = entry_at(entries, idx).and_then(|e| e.checked_add(ENTRY_OBJECT)) else {
        return Slot::Unreadable;
    };
    match crate::safe::read::<usize>(at) {
        None => Slot::Unreadable,
        Some(0) => Slot::Empty,
        Some(p) if plausible(p) => Slot::Loaded(p),
        Some(_) => Slot::Unreadable,
    }
}

/// Is there an object in record entry `idx`? `Some(pointer)` when it holds a
/// non-null one, `None` when it is empty or would not read.
///
/// **This does not apply [`plausible`]**, and that is the difference from
/// [`slot`]: it exists for the cheap "has the table finished loading" counters,
/// which read one pointer per entry, compare it against zero and never use it
/// as a base. A caller that is going to read fields through the pointer wants
/// [`slot`] instead.
#[cfg(windows)]
pub fn record(entries: usize, idx: usize) -> Option<usize> {
    let at = entry_at(entries, idx)?.checked_add(ENTRY_OBJECT)?;
    crate::safe::read::<usize>(at).filter(|&p| p != 0)
}

/// Record `idx`'s file offset in the table body: where the loader parks the
/// stream cursor before parsing it, and, since records are contiguous in index
/// order, where record `idx - 1` ends.
#[cfg(windows)]
pub fn file_offset(entries: usize, idx: usize) -> Option<u32> {
    crate::safe::read::<u32>(entry_at(entries, idx)?.checked_add(ENTRY_FILE_OFFSET)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The band, at both ends. A null is not a pointer, the kernel half is not
    /// ours, and the top of the range is low enough that adding a struct offset
    /// to an accepted value cannot wrap.
    #[test]
    fn plausible_rejects_null_and_the_kernel_half() {
        assert!(!plausible(0));
        assert!(!plausible(0xFFFF), "below the lowest mappable page");
        assert!(plausible(0x10000), "the first accepted address");
        assert!(plausible(0x1_4000_0000), "the image");
        assert!(plausible(0x0020_0000_0000), "the heap");
        assert!(!plausible(0x7FFF_FFFF_0000), "the first rejected address");
        assert!(!plausible(usize::MAX));
    }

    /// Adding any offset a record walk can produce to an accepted pointer stays
    /// inside `usize`. This is why the predicate's ceiling is where it is.
    #[test]
    fn an_accepted_pointer_plus_a_record_offset_cannot_wrap() {
        for p in [0x10000usize, 0x1_4000_0000, 0x7FFF_FFFE_FFFF] {
            assert!(plausible(p));
            // The widest index any caller walks is a `u32` count, so the widest
            // byte offset is `u32::MAX * ENTRY_STRIDE`, plus the widest field.
            let widest = (u64::from(u32::MAX) * ENTRY_STRIDE as u64) as usize + 0x1000;
            assert!(p.checked_add(widest).is_some(), "0x{p:X} + 0x{widest:X} wrapped");
            assert!(entry_at(p, u32::MAX as usize).is_some());
        }
    }

    /// The stride and the two fields, which the getter's `lea rdx,[r8+r8*2];
    /// cmp qword [rax+rdx*8+8],0` and the accessor's `mov eax,[rdx+0x4]` pin.
    #[test]
    fn entries_are_24_bytes_with_the_offset_at_4_and_the_object_at_8() {
        assert_eq!(entry_at(0x1000, 0), Some(0x1000));
        assert_eq!(entry_at(0x1000, 1), Some(0x1018));
        assert_eq!(entry_at(0x1000, 0x2A), Some(0x1000 + 0x2A * 3 * 8));
        assert_eq!((ENTRY_FILE_OFFSET, ENTRY_OBJECT), (0x04, 0x08));
        assert_eq!(entry_at(usize::MAX - 8, 1), None);
        assert_eq!(entry_at(0x1000, usize::MAX), None);
    }

    /// The manager offsets, as the getter (`cmp r8d,[rcx+0x28]`, `mov
    /// rax,[rcx+0x20]`) reads them on 26636972. A typo in either is a table
    /// walked at the wrong place.
    #[test]
    fn the_manager_offsets_are_the_26636972_ones() {
        assert_eq!(MGR_ENTRIES, 0x20);
        assert_eq!(MGR_COUNT, 0x28);
    }
}
