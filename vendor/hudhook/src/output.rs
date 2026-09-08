//! Output colour space of the swap chain hudhook draws into, and the knobs a
//! host uses to steer the conversion.
//!
//! imgui hands the renderer sRGB-encoded, non-linear colours. That is exactly
//! what an SDR swap chain wants, and exactly wrong for an HDR one: written
//! unchanged into an HDR10 (PQ) or scRGB back buffer they come out blown out
//! and oversaturated. The DX12 backend therefore converts in the pixel shader,
//! and this module is where it learns what to convert to.
//!
//! Two independent things live here:
//!
//! * what hudhook *detected*, written by the DX12 hooks from
//!   `IDXGISwapChain3::SetColorSpace1` (or from DXGI's own default for the back
//!   buffer format when the game never called it);
//! * what the host *wants*, an override and a paper-white level, both of which
//!   default to "whatever was detected, at 203 nits".
//!
//! Everything is a plain atomic, so the render thread reads it once per frame
//! with no lock and the host writes it from any thread at any time.

use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use tracing::info;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709, DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020,
    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_TYPE,
};

/// Shader mode: write the sRGB-encoded colour through untouched. What every
/// SDR swap chain wants, and what hudhook did unconditionally before this
/// module existed.
pub const SHADER_MODE_SDR: u32 = 0;

/// Shader mode: BT.709 sRGB in, BT.2020 PQ out, scaled to [`paper_white_nits`].
pub const SHADER_MODE_HDR10: u32 = 1;

/// Shader mode: BT.709 sRGB in, linear BT.709 out with 1.0 = 80 nits, scaled
/// to [`paper_white_nits`].
pub const SHADER_MODE_SCRGB: u32 = 2;

/// Nits that plain white (`0xFFFFFFFF`) is drawn at on an HDR swap chain.
///
/// 203 is the ITU-R BT.2408 reference level for diffuse white, which is also
/// what ReShade's own overlay uses. Higher is brighter and harsher; the useful
/// range is roughly 100 to 400.
pub const DEFAULT_PAPER_WHITE_NITS: f32 = 203.0;

/// What the host wants hudhook to encode its pixels as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorSpaceOverride {
    /// Follow [`detected_color_space`]. The default, and right unless the
    /// detection is wrong.
    #[default]
    Auto,
    /// Force sRGB passthrough, whatever the swap chain says.
    Sdr,
    /// Force HDR10 (BT.2020 primaries, PQ transfer).
    Hdr10,
    /// Force scRGB (linear BT.709, 1.0 = 80 nits).
    ScRgb,
}

impl ColorSpaceOverride {
    /// The name this override parses from and logs as.
    pub fn as_str(self) -> &'static str {
        match self {
            ColorSpaceOverride::Auto => "auto",
            ColorSpaceOverride::Sdr => "sdr",
            ColorSpaceOverride::Hdr10 => "hdr10",
            ColorSpaceOverride::ScRgb => "scrgb",
        }
    }

    fn from_repr(repr: u32) -> Self {
        match repr {
            1 => ColorSpaceOverride::Sdr,
            2 => ColorSpaceOverride::Hdr10,
            3 => ColorSpaceOverride::ScRgb,
            _ => ColorSpaceOverride::Auto,
        }
    }

    fn to_repr(self) -> u32 {
        match self {
            ColorSpaceOverride::Auto => 0,
            ColorSpaceOverride::Sdr => 1,
            ColorSpaceOverride::Hdr10 => 2,
            ColorSpaceOverride::ScRgb => 3,
        }
    }
}

// DXGI_COLOR_SPACE_TYPE is a newtype over i32, so the detected value stores as
// one. sRGB is the value a swap chain that never had SetColorSpace1 called on
// it reports, and is therefore also the starting value here.
static DETECTED: AtomicI32 = AtomicI32::new(DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709.0);
static OVERRIDE: AtomicU32 = AtomicU32::new(0);
static PAPER_WHITE_BITS: AtomicU32 = AtomicU32::new(DEFAULT_PAPER_WHITE_NITS.to_bits());

/// DXGI's colour space for a back buffer nobody called `SetColorSpace1` on:
/// scRGB for `R16G16B16A16_FLOAT`, sRGB for everything else.
pub fn default_color_space_for_format(is_float_back_buffer: bool) -> DXGI_COLOR_SPACE_TYPE {
    if is_float_back_buffer {
        DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709
    } else {
        DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709
    }
}

/// Record the colour space the swap chain is presenting in. Logs at info level
/// when the value actually changes, and says nothing on the frames it does not.
pub fn set_detected_color_space(color_space: DXGI_COLOR_SPACE_TYPE) {
    let previous = DETECTED.swap(color_space.0, Ordering::Release);
    if previous != color_space.0 {
        info!(
            "Swap chain colour space is now {} ({}), was {} ({})",
            color_space.0,
            describe_color_space(color_space),
            previous,
            describe_color_space(DXGI_COLOR_SPACE_TYPE(previous)),
        );
    }
}

/// The colour space last recorded by [`set_detected_color_space`].
pub fn detected_color_space() -> DXGI_COLOR_SPACE_TYPE {
    DXGI_COLOR_SPACE_TYPE(DETECTED.load(Ordering::Acquire))
}

/// Override the detected colour space. [`ColorSpaceOverride::Auto`] gives the
/// detected one back.
pub fn set_color_space_override(value: ColorSpaceOverride) {
    OVERRIDE.store(value.to_repr(), Ordering::Release);
}

/// The override currently in force.
pub fn color_space_override() -> ColorSpaceOverride {
    ColorSpaceOverride::from_repr(OVERRIDE.load(Ordering::Acquire))
}

/// Set the nits plain white is drawn at on an HDR swap chain. A value that is
/// not finite or not positive is ignored in favour of
/// [`DEFAULT_PAPER_WHITE_NITS`] when it is read back.
pub fn set_paper_white_nits(nits: f32) {
    PAPER_WHITE_BITS.store(nits.to_bits(), Ordering::Release);
}

/// The paper-white level the shader scales to. Never returns a value the
/// shader cannot use.
pub fn paper_white_nits() -> f32 {
    let nits = f32::from_bits(PAPER_WHITE_BITS.load(Ordering::Acquire));
    if nits.is_finite() && nits > 0.0 {
        nits
    } else {
        DEFAULT_PAPER_WHITE_NITS
    }
}

/// The `mode` root constant the pixel shader branches on: one of
/// [`SHADER_MODE_SDR`], [`SHADER_MODE_HDR10`] or [`SHADER_MODE_SCRGB`].
///
/// An unrecognised detected colour space is SDR, which is the mode that leaves
/// the pixels exactly as hudhook has always written them.
pub fn shader_mode() -> u32 {
    match color_space_override() {
        ColorSpaceOverride::Sdr => return SHADER_MODE_SDR,
        ColorSpaceOverride::Hdr10 => return SHADER_MODE_HDR10,
        ColorSpaceOverride::ScRgb => return SHADER_MODE_SCRGB,
        ColorSpaceOverride::Auto => {},
    }

    match detected_color_space() {
        DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020 => SHADER_MODE_HDR10,
        DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709 => SHADER_MODE_SCRGB,
        _ => SHADER_MODE_SDR,
    }
}

fn describe_color_space(color_space: DXGI_COLOR_SPACE_TYPE) -> &'static str {
    match color_space {
        DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020 => "HDR10 PQ",
        DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709 => "scRGB",
        DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709 => "sRGB",
        _ => "other, treated as sRGB",
    }
}
