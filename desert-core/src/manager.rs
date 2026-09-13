//! The static-info record manager: the two offsets every parsed game table is
//! reached through, the pointer check in front of every use of one, and the
//! walk over the record-object array.
//!
//! ```text
//! Manager (mgr)
//!   +0x08  u32   record_count
//!   +0x58  ptr   array of record_count object pointers (null = not loaded)
//! ```
//!
//! Every static-info table the game parses - `gimmickinfo`, `FactionNode`,
//! `dropsetinfo` - is reached through that one pair, and two subsystems walk
//! it: `desert_gatherer::hook::reapply` over the gimmick records it remembered,
//! and `desert_dispatch`'s scan and apply passes over the faction-node and
//! dropset records. Both had their own copy of the offsets, their own
//! byte-identical `plausible`, and their own `(count, records)` read; this
//! module is the third copy not being written. What each subsystem *does* with
//! a record stays in that subsystem - this is plumbing only, and nothing here
//! knows what a record means.
//!
//! [`plausible`] is pure and always compiled, so it is unit tested natively on
//! Linux. The reads are `#[cfg(windows)]` because they need a live process, and
//! every one of them goes through [`crate::safe`], so an unmapped page is a
//! `None` rather than a fault.

/// `u32` record count, at `manager+0x08`.
pub const MGR_COUNT: usize = 0x08;
/// Array of record-object pointers, at `manager+0x58`. One `usize` per record,
/// indexed by record index; null = the game has not parsed that record yet.
pub const MGR_RECORDS: usize = 0x58;

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

/// `(record count, record-object array)` of the manager pointed to by `slot`,
/// or `None` while the table is not there yet.
///
/// `slot` is the image address that holds the manager pointer, so this is the
/// reading that starts from a static-info slot rather than from a manager
/// already in hand. The manager pointer and the record array both have to be
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
    let count = crate::safe::read::<u32>(mgr + MGR_COUNT)?;
    let records = crate::safe::read_ptr(mgr + MGR_RECORDS).filter(|r| plausible(*r))?;
    Some((count, records))
}

/// The three readings of one record slot, kept apart because every caller
/// counts them differently.
///
/// A null slot is the **normal** reading for a record the game has not parsed
/// yet - both tables `desert_dispatch` walks are lazy, and `dropsetinfo` rows
/// appear one at a time over a whole session - so flattening it into the same
/// answer as a failed read would turn an ordinary state into a reported fault.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// The slot would not read, or it holds something that is not a pointer the
    /// game wrote ([`plausible`] said no). Callers charge both to the same
    /// counter: either way the array is not what this build thinks it is.
    Unreadable,
    /// The slot is null: nothing has been parsed into it yet.
    Empty,
    /// A record object, safe to use as a base for field reads.
    Loaded(usize),
}

/// Read record slot `idx` of the array at `records`.
///
/// The order of the three checks is the order every hand-written copy of this
/// used: read, then null, then [`plausible`]. `records + idx * 8` is computed
/// with `checked_*` rather than `+`, which the callers could not all promise
/// for themselves - a slot address that would wrap answers [`Slot::Unreadable`]
/// instead of wrapping to some other address and reading that.
#[cfg(windows)]
pub fn slot(records: usize, idx: usize) -> Slot {
    let Some(at) = idx.checked_mul(8).and_then(|off| records.checked_add(off)) else {
        return Slot::Unreadable;
    };
    match crate::safe::read::<usize>(at) {
        None => Slot::Unreadable,
        Some(0) => Slot::Empty,
        Some(p) if plausible(p) => Slot::Loaded(p),
        Some(_) => Slot::Unreadable,
    }
}

/// Is there an object in record slot `idx`? `Some(pointer)` when the slot holds
/// a non-null one, `None` when it is empty or would not read.
///
/// **This does not apply [`plausible`]**, and that is the difference from
/// [`slot`]: it exists for the cheap "has the table finished loading" counters,
/// which read one pointer per slot, compare it against zero and never use it as
/// a base. A caller that is going to read fields through the pointer wants
/// [`slot`] instead.
#[cfg(windows)]
pub fn record(records: usize, idx: usize) -> Option<usize> {
    let at = idx.checked_mul(8).and_then(|off| records.checked_add(off))?;
    crate::safe::read::<usize>(at).filter(|&p| p != 0)
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
            // byte offset is `u32::MAX * 8`, plus the widest struct offset.
            let widest = (u64::from(u32::MAX) * 8) as usize + 0x1000;
            assert!(p.checked_add(widest).is_some(), "0x{p:X} + 0x{widest:X} wrapped");
        }
    }

    /// The offsets themselves. Both were read off the same decompiled manager
    /// in both subsystems before this module existed, and a typo in either is a
    /// table walked at the wrong stride.
    #[test]
    fn the_manager_offsets_are_the_ones_both_subsystems_had() {
        assert_eq!(MGR_COUNT, 0x08);
        assert_eq!(MGR_RECORDS, 0x58);
    }
}
