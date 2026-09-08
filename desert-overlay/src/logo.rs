//! The menu's logo: `assets/logo.svg` as raw pixels, embedded in the DLL.
//!
//! The bytes in `logo.rgba` are produced by `tools/logo-to-rgba.py` (or
//! `just logo`) from `assets/logo.svg`, a 64 x 64 pixel-art goblin. The
//! script rasterises it with nearest-neighbour at 2x, so what is embedded
//! here is [`WIDTH`] x [`HEIGHT`] pixels of straight (non-premultiplied)
//! RGBA8, row-major, top-left first - exactly the layout hudhook's
//! `RenderContext::load_texture` takes, with no decoding step and no image
//! crate linked into the game's address space.
//!
//! It is drawn at roughly a third of its stored size, so imgui's linear
//! sampler shrinks a sharp image rather than blowing up a tiny one and the
//! pixel edges survive. Nothing here is Windows-specific: the module compiles
//! and unit-tests natively, which is where the byte count is checked.

/// Width of [`RGBA`] in pixels.
pub const WIDTH: u32 = 128;
/// Height of [`RGBA`] in pixels.
pub const HEIGHT: u32 = 128;

/// The logo, `WIDTH * HEIGHT * 4` bytes of straight RGBA8.
pub const RGBA: &[u8] = include_bytes!("logo.rgba");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_bytes_are_one_rgba_image_of_the_declared_size() {
        assert_eq!(RGBA.len(), (WIDTH * HEIGHT * 4) as usize);
    }

    /// The goblin sits on a transparent background, so both kinds of pixel
    /// have to be there: all-opaque would mean the alpha channel was lost
    /// somewhere, all-transparent would mean the raster came out empty.
    #[test]
    fn the_logo_has_both_opaque_and_transparent_pixels() {
        let mut alphas = RGBA.iter().skip(3).step_by(4);
        assert!(alphas.clone().any(|&a| a != 0), "no opaque pixel: the raster is empty");
        assert!(alphas.any(|&a| a == 0), "no transparent pixel: the background was filled in");
    }
}
