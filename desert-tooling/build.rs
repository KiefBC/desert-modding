// Embeds a Win32 VERSIONINFO resource in the .asi.
//
// Why: Definitive Mod Manager scans every plugin it installs and flags
// "missing version info" as one of its risk signals; with this resource the
// check passes and DMM's UI can show the mod's name and version. Windows
// itself shows the same fields under Properties -> Details.
//
// This used to exist three times over, once per shipped plugin, kept in sync by
// copying. There is one plugin now, so this is the only copy: desert-tooling is
// the workspace's single cdylib, and a build script in the rlibs it links
// (desert-core, desert-looter, desert-gatherer, desert-overlay) would embed a
// resource into nothing. Everything mod-specific below is derived from this
// crate's own Cargo metadata - name, version, description - so a version bump
// or a renamed package carries into the resource with nothing to edit here.
//
// Nothing happens off Windows: the native `x86_64-unknown-linux-gnu` build,
// which is what `cargo test --target x86_64-unknown-linux-gnu` uses, gets no
// resource and no windres invocation.

// A build script that cannot embed the VERSIONINFO resource must fail the
// build loudly: it runs on the dev machine, not in the game process, so the
// workspace's never-panic rule does not apply here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=WINDRES");
    // A version bump must rebuild the resource even if nothing else changed.
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    // Cross-compilation aware: this is the *target* OS, not the host.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    // "desert-tooling" -> product "Desert Tooling", file base "DesertTooling".
    let pkg = env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME");
    let words: Vec<String> = pkg
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect();
    let product = words.join(" ");
    let file_base = words.concat();
    let original_filename = format!("{file_base}.asi");

    // x.y.z.0 - the fourth field is the build number, which we do not use.
    let major = env::var("CARGO_PKG_VERSION_MAJOR").expect("CARGO_PKG_VERSION_MAJOR");
    let minor = env::var("CARGO_PKG_VERSION_MINOR").expect("CARGO_PKG_VERSION_MINOR");
    let patch = env::var("CARGO_PKG_VERSION_PATCH").expect("CARGO_PKG_VERSION_PATCH");
    let version_commas = format!("{major},{minor},{patch},0");
    let version_dots = format!("{major}.{minor}.{patch}.0");

    let description = escape(&env::var("CARGO_PKG_DESCRIPTION").unwrap_or_else(|_| product.clone()));
    let product_esc = escape(&product);
    let original_esc = escape(&original_filename);

    // Lang 0x0409 (US English), charset 1200 (Unicode) -> block "040904b0".
    // FILETYPE 0x2 is VFT_DLL; FILEOS 0x40004 is VOS_NT_WINDOWS32.
    let rc = format!(
        r#"#include <winver.h>

1 VERSIONINFO
FILEVERSION {version_commas}
PRODUCTVERSION {version_commas}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x2L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "KiefBC"
            VALUE "FileDescription", "{description}"
            VALUE "FileVersion", "{version_dots}"
            VALUE "InternalName", "{original_esc}"
            VALUE "LegalCopyright", "MIT"
            VALUE "OriginalFilename", "{original_esc}"
            VALUE "ProductName", "{product_esc}"
            VALUE "ProductVersion", "{version_dots}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    );

    let rc_path = out_dir.join("version.rc");
    let obj_path = out_dir.join("version.o");
    fs::write(&rc_path, rc).expect("write version.rc");

    // On PATH in the Nix dev shell (flake.nix pulls in the mingw cross cc).
    let windres = env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".to_string());
    let status = Command::new(&windres)
        .arg("--input-format=rc")
        .arg("--output-format=coff")
        .arg("--target=pe-x86-64")
        .arg(&rc_path)
        .arg(&obj_path)
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg={}", obj_path.display());
        }
        // A missing or failing windres must not break the build: the plugin is
        // perfectly functional without the resource, it just loses the version
        // fields. Warn loudly instead, so a release build never ships one by
        // accident without somebody seeing it.
        Ok(s) => println!("cargo:warning={windres} failed ({s}); .asi built without version info"),
        Err(e) => println!("cargo:warning={windres} not runnable ({e}); .asi built without version info"),
    }
}

/// Escape a string for an RC string literal.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
