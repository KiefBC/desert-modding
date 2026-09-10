//! "Parchment": the key art's poster ground, not a scroll. Near-white with
//! soft light-grey shading, antique gold for the structure, and the poster's
//! blood-red paint splash kept for anything pressed, hovered or dragged. The
//! only light theme of the set.

use crate::theme::{rgb, Role, Rgba, Theme};

pub const THEME: Theme = Theme {
    name: "parchment",
    title: "Parchment",
    blurb: "The key art's poster ground: off-white, antique gold and blood red, the one light theme of the set.",
    dim: rgb(0x5A6169, 1.0),
    error: rgb(0x9E1B1E, 1.0),
    window_rounding: 2.0,
    frame_rounding: 2.0,
    grab_rounding: 2.0,
    window_border: 1.0,
    frame_border: 1.0,
    colors,
};

fn colors() -> Vec<(Role, Rgba)> {
    // The poster ground and its shading: off-white, not cream, with a soft
    // light-grey shadow and a brighter near-white highlight. The window
    // itself sits at the ground tone so the lighter highlight on frames, and
    // dark text inside them, still reads as lifted off the page around it.
    let ground = rgb(0xEDEBE7, 0.97);
    let shading = rgb(0xDCD8D2, 1.0);
    let highlight = rgb(0xF7F6F3, 1.0);
    let frame_bg_hovered = rgb(0xE3E0DA, 1.0);
    let frame_bg_active = rgb(0xEAD9A8, 1.0); // a pale gold tint, held rather than pressed

    // Antique gold, the title lettering's gradient: the structural family
    // for title bars, headers, buttons, tabs and grips at rest.
    let gold = rgb(0xC9A648, 1.0);
    let gold_bright = rgb(0xD4B24A, 1.0);
    let gold_shadow = rgb(0x8C7226, 1.0); // collapsed title bar, an unfocused tab

    // The paint splash behind the hero: what gets pressed. Hovered and
    // active headers, buttons and tabs, plus the check mark and slider knob
    // outright.
    let blood = rgb(0x9E1B1E, 1.0);
    let blood_dark = rgb(0x6E1214, 1.0);

    // Armour steel, borders and grips: light steel at rest, dark steel
    // hovered, the banner black pressed all the way.
    let steel = rgb(0x5A6169, 1.0);
    let steel_dark = rgb(0x33383E, 1.0);
    let banner = rgb(0x1C1B1A, 1.0); // the ENHANCED banner, also borders and ink

    // Ink and disabled text, plus the scrim behind a modal or the window
    // switcher.
    let ink_dim = rgb(0x8A8781, 1.0);
    let scrim = rgb(0x1C1B1A, 0.6); // dimming overlays stay dark and translucent even in a light theme

    vec![
        (Role::Text, banner),
        (Role::TextDisabled, ink_dim),
        (Role::WindowBg, ground),
        (Role::ChildBg, highlight),
        (Role::PopupBg, highlight),
        (Role::Border, banner),
        (Role::FrameBg, highlight),
        (Role::FrameBgHovered, frame_bg_hovered),
        (Role::FrameBgActive, frame_bg_active),
        (Role::TitleBg, gold),
        (Role::TitleBgActive, gold_bright),
        (Role::TitleBgCollapsed, gold_shadow),
        (Role::MenuBarBg, shading),
        (Role::ScrollbarBg, shading),
        (Role::ScrollbarGrab, steel),
        (Role::ScrollbarGrabHovered, steel_dark),
        (Role::ScrollbarGrabActive, banner),
        (Role::CheckMark, blood),
        (Role::SliderGrab, blood),
        (Role::SliderGrabActive, blood_dark),
        (Role::Button, gold),
        (Role::ButtonHovered, gold_bright),
        (Role::ButtonActive, blood),
        (Role::Header, gold),
        (Role::HeaderHovered, gold_bright),
        (Role::HeaderActive, blood),
        (Role::Separator, rgb(0xC8C4BD, 1.0)),
        (Role::SeparatorHovered, gold),
        (Role::SeparatorActive, blood),
        (Role::ResizeGrip, steel),
        (Role::ResizeGripHovered, steel_dark),
        (Role::ResizeGripActive, banner),
        (Role::Tab, gold),
        (Role::TabHovered, gold_bright),
        (Role::TabActive, blood),
        (Role::TabUnfocused, gold_shadow),
        (Role::TabUnfocusedActive, gold),
        (Role::TableHeaderBg, gold),
        (Role::TableRowBg, highlight),
        (Role::TableRowBgAlt, rgb(0xEDEBE7, 1.0)),
        (Role::TextSelectedBg, rgb(0xC9A648, 0.4)), // translucent gold
        (Role::NavHighlight, gold),
        (Role::NavWindowingDimBg, scrim),
        (Role::ModalWindowDimBg, scrim),
    ]
}
