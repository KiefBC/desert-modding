//! What the census and triage measured on the calibrated build, pinned.
//!
//! These are the numbers the plan's table was written from, re-measured by
//! the tool itself, and the `#[ignore]`d tests below assert them against the
//! installed exe (`just tools-check-game`). A game update fails every one of
//! them, which is the point: the failure output IS the new row of the table,
//! and the fix is to re-pin here and in `crates/desert-core/tests/gimmick_real.rs`,
//! which asserts the same figures from the plugin's side. The two must agree.

/// The build the exe-side figures below were measured on. This is the code
/// side of the pin; `paths::CALIBRATED_BUILD` is the table side (the walk
/// calibrations re-pinned from the extracted archive tables) and lags it
/// until those have been re-derived.
pub const BUILD: &str = "26636972";

/// Getters: slot loads `mov rcx,[rip+slot]; cmp r8d,[rcx+0x28]` whose call
/// reaches an accessor. 26636972 is the first build with them; through
/// 26480954 the slot load (`mov rbx,[rip+slot]; cmp edx,[rbx+8]`, 0x33 or
/// 0x3B before the head on 26345091..26480954) sat in the accessor's own
/// function.
pub const GETTERS: usize = 159;
pub const GETTER_REGS: (&str, &str) = ("rcx", "r8d");
/// Slot load to the accessor call, as `(distance, getters)`.
pub const GETTER_CALL_DISTS: &[(u32, usize)] = &[(0x26, 1), (0x2B, 1), (0x2D, 3), (0x2E, 6), (0x31, 3), (0x35, 3), (0x37, 142)];
/// Getter prologues by `census::GetterPrologue::label`: the bytes from the
/// function start to the slot load, or `inlined`.
pub const GETTER_PROLOGUES: &[(&str, usize)] = &[
    ("inlined", 7),
    ("standalone 48 83 EC 28 44 0F B6 01", 1),
    ("standalone 48 83 EC 28 44 0F B7 01", 120),
    ("standalone 48 83 EC 28 44 8B 01", 24),
    ("standalone 48 89 5C 24 08 48 89 54 24 10 57 48 83 EC 30 48 8B DA C7 44 24 20 00 00 00 00 45 0F B7 00", 7),
];
/// The manager layout the getter bodies use, as
/// `(count, entries, stride, object, getters)`: record count at +0x28, the
/// entries array at +0x20, 24-byte entries with the parsed object at +8;
/// `AIEventTableInfo`, `NpcFriendlyInfo` and `characterappearanceindexinfo`
/// (four getters) have 32-byte entries with the object at +0x10. Through
/// 26480954: count +0x08, an offsets table at +0x28 (8-byte entries, file
/// offset at +4) and an object-pointer array at +0x58.
pub const MANAGER_LAYOUTS: &[(u8, u8, u32, u8, usize)] = &[(0x28, 0x20, 24, 0x08, 155), (0x28, 0x20, 32, 0x10, 4)];

/// Accessor sites per head encoding (`census::HEADS` order), and their sum.
/// One per table name; 149 through 25477059, 154 on 26345091..26480954
/// (`132 + 20 + 2 + 0` with the old `mov rdx,[rbx+0x10]` heads).
pub const HEAD_COUNTS: [usize; 4] = [131, 20, 0, 0];
pub const SITES: usize = 151;
/// Head offset into the accessor function, as `(offset, accessors)`.
pub const HEAD_OFFSETS: &[(u32, usize)] = &[(0x30, 24), (0x31, 127)];
/// Getters per accessor, as `(getters, accessors)`.
pub const GETTERS_PER_SITE: &[(usize, usize)] = &[(1, 146), (2, 3), (3, 1), (4, 1)];

/// Every tail is `lea rcx,[rbp+0x40]` on this build; none rejected. It was
/// `lea rcx,[rsp+d8]` through 25477059 and `lea rcx,[r11+0x10]` on
/// 26345091..26480954.
pub const TAIL_CLASS: &str = "lea rcx,[rbp+d8]";

/// Sites with a loader call, how many of those targets open with the 15-byte
/// `LOADER_PROLOGUE`, the distinct target prologues, and the grammar start's
/// distance from the head and from the accessor's call. Every site has one.
/// History (from the accessor's call, 12-byte prologue): 153/151 on
/// 26345091, 152/150 on 26356046, 154/152 on 26361232..26480954 at
/// 0x74..0x7E.
pub const LOADER_CALLS: usize = 151;
pub const LOADER_CALLS_WITH_PROLOGUE: usize = 6;
pub const LOADER_PROLOGUES: &[(&str, usize)] = &[
    ("E9 jmp", 2),
    ("48 89 5C 24 20 48 89 54 24 10 48 89 4C 24 08", 6),
    ("48 89 5C 24 20 48 89 54 24 10 55 56 57 41 54", 143),
];
pub const LOADER_HEAD_DISTS: &[(u32, usize)] = &[(0x8C, 3), (0x8D, 148)];
pub const LOADER_DIST: (u32, u32) = (0x7A, 0x7B);

/// The `gimmickinfo` chain on 26636972. On 26480954 the accessor head was
/// `+0x3de288` with its slot load at `+0x3de255` (slot `+0x71c10a8`), loader
/// call `+0x3de324` and loader `+0x3d9280`.
pub const GIMMICK_GETTER: u32 = 0x3dac70;
pub const GIMMICK_SLOT_LOAD: u32 = 0x3dac78;
pub const GIMMICK_SLOT: u32 = 0x7246398;
pub const GIMMICK_ACCESSOR_FN: u32 = 0x3e0cc0;
pub const GIMMICK_ACCESSOR: u32 = 0x3e0cf1;
pub const GIMMICK_LOADER_CALL: u32 = 0x3e0d8c;
pub const GIMMICK_LOADER: u32 = 0x3dc7f0;

/// The stream layout the `gimmickinfo` accessor fills in before its loader
/// call (`gimmick::StreamLayout::NEW`): buffer, size, cursor, now stored
/// rbp-relative. Every site fills it the same way on this build. Through
/// 25477059 it was `(0x10, 0x18, 0x1c)`, the offsets the hook hard-coded
/// until 26361232 moved them.
pub const STREAM_LAYOUT: (u32, u32, u32) = (0x20, 0x28, 0x2c);
/// Sites whose stream fill parses, and to [`STREAM_LAYOUT`].
pub const STREAM_SITES: usize = 151;

/// Every signature in `reference-signatures.txt`, by its `# name - ...`
/// comment, and the RVA it hits on this build. `sigscan` prints FILE offsets;
/// `retarget triage` prints both. On 26361232 every one was exactly 0x50
/// lower than on 26356046; on 26423880 they moved up by amounts from +0x1250
/// (`item_log`) to +0x5a60 (`area_sweep`), not one shift. On 26480954 they
/// were, in this order, 0x161ccb0, 0x29ace30, 0x3b5a4af, 0x2d3bd15,
/// 0x19399b0, 0x29ae283, 0x22da260, 0x22da580, 0x2ddcbd8, 0x2e25810,
/// 0x116bfb0, 0x2a90efc and 0x2a90da6.
pub const SIGNATURES: &[(&str, u32)] = &[
    ("alloc_event", 0x165d9c0),
    ("enqueue", 0x2a0f460),
    ("area_sweep", 0x3bd3faf),
    ("own_check_site", 0x2da26da),
    ("get_pos", 0x199f370),
    ("desc_mask+queue", 0x2a108b3),
    ("interaction fn", 0x234aba0),
    ("category fn", 0x234aec0),
    ("catch_count", 0x2e43af8),
    ("loot_grant", 0x2e8cfc0),
    ("item_log", 0x11a43c0),
    ("banking_deferred_gate", 0x2af501a),
    ("banking_surplus_gate", 0x2af4ec5),
];

/// `static-info-names.txt` on this build: 221 on 26345091..26480954 (216
/// less `reviepointinfo`, plus six table names the new accessor sites
/// passed), less the eight names of the five types 26636972 removed, plus
/// the four of the two it added.
pub const NAMES: usize = 217;
/// Names that left the exe and must stay out of the list.
pub const NAMES_GONE: &[&str] = &[
    "reviepointinfo",
    "DetectDetailInfo",
    "GameConditionInfo",
    "ItemMeshGroupInfo",
    "LightDataInfo",
    "TextGuideInfo",
    "detectdetailinfo",
    "itemmeshgroupinfo",
    "textguideInfo",
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    use super::*;
    use crate::census;
    use crate::{tools_dir, Exe};
    use desert_tools::paths;

    /// One exe and one census for every test: each load maps the whole
    /// image, and nine of them at once in parallel test threads ran WSL out
    /// of memory (`Cannot allocate memory`).
    fn exe() -> &'static Exe {
        static EXE: OnceLock<Exe> = OnceLock::new();
        EXE.get_or_init(|| Exe::load(paths::game_exe(), false).expect("the game exe is installed"))
    }

    fn census() -> &'static census::Census {
        static CENSUS: OnceLock<census::Census> = OnceLock::new();
        CENSUS.get_or_init(|| census::measure(exe(), 0x400, &tools_dir().join("static-info-names.txt")).unwrap())
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
        assert_eq!(c.fns_from_getters(), SITES, "every accessor's function start comes from a getter's call");
        assert_eq!(c.head_offsets(), HEAD_OFFSETS.iter().copied().collect::<BTreeMap<_, _>>());
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn every_accessor_has_getters_agreeing_on_one_slot() {
        let c = census();
        assert_eq!(c.getters.len(), GETTERS);
        assert_eq!(c.getter_regs(), BTreeMap::from([(GETTER_REGS, GETTERS)]));
        assert_eq!(c.getter_call_dists(), GETTER_CALL_DISTS.iter().copied().collect::<BTreeMap<_, _>>());
        let prologues: BTreeMap<String, usize> = GETTER_PROLOGUES.iter().map(|&(p, n)| (p.to_string(), n)).collect();
        assert_eq!(c.getter_prologues(), prologues);
        assert!(c.sites_without_getter().is_empty());
        assert!(c.slot_disagreements().is_empty());
        assert!(c.names_without_one_slot().is_empty(), "{:?}", c.names_without_one_slot());
        assert_eq!(c.distinct_slots(), SITES);
        assert_eq!(c.getters_per_site(), GETTERS_PER_SITE.iter().copied().collect::<BTreeMap<_, _>>());
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn the_getters_read_the_pinned_manager_layout() {
        let c = census();
        let want: BTreeMap<census::ManagerLayout, usize> = MANAGER_LAYOUTS
            .iter()
            .map(|&(count, entries, stride, object, n)| {
                (census::ManagerLayout { count, entries: Some(entries), stride: Some(stride), object: Some(object) }, n)
            })
            .collect();
        assert_eq!(c.manager_layouts(), want);
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn the_loader_calls_are_where_the_plugin_looks() {
        let c = census();
        assert_eq!(c.loaders_found(), LOADER_CALLS);
        assert_eq!(c.loaders_with_prologue(), LOADER_CALLS_WITH_PROLOGUE);
        let prologues: BTreeMap<String, usize> = c.loader_prologues().into_iter().map(|(p, n)| (p.label(), n)).collect();
        assert_eq!(prologues, LOADER_PROLOGUES.iter().map(|&(p, n)| (p.to_string(), n)).collect::<BTreeMap<_, _>>());
        assert_eq!(c.loader_head_dist_hist(), LOADER_HEAD_DISTS.iter().copied().collect::<BTreeMap<_, _>>());
        assert_eq!(c.loader_dist_range(), Some(LOADER_DIST));
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn the_gimmickinfo_site_is_pinned() {
        let c = census();
        let g = c.site_named("gimmickinfo").expect("an accessor passes gimmickinfo");
        assert_eq!((g.fn_start, g.at), (Some(GIMMICK_ACCESSOR_FN), GIMMICK_ACCESSOR));
        assert_eq!(g.slot(), Some(GIMMICK_SLOT));
        let getters: Vec<(Option<u32>, u32, u32)> = c.getters_of("gimmickinfo").iter().map(|x| (x.fn_start, x.at, x.slot)).collect();
        assert_eq!(getters, vec![(Some(GIMMICK_GETTER), GIMMICK_SLOT_LOAD, GIMMICK_SLOT)]);
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
        for gone in NAMES_GONE {
            assert!(!names.iter().any(|n| n == gone), "{gone} left the exe; it is back in the list");
        }
    }

    #[test]
    #[ignore = "needs the game exe"]
    fn every_signature_hits_once_where_it_is_pinned() {
        let exe = exe();
        let rows = crate::triage::run(exe, &tools_dir().join("reference-signatures.txt")).unwrap();
        assert_eq!(rows.len(), SIGNATURES.len(), "the signature list and this pin table differ in length");
        for (name, rva) in SIGNATURES {
            let row = rows.iter().find(|r| r.sig.name == *name).unwrap_or_else(|| panic!("{name} is not in the signature list"));
            assert_eq!(row.hits.len(), 1, "{name} hits {}", row.hits.len());
            assert_eq!(row.hits[0] as u32, *rva, "{name} moved to +{:#x}", row.hits[0]);
        }
    }
}
