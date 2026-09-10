//! The themes, one per file, and the list the picker walks.
//!
//! Adding a theme: a new file here with a `pub const THEME: Theme`, a `mod`
//! line and an entry in [`ALL`]. The tests in [`crate::theme`] check every
//! entry for range, unique names and a title and blurb.

use crate::theme::Theme;

pub mod banner;
pub mod classic;
pub mod gilded;
pub mod parchment;
pub mod splash;
pub mod steel;

/// Every theme, in picker order. `classic` first because it is the unstyled
/// baseline the others are compared against.
pub const ALL: &[&Theme] = &[
    &classic::THEME,
    &parchment::THEME,
    &gilded::THEME,
    &splash::THEME,
    &banner::THEME,
    &steel::THEME,
];

/// The theme used when the ini names none or names one that does not exist.
pub const DEFAULT: &str = "banner";
