//! The fonts compiled into every Letters build, so a document opens without touching the system's fonts and
//! looks the same everywhere. All are variable fonts under the SIL Open Font Licence (see `fonts/OFL-*.txt`).

use std::sync::OnceLock;

use letters_render::Font;

const FILES: [&[u8]; 5] = [
    include_bytes!("../fonts/Inter.ttf"),
    include_bytes!("../fonts/Inter-Italic.ttf"),
    include_bytes!("../fonts/FiraCode.ttf"),
    include_bytes!("../fonts/Montserrat.ttf"),
    include_bytes!("../fonts/Montserrat-Italic.ttf"),
];

/// The family used for body text and the interface.
pub const SANS: &str = "Inter";
/// The family used for code.
pub const MONO: &str = "Fira Code";
/// The family used for display headings.
pub const DISPLAY: &str = "Montserrat";

/// Every bundled face. Repeated calls share the same font data.
pub fn bundled() -> Vec<Font> {
    static FONTS: OnceLock<Vec<Font>> = OnceLock::new();
    FONTS.get_or_init(|| FILES.iter().map(|bytes| Font::new((*bytes).into(), 0)).collect()).clone()
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_fonts_are_truetype_and_shared() {
        let fonts = super::bundled();
        assert!(fonts.iter().all(|font| font.data.starts_with(&[0, 1, 0, 0])));
        assert_eq!(fonts, super::bundled());
    }
}
