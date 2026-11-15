//! The fonts every Letters build shows, so a document opens without touching the system's fonts and looks
//! the same everywhere. All are variable fonts under the SIL Open Font Licence (see `fonts/OFL-*.txt`).
//!
//! A build carries them in its binary, unless the `provided` feature leaves them out for the shell to hand
//! over. The browser shell does, so the fonts are downloaded and cached apart from the code, and a page can
//! show its first frame in the first face to arrive.

#[cfg(feature = "provided")]
use std::sync::Mutex;
#[cfg(not(feature = "provided"))]
use std::sync::OnceLock;

use letters_render::Font;

/// The font files in `fonts/`, in the order [`bundled`] returns their faces. [`provide`] takes their
/// contents in that order too, in one call or over several.
pub const FILES: [&str; 5] =
    ["Inter.ttf", "Inter-Italic.ttf", "FiraCode.ttf", "Montserrat.ttf", "Montserrat-Italic.ttf"];

/// The family used for body text and the interface.
pub const SANS: &str = "Inter";
/// The family used for code.
pub const MONO: &str = "Fira Code";
/// The family used for display headings.
pub const DISPLAY: &str = "Montserrat";

#[cfg(not(feature = "provided"))]
static FONTS: OnceLock<Vec<Font>> = OnceLock::new();
/// What the shell has handed over so far.
#[cfg(feature = "provided")]
static FONTS: Mutex<Vec<Font>> = Mutex::new(Vec::new());

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

/// The faces the shell has handed over with [`provide`] by now, which must be the first of them at least.
/// Repeated calls share the same font data.
#[cfg(feature = "provided")]
pub fn bundled() -> Vec<Font> {
    let fonts = FONTS.lock().expect("nothing fails while the fonts are held").clone();
    assert!(!fonts.is_empty(), "the shell provides a font before anything is laid out");
    fonts
}

/// Hands over the contents of [`FILES`] for a build with the `provided` feature: all at once, or the first
/// and then the rest as they arrive, each call carrying on in that order where the last stopped. A build
/// that carries its own fonts has no use for them.
#[cfg(feature = "provided")]
pub fn provide(files: Vec<Vec<u8>>) {
    let mut fonts = FONTS.lock().expect("nothing fails while the fonts are held");
    fonts.extend(files.into_iter().map(|bytes| Font::new(bytes.into(), 0)));
}

#[cfg(not(feature = "provided"))]
pub fn provide(_files: Vec<Vec<u8>>) {}

#[cfg(all(test, feature = "provided"))]
mod provided {
    #[test]
    fn faces_handed_over_one_after_another_are_all_there_in_the_order_they_came() {
        super::provide(vec![vec![1]]);
        let first = super::bundled();
        assert_eq!(first.len(), 1);
        super::provide(vec![vec![2], vec![3]]);
        let all = super::bundled();
        assert_eq!(all.iter().map(|font| font.data.to_vec()).collect::<Vec<_>>(), [[1], [2], [3]]);
        // The first face is the same data as before, so what was laid out with it still matches.
        assert_eq!(all[0], first[0]);
    }
}

#[cfg(all(test, not(feature = "provided")))]
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
