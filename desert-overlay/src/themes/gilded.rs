//! Gold on black, the game's own logo look. A near-black charcoal window
//! with antique brass headers and borders, ivory text, and bright gold on
//! every interactive control; crimson shows up only when something is
//! actively pressed or dragged, so it reads as a spark rather than a colour.

use crate::theme::{rgb, Role, Rgba, Theme};

pub const THEME: Theme = Theme {
    name: "gilded",
    title: "Gilded Ash",
    blurb: "Gold on black, crimson only when you press something.",
    dim: rgb(0x9C9082, 1.0),
    error: rgb(0xE5484D, 1.0),
    window_rounding: 2.0,
    frame_rounding: 2.0,
    grab_rounding: 2.0,
    window_border: 1.0,
    frame_border: 1.0,
    colors,
};

fn colors() -> Vec<(Role, Rgba)> {
    // Brass and gold.
    let brass_dark = rgb(0x6B5220, 1.0); // antique brass, titles and headers at rest
    let gold = rgb(0xC9A227, 1.0); // brand gold, borders and hovered brass
    let gold_bright = rgb(0xE0BC5A, 1.0); // brand bright gold, grabs and check marks

    // Crimson, reserved for active/pressed states.
    let crimson = rgb(0xA31621, 1.0);

    // Neutrals.
    let ivory = rgb(0xF1E7D0, 1.0); // brand parchment ivory, text
    let khaki_dim = rgb(0x8C8267, 1.0); // dulled ivory, disabled text
    let charcoal = rgb(0x0F0D0C, 0.955); // brand near-black, window background
    let charcoal_solid = rgb(0x161211, 1.0); // same charcoal, fully opaque for popups
    let charcoal_deep = rgb(0x0A0908, 1.0); // darker still, scrollbar track and collapsed title
    let grey_warm = rgb(0x332C26, 1.0); // dark warm grey, frame background at rest
    let grey_warm_hover = rgb(0x453A2E, 1.0); // frame background, hovered
    let grey_warm_active = rgb(0x5A4A36, 1.0); // frame background, held

    vec![
        (Role::Text, ivory),
        (Role::TextDisabled, khaki_dim),
        (Role::WindowBg, charcoal),
        (Role::PopupBg, charcoal_solid),
        (Role::Border, gold),
        (Role::FrameBg, grey_warm),
        (Role::FrameBgHovered, grey_warm_hover),
        (Role::FrameBgActive, grey_warm_active),
        (Role::TitleBg, brass_dark),
        (Role::TitleBgActive, gold),
        (Role::TitleBgCollapsed, charcoal_deep),
        (Role::ScrollbarBg, charcoal_deep),
        (Role::ScrollbarGrab, brass_dark),
        (Role::ScrollbarGrabHovered, gold),
        (Role::ScrollbarGrabActive, gold_bright),
        (Role::CheckMark, gold_bright),
        (Role::SliderGrab, gold_bright),
        (Role::SliderGrabActive, crimson),
        (Role::Button, brass_dark),
        (Role::ButtonHovered, gold),
        (Role::ButtonActive, crimson),
        (Role::Header, brass_dark),
        (Role::HeaderHovered, gold),
        (Role::HeaderActive, crimson),
        (Role::Separator, gold),
        (Role::SeparatorHovered, gold_bright),
        (Role::SeparatorActive, crimson),
        (Role::ResizeGrip, brass_dark),
        (Role::ResizeGripHovered, gold),
        (Role::ResizeGripActive, crimson),
        (Role::TextSelectedBg, crimson),
        (Role::NavHighlight, gold_bright),
    ]
}
