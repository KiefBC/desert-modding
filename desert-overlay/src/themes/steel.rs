//! "Steel and Blood": the hero's plate armour and the blood-red splash behind
//! him from the game's key art. The only cool-toned theme; every other one
//! here runs warm off that same poster's gold and paper tones.

use crate::theme::{rgb, Role, Rgba, Theme};

pub const THEME: Theme = Theme {
    name: "steel",
    title: "Steel and Blood",
    blurb: "Cold plate and a blood-red edge, off the hero's armour and the splash behind him.",
    dim: rgb(0xA9AEB4, 1.0),
    error: rgb(0xD9363B, 1.0),
    window_rounding: 3.0,
    frame_rounding: 2.0,
    grab_rounding: 2.0,
    window_border: 1.0,
    frame_border: 0.0,
    colors,
};

fn colors() -> Vec<(Role, Rgba)> {
    vec![
        (Role::Text, rgb(0xEDEBE7, 1.0)),
        (Role::TextDisabled, rgb(0x8A9098, 1.0)),
        (Role::WindowBg, rgb(0x2A2E33, 0.96)),
        (Role::PopupBg, rgb(0x2A2E33, 0.98)),
        (Role::Border, rgb(0x8A9098, 0.5)),
        (Role::FrameBg, rgb(0x33383E, 1.0)),
        (Role::FrameBgHovered, rgb(0x454A50, 1.0)),
        (Role::FrameBgActive, rgb(0x5A6169, 1.0)),
        (Role::TitleBg, rgb(0x3D4248, 1.0)),
        // The one place gold other than NavHighlight shows up: the focused
        // window's title bar, a tint lifted from the poster lettering's
        // shadow rather than the blood red every other "active" uses.
        (Role::TitleBgActive, rgb(0x8C7226, 1.0)),
        (Role::TitleBgCollapsed, rgb(0x3D4248, 0.8)),
        (Role::ScrollbarBg, rgb(0x1C1B1A, 0.6)),
        (Role::ScrollbarGrab, rgb(0x5A6169, 1.0)),
        (Role::ScrollbarGrabHovered, rgb(0x8A9098, 1.0)),
        (Role::ScrollbarGrabActive, rgb(0x9E1B1E, 1.0)),
        (Role::CheckMark, rgb(0x9E1B1E, 1.0)),
        (Role::SliderGrab, rgb(0x9E1B1E, 1.0)),
        (Role::SliderGrabActive, rgb(0xC8282C, 1.0)),
        (Role::Button, rgb(0x454A50, 1.0)),
        (Role::ButtonHovered, rgb(0x5A6169, 1.0)),
        (Role::ButtonActive, rgb(0x9E1B1E, 1.0)),
        (Role::Header, rgb(0x3D4248, 1.0)),
        (Role::HeaderHovered, rgb(0x5A6169, 1.0)),
        (Role::HeaderActive, rgb(0x9E1B1E, 1.0)),
        (Role::Separator, rgb(0x5A6169, 1.0)),
        (Role::SeparatorHovered, rgb(0x8A9098, 1.0)),
        (Role::SeparatorActive, rgb(0x9E1B1E, 1.0)),
        (Role::ResizeGrip, rgb(0x454A50, 1.0)),
        (Role::ResizeGripHovered, rgb(0x8A9098, 1.0)),
        (Role::ResizeGripActive, rgb(0x9E1B1E, 1.0)),
        (Role::TextSelectedBg, rgb(0x9E1B1E, 0.35)),
        (Role::NavHighlight, rgb(0xC9A648, 1.0)),
    ]
}
