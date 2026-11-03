//! Runs only on request: `cargo test --target x86_64-unknown-linux-gnu -- --ignored`
//! Needs the Steam install mounted at the path below.
//!
//! Only the looter's own signatures live here. The static-info accessor census
//! and the tables resolved through it moved to `crates/desert-core/tests/gimmick_real.rs`,
//! which is where the code they test lives; `just test-game` runs both files.

use desert_harvest::pattern::{self, Found, Pattern};
use desert_harvest::{pe, rtti};

const EXE: &str = "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe";

/// `desert_harvest::looter::game::SIGNATURES`, spelled out again because that
/// module is Windows-only and these tests run natively, with the RVA each one
/// hits on Steam build 26361232. All ten, `loot_grant` and `item_log` included
/// (`item_log` was added on 26361232 and has no older RVA). The patterns
/// were re-derived on 26345091, whose RVAs were, in this order: 0x161AFD0,
/// 0x29A8370, 0x3B539AF, 0x2D35585, 0x1937610, 0x29A97C3, 0x22D6A50, 0x22D6D70,
/// 0x2E1F010. On 26356046 they were 0x161AFD0, 0x29A86A0, 0x3B53D6F,
/// 0x2D358B5, 0x1937610, 0x29A9AF3, 0x22D6A50, 0x22D6D70, 0x2E1F340; every one
/// is exactly 0x50 lower on 26361232.
const SIGNATURES: &[(&str, &str, usize)] = &[
    ("alloc_event", "48 89 5C 24 ?? 48 89 74 24 ?? 4C 89 44 24 ?? 48 89 4C 24 ?? 57 48 83 EC 30 8B F2 E8 ?? ?? ?? ?? BA ?? ?? 00 00 65 48 8B 04 25 58 00 00 00", 0x161AF80),
    ("enqueue", "48 89 5C 24 08 48 89 74 24 20 57 48 83 EC 20 48 8B ?? 38 65 48 8B 04 25 58 00 00 00", 0x29A8650),
    ("area_sweep+0xF", "55 41 54 41 55 41 56 41 57 48 8B EC 48 83 EC 50 C5 F8 29 74 24 40 4C 8B ?? 48 8B ?? 48 8B ?? 44 8B", 0x3B53C9F),
    ("own_check_site", "48 8B 89 20 01 00 00 E8 ?? ?? ?? ?? 84 C0 74 05 40 B7 02", 0x2D35865),
    ("get_pos", "40 53 48 83 EC 50 48 8B 41 68 48 8B 88 ?? 01 00 00 48 8B 01", 0x19375C0),
    ("desc_mask+queue_site", "E8 ?? ?? ?? ?? 44 8B 05 ?? ?? ?? ?? 0F B7 54 24 ?? E8 ?? ?? ?? ?? 4C 8B 25", 0x29A9AA3),
    ("interaction_fn", "88 54 24 10 48 89 4C 24 08 53 55 56 57 41 54 41 55 41 56 41 57 48 83 EC 58 49 8B ?? 44 0F B6 ?? 4C 8B ??", 0x22D6A00),
    ("category_fn", "48 89 5C 24 18 88 54 24 10 55 56 57 41 54 41 55 41 56 41 57 48 8B EC 48 81 EC ?? ?? ?? ?? 41 8B D9 4D 8B F0 44 0F B6 EA 4C 8B F9", 0x22D6D20),
    ("loot_grant", "48 89 5C 24 08 4C 89 4C 24 20 48 89 54 24 10 55 56 57 41 54 41 55 41 56 41 57 48 8D AC 24 ?? ?? ?? ?? 48 81 EC ?? ?? ?? ?? 4D 8B E1 4C 8B EA 4C 8B F9", 0x2E1F2F0),
    ("item_log", "48 89 5C 24 10 48 89 6C 24 18 48 89 74 24 20 48 89 4C 24 08 57 41 54 41 55 41 56 41 57 48 83 EC ?? 41 0F B7 F9 45 0F B7 F8 4C 8B EA 4C 8B E1 80 BC 24 ?? ?? ?? ?? 00 0F 85 ?? ?? ?? ?? 45 8B CF 41 83 E9 07 0F 84 ?? ?? ?? ?? 41 83 E9 11 0F 84", 0x116A5B0),
];

/// The steal check `own_check_site`'s call lands on, and the context static
/// its `lea r9` loads 0xE bytes before the site, on 26361232 (`0x2801700` and
/// `0x58EB918` on 26345091; on 26356046 the check was `0x2801A10` and the
/// context the same as now).
const STEAL_CHECK_RVA: usize = 0x28019C0;
const STEAL_CTX_RVA: usize = 0x58EC918;

/// RVA of the catch-count patch site on 26361232, file offset `0x2DD5B28`
/// (`0x2DD6778`, file `0x2DD5B78`, on 26356046; `0x2DD6448`, file `0x2DD5848`,
/// on 26345091).
const CATCH_RVA: usize = 0x2DD6728;

/// Where the `call rel32` at `e8` lands, `None` if there is no call there.
fn rel32_target(img: &[u8], e8: usize) -> Option<usize> {
    if img.get(e8) != Some(&0xE8) {
        return None;
    }
    desert_core::pattern::resolve_rip(img, e8, 1, 5)
}

#[test]
#[ignore]
fn all_signatures_unique_and_actor_manager_vtable_found() {
    let file = std::fs::read(EXE).expect("game exe present");
    let h = pe::parse(&file).unwrap();
    let img = pe::file_to_image(&file).unwrap();
    assert_eq!(SIGNATURES.len(), 10);
    for (name, text, want) in SIGNATURES {
        let p = Pattern::parse(text).unwrap();
        match p.find_unique(&img) {
            Found::Unique(off) => {
                println!("{name:<22} = +0x{off:X}");
                assert_eq!(off, *want, "{name}");
            }
            other => panic!("{name}: {other:?}"),
        }
    }
    let vt = rtti::vtables_for_class(&img, h.image_base, ".?AVClientActorManager@pa@@");
    println!("ClientActorManager vtables: {vt:x?}");
    assert_eq!(vt.len(), 1);

    // The relations identity was confirmed by: the queue site's builder calls
    // `alloc_event` at +0x22 and `enqueue` at +0x98, and the steal-check call
    // and its context sit where `events::resolve` reads them.
    let rva = |n: &str| SIGNATURES.iter().find(|(s, _, _)| *s == n).unwrap().2;
    let q = rva("desc_mask+queue_site");
    assert_eq!(rel32_target(&img, q + 0x22), Some(rva("alloc_event")));
    assert_eq!(rel32_target(&img, q + 0x98), Some(rva("enqueue")));
    let own = rva("own_check_site");
    assert_eq!(rel32_target(&img, own + 7), Some(STEAL_CHECK_RVA));
    let lea = own - 0xE;
    assert_eq!(&img[lea..lea + 3], &[0x4C, 0x8D, 0x0D]);
    let disp = i32::from_le_bytes(img[lea + 3..lea + 7].try_into().unwrap());
    assert_eq!((lea as i64 + 7 + disp as i64) as usize, STEAL_CTX_RVA);

    // The gatherer's catch-count site is a code patch like the looter's hooks:
    // found once, at the pinned RVA, with the 13 bytes the stub replaces and
    // replays. `crates/desert-core/tests/gimmick_real.rs` checks it from the core side.
    use desert_core::creature::{CATCH_BYTES, CATCH_SITE, CATCH_STOLEN};
    let p = Pattern::parse(CATCH_SITE).unwrap();
    assert_eq!(p.find_unique(&img), Found::Unique(CATCH_RVA));
    assert_eq!(&img[CATCH_RVA..CATCH_RVA + CATCH_STOLEN], &CATCH_BYTES[..]);
    println!("catch site             = +0x{CATCH_RVA:X}");

    // The item-log gate replays the handler's first 15 bytes; they must be the
    // ones `looter::itemlog::ITEM_LOG_PROLOGUE` checks (that module is
    // Windows-only, so they are spelled again here).
    let il = rva("item_log");
    assert_eq!(
        &img[il..il + 15],
        &[0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x6C, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20]
    );
}

/// `game::resolve` scans for all ten in one walk of the image rather than one
/// walk each (1.56 s to 0.22 s measured on build 25246367). One walk is only
/// worth having if it answers what ten walks did, and a signature table is exactly
/// where that could quietly stop being true: the walk anchors each pattern on
/// its rarest byte, so which byte carries which signature depends on the image.
/// The test above is the oracle for the addresses; this is the oracle for the
/// scan that finds them.
#[test]
#[ignore]
fn the_one_pass_scan_agrees_with_one_pattern_at_a_time() {
    let file = std::fs::read(EXE).expect("game exe present");
    let img = pe::file_to_image(&file).unwrap();
    let pats: Vec<Pattern> =
        SIGNATURES.iter().filter_map(|(_, t, _)| Pattern::parse(t)).collect();
    assert_eq!(pats.len(), SIGNATURES.len(), "every signature parses");

    let one_at_a_time: Vec<Found> = pats.iter().map(|p| p.find_unique(&img)).collect();
    let one_pass = pattern::find_unique_multi(&pats, &img);

    for (i, (name, _, _)) in SIGNATURES.iter().enumerate() {
        assert_eq!(one_pass[i], one_at_a_time[i], "{name}");
        println!("{name:<22} {:?}", one_pass[i]);
    }
}
