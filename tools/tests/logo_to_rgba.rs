//! `logo-to-rgba` against its own committed output.
//!
//! This is the strongest test in the tools workspace and it costs nothing: the
//! SVG and the blob it rasterises to are both in the repository, so a run that
//! reproduces `desert-overlay/src/logo.rgba` byte for byte proves the whole
//! pipeline - path parsing, even-odd fill, paint order, upscale, byte layout -
//! against 65536 bytes of committed evidence. No game, no network.
//!
//! The tool writes to `<repo root>/desert-overlay/src/logo.rgba`, and repo root
//! is "the nearest ancestor with both a justfile and a flake.nix". So the test
//! builds a throwaway root in a temp directory rather than writing into the
//! working tree: a test that rewrites a tracked file would leave a dirty tree
//! whenever it failed, which is exactly when you least want one.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tools/ has a parent")
        .to_path_buf()
}

/// A temp directory that `paths::repo_root()` will accept, holding a copy of
/// the real SVG and an empty place for the output.
fn fake_root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("justfile"), "").unwrap();
    std::fs::write(dir.path().join("flake.nix"), "").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    std::fs::create_dir_all(dir.path().join("desert-overlay/src")).unwrap();
    std::fs::copy(
        repo().join("assets/logo.svg"),
        dir.path().join("assets/logo.svg"),
    )
    .unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_logo-to-rgba"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the tool should start")
}

#[test]
fn the_committed_svg_rasterises_to_the_committed_blob() {
    let dir = fake_root();
    let out = run(dir.path(), &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let got = std::fs::read(dir.path().join("desert-overlay/src/logo.rgba")).unwrap();
    let want = std::fs::read(repo().join("desert-overlay/src/logo.rgba")).unwrap();
    assert_eq!(got.len(), want.len(), "byte count moved");
    // Report the first disagreement rather than 65 kB of hex.
    if let Some(i) = (0..got.len()).find(|&i| got[i] != want[i]) {
        panic!(
            "byte {i} (pixel {}, channel {}) is {:#04x}, the committed blob has {:#04x}",
            i / 4,
            i % 4,
            got[i],
            want[i]
        );
    }
}

#[test]
fn the_summary_line_reports_the_shape_the_overlay_expects() {
    let dir = fake_root();
    let out = run(dir.path(), &[]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        text.trim_end(),
        "logo-to-rgba: desert-overlay/src/logo.rgba: 128x128 RGBA, 65536 bytes, \
         2039 of 4096 source pixels opaque"
    );
}

/// Rerunning must not change the file; the overlay embeds it with
/// `include_bytes!` and a rebuild that churns the blob churns the DLL.
#[test]
fn the_rasteriser_is_deterministic() {
    let dir = fake_root();
    let path = dir.path().join("desert-overlay/src/logo.rgba");
    run(dir.path(), &[]);
    let first = std::fs::read(&path).unwrap();
    run(dir.path(), &[]);
    assert_eq!(first, std::fs::read(&path).unwrap());
}

#[test]
fn png_output_is_a_png_of_the_same_pixels() {
    let dir = fake_root();
    let png = dir.path().join("logo.png");
    let out = run(dir.path(), &["--png", png.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(&png).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&bytes[12..16], b"IHDR");
    assert_eq!(&bytes[16..24], &[0, 0, 0, 128, 0, 0, 0, 128]); // 128 x 128
    assert_eq!(&bytes[bytes.len() - 8..bytes.len() - 4], b"IEND");
}

/// Nothing verifies the committed blob against the committed SVG in CI, which
/// the README admits; this test is that verification, so it must fail loudly if
/// either file goes missing rather than quietly passing on an empty compare.
#[test]
fn both_committed_files_are_present() {
    assert!(repo().join("assets/logo.svg").is_file());
    assert_eq!(
        std::fs::metadata(repo().join("desert-overlay/src/logo.rgba"))
            .unwrap()
            .len(),
        65536
    );
}

/// `just logo` runs the binary from the REPO ROOT, but nothing stops anyone
/// running it from inside `tools/` or `desert-overlay/`. The paths come from
/// `paths::repo_root()`, which walks up for the justfile and flake.nix, so the
/// output must land in the same place from any working directory inside the
/// checkout - a cwd-relative `assets/logo.svg` would have written the blob into
/// whatever directory the caller happened to be in, or not at all.
#[test]
fn the_output_path_does_not_depend_on_the_working_directory() {
    let dir = fake_root();
    let nested = dir.path().join("desert-overlay/src");
    std::fs::create_dir_all(dir.path().join("tools/src/bin")).unwrap();

    for cwd in [
        dir.path().to_path_buf(),
        nested.clone(),
        dir.path().join("tools/src/bin"),
    ] {
        std::fs::remove_file(nested.join("logo.rgba")).ok();
        let out = run(&cwd, &[]);
        assert!(
            out.status.success(),
            "from {}: {}",
            cwd.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        // The summary line is repo-relative too, not cwd-relative.
        assert!(
            String::from_utf8_lossy(&out.stdout)
                .starts_with("logo-to-rgba: desert-overlay/src/logo.rgba:"),
            "from {}: {}",
            cwd.display(),
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(
            std::fs::read(nested.join("logo.rgba")).unwrap(),
            std::fs::read(repo().join("desert-overlay/src/logo.rgba")).unwrap(),
            "from {}",
            cwd.display()
        );
    }
}
