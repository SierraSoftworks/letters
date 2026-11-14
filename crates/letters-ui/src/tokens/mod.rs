//! The yewi design tokens, mirrored from `yewi-tokens` until that crate is published from yewi's main branch.
//!
//! The types and `generated.rs` are copies of `yewi-tokens` as it stands in SierraSoftworks/yewi#87, so the
//! switch is replacing this module with `pub use yewi_tokens as tokens`. Do not add tokens here: add them to
//! yewi's `tokens.scss`.

/// Which of a colour's two values to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scheme {
    #[default]
    Light,
    Dark,
}

/// An sRGB colour with straight (not premultiplied) alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

/// A colour token: one value for each scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub light: Rgba,
    pub dark: Rgba,
}

impl Color {
    pub const fn resolve(self, scheme: Scheme) -> Rgba {
        match scheme {
            Scheme::Light => self.light,
            Scheme::Dark => self.dark,
        }
    }
}

/// One layer of a shadow token. Lengths are logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

#[rustfmt::skip]
mod generated;
pub use generated::*;
