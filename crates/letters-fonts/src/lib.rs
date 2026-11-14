//! The fonts every Letters build shows, so a document opens without touching the system's fonts and looks
//! the same everywhere. All are variable fonts under the SIL Open Font Licence (see `fonts/OFL-*.txt`).
//!
//! A build carries them in its binary, unless the `provided` feature leaves them out for the shell to hand
//! over. The browser shell does, so the fonts are downloaded and cached apart from the code.

use std::sync::OnceLock;

use letters_render::Font;

/// The font files in `fonts/`, in the order [`bundled`] returns their faces and [`provide`] takes them.
pub const FILES: [&str; 5] =
    ["Inter.ttf", "Inter-Italic.ttf", "FiraCode.ttf", "Montserrat.ttf", "Montserrat-Italic.ttf"];

/// The family used for body text and the interface.
pub const SANS: &str = "Inter";
/// The family used for code.
pub const MONO: &str = "Fira Code";
/// The family used for display headings.
pub const DISPLAY: &str = "Montserrat";

static FONTS: OnceLock<Vec<Font>> = OnceLock::new();

/// Every bundled face. Repeated calls share the same font data.
#[cfg(not(feature = "provided"))]
pub fn bundled() -> Vec<Font> {
    const BYTES: [&[u8]; 5] = [
        include_bytes!("../fonts/Inter.ttf"),
        include_bytes!("../fonts/Inter-Italic.ttf"),
        include_bytes!("../fonts/FiraCode.ttf"),
        include_bytes!("../fonts/Montserrat.ttf"),
        include_bytes!("../fonts/Montserrat-Italic.ttf"),
    ];
    FONTS.get_or_init(|| BYTES.iter().map(|bytes| Font::new((*bytes).into(), 0)).collect()).clone()
}

/// Every bundled face, which the shell must have handed over with [`provide`] by now.
#[cfg(feature = "provided")]
pub fn bundled() -> Vec<Font> {
    FONTS.get().expect("the shell provides the fonts before anything is laid out").clone()
}

/// Hands over the contents of [`FILES`], in that order, for a build with the `provided` feature. They are
/// taken once. A build that carries its own fonts has no use for them.
pub fn provide(files: Vec<Vec<u8>>) {
    if cfg!(feature = "provided") {
        let _ = FONTS.set(files.into_iter().map(|bytes| Font::new(bytes.into(), 0)).collect());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_fonts_are_truetype_and_shared() {
        let fonts = super::bundled();
        assert_eq!(fonts.len(), super::FILES.len());
        assert!(fonts.iter().all(|font| font.data.starts_with(&[0, 1, 0, 0])));
        assert_eq!(fonts, super::bundled());
    }

    #[test]
    fn a_build_that_carries_its_fonts_keeps_them_when_handed_others() {
        super::provide(vec![vec![1, 2, 3]]);
        assert_eq!(super::bundled().len(), super::FILES.len());
    }
}
