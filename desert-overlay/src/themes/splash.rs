//! "Crimson Splash": the game's key art rendered as a settings window, a
//! near-white poster ground behind antique gold lettering and a blood-red
//! paint splash. Where the sister theme, "parchment", leads with gold and
//! keeps crimson for hover and press, splash inverts it: the structure sits
//! in a quiet warm grey, gold only ever marks a hover, and blood red carries
//! every border, check mark, slider knob and pressed state.

use crate::theme::{rgb, Role, Rgba, Theme};

pub const THEME: Theme = Theme {
    name: "splash",
    title: "Crimson Splash",
    blurb: "The key art as a menu: poster white, antique gold and a blood-red splash.",
    dim: rgb(0x5A6169, 1.0),
    error: rgb(0x6E1214, 1.0),
    window_rounding: 2.0,
    frame_rounding: 2.0,
    grab_rounding: 2.0,
    window_border: 1.0,
    frame_border: 1.0,
    colors,
};

fn colors() -> Vec<(Role, Rgba)> {
    // Poster white, the backgrounds. Ground, shading and highlight straight
    // off the key art; the window sits at the ground tone so the highlight
    // reads as lighter still, the way a frame should against the page.
    let ground = rgb(0xEDEBE7, 1.0);
    let shading = rgb(0xDCD8D2, 1.0);
    let highlight = rgb(0xF7F6F3, 1.0);

    // The paint splash, pale to full strength. Pale red is what a frame
    // becomes under the mouse or under the mark; full strength is every
    // border, check mark and knob outright; dark is whatever is being
    // dragged or has failed.
    let red_pale_hover = rgb(0xF3E1E1, 1.0);
    let red_pale_active = rgb(0xE9C9C9, 1.0);
    let red = rgb(0x9E1B1E, 1.0);
    let red_dark = rgb(0x6E1214, 1.0);

    // Antique gold, trim only: a hover on the structural bars, and the
    // button family, which cannot itself go red because button text is the
    // global black and red-on-black does not read.
    let gold_bright = rgb(0xD4B24A, 1.0);
    let gold_mid = rgb(0xC9A648, 1.0);
    let gold_pale = rgb(0xE6D394, 1.0);

    // Armour steel and leather, for the grips imgui needs somewhere to put.
    let steel = rgb(0x5A6169, 1.0);
    let steel_dark = rgb(0x33383E, 1.0);
    let leather = rgb(0x5A4634, 1.0);

    // Ink, and the banner black doubling as the scrim behind a modal or the
    // window switcher: dimming overlays stay dark even in a light theme.
    let ink = rgb(0x1C1B1A, 1.0);
    let ink_dim = rgb(0x8A8781, 1.0);
    let scrim = rgb(0x1C1B1A, 0.6);

    vec![
        (Role::Text, ink),
        (Role::TextDisabled, ink_dim),
        (Role::WindowBg, rgb(0xEDEBE7, 0.97)),
        (Role::ChildBg, ground),
        (Role::PopupBg, highlight),
        (Role::Border, red),
        (Role::FrameBg, highlight),
        (Role::FrameBgHovered, red_pale_hover),
        (Role::FrameBgActive, red_pale_active),
        (Role::TitleBg, shading),
        (Role::TitleBgActive, red),
        (Role::TitleBgCollapsed, shading),
        (Role::MenuBarBg, shading),
        (Role::ScrollbarBg, shading),
        (Role::ScrollbarGrab, steel),
        (Role::ScrollbarGrabHovered, steel_dark),
        (Role::ScrollbarGrabActive, leather),
        (Role::CheckMark, red),
        (Role::SliderGrab, red),
        (Role::SliderGrabActive, red_dark),
        (Role::Button, gold_pale),
        (Role::ButtonHovered, gold_bright),
        (Role::ButtonActive, red),
        (Role::Header, shading),
        (Role::HeaderHovered, gold_mid),
        (Role::HeaderActive, red),
        (Role::Separator, rgb(0x9E1B1E, 0.5)),
        (Role::SeparatorHovered, red),
        (Role::SeparatorActive, red_dark),
        (Role::ResizeGrip, rgb(0x5A6169, 0.5)),
        (Role::ResizeGripHovered, rgb(0x33383E, 0.7)),
        (Role::ResizeGripActive, leather),
        (Role::Tab, shading),
        (Role::TabHovered, gold_mid),
        (Role::TabActive, red),
        (Role::TabUnfocused, ground),
        (Role::TabUnfocusedActive, shading),
        (Role::TableHeaderBg, shading),
        (Role::TableRowBg, highlight),
        (Role::TableRowBgAlt, ground),
        (Role::TextSelectedBg, rgb(0x9E1B1E, 0.35)),
        (Role::NavHighlight, gold_mid),
        (Role::NavWindowingDimBg, scrim),
        (Role::ModalWindowDimBg, scrim),
    ]
}
