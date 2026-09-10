//! The game's own key art: a black banner with gold lettering, taken over
//! the whole menu. Square corners, a near-black window, and every ordinary
//! text label set in the same antique gold as the poster's ENHANCED banner;
//! blood red is reserved for whatever is actively pressed or dragged.

use crate::theme::{rgb, Role, Rgba, Theme};

pub const THEME: Theme = Theme {
    name: "banner",
    title: "Enhanced Banner",
    blurb: "Gold lettering on the game's black banner, square corners throughout.",
    dim: rgb(0xA08F5A, 1.0),
    error: rgb(0xC8282C, 1.0),
    window_rounding: 0.0,
    frame_rounding: 0.0,
    grab_rounding: 0.0,
    window_border: 1.0,
    frame_border: 0.0,
    colors,
};

fn colors() -> Vec<(Role, Rgba)> {
    // Gold, the banner lettering.
    let gold_bright = rgb(0xD4B24A, 1.0); // title-art bright gold, text and check marks
    let gold_dragged = rgb(0xF0D060, 1.0); // brighter still, a grab under the pointer
    let gold_mid = rgb(0xC9A648, 1.0); // title-art mid gold, borders
    let gold_shadow = rgb(0x8C7226, 1.0); // title-art shadow gold, rest-state hovers
    let gold_dim = rgb(0x6B5A22, 1.0); // shadow gold dimmed further, disabled text

    // Blood red, reserved for active and pressed states.
    let blood = rgb(0x9E1B1E, 1.0);
    let blood_deep = rgb(0x6E1214, 1.0); // the splash's darker red, section headers at rest

    // Neutrals: the banner black and the warm greys inside it.
    let banner_black = rgb(0x1C1B1A, 0.96); // the poster's black banner, window background
    let banner_black_solid = rgb(0x1C1B1A, 1.0); // same black, fully opaque for popups
    let charcoal = rgb(0x262421, 1.0); // dark warm charcoal, title bars and headers at rest
    let charcoal_deep = rgb(0x161514, 1.0); // darker still, a collapsed title bar
    let frame = rgb(0x2A2826, 1.0); // frame background at rest
    let frame_hovered = rgb(0x3A3733, 1.0); // frame background, hovered
    let frame_active = rgb(0x4A4640, 1.0); // frame background, held
    let button = rgb(0x33302B, 1.0); // button at rest

    // Steel, the armour grey, for the scrollbar track and thumb.
    let steel = rgb(0x5A6169, 1.0);
    let steel_dark = rgb(0x33383E, 1.0);

    vec![
        (Role::Text, gold_bright),
        (Role::TextDisabled, gold_dim),
        (Role::WindowBg, banner_black),
        (Role::PopupBg, banner_black_solid),
        (Role::Border, gold_mid),
        (Role::FrameBg, frame),
        (Role::FrameBgHovered, frame_hovered),
        (Role::FrameBgActive, frame_active),
        (Role::TitleBg, charcoal),
        (Role::TitleBgActive, blood),
        (Role::TitleBgCollapsed, charcoal_deep),
        (Role::ScrollbarBg, steel_dark),
        (Role::ScrollbarGrab, steel),
        (Role::ScrollbarGrabHovered, gold_bright),
        (Role::ScrollbarGrabActive, gold_dragged),
        (Role::CheckMark, gold_bright),
        (Role::SliderGrab, gold_bright),
        (Role::SliderGrabActive, gold_dragged),
        (Role::Button, button),
        (Role::ButtonHovered, gold_shadow),
        (Role::ButtonActive, blood),
        // Section headers are separators, not frames, so they carry the
        // splash's red rather than the charcoal the widgets sit on: deep red at
        // rest, the brighter splash red under the pointer, gold when pressed.
        (Role::Header, blood_deep),
        (Role::HeaderHovered, blood),
        (Role::HeaderActive, gold_shadow),
        (Role::Separator, rgb(0xC9A648, 0.4)),
        (Role::SeparatorHovered, gold_bright),
        (Role::SeparatorActive, blood),
        (Role::ResizeGrip, button),
        (Role::ResizeGripHovered, gold_shadow),
        (Role::ResizeGripActive, blood),
        (Role::TextSelectedBg, rgb(0xD4B24A, 0.35)),
        (Role::NavHighlight, gold_bright),
    ]
}
