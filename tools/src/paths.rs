//! Where things are: the repository, the game, DMM's table.
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

/// `CrimsonDesert.exe`. `EXE` wins over `CD_BIN64`, as `dis.sh` had it.
pub fn game_exe() -> PathBuf {
    match env::var_os("EXE") {
        Some(p) => PathBuf::from(p),
        None => bin64().join("CrimsonDesert.exe"),
    }
}

/// DMM's extracted clean `gimmickinfo` table body, from `CD_DMM_TABLE`.
pub fn dmm_table() -> PathBuf {
    match env::var_os("CD_DMM_TABLE") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("/mnt/f/DMM/backups/gimmickinfo_pabgb_clean.bin"),
    }
}

/// The Steam appmanifest carrying the build id, from `CD_APPMANIFEST`.
pub fn appmanifest() -> PathBuf {
    match env::var_os("CD_APPMANIFEST") {
        Some(p) => PathBuf::from(p),
        None => bin64().join("../../../appmanifest_3321460.acf"),
    }
}

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
/// `flake.nix`. Falls back to the compiled-in manifest directory's parent, so a
/// tool run from anywhere still finds the checkout it was built from.
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

fn walk_up(from: &Path) -> Option<PathBuf> {
    for dir in from.ancestors() {
        if dir.join("justfile").is_file() && dir.join("flake.nix").is_file() {
            return Some(dir.to_path_buf());
        }
    }
    None
}
