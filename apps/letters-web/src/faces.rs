//! The faces that back up the fonts in a browser. A page cannot read the system's fonts, so a face for
//! emoji and one for each way of writing Chinese, Japanese and Korean are kept beside the wasm, and
//! fetched when a document first shows a character the fonts lack.

use std::ops::RangeInclusive;

/// A face that backs up the fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Emoji,
    Japanese,
    Korean,
    /// Chinese as Taiwan and Hong Kong write it.
    Traditional,
    /// Chinese as the mainland writes it. It has the most Han characters, so it is the face for Han that
    /// nothing says more about.
    Simplified,
}

impl Face {
    pub const ALL: [Self; 5] = [Self::Emoji, Self::Japanese, Self::Korean, Self::Traditional, Self::Simplified];

    /// Its file in `fallback/`.
    pub fn file(self) -> &'static str {
        match self {
            Self::Emoji => "NotoColorEmoji.ttf",
            Self::Japanese => "NotoSansJP.otf",
            Self::Korean => "NotoSansKR.otf",
            Self::Traditional => "NotoSansTC.otf",
            Self::Simplified => "NotoSansSC.otf",
        }
    }
}

const HANGUL: [RangeInclusive<u32>; 5] =
    [0x1100..=0x11FF, 0x3130..=0x318F, 0xA960..=0xA97F, 0xAC00..=0xD7FF, 0xFFA0..=0xFFDC];
const KANA: [RangeInclusive<u32>; 4] = [0x3040..=0x30FF, 0x31F0..=0x31FF, 0xFF66..=0xFF9F, 0x1B000..=0x1B16F];
const BOPOMOFO: [RangeInclusive<u32>; 2] = [0x3100..=0x312F, 0x31A0..=0x31BF];
/// Han, and the signs and full-width forms the three languages share.
const SHARED: [RangeInclusive<u32>; 8] = [
    0x2E80..=0x2FDF,
    0x3000..=0x303F,
    0x3190..=0x33FF,
    0x3400..=0x4DBF,
    0x4E00..=0x9FFF,
    0xF900..=0xFAFF,
    0xFE30..=0xFE4F,
    0x20000..=0x323AF,
];
const FULL_WIDTH: [RangeInclusive<u32>; 2] = [0xFF00..=0xFF65, 0xFFE0..=0xFFEF];

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

/// The faces to fetch for characters no font has: each as layout tells it, with whether it shows as an
/// emoji and the language of its text. `had` are the faces fetched before.
///
/// Han in text that names no language that writes it is drawn as its neighbours are: in the face that
/// kana or hangul beside it asked for, now or before. A character of a script no face is kept for asks
/// for nothing.
pub fn wanted<'a>(uncovered: impl IntoIterator<Item = (char, bool, Option<&'a str>)>, had: &[Face]) -> Vec<Face> {
    let within = |ranges: &[RangeInclusive<u32>], ch: char| ranges.iter().any(|range| range.contains(&(ch as u32)));
    let (mut wanted, mut unsaid) = (Vec::new(), false);
    for (ch, emoji, lang) in uncovered {
        let face = match ch {
            _ if emoji => Some(Face::Emoji),
            _ if within(&HANGUL, ch) => Some(Face::Korean),
            _ if within(&KANA, ch) => Some(Face::Japanese),
            _ if within(&BOPOMOFO, ch) => Some(Face::Traditional),
            _ if within(&SHARED, ch) || within(&FULL_WIDTH, ch) => {
                let said = lang.and_then(written);
                unsaid |= said.is_none();
                said
            }
            _ => None,
        };
        wanted.extend(face.filter(|face| !wanted.contains(face) && !had.contains(face)));
    }
    let beside = |faces: &[Face]| faces.iter().any(|face| *face != Face::Emoji);
    if unsaid && !beside(&wanted) && !beside(had) {
        wanted.push(Face::Simplified);
    }
    wanted
}
