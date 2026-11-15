//! The KaTeX faces, compiled in so a formula looks the same everywhere. All are under the SIL Open Font
//! Licence (see `fonts/OFL-KaTeX.txt`).
//!
//! A build with the `provided` feature leaves them out for the shell to hand over, as the browser shell
//! does: a page with no maths shows its first frame without them.

use std::sync::OnceLock;

use letters_render::Font;
use skrifa::charmap::Charmap;
use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, GlyphId, MetadataProvider};

macro_rules! faces {
    ($($name:literal),* $(,)?) => {
        /// Each face's name, as the engine asks for it. The main face is first.
        const NAMES: [&str; 19] = [$($name),*];
        /// The font files in `fonts/`, in the order [`provide`] takes them.
        pub const FILES: [&str; 19] = [$(concat!("KaTeX_", $name, ".ttf")),*];
        #[cfg(not(feature = "provided"))]
        const BYTES: [&[u8]; 19] = [$(include_bytes!(concat!("../fonts/KaTeX_", $name, ".ttf"))),*];
    };
}

faces![
    "Main-Regular",
    "Main-Bold",
    "Main-Italic",
    "Main-BoldItalic",
    "Math-Italic",
    "Math-BoldItalic",
    "AMS-Regular",
    "Caligraphic-Regular",
    "Fraktur-Regular",
    "Fraktur-Bold",
    "SansSerif-Regular",
    "SansSerif-Bold",
    "SansSerif-Italic",
    "Script-Regular",
    "Typewriter-Regular",
    "Size1-Regular",
    "Size2-Regular",
    "Size3-Regular",
    "Size4-Regular",
];

struct Face {
    name: &'static str,
    font: Font,
    chars: Charmap<'static>,
}

static FACES: OnceLock<Vec<Face>> = OnceLock::new();

/// Reads the faces from the contents of [`FILES`].
fn load(files: &[&'static [u8]]) -> Vec<Face> {
    let face = |(name, bytes): (&'static str, &&'static [u8])| {
        let chars = FontRef::new(bytes).ok()?.charmap();
        Some(Face { name, font: Font::new((*bytes).into(), 0), chars })
    };
    NAMES.into_iter().zip(files).filter_map(face).collect()
}

#[cfg(not(feature = "provided"))]
fn faces() -> &'static [Face] {
    FACES.get_or_init(|| load(&BYTES))
}

/// The faces the shell handed over. Until it has, the body face draws what it can.
#[cfg(feature = "provided")]
fn faces() -> &'static [Face] {
    FACES.get().map_or(&[], Vec::as_slice)
}

/// Hands over the contents of [`FILES`], in that order, for a build with the `provided` feature. They are
/// taken once and kept for as long as the program runs. A formula set before then is drawn without them,
/// so hand them over before the first is set. A build that carries its faces has no use for them.
pub fn provide(files: Vec<Vec<u8>>) {
    if cfg!(feature = "provided") {
        let files: Vec<&'static [u8]> = files.into_iter().map(|file| &*file.leak()).collect();
        let _ = FACES.set(load(&files));
    }
}

/// A maths face's place among [`FILES`], if `font` is one of them.
pub(crate) fn place(font: &Font) -> Option<usize> {
    let held = faces().iter().find(|face| face.font == *font)?;
    NAMES.iter().position(|name| *name == held.name)
}

/// The maths face at a place among [`FILES`], if it is here.
pub(crate) fn placed(place: usize) -> Option<Font> {
    let name = NAMES.get(place)?;
    faces().iter().find(|face| face.name == *name).map(|face| face.font.clone())
}

/// The body face, which draws the letters the maths faces lack.
pub(crate) fn body_font() -> Option<Font> {
    letters_fonts::bundled().into_iter().next()
}

/// The face and glyph that draw `character`: the named face, else the main one, else the body face. `None`
/// when no bundled face has it, as for Chinese.
pub(crate) fn glyph(face: &str, character: char) -> Option<(Font, u32)> {
    let faces = faces();
    let named = faces.iter().find(|candidate| candidate.name == face);
    let maths = named.into_iter().chain(faces.first()).find_map(|face| {
        let id = face.chars.map(character).filter(|id| *id != GlyphId::NOTDEF)?;
        Some((face.font.clone(), id.to_u32()))
    });
    maths.or_else(|| body(character).map(|(font, id)| (font, id.to_u32())))
}

/// The width, height and depth, in em, of a character that only the body face draws. The engine has no
/// measurements for it.
pub(crate) fn body_metrics(character: char) -> Option<[f64; 3]> {
    if faces().first().is_some_and(|main| main.chars.map(character).is_some()) {
        return None;
    }
    let (font, id) = body(character)?;
    let face = FontRef::from_index(&font.data, font.index).ok()?;
    let em = f64::from(face.metrics(Size::unscaled(), LocationRef::default()).units_per_em);
    let metrics = face.glyph_metrics(Size::unscaled(), LocationRef::default());
    let width = f64::from(metrics.advance_width(id)?) / em;
    let bounds = metrics.bounds(id).unwrap_or_default();
    Some([width, f64::from(bounds.y_max.max(0.0)) / em, f64::from((-bounds.y_min).max(0.0)) / em])
}

/// A glyph of the body face, which draws the letters the maths faces lack, such as Cyrillic in `\text`.
fn body(character: char) -> Option<(Font, GlyphId)> {
    let font = body_font()?;
    let chars = FontRef::from_index(&font.data, font.index).ok()?.charmap();
    let id = chars.map(character).filter(|id| *id != GlyphId::NOTDEF)?;
    Some((font, id))
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_face_is_bundled_and_shared() {
        let faces = super::faces();
        assert_eq!(faces.len(), super::FILES.len());
        assert_eq!(super::glyph("Math-Italic", 'x').map(|(font, _)| font), Some(faces[4].font.clone()));
    }
}
