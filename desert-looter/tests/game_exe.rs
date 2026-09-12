//! Runs only on request: `cargo test --target x86_64-unknown-linux-gnu -- --ignored`
//! Needs the Steam install mounted at the path below.
//!
//! Only the looter's own signatures live here. The static-info accessor census
//! and the tables resolved through it moved to `desert-core/tests/gimmick_real.rs`,
//! which is where the code they test lives; `just test-game` runs both files.

use desert_looter::pattern::{self, Found, Pattern};
use desert_looter::{pe, rtti};

const EXE: &str = "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe";

const SIGNATURES: &[(&str, &str)] = &[
    ("alloc_event", "48 89 5C 24 ?? 4C 89 44 24 ?? 57 48 83 EC 20 8B ?? BA ?? ?? 00 00"),
    ("enqueue", "48 89 5C 24 08 57 48 83 EC 20 48 8B ?? 38 65 48 8B 04 25 58 00 00 00"),
    ("area_sweep+0xF", "55 41 54 41 55 41 56 41 57 48 8B EC 48 83 EC 50 C5 F8 29 74 24 40 4C 8B ?? 48 8B ?? 48 8B ?? 44 8B"),
    ("own_check_site", "48 8B 89 20 01 00 00 E8 ?? ?? ?? ?? 84 C0 74 04 B3 02"),
    ("get_pos", "40 53 48 83 EC 50 48 8B 41 68 48 8B 88 ?? 01 00 00 48 8B 01"),
    ("desc_mask+queue_site", "E8 ?? ?? ?? ?? 44 8B 05 ?? ?? ?? ?? 0F B7 54 24 ?? E8 ?? ?? ?? ?? 4C 8B 25"),
    ("interaction_fn", "88 54 24 10 48 89 4C 24 08 53 55 56 57 41 54 41 55 41 56 41 57 48 83 EC 58 49 8B ?? 44 0F B6 ?? 4C 8B ??"),
    ("category_fn", "48 89 5C 24 18 88 54 24 10 55 56 57 41 54 41 55 41 56 41 57 48 8B EC 48 81 EC ?? ?? ?? ?? 41 8B D9 4D 8B F0 0F B6 F2 4C 8B F9"),
];

#[test]
#[ignore]
fn all_signatures_unique_and_actor_manager_vtable_found() {
    let file = std::fs::read(EXE).expect("game exe present");
    let h = pe::parse(&file).unwrap();
    let img = pe::file_to_image(&file).unwrap();
    for (name, text) in SIGNATURES {
        let p = Pattern::parse(text).unwrap();
        match p.find_unique(&img) {
            Found::Unique(off) => println!("{name:<22} = +0x{off:X}"),
            other => panic!("{name}: {other:?}"),
        }
    }
    let vt = rtti::vtables_for_class(&img, h.image_base, ".?AVClientActorManager@pa@@");
    println!("ClientActorManager vtables: {vt:x?}");
    assert_eq!(vt.len(), 1);
}

/// `game::resolve` scans for all eight in one walk of the image rather than one
/// walk each (1.56 s to 0.22 s measured on build 25246367). One walk is only
/// worth having if it answers what eight did, and a signature table is exactly
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
        SIGNATURES.iter().filter_map(|(_, t)| Pattern::parse(t)).collect();
    assert_eq!(pats.len(), SIGNATURES.len(), "every signature parses");

    let one_at_a_time: Vec<Found> = pats.iter().map(|p| p.find_unique(&img)).collect();
    let one_pass = pattern::find_unique_multi(&pats, &img);

    for (i, (name, _)) in SIGNATURES.iter().enumerate() {
        assert_eq!(one_pass[i], one_at_a_time[i], "{name}");
        println!("{name:<22} {:?}", one_pass[i]);
    }
}
