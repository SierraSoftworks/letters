//! Which face backs up the fonts for the characters they lack.

use letters_web::faces::{Face, wanted};

/// The faces asked for by text no font covers, written in `lang`, after `had` were fetched.
fn asked(text: &str, lang: Option<&str>, had: &[Face]) -> Vec<Face> {
    // Layout tells a pictograph apart from text. Here, whatever is outside the first plane is one.
    wanted(text.chars().map(|ch| (ch, ch as u32 >= 0x1F000, lang)), had)
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
        assert_eq!(asked(text, lang, &[]), faces, "{text} in {lang:?}");
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
        assert_eq!(asked("漢字", Some(lang), &[]), [face], "{lang}");
    }
}

#[test]
fn han_that_names_no_language_is_drawn_as_its_neighbours_are() {
    assert_eq!(asked("日本語のテキスト", None, &[]), [Face::Japanese], "kana beside it");
    assert_eq!(asked("漢字", Some("en"), &[Face::Emoji, Face::Korean]), [], "hangul met before");
    assert_eq!(asked("漢字", Some("en"), &[Face::Emoji]), [Face::Simplified], "an emoji says nothing of it");
}

#[test]
fn nothing_is_fetched_twice_or_for_a_script_no_face_is_kept_for() {
    assert_eq!(asked("🙂🙂 かな", None, &[Face::Emoji, Face::Japanese]), []);
    assert_eq!(asked("हिन्दी العربية", None, &[]), []);
}
