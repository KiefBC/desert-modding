//! `[Gatherer] DumpTable=1`: the decision to copy the gimmickinfo table body
//! out of the loader's stream buffer, and the name of the file it lands in.
//!
//! The offline tools in `tools/` read the raw table body (`CD_DMM_TABLE`), and
//! until this key existed the only copy of it on disk was the one DMM keeps in
//! its backups folder - which DMM deletes and restores at will, so a tool run
//! could find it gone. The game itself parks the identical bytes in the
//! record loader's stream buffer while it loads the table (`hook.rs`'s module
//! docs, `Stream +0x10`), so the plugin can take its own copy there, once per
//! session, and write it beside the log.
//!
//! The copy happens in `hook.rs`, on a game thread, because the buffer is
//! freed a couple of seconds after the last load; the write happens in
//! `entry.rs`, on the plugin's own thread, because file IO does not belong on a
//! game thread. What is here is the part between the two that can be wrong
//! without the game being involved at all - whether to take the copy - and it
//! lives in its own module rather than in `hook.rs` because that whole module
//! is `#[cfg(windows)]` (it reads game memory), and this has to be testable on
//! the native Linux target like the rest of the pure logic.

/// What the dump is written as, beside `DesertTooling.log` in `bin64`. The
/// justfile and `tools/src/paths.rs` look for this exact name before falling
/// back to DMM's copy, so it is part of the interface and does not move.
pub const FILE_NAME: &str = "DesertTooling.gimmickinfo.bin";

/// The largest stream size a dump is taken for.
///
/// The body is ~22 MB today (`size=0x1562667` on the build this was written
/// against). A size beyond this means the layout model is wrong - `+0x18` is
/// not the size any more - not that the table grew tenfold, and allocating
/// whatever a misread field says on a game thread is how a diagnostic turns
/// into an out-of-memory abort.
pub const MAX_DUMP: usize = 256 << 20;

/// What the hook's first call does about the dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpDecision {
    /// `DumpTable=0`: nothing to do, and nothing to say.
    Skip,
    /// `DumpTable=1`, but the copy would not be trustworthy or the stream does
    /// not look like the one the layout model describes. The text is the
    /// log line's body after `[dump] `; the preceding `[gimmick] first call:`
    /// line carries the raw size, so it is not repeated here.
    Refuse(&'static str),
    /// Copy `size` bytes from the stream buffer.
    Take,
}

/// Decide whether the hook's first call copies the table body.
///
/// The `DryRun` requirement is what makes the copy vanilla. Several game
/// threads can be inside the hook at once for different records, and each one
/// that multiplies writes into this same buffer, so without `DryRun` some of
/// the records the copy reads could already be multiplied - and nothing in the
/// bytes would say which. Under `DryRun=1` the hook writes nothing at all, so
/// every byte in the buffer is the one the game read off disk.
///
/// `DryRun` is checked before the size so that a player who set only
/// `DumpTable=1` is told the thing they can fix, not a layout complaint that
/// might never have come up.
pub fn dump_decision(dump_table: bool, dry_run: bool, size: usize) -> DumpDecision {
    if !dump_table {
        return DumpDecision::Skip;
    }
    if !dry_run {
        return DumpDecision::Refuse("DumpTable=1 ignored: needs DryRun=1 so the copy is vanilla");
    }
    if size == 0 {
        return DumpDecision::Refuse(
            "table NOT dumped: the stream reports a size of 0, so the stream layout does not hold \
             on this build",
        );
    }
    if size > MAX_DUMP {
        return DumpDecision::Refuse(
            "table NOT dumped: the stream reports a size over 256 MiB against ~22 MB expected, so \
             the stream layout does not hold on this build",
        );
    }
    DumpDecision::Take
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default: whatever else is true, `DumpTable=0` never copies and
    /// never logs - not even for a size that would be refused.
    #[test]
    fn dump_table_off_is_skip_whatever_else_holds() {
        for dry in [false, true] {
            for size in [0, 1, 0x1562667, MAX_DUMP + 1, usize::MAX] {
                assert_eq!(dump_decision(false, dry, size), DumpDecision::Skip, "{dry} {size:#x}");
            }
        }
    }

    /// Without `DryRun` the copy could hold multiplied records, so it is
    /// refused - and it says so in the words the startup line uses, before any
    /// size complaint.
    #[test]
    fn without_dry_run_it_is_refused_before_the_size_is_looked_at() {
        for size in [0, 0x1562667, usize::MAX] {
            match dump_decision(true, false, size) {
                DumpDecision::Refuse(why) => {
                    assert!(why.contains("needs DryRun=1"), "{why}");
                    assert!(why.starts_with("DumpTable=1 ignored"), "{why}");
                }
                other => panic!("{size:#x}: {other:?}"),
            }
        }
    }

    #[test]
    fn a_zero_size_is_refused() {
        match dump_decision(true, true, 0) {
            DumpDecision::Refuse(why) => assert!(why.contains("size of 0"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    /// The cap is inclusive: exactly `MAX_DUMP` is taken, one byte more is not.
    #[test]
    fn a_size_past_the_cap_is_refused_and_the_cap_itself_is_taken() {
        assert_eq!(dump_decision(true, true, MAX_DUMP), DumpDecision::Take);
        for size in [MAX_DUMP + 1, u32::MAX as usize, usize::MAX] {
            match dump_decision(true, true, size) {
                DumpDecision::Refuse(why) => assert!(why.contains("256 MiB"), "{size:#x}: {why}"),
                other => panic!("{size:#x}: {other:?}"),
            }
        }
    }

    /// The real table, as the log reported it on 2026-09-24, and the smallest
    /// body there could be.
    #[test]
    fn a_plausible_size_under_dry_run_is_taken() {
        assert_eq!(dump_decision(true, true, 0x1562667), DumpDecision::Take);
        assert_eq!(dump_decision(true, true, 1), DumpDecision::Take);
    }

    /// Every refusal is a line of its own in the log, so none may be empty or
    /// carry the prefix the caller adds.
    #[test]
    fn refusal_texts_are_log_ready() {
        for (dry, size) in [(false, 1), (true, 0), (true, MAX_DUMP + 1)] {
            let DumpDecision::Refuse(why) = dump_decision(true, dry, size) else {
                panic!("{dry} {size:#x} should be refused");
            };
            assert!(!why.is_empty());
            assert!(!why.starts_with("[dump]"), "{why}");
        }
    }

    #[test]
    fn the_file_name_is_the_one_the_tools_look_for() {
        assert_eq!(FILE_NAME, "DesertTooling.gimmickinfo.bin");
    }
}
