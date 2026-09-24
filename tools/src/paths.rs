//! Where things are: the repository, the game, the gimmickinfo table body.
//!
//! Every default here can be overridden by the same environment variable the
//! justfile uses, so a tool run by hand and the same tool run by `just` look in
//! the same place.

use std::env;
use std::ffi::OsString;
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

/// What the plugin writes the table body as, beside its log in `bin64`, when
/// `[Gatherer] DumpTable=1` and `DryRun=1` are both set. The plugin's
/// `desert_gatherer::dump::FILE_NAME`.
pub const TABLE_DUMP_NAME: &str = "DesertTooling.gimmickinfo.bin";

/// Where DMM leaves its extracted clean copy of the same bytes.
pub const DMM_BACKUP_TABLE: &str = "/mnt/f/DMM/backups/gimmickinfo_pabgb_clean.bin";

/// The clean `gimmickinfo` table body, by this three-step rule (the justfile
/// no longer keeps its own copy; `dmm-rebase` applies this one):
///
/// 1. `CD_DMM_TABLE`, if it is set - an explicit choice always wins, even one
///    naming a file that is not there (that is how a test drives the
///    "no table" branch on purpose);
/// 2. else `<bin64>/DesertTooling.gimmickinfo.bin`, if that file exists - the
///    plugin's own dump, copied out of the game's loader buffer;
/// 3. else DMM's backup copy.
///
/// The dump is preferred because it is the one that stays put: DMM deletes
/// and restores its backups folder at will, so a tool run could find its copy
/// gone, while the dump is only ever rewritten by a launch that asked for it.
/// Both are byte-identical when both are current. The name is kept from the
/// days DMM's copy was the only one, because every caller and test uses it.
pub fn dmm_table() -> PathBuf {
    table_from(env::var_os("CD_DMM_TABLE"), &bin64())
}

/// [`dmm_table`]'s rule with its two inputs passed in, so it can be tested
/// without touching the process environment.
fn table_from(explicit: Option<OsString>, bin64: &Path) -> PathBuf {
    if let Some(p) = explicit {
        return PathBuf::from(p);
    }
    let dump = bin64.join(TABLE_DUMP_NAME);
    if dump.is_file() {
        return dump;
    }
    PathBuf::from(DMM_BACKUP_TABLE)
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
pub const CALIBRATED_BUILD: &str = "25477059";

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_wins_even_over_an_existing_dump() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(TABLE_DUMP_NAME), b"body").unwrap();
        let chosen = table_from(Some("/nowhere/at/all.bin".into()), dir.path());
        assert_eq!(chosen, PathBuf::from("/nowhere/at/all.bin"));
    }

    #[test]
    fn the_dump_is_preferred_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let dump = dir.path().join(TABLE_DUMP_NAME);
        std::fs::write(&dump, b"body").unwrap();
        assert_eq!(table_from(None, dir.path()), dump);
    }

    /// No dump, or a directory squatting on its name: DMM's copy, the old
    /// default, whether or not it exists either.
    #[test]
    fn without_a_dump_it_falls_back_to_dmm() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(table_from(None, dir.path()), PathBuf::from(DMM_BACKUP_TABLE));
        std::fs::create_dir(dir.path().join(TABLE_DUMP_NAME)).unwrap();
        assert_eq!(table_from(None, dir.path()), PathBuf::from(DMM_BACKUP_TABLE));
    }
}
