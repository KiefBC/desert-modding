//! Runs only on request, against the installed game exe:
//! `cargo test --release --target x86_64-unknown-linux-gnu -p desert-dispatch --test banking_real -- --ignored --nocapture`
//!
//! The `[banking]` line's two gates, found by signature the way the plugin
//! finds them at startup (`node::globals::resolve_gates`), pinned to Steam
//! build 26361232. The signatures were derived on 26345091; the gates resolve
//! to the same two addresses on 26356046 and 26361232.

use desert_core::pe;
use desert_dispatch::node::globals;

const EXE: &str = "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe";

/// The deferred-reward and surplus-worker gates on 26361232, the same as on
/// 26356046 (`0x714FE68` and
/// `0x7127CF8` on 26345091; `0x6BC6AA8` and `0x6BA07C8` on 25116796, never
/// re-derived on the builds between).
const DEFERRED_GATE_RVA: usize = 0x7150E18;
const SURPLUS_GATE_RVA: usize = 0x7128CA8;
/// The surplus-worker rate global the same arithmetic multiplies by: 0xA0
/// below the surplus gate on 25116796 (`0x6BA0728`), on 26345091 and on
/// 26361232.
const RATE_BELOW_SURPLUS: usize = 0xA0;

#[test]
#[ignore]
fn the_banking_gates_resolve_once_each_to_zero_initialised_data() {
    let file = std::fs::read(EXE).expect("game exe present");
    let img = pe::file_to_image(&file).expect("image layout");
    let (deferred, surplus) = globals::resolve_gates(&img);
    println!("deferred-reward gate = {deferred:X?}");
    println!("surplus-worker gate  = {surplus:X?}");
    assert_eq!(deferred, Ok(DEFERRED_GATE_RVA));
    assert_eq!(surplus, Ok(SURPLUS_GATE_RVA));
    // Both are zero in the file image: zero-initialised data, as on 25116796.
    assert_eq!(img.get(DEFERRED_GATE_RVA), Some(&0));
    assert_eq!(img.get(SURPLUS_GATE_RVA), Some(&0));

    // The surplus read's arithmetic still names the rate global the old build
    // kept at the same distance: `movsxd rax,[rip+rate]` 0x1E past the `38`.
    let p = desert_core::pattern::Pattern::parse(globals::SURPLUS_GATE_SIG).unwrap();
    let hit = match p.find_unique(&img) {
        desert_core::pattern::Found::Unique(h) => h,
        other => panic!("{other:?}"),
    };
    let movsxd = hit + 0x1E;
    assert_eq!(&img[movsxd..movsxd + 3], &[0x48, 0x63, 0x05]);
    let rate = desert_core::pattern::resolve_rip(&img, movsxd, 3, 7).unwrap();
    assert_eq!(rate, SURPLUS_GATE_RVA - RATE_BELOW_SURPLUS);
}
