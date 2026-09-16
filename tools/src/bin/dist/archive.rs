//! Writing the release zips, and listing one back for a human to read.
//!
//! Reproducibility is the whole point of this module. Two runs of the same
//! commit must give byte-identical archives, which means nothing that varies
//! between runs may reach the file: not the wall clock, not the source files'
//! own mtimes, not the umask, not the order a directory listing happens to
//! come back in. So every entry is written with a fixed stamp, a fixed mode
//! and a caller-chosen order.
//!
//! The archives are NOT byte-identical to the ones `dist.sh` built with
//! Info-ZIP: the deflate streams differ at the same nominal level, and no
//! setting makes two different compressors agree. What is identical is
//! everything that is a decision rather than an encoding - the entry order,
//! names, stamps, modes, and of course the bytes that come back out. Each
//! release publishes the checksums of the bytes it actually attached, so a
//! digest changing across the port is not something anyone can observe.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

/// One file as it will appear in the archive: the name inside the zip, and
/// where its bytes come from. The two differ for the plugin, whose
/// `desert_tooling.dll` ships as `DesertTooling.asi` - the rename the shell
/// version needed a staging directory to perform.
pub struct Entry {
    pub name: String,
    pub source: std::path::PathBuf,
}

/// Write `entries` to `path`, in the order given.
///
/// There is no staging directory. `dist.sh` had one only because `zip` takes
/// paths on disk: it had to copy the files somewhere under their shipped names
/// and `touch` them before archiving. Reading the bytes and naming the entry
/// are separate arguments here, so `dist/.stage` has no reason to exist.
pub fn write(path: &Path, entries: &[Entry], stamp: DateTime) -> Result<()> {
    // Deleted rather than truncated, so a shorter archive cannot leave the
    // tail of a longer one behind it.
    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("cannot remove the old {}", path.display()))?;
    }
    let file = std::fs::File::create(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    let mut zip = ZipWriter::new(std::io::BufWriter::new(file));

    // `last_modified_time` is not optional for our feature set: with the `time`
    // feature on, the default stamp is "now", so leaving it unset would make
    // every run differ. 0o644 matches the `install -m 644` the shell version
    // did, and is set explicitly because the umask must not reach the archive.
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(9))
        .last_modified_time(stamp)
        .unix_permissions(0o644)
        .large_file(false);

    for entry in entries {
        let data = std::fs::read(&entry.source)
            .with_context(|| format!("cannot read {}", entry.source.display()))?;
        zip.start_file(entry.name.as_str(), options)
            .with_context(|| format!("cannot start the entry for {}", entry.name))?;
        zip.write_all(&data)
            .with_context(|| format!("cannot write the entry for {}", entry.name))?;
    }
    zip.finish().context("cannot finish the archive")?;
    Ok(())
}

/// The stamp every entry carries, from `DIST_EPOCH` (a unix timestamp, UTC).
///
/// Deliberately NOT `SOURCE_DATE_EPOCH`: nix's dev shell exports that as
/// 315532800 (1980-01-01 UTC), which is the oldest stamp a zip can hold at all
/// and underflows in any timezone west of UTC. Override `DIST_EPOCH` if you
/// want a specific stamp.
///
/// The conversion is fixed to UTC, where the shell version's `touch -d @epoch`
/// let Info-ZIP convert through the local zone. That is the same bug seen from
/// the other side: it is why 315532800 blows up in Los Angeles and not in
/// London. Here the archive is the same in both, and `TZ` cannot reach it.
pub fn stamp() -> Result<DateTime> {
    const DEFAULT: i64 = 1_577_836_800; // 2020-01-01 UTC
    let epoch = match std::env::var("DIST_EPOCH") {
        Ok(v) => v
            .trim()
            .parse::<i64>()
            .with_context(|| format!("DIST_EPOCH is not a unix timestamp: {v:?}"))?,
        Err(_) => DEFAULT,
    };
    let (y, mo, d, h, mi, s) = utc_from_epoch(epoch);
    DateTime::from_date_and_time(y, mo, d, h, mi, s).map_err(|_| {
        anyhow::anyhow!(
            "DIST_EPOCH {epoch} is {y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z, \
             which a zip cannot store (DOS timestamps start at 1980-01-01)"
        )
    })
}

/// Unix timestamp to a UTC calendar date, by Howard Hinnant's days-from-civil
/// inverse. `div_euclid` rather than `/` so a pre-1970 epoch floors instead of
/// truncating toward zero, which would land it a day late.
fn utc_from_epoch(epoch: i64) -> (u16, u8, u8, u8, u8, u8) {
    let days = epoch.div_euclid(86_400);
    let secs = epoch.rem_euclid(86_400);

    // Shift the era so the leap day is the last day of the year, which is what
    // makes the month arithmetic below a closed form.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    (
        y as u16,
        m as u8,
        d as u8,
        (secs / 3600) as u8,
        ((secs % 3600) / 60) as u8,
        (secs % 60) as u8,
    )
}

/// The `unzip -l` table for an archive, as the shell version printed it.
///
/// Kept because a person reads it to confirm what went into the archive - the
/// asi is there, the licence rode along, nothing stale came with them. The
/// columns are read back out of the finished file rather than from what was
/// just written, so the listing describes the artefact and not the intent.
pub fn listing(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
        .with_context(|| format!("cannot read {} back", path.display()))?;

    let mut out = String::new();
    out.push_str(&format!("Archive:  {}\n", path.display()));
    out.push_str("  Length      Date    Time    Name\n");
    out.push_str("---------  ---------- -----   ----\n");
    let mut total = 0u64;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).with_context(|| format!("entry {i}"))?;
        let t = entry.last_modified().context("entry has no timestamp")?;
        out.push_str(&format!(
            "{:>9}  {:02}-{:02}-{:04} {:02}:{:02}   {}\n",
            entry.size(),
            t.month(),
            t.day(),
            t.year(),
            t.hour(),
            t.minute(),
            entry.name()
        ));
        total += entry.size();
    }
    out.push_str("---------                     -------\n");
    out.push_str(&format!(
        "{total:>9}                     {} files\n",
        zip.len()
    ));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_epoch_is_2020_01_01_utc() {
        assert_eq!(utc_from_epoch(1_577_836_800), (2020, 1, 1, 0, 0, 0));
    }

    /// The stamp the dev shell would have handed us. It is representable in
    /// UTC and nowhere west of it - which is the entire reason this tool reads
    /// DIST_EPOCH instead of SOURCE_DATE_EPOCH, and why the conversion above
    /// is pinned to UTC.
    #[test]
    fn the_source_date_epoch_trap_is_the_very_first_representable_second() {
        assert_eq!(utc_from_epoch(315_532_800), (1980, 1, 1, 0, 0, 0));
        assert!(DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).is_ok());
        // One second earlier is 1979, and no zip can hold it.
        assert_eq!(utc_from_epoch(315_532_799), (1979, 12, 31, 23, 59, 59));
        assert!(DateTime::from_date_and_time(1979, 12, 31, 23, 59, 59).is_err());
    }

    /// Dates a closed-form conversion gets wrong when it is subtly off: a leap
    /// day, the century that is not a leap year's neighbours, and a pre-epoch
    /// value where truncating division would land a day late.
    #[test]
    fn awkward_dates() {
        assert_eq!(utc_from_epoch(951_782_400), (2000, 2, 29, 0, 0, 0));
        // 2100 is not a leap year - a naive "divisible by 4" rule puts a
        // 29th of February here and shifts every later date by a day.
        assert_eq!(utc_from_epoch(4_107_456_000), (2100, 2, 28, 0, 0, 0));
        assert_eq!(utc_from_epoch(4_107_542_400), (2100, 3, 1, 0, 0, 0));
        assert_eq!(utc_from_epoch(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(utc_from_epoch(-1), (1969, 12, 31, 23, 59, 59));
        assert_eq!(utc_from_epoch(1_577_836_800 + 86_399), (2020, 1, 1, 23, 59, 59));
    }

    /// `TZ` must not reach the archive. This is the bug DIST_EPOCH exists for,
    /// asserted directly rather than through a zip.
    #[test]
    fn the_stamp_ignores_the_local_timezone() {
        // Safety: single-threaded within this test, and `stamp()` reads only
        // DIST_EPOCH; nothing else here looks at TZ.
        let here = stamp().unwrap();
        std::env::set_var("TZ", "America/Los_Angeles");
        let west = stamp().unwrap();
        std::env::remove_var("TZ");
        assert_eq!(
            (here.year(), here.month(), here.day(), here.hour()),
            (west.year(), west.month(), west.day(), west.hour())
        );
    }
}
