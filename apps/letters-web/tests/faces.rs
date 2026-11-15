//! Which face backs up the fonts for the characters they lack.

use letters_web::faces::{Face, chosen, lacking};

/// The characters of `text` as layout tells them when no font has them, written in `lang`.
fn told<'a>(text: &'a str, lang: Option<&'a str>) -> impl Iterator<Item = (char, bool, Option<&'a str>)> {
    // Layout tells a pictograph apart from text. Here, whatever is outside the first plane is one.
    text.chars().filter(|ch| !ch.is_ascii()).map(move |ch| (ch, ch as u32 >= 0x1F000, lang))
}

/// The faces to fetch for `text`, after `asked` were asked for, when every face has what it is asked for.
fn fetched(text: &str, lang: Option<&str>, asked: &[Face]) -> Vec<Face> {
    lacking(&chosen(told(text, lang), asked, |_, _| false), asked)
}

#[test]
fn a_character_asks_for_the_face_of_its_script() {
    let cases: [(&str, Option<&str>, &[Face]); 9] = [
        ("🙂🎉", None, &[Face::Emoji]),
        ("한글", None, &[Face::Korean]),
        ("ひらがなカタカナ", None, &[Face::Japanese]),
        ("ㄅㄆㄇ", None, &[Face::Traditional]),
        ("汉字", None, &[Face::Simplified]),
        ("汉字。", Some("en"), &[Face::Simplified]),
        ("漢字", Some("ja"), &[Face::Japanese]),
        ("漢字", Some("ko-KR"), &[Face::Korean]),
        ("🙂 한글 かな", None, &[Face::Emoji, Face::Korean, Face::Japanese]),
    ];
    for (text, lang, faces) in cases {
        assert_eq!(fetched(text, lang, &[]), faces, "{text} in {lang:?}");
    }
}

#[test]
fn chinese_is_drawn_as_its_language_writes_it() {
    let cases = [
        ("zh", Face::Simplified),
        ("zh-CN", Face::Simplified),
        ("zh-Hans-SG", Face::Simplified),
        ("zh-TW", Face::Traditional),
        ("zh-Hant", Face::Traditional),
        ("zh-hant-HK", Face::Traditional),
    ];
    for (lang, face) in cases {
        assert_eq!(fetched("漢字", Some(lang), &[]), [face], "{lang}");
    }
}

#[test]
fn han_that_names_no_language_is_drawn_as_its_neighbours_are() {
    assert_eq!(fetched("日本語のテキスト", None, &[]), [Face::Japanese], "kana after it");
    assert_eq!(fetched("漢字", Some("en"), &[Face::Emoji, Face::Korean]), [], "hangul met before");
    assert_eq!(fetched("漢字", Some("en"), &[Face::Emoji]), [Face::Simplified], "an emoji says nothing of it");
}

#[test]
fn a_character_its_face_lacks_takes_the_next_that_may_have_it() {
    // The Japanese face is here and has no simplified character. The emoji face could not be fetched.
    let lacks = |face: Face, ch: char| matches!((face, ch), (Face::Japanese, '汉') | (Face::Emoji, _));
    let asked = [Face::Japanese, Face::Emoji];
    let faces = chosen(told("日本語 汉 🙂", Some("ja")), &asked, lacks);
    assert_eq!(faces[0], Some(Face::Japanese), "what the face has stays in it");
    assert_eq!(faces[3], Some(Face::Simplified));
    assert_eq!(faces[4], None, "an emoji has no other face");
    assert_eq!(lacking(&faces, &asked), [Face::Simplified]);
}

#[test]
fn nothing_is_fetched_twice_or_for_a_script_no_face_is_kept_for() {
    assert_eq!(fetched("🙂🙂 かな", None, &[Face::Emoji, Face::Japanese]), []);
    assert_eq!(fetched("🙂🙂 かな かな", None, &[]), [Face::Emoji, Face::Japanese]);
    assert_eq!(fetched("हिन्दी العربية", None, &[]), []);
}
