//! What the census and triage measured on the calibrated build, pinned.
//!
//! These are the numbers the plan's table was written from, re-measured by
//! the tool itself, and the `#[ignore]`d tests below assert them against the
//! installed exe (`just tools-check-game`). A game update fails every one of
//! them, which is the point: the failure output IS the new row of the table,
//! and the fix is to re-pin here and in `desert-core/tests/gimmick_real.rs`,
//! which asserts the same figures from the plugin's side. The two must agree.

/// The build the exe-side figures below were measured on. This is the code
/// side of the pin; `paths::CALIBRATED_BUILD` is the table side (the walk
/// calibrations re-pinned from a `DumpTable=1` launch) and lags it until that
/// dump has been read.
pub const BUILD: &str = "26361232";

/// Accessor sites per head encoding (`census::HEADS` order), and their sum.
pub const HEAD_COUNTS: [usize; 4] = [132, 20, 2, 0];
pub const SITES: usize = 154;

/// Every tail is `lea rcx,[r11+0x10]` on this build; none rejected.
pub const TAIL_CLASS: &str = "lea rcx,[r11+d8]";

/// Slot-load distance before the head, as `(distance, sites)`.
pub const SLOT_HIST: &[(u32, usize)] = &[(0x33, 130), (0x3B, 24)];
/// The bounds check register: `cmp edx,[rbx+8]` everywhere.
pub const SLOT_REG: &str = "edx";

/// Sites with a loader call before the next accessor, how many of those
/// targets open with `LOADER_PROLOGUE`, and the distance range from the
/// accessor's call to the grammar start. On this build every site has one;
/// `KeyMap` and `AutoSpawnFilterInfo` call a loader of a different shape
/// (counted, prologue absent). `characterinfo` and `tribeinfo` reach theirs
/// with the `48 89 D9` (`mov rcx,rbx`) encoding and no index load. History:
/// 153 and 151 on 26345091 (`characterinfo` lacked the call), 152 and 150 on
/// 26356046 (`characterinfo` and `tribeinfo` both had an accessor the
/// compiler rearranged past the grammar), 154 and 152 again on 26361232.
pub const LOADER_CALLS: usize = 154;
pub const LOADER_CALLS_WITH_PROLOGUE: usize = 152;
pub const LOADER_DIST: (u32, u32) = (0x74, 0x7E);

/// The `gimmickinfo` site. The code side did not move between 26345091,
/// 26356046 and 26361232; the slot (a data cell) moved once, from
/// `+0x71b6c48` on 26345091, and is the same on 26356046 and 26361232.
pub const GIMMICK_ACCESSOR: u32 = 0x3dd9a8;
pub const GIMMICK_SLOT_LOAD: u32 = 0x3dd975;
pub const GIMMICK_SLOT: u32 = 0x71b7bf8;
pub const GIMMICK_LOADER_CALL: u32 = 0x3dda44;
pub const GIMMICK_LOADER: u32 = 0x3d89a0;

/// The stream layout the `gimmickinfo` accessor fills in before its loader
/// call (`gimmick::StreamLayout::NEW`): buffer, size, cursor. Every site with
/// a loader call fills it the same way on this build. Through 25477059 it was
/// `(0x10, 0x18, 0x1c)`, the offsets the hook hard-coded until 26361232 moved
/// them.
pub const STREAM_LAYOUT: (u32, u32, u32) = (0x20, 0x28, 0x2c);
/// Sites whose stream fill parses, and to [`STREAM_LAYOUT`].
pub const STREAM_SITES: usize = 154;

/// Every signature in `reference-signatures.txt`, by its `# name - ...`
/// comment, and the RVA it hits on this build. `sigscan` prints FILE offsets;
/// `retarget triage` prints both. Every one is exactly 0x50 lower than on
/// 26356046.
pub const SIGNATURES: &[(&str, u32)] = &[
    ("alloc_event", 0x161af80),
    ("enqueue", 0x29a8650),
    ("area_sweep", 0x3b53c9f),
    ("own_check_site", 0x2d35865),
    ("get_pos", 0x19375c0),
    ("desc_mask+queue", 0x29a9aa3),
    ("interaction fn", 0x22d6a00),
    ("category fn", 0x22d6d20),
    ("catch_count", 0x2dd6728),
    ("loot_grant", 0x2e1f2f0),
    ("banking_deferred_gate", 0x2a8ab2c),
    ("banking_surplus_gate", 0x2a8a9d6),
];

/// `static-info-names.txt` on this build: 216 less `reviepointinfo`, plus the
/// six table names the new accessor sites pass.
pub const NAMES: usize = 221;
pub const NAME_GONE: &str = "reviepointinfo";

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::census;
    use crate::{tools_dir, Exe};
    use desert_tools::paths;

    fn exe() -> Exe {
        Exe::load(paths::game_exe(), false).expect("the game exe is installed")
    }

    fn census() -> census::Census {
        census::measure(&exe(), 0x400, &tools_dir().join("static-info-names.txt")).unwrap()
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn the_build_is_the_calibrated_one() {
        assert_eq!(paths::build_id().as_deref(), Some(BUILD), "appmanifest build id");
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn the_accessor_census_matches() {
        let c = census();
        assert_eq!(c.head_counts(), HEAD_COUNTS);
        assert_eq!(c.sites.len(), SITES);
        assert_eq!(c.tails(), BTreeMap::from([(TAIL_CLASS, SITES)]));
        assert!(c.rejected_tails().is_empty());
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn every_site_has_one_slot_load_at_a_known_distance() {
        let c = census();
        assert_eq!(c.slots_found(), SITES);
        assert_eq!(c.slot_ties(), 0);
        assert_eq!(c.slot_hist(), SLOT_HIST.iter().copied().collect::<BTreeMap<_, _>>());
        assert_eq!(c.slot_regs(), BTreeMap::from([(SLOT_REG, SITES)]));
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn the_loader_calls_are_where_the_plugin_looks() {
        let c = census();
        assert_eq!(c.loaders_found(), LOADER_CALLS);
        assert_eq!(c.loaders_with_prologue(), LOADER_CALLS_WITH_PROLOGUE);
        assert_eq!(c.loader_dist_range(), Some(LOADER_DIST));
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn the_gimmickinfo_site_is_pinned() {
        let c = census();
        let g = c.site_named("gimmickinfo").expect("an accessor passes gimmickinfo");
        assert_eq!(g.at, GIMMICK_ACCESSOR);
        assert_eq!(g.slot.as_ref().map(|s| (s.at, s.target)), Some((GIMMICK_SLOT_LOAD, GIMMICK_SLOT)));
        let l = g.loader.as_ref().expect("the gimmickinfo loader call");
        assert_eq!((l.call_at, l.target, l.prologue_ok), (GIMMICK_LOADER_CALL, GIMMICK_LOADER, true));
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn the_stream_fill_is_pinned() {
        let c = census();
        let (buf, size, cursor) = STREAM_LAYOUT;
        let want = census::StreamLayout { buf, size, cursor };
        let g = c.site_named("gimmickinfo").expect("an accessor passes gimmickinfo");
        assert_eq!(g.stream, Some(want), "the gimmickinfo stream fill");
        assert_eq!(c.stream_hist(), BTreeMap::from([(want, STREAM_SITES)]), "{}", c.stream_summary());
        assert!(c.streams_unparsed().is_empty(), "{}", c.stream_summary());
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn every_passed_name_is_in_the_list_and_the_list_is_current() {
        let c = census();
        assert!(c.unknown_names().is_empty(), "sites passing a name not in the list: {:?}", c.unknown_names().iter().map(|s| &s.name).collect::<Vec<_>>());
        assert_eq!(c.names_total, NAMES);
        let names = census::read_names(&tools_dir().join("static-info-names.txt")).unwrap();
        assert!(!names.iter().any(|n| n == NAME_GONE), "{NAME_GONE} left the exe on {BUILD}");
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn every_signature_hits_once_where_it_is_pinned() {
        let exe = exe();
        let rows = crate::triage::run(&exe, &tools_dir().join("reference-signatures.txt")).unwrap();
        assert_eq!(rows.len(), SIGNATURES.len(), "the signature list and this pin table differ in length");
        for (name, rva) in SIGNATURES {
            let row = rows.iter().find(|r| r.sig.name == *name).unwrap_or_else(|| panic!("{name} is not in the signature list"));
            assert_eq!(row.hits.len(), 1, "{name} hits {}", row.hits.len());
            assert_eq!(row.hits[0] as u32, *rva, "{name} moved to +{:#x}", row.hits[0]);
        }
    }
}
