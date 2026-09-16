//! `desert_tools::pe::import_dlls` against the plugin this repo actually ships.
//!
//! `check-imports` is the only thing proving libstdc++-6.dll stays out of
//! DesertTooling.asi, and the symptom of that regressing is an .asi the game
//! silently refuses to load. So the import reader had better be right. The old
//! shell script shelled out to the mingw objdump for this; the Rust one parses
//! the import directory itself, and this test is what says the two agree.
//!
//! Ignored by default: it needs `just build` to have run.

use std::path::PathBuf;

use desert_tools::paths;
use desert_tools::pe;

fn built_plugin() -> Option<PathBuf> {
    let p = paths::repo_root()
        .ok()?
        .join("target/x86_64-pc-windows-gnu/release/desert_tooling.dll");
    p.is_file().then_some(p)
}

#[test]
#[ignore = "needs `just build` to have produced the Windows DLL"]
fn the_shipped_plugin_imports_system_dlls_only() {
    let Some(path) = built_plugin() else {
        panic!("no built desert_tooling.dll - run `just build` first");
    };
    let data = std::fs::read(&path).unwrap();
    let imports = pe::import_dlls(&data).unwrap();

    assert!(!imports.is_empty(), "a DLL with no imports at all is not credible");

    // Normalisation is the point: one binary can carry both KERNEL32.dll and
    // kernel32.dll, and the allowlist in the justfile is lowercase and
    // suffix-free.
    for name in &imports {
        assert_eq!(*name, name.to_ascii_lowercase(), "{name} was not lowercased");
        assert!(!name.ends_with(".dll"), "{name} kept its suffix");
    }
    assert!(imports.windows(2).all(|w| w[0] < w[1]), "not sorted and deduplicated");

    // The regression this whole check exists for.
    assert!(
        !imports.iter().any(|n| n.contains("libstdc++")),
        "libstdc++ is back in the import table; see the [env] block in .cargo/config.toml"
    );
    assert!(imports.iter().any(|n| n == "kernel32"));
}
