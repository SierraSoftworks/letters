//! Faces for what the bundled fonts cannot draw, found among the fonts of a folder as they are among a
//! system's.

mod common;

use std::sync::mpsc;
use std::time::Duration;

use common::Scratch;
use letters::fallback::{BUNDLED, Finder, Han, Shelf};
use letters_fonts::FILES;
use letters_layout::Uncovered;
use letters_render::Font;
use skrifa::attribute::Style;
use skrifa::{FontRef, MetadataProvider};

/// A folder that holds the bundled font files, as a system holds its own.
fn folder(name: &str) -> Scratch {
    let scratch = Scratch::new(name);
    for (file, font) in FILES.iter().zip(letters_fonts::bundled()) {
        std::fs::write(scratch.0.join(file), &font.data).unwrap();
    }
    scratch
}

/// The characters of `text` as a layout tells of them, in a document of the language `lang`.
fn uncovered(text: &str, lang: Option<&str>) -> Vec<Uncovered> {
    let emoji = |ch: char| (0x1F000..0x20000).contains(&(ch as u32));
    text.chars().map(|ch| Uncovered { ch, emoji: emoji(ch), lang: lang.map(str::to_owned) }).collect()
}

fn draws(font: &Font, c: char) -> bool {
    let face = FontRef::from_index(&font.data, font.index).unwrap();
    face.charmap().map(c).is_some_and(|glyph| glyph.to_u32() != 0)
}

#[test]
fn each_character_gets_a_face_that_draws_it_and_one_nothing_draws_gets_none() {
    let fonts = folder("fallback-find");
    let mut shelf = Shelf::of(&[&fonts.0], &[]);
    // (what is asked for, the characters of it some font in the folder draws)
    let cases = [("a", "a"), ("aé→", "aé→"), ("日", ""), ("😀", ""), ("a日😀", "a"), ("", "")];
    for (asked, drawn) in cases {
        let faces = shelf.find(&uncovered(asked, None), Han::default());
        for c in drawn.chars() {
            assert!(faces.iter().any(|face| draws(face, c)), "{asked:?}: a face for {c:?}");
        }
        assert!(faces.len() <= drawn.chars().count(), "{asked:?}: at most a face for each character");
    }
}

#[test]
fn a_family_gives_its_upright_face_and_the_bundled_families_give_none() {
    let fonts = folder("fallback-families");
    let faces = Shelf::of(&[&fonts.0], &[]).find(&uncovered("a", None), Han::default());
    let upright =
        |font: &Font| FontRef::from_index(&font.data, font.index).unwrap().attributes().style == Style::Normal;
    assert!(!faces.is_empty() && faces.iter().all(upright), "slanted text is drawn from the upright face");

    // The window has the bundled families already, whatever the system holds of them.
    let mut others = Shelf::of(&[&fonts.0], BUNDLED);
    assert!(others.find(&uncovered("a", None), Han::default()).is_empty());
}

#[test]
fn a_face_taken_twice_is_the_same_bytes() {
    let fonts = folder("fallback-shared");
    let mut shelf = Shelf::of(&[&fonts.0], &[]);
    let first = shelf.find(&uncovered("a", None), Han::default());
    let again = shelf.find(&uncovered("ab", None), Han::default());
    assert!(first[0].data.same(&again[0].data), "a file is read once, and the layout knows the face");
}

#[test]
fn han_characters_lean_to_the_language_the_text_or_the_locale_shows() {
    let languages = [
        ("ja", Some(Han::Japanese)),
        ("ja_JP.UTF-8", Some(Han::Japanese)),
        ("ko-KR", Some(Han::Korean)),
        ("zh_TW.UTF-8", Some(Han::Traditional)),
        ("zh-Hant-HK", Some(Han::Traditional)),
        ("zh", Some(Han::Simplified)),
        ("zh-Hans-CN", Some(Han::Simplified)),
        ("en_GB.UTF-8@euro", None),
        ("C", None),
    ];
    for (language, han) in languages {
        assert_eq!(Han::of_language(language), han, "{language}");
    }
    // The text's language says most, and then what it holds: only Japanese writes kana, only Korean Hangul.
    let texts = [
        ("漢字", None, Han::Traditional, Han::Traditional),
        ("漢字", Some("ja"), Han::Traditional, Han::Japanese),
        ("漢字", Some("en"), Han::Korean, Han::Korean),
        ("漢字かな", None, Han::Traditional, Han::Japanese),
        ("カタカナ", Some("en"), Han::Simplified, Han::Japanese),
        ("한국어", None, Han::Japanese, Han::Korean),
        ("かな", Some("zh-TW"), Han::Simplified, Han::Traditional),
        ("plain", None, Han::Korean, Han::Korean),
    ];
    for (text, lang, before, after) in texts {
        assert_eq!(before.told(&uncovered(text, lang)), after, "{text} in {lang:?}");
    }
}

#[test]
fn a_finder_answers_each_asker_later_and_stays_silent_when_nothing_is_found() {
    let fonts = folder("fallback-finder");
    let (found, answers) = mpsc::channel();
    let folder = fonts.0.clone();
    let mut finder = Finder::over(
        move || Shelf::of(&[&folder], &[]),
        Han::default(),
        move |asker: u32, faces: Vec<Font>| _ = found.send((asker, faces)),
    );
    finder.ask(1, uncovered("a", None));
    finder.ask(2, uncovered("日", None));
    finder.ask(3, uncovered("é", None));
    let mut answered = Vec::new();
    for _ in 0..2 {
        let (asker, faces) = answers.recv_timeout(Duration::from_secs(20)).expect("an answer");
        assert!(!faces.is_empty(), "asker {asker}");
        answered.push(asker);
    }
    assert_eq!(answered, [1, 3], "nothing in the folder draws what the second asked for");
}
