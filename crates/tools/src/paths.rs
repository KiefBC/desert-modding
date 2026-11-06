//! Where things are: the repository and the game.
//!
//! Every default here can be overridden by the same environment variable the
//! justfile uses, so a tool run by hand and the same tool run by `just` look in
//! the same place.

use std::env;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// The game's `bin64`, from `CD_BIN64` or the usual Steam library.
pub fn bin64() -> PathBuf {
    match env::var_os("CD_BIN64") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64"),
    }
}

/// The game's install directory, holding the `<group>/0.pamt` archives:
/// `CD_GAME_DIR`, else the parent of [`bin64`].
pub fn game_dir() -> PathBuf {
    match env::var_os("CD_GAME_DIR") {
        Some(p) => PathBuf::from(p),
        None => bin64().join(".."),
    }
}

/// `CrimsonDesert.exe`. `EXE` wins over `CD_BIN64`, as `dis.sh` had it.
pub fn game_exe() -> PathBuf {
    match env::var_os("EXE") {
        Some(p) => PathBuf::from(p),
        None => bin64().join("CrimsonDesert.exe"),
    }
}

/// The Steam appmanifest carrying the build id, from `CD_APPMANIFEST`.
pub fn appmanifest() -> PathBuf {
    match env::var_os("CD_APPMANIFEST") {
        Some(p) => PathBuf::from(p),
        None => bin64().join("../../../appmanifest_3321460.acf"),
    }
}

/// The game build whose `gimmickinfo` table body the walk calibrations were
/// measured on: `gen-collect-names`' `CALIBRATION` and `items`' expected
/// counts and anchors. Both move together, so both name this one constant.
pub const CALIBRATED_BUILD: &str = "26480954";

/// The game build id read out of the appmanifest, if it is there.
pub fn build_id() -> Option<String> {
    let text = std::fs::read_to_string(appmanifest()).ok()?;
    for line in text.lines() {
        // `	"buildid"		"25246367"` - the first quoted field after the key.
        let Some(rest) = line.trim_start().strip_prefix("\"buildid\"") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('"') else { continue };
        let Some(end) = rest.find('"') else { continue };
        return Some(rest[..end].to_string());
    }
    None
}

/// The repository root: the nearest ancestor holding both `justfile` and
/// `flake.nix`. Falls back to the nearest such ancestor of the compiled-in
/// manifest directory (`<repo>/crates/tools`), so a tool run from anywhere still
/// finds the checkout it was built from.
pub fn repo_root() -> Result<PathBuf> {
    let start = env::current_dir().context("cannot read the current directory")?;
    if let Some(found) = walk_up(&start) {
        return Ok(found);
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(found) = walk_up(manifest) {
        return Ok(found);
    }
    bail!("not inside the repository: no ancestor of {} has both justfile and flake.nix", start.display())
}

/// The directory under the repository root that holds every Cargo crate: the
/// five `desert-*` crates of the root workspace and `tools`, which is its own
/// workspace. The root keeps only the workspace manifest, the justfile, the
/// docs and the data (`assets/`, `vendor/`).
pub const CRATES_DIR: &str = "crates";

/// `<repo>/crates`, the parent of every crate. See [`CRATES_DIR`].
pub fn crates_dir() -> Result<PathBuf> {
    Ok(crates_dir_in(&repo_root()?))
}

/// `<repo>/crates/<name>`, one crate's directory, for example
/// `crate_dir("desert-tooling")`.
pub fn crate_dir(name: &str) -> Result<PathBuf> {
    Ok(crate_dir_in(&repo_root()?, name))
}

/// `<repo>/crates/tools`, this workspace: the signature and table-name lists,
/// `extra-families.json` and `nexus-targets.json` live here.
pub fn tools_dir() -> Result<PathBuf> {
    crate_dir("tools")
}

/// [`crates_dir`] under a root already in hand (a tool that resolved the root
/// once, or a test's fake checkout).
pub fn crates_dir_in(root: &Path) -> PathBuf {
    root.join(CRATES_DIR)
}

/// [`crate_dir`] under a root already in hand.
pub fn crate_dir_in(root: &Path, name: &str) -> PathBuf {
    crates_dir_in(root).join(name)
}

fn walk_up(from: &Path) -> Option<PathBuf> {
    for dir in from.ancestors() {
        if dir.join("justfile").is_file() && dir.join("flake.nix").is_file() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_dirs_hang_off_crates() {
        let root = Path::new("/repo");
        assert_eq!(crates_dir_in(root), PathBuf::from("/repo/crates"));
        assert_eq!(crate_dir_in(root, "tools"), PathBuf::from("/repo/crates/tools"));
    }
}
