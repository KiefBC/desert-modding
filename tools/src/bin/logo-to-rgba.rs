//! Rasterise `assets/logo.svg` into the raw RGBA bytes the overlay embeds.
//!
//!     logo-to-rgba                 writes desert-overlay/src/logo.rgba
//!     logo-to-rgba --png out.png   also writes a PNG to look at
//!
//! The logo is pixel art: every path is made of M/m, h, v and z commands on an
//! integer grid inside a 64 x 64 viewBox, with crispEdges rendering. That is
//! simple enough to fill by hand (even-odd rule at pixel centres, paths painted
//! in document order) and keeps an SVG library out of both the build and the
//! DLL.
//!
//! The output is the picture scaled up SCALE times with nearest-neighbour, as
//! SIZE x SIZE pixels of straight (non-premultiplied) RGBA, row-major, top-left
//! first: exactly what hudhook's `RenderContext::load_texture` takes. Scaling up
//! here means the linear sampler in the game shrinks a sharp image rather than
//! enlarging a tiny one, which keeps the pixel look.
//!
//! Rerun after editing the SVG; the overlay's unit test checks the byte count.
//!
//! Deliberately NOT a general SVG parser, and no SVG crate. The narrow subset
//! is the design: a renderer that only does what the artwork does cannot
//! silently disagree with a browser about arcs, transforms or stroke. The one
//! thing this port changes is the failure mode `tools/README.md` warns about -
//! an unsupported command used to be silently misparsed, because the tokeniser
//! only recognised `[MmHhVvZz]` and let any other letter's operands dribble
//! into the previous command, drawing a plausible wrong shape and reporting
//! success. The tokeniser here matches ANY ASCII letter, so a `C` or an `A`
//! added to the artwork reaches the state machine and is rejected there.

use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;
use desert_tools::paths;
use flate2::{write::ZlibEncoder, Compression};
use regex::Regex;

/// The SVG's viewBox is 64 x 64 and every coordinate in it is an integer, so
/// one grid cell is one source pixel and no sub-pixel coverage ever arises.
const GRID: usize = 64;
/// Upscale factor. The game samples the texture linearly; handing it a bigger
/// image to shrink keeps the hard pixel edges that handing it a 64 x 64 one to
/// enlarge would smear away.
const SCALE: usize = 2;
const SIZE: usize = GRID * SCALE;

#[derive(Parser)]
#[command(
    name = "logo-to-rgba",
    about = "Rasterise assets/logo.svg into desert-overlay/src/logo.rgba"
)]
struct Args {
    /// Also write a PNG of the same pixels, to look at.
    #[arg(long, value_name = "PATH")]
    png: Option<PathBuf>,
}

type Rgba = [u8; 4];

fn main() -> Result<()> {
    let args = Args::parse();
    let root = paths::repo_root()?;
    let svg = root.join("assets").join("logo.svg");
    let out = root.join("desert-overlay").join("src").join("logo.rgba");

    let text =
        std::fs::read_to_string(&svg).with_context(|| format!("cannot read {}", svg.display()))?;

    // `fill` before `d`, both double-quoted, is how every path in the artwork is
    // written; the README says so out loud because this regex is the contract.
    let path_re = Regex::new(r#"<path fill="(#[0-9a-fA-F]{6})" d="([^"]+)""#)
        .expect("the path regex is a literal");
    let paths: Vec<(&str, &str)> = path_re
        .captures_iter(&text)
        .map(|c| {
            let get = |i: usize| c.get(i).expect("both groups always participate").as_str();
            (get(1), get(2))
        })
        .collect();
    if paths.is_empty() {
        bail!("logo-to-rgba: no <path fill= d=> elements found");
    }

    // Paths are painted in document order; a later one overwrites an earlier
    // one's pixels outright, which is what `crispEdges` with opaque fills does.
    let mut grid: Vec<Rgba> = vec![[0, 0, 0, 0]; GRID * GRID];
    for (fill, d) in paths {
        let rgb = parse_fill(fill)?;
        for poly in subpaths(d)? {
            paint(&mut grid, &poly, rgb);
        }
    }

    // Nearest-neighbour upscale, row-major, top-left first.
    let mut blob = Vec::with_capacity(SIZE * SIZE * 4);
    for y in 0..SIZE {
        let row = (y / SCALE) * GRID;
        for x in 0..SIZE {
            blob.extend_from_slice(&grid[row + x / SCALE]);
        }
    }
    std::fs::write(&out, &blob).with_context(|| format!("cannot write {}", out.display()))?;

    let opaque = grid.iter().filter(|px| px[3] != 0).count();
    let rel = out.strip_prefix(&root).unwrap_or(&out);
    println!(
        "logo-to-rgba: {}: {SIZE}x{SIZE} RGBA, {} bytes, {opaque} of {} source pixels opaque",
        rel.display(),
        blob.len(),
        GRID * GRID
    );

    if let Some(png_path) = args.png {
        let png = encode_png(&blob, SIZE, SIZE)?;
        std::fs::write(&png_path, png)
            .with_context(|| format!("cannot write {}", png_path.display()))?;
        println!("logo-to-rgba: wrote {}", png_path.display());
    }
    Ok(())
}

/// `#rrggbb` to its three bytes. The path regex already vetted the shape.
fn parse_fill(s: &str) -> Result<[u8; 3]> {
    let hex = s.trim_start_matches('#');
    let byte = |i: usize| -> Result<u8> {
        u8::from_str_radix(&hex[i..i + 2], 16).with_context(|| format!("bad fill colour {s:?}"))
    };
    Ok([byte(0)?, byte(2)?, byte(4)?])
}

/// One path command letter, or one number that followed it.
enum Token<'a> {
    Cmd(char),
    Num(&'a str),
}

/// Every closed subpath of a `d` attribute, as a list of vertices.
///
/// The state machine is SVG's, narrowed to the commands the artwork uses. `M`
/// starts a subpath and then degrades to `L`, which is where an unsupported
/// command lands and gets rejected: the logo has no segment that is not axis
/// aligned, so reaching `L` means the artwork grew a command this renderer
/// cannot draw.
fn subpaths(d: &str) -> Result<Vec<Vec<(f64, f64)>>> {
    let mut out: Vec<Vec<(f64, f64)>> = Vec::new();
    let (mut x, mut y) = (0.0f64, 0.0f64);
    let mut start = (0.0f64, 0.0f64);
    let mut pts: Vec<(f64, f64)> = Vec::new();
    let mut cmd: Option<char> = None;
    let mut nums: Vec<f64> = Vec::new();

    for tok in tokenize(d)? {
        let value = match tok {
            Token::Cmd(letter) => {
                // A half-finished operand run when the next command arrives
                // means the `d` is malformed. Carrying it forward is how the
                // old parser turned a typo into a wrong picture.
                if !nums.is_empty() {
                    bail!("logo-to-rgba: incomplete coordinate run before '{letter}' in {d:?}");
                }
                if letter == 'Z' || letter == 'z' {
                    if !pts.is_empty() {
                        out.push(std::mem::take(&mut pts));
                    }
                    // `z` returns the pen to the subpath's start, so a following
                    // relative `m` is measured from there and not from the last
                    // vertex drawn.
                    (x, y) = start;
                }
                cmd = Some(letter);
                continue;
            }
            Token::Num(text) => text
                .parse::<f64>()
                .with_context(|| format!("logo-to-rgba: bad number {text:?}"))?,
        };
        nums.push(value);
        let Some(c) = cmd else {
            bail!("logo-to-rgba: a coordinate before any path command in {d:?}");
        };
        let need = if c == 'M' || c == 'm' { 2 } else { 1 };
        if nums.len() < need {
            continue;
        }
        match c {
            'M' => {
                x = nums[0];
                y = nums[1];
                start = (x, y);
                pts = vec![(x, y)];
                cmd = Some('L'); // further pairs would be lineto; the logo has none
            }
            'm' => {
                x += nums[0];
                y += nums[1];
                start = (x, y);
                pts = vec![(x, y)];
                cmd = Some('l');
            }
            'H' => {
                x = nums[0];
                pts.push((x, y));
            }
            'h' => {
                x += nums[0];
                pts.push((x, y));
            }
            'V' => {
                y = nums[0];
                pts.push((x, y));
            }
            'v' => {
                y += nums[0];
                pts.push((x, y));
            }
            other => bail!("logo-to-rgba: unsupported path command '{other}'"),
        }
        nums.clear();
    }
    if !pts.is_empty() {
        out.push(pts);
    }
    Ok(out)
}

/// Split a `d` attribute into command letters and numbers.
///
/// Any ASCII letter is a command here, even one this tool cannot draw, so that
/// it reaches `subpaths` and is rejected. Matching only the drawable letters -
/// what the Python did - left an unknown command's operands to be read as more
/// of the *previous* command.
fn tokenize(d: &str) -> Result<Vec<Token<'_>>> {
    let mut out = Vec::new();
    let bytes = d.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphabetic() {
            out.push(Token::Cmd(b as char));
            i += 1;
        } else if b == b'-' || b.is_ascii_digit() {
            let start = i;
            if b == b'-' {
                i += 1;
            }
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'.' {
                let dot = i;
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i == dot + 1 {
                    i = dot; // a lone trailing '.' is not part of the number
                }
            }
            out.push(Token::Num(&d[start..i]));
        } else if b == b',' || b.is_ascii_whitespace() {
            i += 1;
        } else {
            bail!(
                "logo-to-rgba: unexpected character {:?} in path data",
                b as char
            );
        }
    }
    Ok(out)
}

/// Fill one closed polygon into the grid, even-odd, sampled at pixel centres.
fn paint(grid: &mut [Rgba], poly: &[(f64, f64)], rgb: [u8; 3]) {
    let (mut lo_x, mut hi_x) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut lo_y, mut hi_y) = (f64::INFINITY, f64::NEG_INFINITY);
    for &(px, py) in poly {
        lo_x = lo_x.min(px);
        hi_x = hi_x.max(px);
        lo_y = lo_y.min(py);
        hi_y = hi_y.max(py);
    }
    // Truncate-toward-zero then clamp, exactly as the Python's int()/max()/min()
    // did: the bounding box only bounds the scan, `inside` decides every pixel.
    let lo = |v: f64| (v as i64).max(0) as usize;
    let hi = |v: f64| ((v as i64) + 1).clamp(0, GRID as i64) as usize;
    for y in lo(lo_y)..hi(hi_y) {
        for x in lo(lo_x)..hi(hi_x) {
            if inside(poly, x as f64 + 0.5, y as f64 + 0.5) {
                grid[y * GRID + x] = [rgb[0], rgb[1], rgb[2], 255];
            }
        }
    }
}

/// Even-odd point-in-polygon for a rectilinear polygon.
fn inside(poly: &[(f64, f64)], px: f64, py: f64) -> bool {
    let mut hit = false;
    let n = poly.len();
    for (i, &(x1, y1)) in poly.iter().enumerate() {
        let (x2, y2) = poly[(i + 1) % n];
        if (y1 > py) != (y2 > py) {
            let xi = x1 + (py - y1) * (x2 - x1) / (y2 - y1);
            if px < xi {
                hit = !hit;
            }
        }
    }
    hit
}

/// An 8-bit RGBA PNG of `rgba`, for eyeballing the result.
///
/// Hand-rolled rather than pulled from an image crate, for the same reason the
/// SVG parser is: three chunks and a deflate call is less to carry than a
/// dependency. `flate2` does the deflate, at level 9 to match the Python - it
/// is `zip`'s own backend and so was already in the lock file, which is why
/// naming it adds nothing to the tree.
fn encode_png(rgba: &[u8], w: usize, h: usize) -> Result<Vec<u8>> {
    let mut raw = Vec::with_capacity(h * (1 + w * 4));
    for y in 0..h {
        raw.push(0); // per-row filter type 0, None
        raw.extend_from_slice(&rgba[y * w * 4..(y + 1) * w * 4]);
    }
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(9));
    enc.write_all(&raw)
        .context("cannot deflate the image data")?;
    let idat = enc.finish().context("cannot finish the zlib stream")?;

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bits/channel, truecolour + alpha
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &idat);
    chunk(&mut png, b"IEND", &[]);
    Ok(png)
}

fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len(); // the CRC covers the tag and the data, not the length
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m_h_v_z_walks_a_rectangle() {
        let polys = subpaths("M1 2h3v4h-3z").unwrap();
        assert_eq!(polys.len(), 1);
        assert_eq!(
            polys[0],
            vec![(1.0, 2.0), (4.0, 2.0), (4.0, 6.0), (1.0, 6.0)]
        );
    }

    #[test]
    fn absolute_h_and_v_are_coordinates_not_deltas() {
        let polys = subpaths("M40 10H44V20z").unwrap();
        assert_eq!(polys[0], vec![(40.0, 10.0), (44.0, 10.0), (44.0, 20.0)]);
    }

    /// After `z` the pen is back at the subpath's start, so the following `m`
    /// is relative to (0,0) here and not to the last vertex, (0,2).
    #[test]
    fn several_subpaths_in_one_d() {
        let polys = subpaths("M0 0h2v2h-2zm5 0h2v2h-2z").unwrap();
        assert_eq!(polys.len(), 2);
        assert_eq!(polys[1][0], (5.0, 0.0));
    }

    /// The README's warning was that this case drew a wrong logo in silence.
    #[test]
    fn an_unsupported_command_is_an_error_not_a_misparse() {
        let err = subpaths("M0 0h2C1 2 3 4 5 6z").unwrap_err().to_string();
        assert!(err.contains("unsupported path command"), "{err}");
    }

    #[test]
    fn even_odd_fill_covers_pixel_centres() {
        let square = [(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)];
        assert!(inside(&square, 1.5, 1.5));
        assert!(inside(&square, 2.5, 2.5));
        assert!(!inside(&square, 0.5, 1.5));
        assert!(!inside(&square, 3.5, 1.5));
    }

    #[test]
    fn fill_colours_are_three_bytes() {
        assert_eq!(parse_fill("#aa7540").unwrap(), [0xaa, 0x75, 0x40]);
    }

    #[test]
    fn crc32_matches_the_known_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    /// The PNG must survive a real decoder, not just look like one: a wrong
    /// chunk CRC or a truncated zlib stream is silent until something opens it.
    #[test]
    fn the_png_round_trips_through_a_decoder() {
        use flate2::read::ZlibDecoder;
        use std::io::Read as _;

        let pixels: Vec<u8> = (0..4 * 4 * 4).map(|i| (i % 251) as u8).collect();
        let png = encode_png(&pixels, 4, 4).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

        // Walk the chunks, checking every CRC, and collect the IDAT bytes.
        let mut idat = Vec::new();
        let mut i = 8;
        while i < png.len() {
            let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
            let body = &png[i + 4..i + 8 + len];
            let want = u32::from_be_bytes(png[i + 8 + len..i + 12 + len].try_into().unwrap());
            assert_eq!(crc32(body), want, "chunk at {i} has a bad CRC");
            if &body[..4] == b"IDAT" {
                idat.extend_from_slice(&body[4..]);
            }
            i += 12 + len;
        }
        assert_eq!(i, png.len(), "the chunk walk did not land on the end");

        let mut raw = Vec::new();
        ZlibDecoder::new(&idat[..]).read_to_end(&mut raw).unwrap();
        // One filter byte per row, filter type 0, then the row's bytes back.
        let unfiltered: Vec<u8> = raw
            .chunks(1 + 4 * 4)
            .flat_map(|row| {
                assert_eq!(row[0], 0);
                row[1..].to_vec()
            })
            .collect();
        assert_eq!(unfiltered, pixels);
    }
}
