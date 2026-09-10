//! The stock Dear ImGui dark palette, untouched. The baseline every other
//! theme is judged against, and what the menu looked like before themes.

use crate::theme::{rgb, Role, Rgba, Theme};

pub const THEME: Theme = Theme {
    name: "classic",
    title: "Classic",
    blurb: "Dear ImGui's stock dark look, exactly as before.",
    dim: rgb(0xA6A6A6, 1.0),
    error: rgb(0xFF5959, 1.0),
    window_rounding: 0.0,
    frame_rounding: 0.0,
    grab_rounding: 0.0,
    window_border: 1.0,
    frame_border: 0.0,
    colors,
};

fn colors() -> Vec<(Role, Rgba)> {
    Vec::new()
}
