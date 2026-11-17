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

#[cfg(feature = "library")]
pub mod library;

/// The font files in `fonts/`, in the order [`bundled`] returns their faces. [`provide`] takes their
/// contents in that order too, in one call or over several.
pub const FILES: [&str; 5] =
    ["Inter.ttf", "Inter-Italic.ttf", "FiraCode.ttf", "Montserrat.ttf", "Montserrat-Italic.ttf"];

/// A stand-in for the first of [`FILES`], for a shell that fetches its fonts: the same face with only the
/// characters of [`FIRST_COVERS`], at a third of the download. Its glyphs keep their numbers, advances and
/// layout features, so text set in it is set the same in the whole face. A shell hands it over with
/// [`provide_first`] and shows its first frame in it. `scripts/subset-fonts.sh` makes the file.
pub const FIRST: &str = "Inter-Latin.ttf";

/// The characters [`FIRST`] is cut down to, as ranges with both ends in: the Latin of western and central
/// Europe, punctuation, arrows and the signs the interface writes. It has those of them that the whole
/// face has.
pub const FIRST_COVERS: [(char, char); 11] = [
    (' ', '~'),
    ('\u{a0}', '\u{17f}'),
    ('\u{2000}', '\u{206f}'),
    ('\u{20ac}', '\u{20ac}'),
    ('\u{2122}', '\u{2122}'),
    ('\u{2190}', '\u{21ff}'),
    ('\u{2212}', '\u{2212}'),
    ('\u{2300}', '\u{23ff}'),
    ('\u{2713}', '\u{2713}'),
    ('\u{feff}', '\u{feff}'),
    ('\u{fffd}', '\u{fffd}'),
];

/// Whether [`FIRST`] sets `text` as the whole face would: none of its characters is outside
/// [`FIRST_COVERS`]. A line end or a tab is no glyph of either.
pub fn first_writes(text: &str) -> bool {
    text.chars().all(|c| c.is_control() || FIRST_COVERS.iter().any(|(from, to)| (*from..=*to).contains(&c)))
}

/// The family used for body text and the interface.
pub const SANS: &str = "Inter";
/// The family used for code.
pub const MONO: &str = "Fira Code";
/// The family used for display headings.
pub const DISPLAY: &str = "Montserrat";

#[cfg(not(feature = "provided"))]
static FONTS: OnceLock<Vec<Font>> = OnceLock::new();
/// What the shell has handed over so far, and whether the first of it is [`FIRST`] standing in.
#[cfg(feature = "provided")]
static FONTS: Mutex<(Vec<Font>, bool)> = Mutex::new((Vec::new(), false));

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
    let fonts = FONTS.lock().expect("nothing fails while the fonts are held").0.clone();
    assert!(!fonts.is_empty(), "the shell provides a font before anything is laid out");
    fonts
}

/// Hands over the contents of [`FILES`] for a build with the `provided` feature: all at once, or the first
/// and then the rest as they arrive, each call carrying on in that order where the last stopped. A build
/// that carries its own fonts has no use for them.
///
/// After [`provide_first`] the first file given here takes the stand-in's place.
#[cfg(feature = "provided")]
pub fn provide(files: Vec<Vec<u8>>) {
    let mut held = FONTS.lock().expect("nothing fails while the fonts are held");
    let (fonts, stand_in) = &mut *held;
    if !files.is_empty() && std::mem::take(stand_in) {
        fonts.clear();
    }
    fonts.extend(files.into_iter().map(|bytes| Font::new(bytes.into(), 0)));
}

#[cfg(not(feature = "provided"))]
pub fn provide(_files: Vec<Vec<u8>>) {}

/// Hands over the contents of [`FIRST`] before any of [`FILES`], for a build with the `provided` feature
/// that would rather show its first frame sooner: [`bundled`] is the stand-in alone until [`provide`]
/// brings the whole face. It is for text that [`first_writes`] says it sets, and changes nothing once a
/// face has been handed over.
#[cfg(feature = "provided")]
pub fn provide_first(file: Vec<u8>) {
    let mut held = FONTS.lock().expect("nothing fails while the fonts are held");
    if held.0.is_empty() {
        *held = (vec![Font::new(file.into(), 0)], true);
    }
}

#[cfg(not(feature = "provided"))]
pub fn provide_first(_file: Vec<u8>) {}

/// Whether `bytes` are a font file that text can be set in. What a document holds under a font's name
/// is asked this before it is kept or used.
pub fn is_font(bytes: &[u8]) -> bool {
    skrifa::FontRef::new(bytes).is_ok()
}

#[cfg(all(test, feature = "provided"))]
mod provided {
    // The fonts are one store for the whole process, so one test hands them all over.
    #[test]
    fn faces_handed_over_one_after_another_are_all_there_in_the_order_they_came() {
        let data = || super::bundled().iter().map(|font| font.data.to_vec()).collect::<Vec<_>>();
        // A stand-in is all there is until the whole face comes, which takes its place.
        super::provide_first(vec![0]);
        assert_eq!(data(), [[0]]);
        super::provide(vec![vec![1]]);
        let first = super::bundled();
        assert_eq!(data(), [[1]]);
        super::provide(vec![vec![2], vec![3]]);
        let all = super::bundled();
        assert_eq!(data(), [[1], [2], [3]]);
        // The first face is the same data as before, so what was laid out with it still matches.
        assert_eq!(all[0], first[0]);
        // A stand-in that comes after a face stands in for nothing.
        super::provide_first(vec![9]);
        assert_eq!(data(), [[1], [2], [3]]);
    }
}

#[cfg(test)]
mod first {
    use skrifa::instance::{LocationRef, Size};
    use skrifa::{FontRef, MetadataProvider};

    use super::FIRST_COVERS;

    const WHOLE: &[u8] = include_bytes!("../fonts/Inter.ttf");
    const FIRST: &[u8] = include_bytes!("../fonts/Inter-Latin.ttf");

    #[test]
    fn the_stand_in_sets_its_characters_as_the_whole_face_does() {
        let [whole, first] = [WHOLE, FIRST].map(|file| FontRef::new(file).expect("a font file"));
        let covered = FIRST_COVERS.iter().flat_map(|(from, to)| *from..=*to);
        let both: Vec<char> = covered.filter(|c| whole.charmap().map(*c).is_some()).collect();
        assert!(both.len() > 300, "the stand-in is for the Latin a first frame shows: {}", both.len());
        // Regular and bold text, a heading, and the heaviest and largest the face is drawn at.
        for (weight, size) in [(400.0, 14.0), (700.0, 14.0), (600.0, 28.0), (900.0, 32.0)] {
            let at = |font: &FontRef| font.axes().location([("wght", weight), ("opsz", size)]);
            let (there, here) = (at(&whole), at(&first));
            let advance = |font: &FontRef, location: LocationRef, c: char| {
                let glyph = font.charmap().map(c)?;
                Some((glyph, font.glyph_metrics(Size::unscaled(), location).advance_width(glyph)))
            };
            for c in &both {
                let (theirs, ours) = (advance(&whole, (&there).into(), *c), advance(&first, (&here).into(), *c));
                assert_eq!(ours, theirs, "{c:?} at weight {weight}, size {size}");
            }
        }
    }

    #[test]
    fn the_stand_in_is_a_fraction_of_the_whole_face_and_says_truly_what_it_writes() {
        assert!(FIRST.len() * 2 < WHOLE.len(), "{} bytes for {}", FIRST.len(), WHOLE.len());
        let first = FontRef::new(FIRST).expect("a font file");
        let cases =
            [("Plain text, with \u{201c}quotes\u{201d} \u{2026}\n\tand a tab.", true), ("Ærø, Łódź, 10 €", true)];
        let beyond = [
            ("\u{391}\u{3b8}\u{3ae}\u{3bd}\u{3b1}", false),
            ("\u{41c}\u{43e}\u{441}\u{43a}\u{432}\u{430}", false),
            ("a \u{1f600}", false),
        ];
        for (text, written) in cases.into_iter().chain(beyond) {
            assert_eq!(super::first_writes(text), written, "{text}");
            let glyphs = text.chars().filter(|c| !c.is_control()).all(|c| first.charmap().map(c).is_some());
            assert_eq!(glyphs, written, "{text}: what it is said to write, it has the glyphs for");
        }
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
