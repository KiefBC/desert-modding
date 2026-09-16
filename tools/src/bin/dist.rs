//! Build the release packages: the plugin and the DMM module pack.
//!
//! ```text
//! dist                   both packages
//! dist desert-tooling    just that one
//! ```
//!
//! Writes dist/DesertTooling-<version>.zip,
//! dist/DesertGatherer-DMM-<version>.zip and dist/SHA256SUMS. dist/ is
//! gitignored; the zips are what gets attached to a GitHub release.
//!
//! The release workflow passes the single package its tag names, so a release
//! carries only the zip it is actually about. That is not tidiness: the two
//! packages are versioned separately (the plugin from its Cargo.toml, the pack
//! from dmm_pack.json), so rebuilding both for every tag would eventually
//! publish an untagged package's OLD version number over NEW bytes, and two
//! releases would then disagree about the contents of one version.
//!
//! dist/ is wiped first, so SHA256SUMS only ever lists what this run built.
//!
//! The layout inside the plugin zip is FLAT - its files sit at the zip root,
//! with no wrapper folder. That is what makes one archive serve both kinds of
//! user:
//!
//!   * by hand: extract straight into <game>\bin64 and the .asi lands next to
//!     winmm.dll with nothing to move afterwards.
//!   * Definitive Mod Manager: DMM takes "an .asi / ReShade add-on file or a
//!     directory", so it sees the .asi immediately whether the zip is dropped
//!     on its window or the extracted folder is put under <DMM>/mods/_asi/.
//!
//! The pack zip is flat for a different reason: DMM only picks up a pack's
//! title and description when dmm_pack.json sits at the zip ROOT, so a wrapper
//! folder would leave the pack card showing the folder name instead.
//!
//! Reproducible: fixed file order, no per-file extra fields, a fixed stamp on
//! every entry (see `archive::stamp` for why it is DIST_EPOCH and not
//! SOURCE_DATE_EPOCH), and each zip deleted before it is rebuilt. Two runs of
//! the same commit give byte-identical archives.
//!
//! Ported from `tools/dist.sh`. Two things it shelled out for are now done in
//! process - the zip writing and the checksums - so `zip`, `unzip` and
//! `sha256sum` no longer have to be on PATH. `cargo build --release` is still
//! a subprocess, because that is invoking the build, not reimplementing a
//! tool. The archives are therefore not byte-identical to the ones Info-ZIP
//! produced; see `archive`, which explains why that is not observable.
//!
//! Every path is resolved through `paths::repo_root()`, never against the
//! working directory. This tool deletes a directory called `dist`, and one
//! resolved relative to wherever it happened to be started would eventually
//! delete the wrong one.

// `src/bin/<name>.rs` is a crate root but NOT a module directory root: rustc
// looks for `mod archive` at `src/bin/archive.rs`, which would be a second
// binary. `#[path]` puts the parts in `src/bin/dist/` where they belong, the
// way `gen-collect-names` does. They live here rather than in the library
// because nothing else needs them - `lib.rs` says only what more than one tool
// uses belongs there.
#[path = "dist/archive.rs"]
mod archive;
#[path = "dist/sha256.rs"]
mod sha256;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::Parser;

use desert_tools::paths;

use archive::Entry;
use sha256::{hex, Sha256};

/// Package names are the release-tag prefixes (VERSIONING.md step 6), so the
/// workflow can pass through what `release-notes --package` printed.
const ALL_PACKAGES: [&str; 2] = ["desert-tooling", "desert-gatherer-dmm"];

#[derive(Parser)]
#[command(
    name = "dist",
    about = "Build the release zips into dist/ (the plugin, the DMM pack, SHA256SUMS)."
)]
struct Cli {
    /// Packages to build; all of them if none is named.
    packages: Vec<String>,
}

fn main() {
    if let Err(e) = run() {
        // The shell version's diagnostics, not anyhow's `Error:` chain: these
        // are messages a person is meant to read in a CI log and act on.
        eprintln!("dist: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = paths::repo_root()?;

    let selected: Vec<String> = if cli.packages.is_empty() {
        ALL_PACKAGES.iter().map(|s| s.to_string()).collect()
    } else {
        cli.packages.clone()
    };

    // Validated before anything is deleted: a typo in a tag name must not cost
    // somebody the dist/ they were about to upload.
    let mut needs_cargo = false;
    for name in &selected {
        if !ALL_PACKAGES.contains(&name.as_str()) {
            bail!(
                "unknown package '{name}'; expected one of {}",
                ALL_PACKAGES.join(" ")
            );
        }
        if name != "desert-gatherer-dmm" {
            needs_cargo = true;
        }
    }

    if needs_cargo {
        emit("==> cargo build --release\n");
        build_plugin(&root)?;
    }

    let dist = root.join("dist");
    if dist.exists() {
        std::fs::remove_dir_all(&dist)
            .with_context(|| format!("cannot wipe {}", dist.display()))?;
    }
    std::fs::create_dir_all(&dist)
        .with_context(|| format!("cannot create {}", dist.display()))?;

    let stamp = archive::stamp()?;
    for name in &selected {
        let zipfile = match name.as_str() {
            "desert-tooling" => package_plugin(&root, &dist, stamp)?,
            "desert-gatherer-dmm" => package_dmm(&root, &dist, stamp)?,
            _ => unreachable!("validated above"),
        };
        emit(&format!(
            "==> {}\n",
            zipfile.file_name().unwrap_or_default().to_string_lossy()
        ));
    }

    write_sums(&dist)?;

    emit("\n");
    for zipfile in zips(&dist)? {
        emit(&archive::listing(&zipfile)?);
    }
    emit("dist/SHA256SUMS:\n");
    let sums = std::fs::read_to_string(dist.join("SHA256SUMS"))?;
    for line in sums.lines() {
        emit(&format!("  {line}\n"));
    }
    Ok(())
}

/// Print, and treat a closed stdout as "the reader has seen enough".
///
/// The listing is long and `dist | head` is the obvious way to skim it. A bare
/// `println!` panics on the broken pipe and leaves a backtrace where the shell
/// version simply stopped, which reads like the build failed when it did not -
/// and by this point the zips are already on disk and correct.
fn emit(text: &str) {
    if let Err(e) = std::io::stdout().write_all(text.as_bytes()) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        eprintln!("dist: cannot write to stdout: {e}");
        std::process::exit(1);
    }
}

/// The plugin build, from the repo root so the root workspace's
/// `.cargo/config.toml` - the one that selects the Windows target and keeps
/// libstdc++ out - is the config that applies.
fn build_plugin(root: &Path) -> Result<()> {
    let status = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(root)
        .status()
        .context("cannot run cargo - is it on PATH?")?;
    if !status.success() {
        // The build has already said why, at length. Anything added here is
        // noise between the error and the reader.
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// The shipped plugin. Fixed order: the plugin, its ini, the two docs, then
/// the licence. That is the whole install - one .asi and one .ini, both
/// dropped straight into bin64. The licence rides along because the zip, not
/// the repository, is what most people ever see of this project.
fn package_plugin(root: &Path, dist: &Path, stamp: zip::DateTime) -> Result<PathBuf> {
    let crate_dir = root.join("desert-tooling");
    let name = "DesertTooling";
    // The crate's Cargo.toml version is the single source of truth (VERSIONING.md).
    let version = cargo_version(&crate_dir.join("Cargo.toml"))?;

    let built = root.join("target/x86_64-pc-windows-gnu/release/desert_tooling.dll");
    let entries = vec![
        Entry {
            name: format!("{name}.asi"),
            source: built,
        },
        Entry {
            name: format!("{name}.ini"),
            source: crate_dir.join(format!("{name}.ini")),
        },
        Entry {
            name: "README.md".into(),
            source: crate_dir.join("README.md"),
        },
        Entry {
            name: "CHANGELOG.md".into(),
            source: crate_dir.join("CHANGELOG.md"),
        },
        Entry {
            name: "LICENSE".into(),
            source: root.join("LICENSE"),
        },
    ];

    let zipfile = dist.join(format!("{name}-{version}.zip"));
    archive::write(&zipfile, &entries, stamp)?;
    Ok(zipfile)
}

/// The DMM module pack. No build step: the JSONs in desert-gatherer-dmm/ are
/// the shipped artefact. Everything goes in FLAT so dmm_pack.json is at the
/// zip root.
fn package_dmm(root: &Path, dist: &Path, stamp: zip::DateTime) -> Result<PathBuf> {
    let src = root.join("desert-gatherer-dmm");
    let name = "DesertGatherer-DMM";

    // dmm_pack.json's version is the single source of truth for the pack, the
    // way Cargo.toml's is for a plugin. The module JSONs carry the same
    // version, which `sync-versions` is what keeps true.
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(src.join("dmm_pack.json"))
            .context("cannot read desert-gatherer-dmm/dmm_pack.json")?,
    )
    .context("desert-gatherer-dmm/dmm_pack.json is not valid JSON")?;
    let version = manifest
        .get("version")
        .and_then(|v| v.as_str())
        .context("no version in desert-gatherer-dmm/dmm_pack.json")?
        .to_string();

    // Fixed order: the manifest, then the twelve modules by name (byte order,
    // so the order does not drift with the locale), then the docs and the
    // rebaser.
    let modules = pack_modules(&src)?;
    if modules.len() != 12 {
        bail!("expected 12 pack modules, got {}", modules.len());
    }

    let mut names = vec!["dmm_pack.json".to_string()];
    names.extend(modules);
    names.extend(["README.md", "VERIFICATION.txt", "rebase.py"].map(String::from));

    let entries: Vec<Entry> = names
        .into_iter()
        .map(|n| Entry {
            source: src.join(&n),
            name: n,
        })
        .collect();

    let zipfile = dist.join(format!("{name}-{version}.zip"));
    archive::write(&zipfile, &entries, stamp)?;
    Ok(zipfile)
}

/// The pack's module files: the shell glob `*' - '*X.json`, sorted by bytes.
///
/// The shape of the name is the selector, not a hard-coded list, so adding a
/// multiplier is a matter of dropping the file in - and the count check in the
/// caller is what stops a half-finished addition from shipping.
fn pack_modules(src: &Path) -> Result<Vec<String>> {
    let mut modules: Vec<String> = std::fs::read_dir(src)
        .with_context(|| format!("cannot list {}", src.display()))?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.contains(" - ") && n.ends_with("X.json"))
        .collect();
    modules.sort();
    Ok(modules)
}

/// The first `version = "x.y.z"` in a Cargo.toml, which is the package's own -
/// dependencies come later in the file.
fn cargo_version(manifest: &Path) -> Result<String> {
    let text = std::fs::read_to_string(manifest)
        .with_context(|| format!("cannot read {}", manifest.display()))?;
    for line in text.lines() {
        if line.starts_with("version") {
            if let Some(v) = line.split('"').nth(1) {
                return Ok(v.to_string());
            }
        }
    }
    bail!("no version in {}", manifest.display())
}

/// `dist/SHA256SUMS`, in the exact format `sha256sum --check --strict` parses:
/// the digest, two spaces, the bare file name. The release workflow runs that
/// check from inside dist/, so a name with a `./` on the front - which is what
/// `sha256sum ./*.zip` emits, and why the shell version piped it through sed -
/// would be a different path from the one it is verifying.
fn write_sums(dist: &Path) -> Result<()> {
    let mut out = String::new();
    for zipfile in zips(dist)? {
        let mut hasher = Sha256::new();
        hasher.update(
            &std::fs::read(&zipfile)
                .with_context(|| format!("cannot read {}", zipfile.display()))?,
        );
        let name = zipfile.file_name().unwrap_or_default().to_string_lossy();
        out.push_str(&format!("{}  {name}\n", hex(&hasher.finish())));
    }
    std::fs::write(dist.join("SHA256SUMS"), out).context("cannot write dist/SHA256SUMS")?;
    Ok(())
}

/// Every zip in dist/, in name order - the shell glob's order, which is what
/// the checksum file and the listing both follow.
fn zips(dist: &Path) -> Result<Vec<PathBuf>> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dist)
        .with_context(|| format!("cannot list {}", dist.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("zip"))
        .collect();
    found.sort();
    Ok(found)
}
