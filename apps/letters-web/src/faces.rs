//! The faces that back up the fonts in a browser. A page cannot read the system's fonts, so a face for
//! emoji, one for each way of writing Chinese, Japanese and Korean, and one for each other script the
//! desktop shows are kept beside the wasm, and fetched when a document first shows a character the fonts
//! lack.

use std::ops::RangeInclusive;

use skrifa::{FontRef, MetadataProvider};

/// A face that backs up the fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Emoji,
    Japanese,
    Korean,
    /// Chinese as Taiwan and Hong Kong write it.
    Traditional,
    /// Chinese as the mainland writes it. It has the most Han characters, so it is the first face for Han
    /// that nothing says more about.
    Simplified,
    Arabic,
    Hebrew,
    Devanagari,
    Thai,
}

impl Face {
    pub const ALL: [Self; 9] = [
        Self::Emoji,
        Self::Japanese,
        Self::Korean,
        Self::Traditional,
        Self::Simplified,
        Self::Arabic,
        Self::Hebrew,
        Self::Devanagari,
        Self::Thai,
    ];

    /// Its file in `fallback/`.
    pub fn file(self) -> &'static str {
        match self {
            Self::Emoji => "NotoColorEmoji.ttf",
            Self::Japanese => "NotoSansJP.otf",
            Self::Korean => "NotoSansKR.otf",
            Self::Traditional => "NotoSansTC.otf",
            Self::Simplified => "NotoSansSC.otf",
            Self::Arabic => "NotoSansArabic.ttf",
            Self::Hebrew => "NotoSansHebrew.ttf",
            Self::Devanagari => "NotoSansDevanagari.ttf",
            Self::Thai => "NotoSansThai.ttf",
        }
    }
}

/// The scripts that one face writes, and where their characters are.
#[expect(clippy::single_range_in_vec_init, reason = "a script may be in one stretch of Unicode or in several")]
const SCRIPTS: [(Face, &[RangeInclusive<u32>]); 7] = [
    (Face::Korean, &[0x1100..=0x11FF, 0x3130..=0x318F, 0xA960..=0xA97F, 0xAC00..=0xD7FF, 0xFFA0..=0xFFDC]),
    (Face::Japanese, &[0x3040..=0x30FF, 0x31F0..=0x31FF, 0xFF66..=0xFF9F, 0x1B000..=0x1B16F]),
    // Bopomofo.
    (Face::Traditional, &[0x3100..=0x312F, 0x31A0..=0x31BF]),
    (Face::Arabic, &[0x0600..=0x06FF, 0x0750..=0x077F, 0x08A0..=0x08FF, 0xFB50..=0xFDFF, 0xFE70..=0xFEFF]),
    (Face::Hebrew, &[0x0590..=0x05FF, 0xFB1D..=0xFB4F]),
    (Face::Devanagari, &[0x0900..=0x097F, 0x1CD0..=0x1CFF, 0xA8E0..=0xA8FF]),
    (Face::Thai, &[0x0E00..=0x0E7F]),
];
/// Han, and the signs and full-width forms the three languages share.
const SHARED: [RangeInclusive<u32>; 10] = [
    0x2E80..=0x2FDF,
    0x3000..=0x303F,
    0x3190..=0x33FF,
    0x3400..=0x4DBF,
    0x4E00..=0x9FFF,
    0xF900..=0xFAFF,
    0xFE30..=0xFE4F,
    0xFF00..=0xFF65,
    0xFFE0..=0xFFEF,
    0x20000..=0x323AF,
];
/// The faces that have Han, the one with the most first.
const HAN: [Face; 4] = [Face::Simplified, Face::Traditional, Face::Japanese, Face::Korean];

/// The face a language's Han is drawn in, when the language is one that writes Han.
fn written(lang: &str) -> Option<Face> {
    let mut parts = lang.split(['-', '_']).map(str::to_ascii_lowercase);
    match parts.next()?.as_str() {
        "ja" => Some(Face::Japanese),
        "ko" => Some(Face::Korean),
        "zh" => Some(match parts.any(|part| ["hant", "tw", "hk", "mo"].contains(&part.as_str())) {
            true => Face::Traditional,
            false => Face::Simplified,
        }),
        _ => None,
    }
}

/// What a character says of the face to set it in.
enum Told {
    Face(Face),
    /// Han, and the face its language says, if it names one.
    Han(Option<Face>),
    Nothing,
}

/// Whether a face's file has a glyph for a character.
pub fn has(file: &[u8], ch: char) -> bool {
    FontRef::new(file).is_ok_and(|face| face.charmap().map(ch).is_some())
}

/// The face to set each character in that no font has, in the order they are given: each as layout tells
/// it, with whether it shows as an emoji and the language of its text. `None` is for a character that no
/// face kept here has.
///
/// `asked` are the faces asked for before. `lacks` says of a face that it is known to be of no use for a
/// character: it was fetched and has no glyph for it, or it could not be fetched.
///
/// Han is drawn as its language writes it. In text that names no such language it is drawn as its
/// neighbours are: in the face that kana or hangul beside it took, now or before. Where that face lacks
/// the character it takes the next that has it, since a box is worse than a stroke drawn another way.
pub fn chosen<'a>(
    uncovered: impl IntoIterator<Item = (char, bool, Option<&'a str>)>,
    asked: &[Face],
    lacks: impl Fn(Face, char) -> bool,
) -> Vec<Option<Face>> {
    let within = |ranges: &[RangeInclusive<u32>], ch: char| ranges.iter().any(|range| range.contains(&(ch as u32)));
    let told: Vec<(char, Told)> = uncovered
        .into_iter()
        .map(|(ch, emoji, lang)| {
            let script = SCRIPTS.iter().find(|(_, ranges)| within(ranges, ch));
            let told = match script {
                _ if emoji => Told::Face(Face::Emoji),
                Some((face, _)) => Told::Face(*face),
                None if within(&SHARED, ch) => Told::Han(lang.and_then(written)),
                None => Told::Nothing,
            };
            (ch, told)
        })
        .collect();
    // The faces Han's neighbours took: those asked for before, then those these characters say.
    let said = told.iter().filter_map(|(_, told)| match told {
        Told::Face(face) => Some(*face),
        Told::Han(face) => *face,
        Told::Nothing => None,
    });
    let beside: Vec<Face> = asked.iter().copied().chain(said).filter(|face| HAN.contains(face)).collect();
    let choose = |(ch, told): &(char, Told)| {
        let of_use = |face: &Face| !lacks(*face, *ch);
        match told {
            Told::Face(face) => Some(*face).filter(of_use),
            Told::Han(Some(said)) => [*said].into_iter().chain(HAN).find(of_use),
            Told::Han(None) => beside.iter().copied().chain(HAN).find(of_use),
            Told::Nothing => None,
        }
    };
    told.iter().map(choose).collect()
}

/// The faces among those [`chosen`] that are not `had`, each once, in the order they are first needed.
pub fn lacking(chosen: &[Option<Face>], had: &[Face]) -> Vec<Face> {
    let mut lacking = Vec::new();
    for face in chosen.iter().flatten().filter(|face| !had.contains(face)) {
        if !lacking.contains(face) {
            lacking.push(*face);
        }
    }
    lacking
}
