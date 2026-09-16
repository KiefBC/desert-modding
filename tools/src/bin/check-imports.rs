//! Fail if a built plugin imports a DLL that is not part of Windows.
//!
//! ```text
//! check-imports <allowed> [<allowed> ...]
//! ```
//!
//! `just check-imports` calls this with the `allowed_imports` list from the
//! justfile, which is where the allowlist and the reason for every entry live.
//!
//! Why this exists: desert-overlay links imgui, which is C++, and everything is
//! linked into the one shipped plugin, so that is DesertTooling.asi's import
//! table. By default the `cc` crate links the target's C++ standard library,
//! and for our mingw target that is libstdc++-6.dll - a DLL the game's bin64
//! does not have. The .asi then fails to load, and the ASI loader says nothing
//! useful about why. The `CXXSTDLIB=""` / `-fno-threadsafe-statics` pair in the
//! `[env]` block of `.cargo/config.toml` is what keeps that dependency out;
//! this tool is what proves it stayed out, on every `just ci`.
//!
//! Every .dll and .asi in the release directory is checked, not just the one
//! that ships: a stale artefact from an older layout showing up here is worth
//! knowing about, and the check costs nothing.
//!
//! Names are compared lowercased and without the `.dll` suffix, because the
//! import table's capitalisation is not stable (KERNEL32.dll and kernel32.dll
//! both appear in one binary). That normalisation is `pe::import_dlls`'s job,
//! not this tool's - the allowlist in the justfile is written in the same form.
//!
//! Ported from `tools/check-imports.sh`. The one thing deliberately dropped is
//! the shell version's "run inside 'nix develop'" preflight: that existed only
//! because the script shelled out to `x86_64-w64-mingw32-objdump`, and reading
//! the import directory in process means there is no external tool left to
//! find. Everything else - the messages, the per-file `ok (N imports)` line and
//! the exit codes - is reproduced, because CI logs from before the port are
//! still the reference for what this prints.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;

use desert_tools::{paths, pe};

#[derive(Parser)]
#[command(
    name = "check-imports",
    about = "Fail if a built plugin imports a DLL outside the allowlist."
)]
struct Cli {
    /// Allowed DLL names, lowercase and without `.dll` (the justfile's `allowed_imports`).
    allowed: Vec<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Status 2, not 1: a missing allowlist is a caller bug (an empty justfile
    // variable would otherwise "pass" everything silently), and is worth
    // distinguishing from the genuine failure this tool is for.
    if cli.allowed.is_empty() {
        eprintln!("check-imports: no allowlist given");
        std::process::exit(2);
    }

    let built = paths::repo_root()?.join("target/x86_64-pc-windows-gnu/release");
    let files = built_binaries(&built)?;
    if files.is_empty() {
        eprintln!(
            "check-imports: nothing built in {} - run 'just build' first",
            built.display()
        );
        std::process::exit(1);
    }

    let mut rc = 0;
    for file in &files {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let data = std::fs::read(file)
            .with_context(|| format!("cannot read {}", file.display()))?;
        let imports = pe::import_dlls(&data)
            .with_context(|| format!("cannot read the imports of {name}"))?;

        let bad: Vec<&String> = imports.iter().filter(|d| !cli.allowed.contains(d)).collect();
        if bad.is_empty() {
            println!("check-imports: {name} ok ({} imports)", imports.len());
        } else {
            // One line naming the binary and every offending DLL, so a CI log
            // says what regressed without anyone re-running objdump by hand.
            let list: String = bad.iter().map(|d| format!(" {d}")).collect();
            eprintln!("check-imports: {name} imports non-system DLL(s):{list}");
            rc = 1;
        }
    }
    std::process::exit(rc);
}

/// Every `.dll` then every `.asi` in `dir`, each group sorted by name.
///
/// The order is the shell glob's (`*.dll` then `*.asi`, each expanded in
/// collation order) so the output of the two versions can be diffed line for
/// line. A missing directory is not an error here: it is the "nothing built"
/// case, which the caller reports with its own message.
fn built_binaries(dir: &Path) -> Result<Vec<PathBuf>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut dlls = Vec::new();
    let mut asis = Vec::new();
    for entry in entries {
        let path = entry.with_context(|| format!("cannot list {}", dir.display()))?.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("dll") => dlls.push(path),
            Some("asi") => asis.push(path),
            _ => {}
        }
    }
    dlls.sort();
    asis.sort();
    dlls.extend(asis);
    Ok(dlls)
}
