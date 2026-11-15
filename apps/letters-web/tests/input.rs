use letters_editor::{InputEvent, Key, KeyEvent, Modifiers};
use letters_render::Point;
use letters_web::input::{Around, Clipboard, Dom, Finger, Gesture, Held, Reply, TextInput, Touch};

const NONE: Held = Held { ctrl: false, meta: false, shift: false, alt: false, alt_graph: false };
const CTRL: Held = Held { ctrl: true, ..NONE };
const CMD: Held = Held { meta: true, ..NONE };
const SHIFT: Held = Held { shift: true, ..NONE };
const ALT: Held = Held { alt: true, ..NONE };

fn with(a: Held, b: Held) -> Held {
    Held { ctrl: a.ctrl || b.ctrl, meta: a.meta || b.meta, shift: a.shift || b.shift, alt: a.alt || b.alt, ..NONE }
}

fn press<'a>(key: &'a str, code: &'a str, held: Held) -> Dom<'a> {
    Dom::KeyDown { key, code, held, ime: false }
}

fn key(key: Key, primary: bool, shift: bool, alt: bool) -> KeyEvent {
    KeyEvent { key, mods: Modifiers { primary, shift, alt } }
}

/// An edit the browser announces at the caret of an input that holds nothing but what it rests on.
fn asked(kind: &'static str, data: Option<&'static str>) -> Dom<'static> {
    Dom::BeforeInput { kind, data, cancelable: true, selection: 1..1 }
}

fn typed(kind: &'static str, data: &'static str) -> Dom<'static> {
    asked(kind, Some(data))
}

/// An edit the browser announces and will not let the page stop, as a soft keyboard's often are.
fn unstoppable(kind: &'static str, data: Option<&'static str>) -> Dom<'static> {
    Dom::BeforeInput { kind, data, cancelable: false, selection: 1..1 }
}

/// A composition ending with `data`, which the hidden input then holds after what it rests on.
fn ends(data: &'static str) -> Dom<'static> {
    Dom::CompositionEnd { data, value: format!(" {data}").leak() }
}

/// Text written as it is held, with `|` at the caret or at each end of the selection: the text, and the
/// selection in it in bytes.
fn marked(text: &str) -> (String, std::ops::Range<usize>) {
    let ends: Vec<usize> = text.match_indices('|').enumerate().map(|(n, (at, _))| at - n).collect();
    (text.replace('|', ""), ends[0]..*ends.last().expect("the text marks a caret"))
}

/// An input that holds `around`, a block's text marked as [`marked`] reads it.
fn holding(around: &str) -> TextInput {
    let (text, selection) = marked(around);
    let mut input = TextInput::new(false);
    input.hold(&Around::of(&text, selection));
    input
}

/// The hidden input holding the space it rests on and then `text`, marked as [`marked`] reads it.
fn now_holds(text: &str) -> (String, std::ops::Range<u32>) {
    let (text, selection) = marked(text);
    let value = format!(" {text}");
    let units = |bytes: usize| value[..1 + bytes].encode_utf16().count() as u32;
    let selection = units(selection.start)..units(selection.end);
    (value, selection)
}

/// The hidden input holding `value`, with the caret after it. At rest it holds one space.
fn holds(value: &str) -> Dom<'_> {
    let caret = value.encode_utf16().count() as u32;
    Dom::Input { value, selection: caret..caret }
}

/// A composition showing `text`, with the caret after it.
fn composition(text: &str) -> InputEvent {
    InputEvent::SetComposition { text: text.into(), selection: Some(text.len()..text.len()) }
}

/// The text events a run of DOM events sends the editor.
fn text_of(input: &mut TextInput, events: &[Dom<'_>]) -> Vec<InputEvent> {
    let replies = events.iter().map(|event| input.event(event.clone()));
    replies.filter_map(|reply| if let Reply::Text(text) = reply { Some(text) } else { None }).collect()
}

#[test]
fn typing_is_left_to_the_browser_and_arrives_as_text() {
    let alt_gr = Held { ctrl: true, alt: true, alt_graph: true, ..NONE };
    let cases =
        [("a", "KeyA", NONE), ("A", "KeyA", SHIFT), ("é", "KeyE", NONE), (" ", "Space", NONE), ("@", "KeyQ", alt_gr)];
    for (text, code, held) in cases {
        let mut input = TextInput::new(false);
        assert_eq!(input.event(press(text, code, held)), Reply::Pass, "{text:?}");
        let reply = input.event(typed("insertText", text));
        assert_eq!(reply, Reply::Text(InputEvent::Insert(text.into())), "{text:?}");
    }
}

#[test]
fn editing_keys_are_keys_the_browser_must_not_act_on() {
    let cases = [
        ("Enter", Key::Enter),
        ("Backspace", Key::Backspace),
        ("Delete", Key::Delete),
        ("ArrowLeft", Key::Left),
        ("ArrowDown", Key::Down),
        ("Home", Key::Home),
        ("PageUp", Key::PageUp),
    ];
    for (name, expected) in cases {
        let reply = TextInput::new(false).event(press(name, name, SHIFT));
        assert_eq!(reply, Reply::Key { event: key(expected, false, true, false), claim: true }, "{name}");
    }
}

#[test]
fn tab_and_escape_stay_the_browsers_unless_the_editor_uses_them() {
    for (name, expected) in [("Tab", Key::Tab), ("Escape", Key::Escape)] {
        let reply = TextInput::new(false).event(press(name, name, NONE));
        assert_eq!(reply, Reply::Key { event: key(expected, false, false, false), claim: false }, "{name}");
    }
}

#[test]
fn shortcuts_are_matched_on_the_character_the_key_types_unmodified() {
    let cases = [
        // (what the browser reports, held, macOS, the key the editor is offered)
        (("b", "KeyB"), CTRL, false, key(Key::Char('b'), true, false, false)),
        (("X", "KeyX"), with(CTRL, SHIFT), false, key(Key::Char('x'), true, true, false)),
        // A layout that moves the letters is followed.
        (("b", "KeyN"), CTRL, false, key(Key::Char('b'), true, false, false)),
        // Shift and Option change what a key types. The shortcut is still the key's own character.
        (("&", "Digit7"), with(CTRL, SHIFT), false, key(Key::Char('7'), true, true, false)),
        ((">", "Period"), with(CMD, SHIFT), true, key(Key::Char('.'), true, true, false)),
        (("¡", "Digit1"), with(CMD, ALT), true, key(Key::Char('1'), true, false, true)),
        (("ç", "KeyC"), with(CMD, ALT), true, key(Key::Char('c'), true, false, true)),
        ((" ", "Space"), CTRL, false, key(Key::Char(' '), true, false, false)),
    ];
    for ((typed, code), held, mac, expected) in cases {
        let reply = TextInput::new(mac).event(press(typed, code, held));
        assert_eq!(reply, Reply::Key { event: expected, claim: false }, "{typed:?} on {code}");
    }
}

#[test]
fn the_primary_modifier_is_cmd_on_macos_and_ctrl_elsewhere() {
    let cases = [(CTRL, false, true), (CMD, false, false), (CMD, true, true), (CTRL, true, false)];
    for (held, mac, is_shortcut) in cases {
        let reply = TextInput::new(mac).event(press("b", "KeyB", held));
        assert_eq!(matches!(reply, Reply::Key { .. }), is_shortcut, "{held:?}, macOS: {mac}");
    }
}

#[test]
fn clipboard_shortcuts_are_left_to_the_browser_to_raise_as_events() {
    let mut input = TextInput::new(false);
    for (typed, code, held) in [("c", "KeyC", CTRL), ("x", "KeyX", CTRL), ("v", "KeyV", CTRL)] {
        assert_eq!(input.event(press(typed, code, held)), Reply::Pass, "{typed}");
    }
    assert_eq!(input.clipboard_key(Clipboard::Copy), key(Key::Char('c'), true, false, false));
    assert_eq!(input.clipboard_key(Clipboard::Cut), key(Key::Char('x'), true, false, false));
    assert_eq!(input.clipboard_key(Clipboard::Paste), key(Key::Char('v'), true, false, false));
}

#[test]
fn shift_asks_for_a_paste_without_formatting_until_the_next_plain_paste() {
    let mut input = TextInput::new(false);
    assert_eq!(input.event(press("V", "KeyV", with(CTRL, SHIFT))), Reply::Pass);
    assert_eq!(input.clipboard_key(Clipboard::Paste), key(Key::Char('v'), true, true, false));
    assert_eq!(input.event(press("v", "KeyV", CTRL)), Reply::Pass);
    assert_eq!(input.clipboard_key(Clipboard::Paste), key(Key::Char('v'), true, false, false));
}

#[test]
fn a_composition_is_followed_from_its_first_key_to_its_commit() {
    let ime = |key| Dom::KeyDown { key, code: "", held: NONE, ime: true };
    let commit = |text: &str| InputEvent::Commit(text.into());
    let traces: [(&str, &[Dom<'_>], &[InputEvent]); 6] = [
        (
            // Firefox ends the composition and then reports its text once more.
            "Firefox, pinyin",
            &[
                ime("Process"),
                Dom::CompositionStart,
                holds(" n"),
                ime("Process"),
                holds(" ni"),
                ends("你"),
                holds(" 你"),
            ],
            &[composition("n"), composition("ni"), commit("你")],
        ),
        (
            // Chromium reports the final text in an `input` event before the composition ends.
            "Chromium, kana",
            &[
                ime("Process"),
                Dom::CompositionStart,
                unstoppable("insertCompositionText", Some("に")),
                holds(" に"),
                unstoppable("insertCompositionText", Some("日")),
                holds(" 日"),
                ends("日"),
            ],
            &[composition("に"), composition("日"), commit("日")],
        ),
        (
            // Safari sends the Enter that picked the candidate after the composition has ended.
            "Safari, late Enter",
            &[Dom::CompositionStart, holds(" ㅎ"), ends("한"), ime("Enter")],
            &[composition("ㅎ"), commit("한")],
        ),
        (
            "given up",
            &[Dom::CompositionStart, holds(" n"), ends("")],
            &[composition("n"), InputEvent::SetComposition { text: String::new(), selection: None }],
        ),
        (
            // A soft keyboard composes every word, and commits it when the space is typed.
            "Android, a word and a space",
            &[
                ime("Unidentified"),
                Dom::CompositionStart,
                unstoppable("insertCompositionText", Some("h")),
                holds(" h"),
                ime("Unidentified"),
                unstoppable("insertCompositionText", Some("hi")),
                holds(" hi"),
                ime("Unidentified"),
                ends("hi"),
                typed("insertText", " "),
            ],
            &[composition("h"), composition("hi"), commit("hi"), InputEvent::Insert(" ".into())],
        ),
        (
            // Backspace takes the composition apart a letter at a time, and ends it when nothing is left.
            "Android, a word deleted",
            &[
                Dom::CompositionStart,
                holds(" h"),
                ime("Unidentified"),
                unstoppable("deleteCompositionText", None),
                holds(" "),
                ends(""),
            ],
            &[composition("h"), composition(""), InputEvent::SetComposition { text: String::new(), selection: None }],
        ),
    ];
    for (name, events, expected) in traces {
        let mut input = TextInput::new(false);
        assert_eq!(text_of(&mut input, events), expected, "{name}");
        assert!(!input.composing(), "{name}");
    }
}

#[test]
fn keys_pressed_during_a_composition_belong_to_the_input_method() {
    let mut input = TextInput::new(false);
    input.event(Dom::CompositionStart);
    for name in ["Enter", "Backspace", "ArrowLeft", "Escape"] {
        assert_eq!(input.event(press(name, name, NONE)), Reply::Pass, "{name}");
    }
    assert!(input.composing());
}

#[test]
fn the_caret_inside_a_composition_is_counted_in_bytes() {
    // What the input holds after the space it rests on, the selection in the whole of it in UTF-16 units,
    // and the same selection in the composition in bytes.
    let cases =
        [("日本語", 2..3, 3..6), ("😀a", 3..4, 4..5), ("abc", 1..4, 0..3), ("é", 6..10, 2..2), ("ab", 0..1, 0..0)];
    for (text, utf16, bytes) in cases {
        let mut input = TextInput::new(false);
        input.event(Dom::CompositionStart);
        let value = format!("{}{text}", TextInput::REST);
        let reply = input.event(Dom::Input { value: &value, selection: utf16 });
        let expected = InputEvent::SetComposition { text: text.into(), selection: Some(bytes) };
        assert_eq!(reply, Reply::Text(expected), "{text:?}");
    }
}

#[test]
fn edits_made_without_a_key_press_become_the_keys_that_make_them() {
    let cases = [
        // (inputType, macOS, the key)
        ("insertParagraph", false, key(Key::Enter, false, false, false)),
        // A hidden input of several lines calls Enter a line break. Shift with Enter is a key press.
        ("insertLineBreak", false, key(Key::Enter, false, false, false)),
        ("deleteContentBackward", false, key(Key::Backspace, false, false, false)),
        ("deleteContentForward", true, key(Key::Delete, false, false, false)),
        ("deleteWordBackward", false, key(Key::Backspace, true, false, false)),
        ("deleteWordBackward", true, key(Key::Backspace, false, false, true)),
        ("deleteWordForward", true, key(Key::Delete, false, false, true)),
        ("historyUndo", false, key(Key::Char('z'), true, false, false)),
        ("historyRedo", true, key(Key::Char('z'), true, true, false)),
    ];
    for (kind, mac, expected) in cases {
        let reply = TextInput::new(mac).event(asked(kind, None));
        assert_eq!(reply, Reply::Key { event: expected, claim: true }, "{kind}, macOS: {mac}");
    }
}

#[test]
fn edits_the_shell_has_no_meaning_for_never_reach_the_hidden_input() {
    for kind in ["insertFromPaste", "insertFromDrop", "formatBold", "deleteByCut"] {
        let reply = TextInput::new(false).event(asked(kind, None));
        assert_eq!(reply, Reply::Drop, "{kind}");
    }
}

#[test]
fn text_from_dictation_or_a_replacement_is_typed() {
    for kind in ["insertReplacementText", "insertFromYank", "insertText"] {
        let reply = TextInput::new(false).event(typed(kind, "word"));
        assert_eq!(reply, Reply::Text(InputEvent::Insert("word".into())), "{kind}");
    }
}

#[test]
fn an_edit_that_cannot_be_stopped_is_read_from_what_the_input_then_holds() {
    let text = |text: &str| Reply::Text(InputEvent::Insert(text.into()));
    let press = |key| Reply::Key { event: self::key(key, false, false, false), claim: true };
    let cases = [
        // (what was announced, what the input holds afterwards, what the editor is told)
        (unstoppable("deleteContentBackward", None), "", press(Key::Backspace)),
        (unstoppable("insertText", Some("a")), " a", text("a")),
        (unstoppable("insertText", Some("né")), " né", text("né")),
        (unstoppable("insertLineBreak", None), " \n", press(Key::Enter)),
        // The space the input rests on was replaced along with what was typed after it.
        (unstoppable("insertReplacementText", Some(". ")), ". ", text(". ")),
    ];
    for (announced, held, expected) in cases {
        let mut input = TextInput::new(false);
        assert_eq!(input.event(announced.clone()), Reply::Pass, "{announced:?} is the browser's to make");
        assert_eq!(input.event(holds(held)), expected, "{announced:?}");
    }
}

#[test]
fn a_soft_keyboards_backspace_is_a_key_whether_or_not_it_can_be_stopped() {
    let ime = Dom::KeyDown { key: "Unidentified", code: "", held: NONE, ime: true };
    let backspace = Reply::Key { event: key(Key::Backspace, false, false, false), claim: true };

    let mut stopped = TextInput::new(false);
    assert_eq!(stopped.event(ime.clone()), Reply::Pass);
    assert_eq!(stopped.event(asked("deleteContentBackward", None)), backspace);

    let mut unstopped = TextInput::new(false);
    let replies = [ime, unstoppable("deleteContentBackward", None), holds("")].map(|event| unstopped.event(event));
    assert_eq!(replies, [Reply::Pass, Reply::Pass, backspace], "it is counted once");
}

#[test]
fn an_input_event_that_changed_nothing_says_nothing() {
    assert_eq!(TextInput::new(false).event(holds(TextInput::REST)), Reply::Pass);
}

// The text around the caret

fn insert(text: &str) -> Reply {
    Reply::Text(InputEvent::Insert(text.into()))
}

fn replace(range: std::ops::Range<usize>, text: &str) -> Reply {
    Reply::Text(InputEvent::ReplaceRange { range, text: text.into() })
}

fn pressed(key: Key) -> Reply {
    Reply::Key { event: self::key(key, false, false, false), claim: true }
}

#[test]
fn the_input_holds_the_text_around_the_caret_with_the_selection_where_the_editors_is() {
    let cases = [
        // (the caret's block, what the input holds, the selection in it in UTF-16 units)
        ("|", " ", 1..1),
        ("Hello|", " Hello", 6..6),
        ("né|e", " née", 3..3),
        ("a |😀| b", " a 😀 b", 3..5),
    ];
    for (block, value, selection) in cases {
        let (text, bytes) = marked(block);
        let mut input = TextInput::new(false);
        assert_eq!(input.hold(&Around::of(&text, bytes)), Some((value, selection)), "{block}");
    }
}

#[test]
fn a_long_block_is_held_only_near_the_caret_and_edits_still_land_where_they_were_made() {
    let (text, selection) = marked(&format!("{}wrd|{}", "é ".repeat(3000), " ü".repeat(3000)));
    let around = Around::of(&text, selection.clone());
    assert!(around.text.len() < text.len() / 2, "{} of {} bytes are held", around.text.len(), text.len());
    assert_eq!(around.text, text[around.start..around.start + around.text.len()], "it is the block's own text");
    assert_eq!(around.start + around.selection.start, selection.start, "the caret is where it was");

    let mut input = TextInput::new(false);
    let (_, caret) = input.hold(&around).expect("it is held");
    // The keyboard mends the word before the caret.
    let mend = Dom::BeforeInput {
        kind: "insertReplacementText",
        data: Some("word"),
        cancelable: true,
        selection: caret.start - 3..caret.start,
    };
    assert_eq!(input.event(mend), replace(selection.start - 3..selection.start, "word"));
}

#[test]
fn an_edit_aimed_away_from_the_caret_replaces_what_it_is_aimed_at() {
    let cases = [
        // (what is held, what the edit is aimed at, its kind and data, what the editor is told)
        ("I saw teh cat|", "I saw |teh| cat", "insertReplacementText", Some("the"), replace(6..9, "the")),
        // The menu of accents that a held key brings up swaps the letter just typed.
        ("cafe|", "caf|e|", "insertText", Some("é"), replace(3..4, "é")),
        ("naïve wrd| now", "naïve |wrd| now", "insertReplacementText", Some("word"), replace(7..10, "word")),
        ("one two| three", "one |two| three", "deleteContentBackward", None, replace(4..7, "")),
        // At the caret, or over what is selected, it is typing as it always was.
        ("one two|", "one two|", "insertText", Some("!"), insert("!")),
        ("one |two|", "one |two|", "insertText", Some("2"), insert("2")),
        ("one |two|", "one |two|", "deleteContentBackward", None, pressed(Key::Backspace)),
        ("one two|", "one |two|", "insertParagraph", None, pressed(Key::Enter)),
    ];
    for (held, aim, kind, data, expected) in cases {
        let mut input = holding(held);
        let (_, selection) = now_holds(aim);
        let reply = input.event(Dom::BeforeInput { kind, data, cancelable: true, selection });
        assert_eq!(reply, expected, "{kind} at {aim:?} of {held:?}");
    }
}

#[test]
fn an_edit_nothing_announced_is_read_from_how_the_text_around_the_caret_changed() {
    let cases = [
        // (what was held, what the input holds now, what the editor is told)
        ("Hello|", "Hello!|", insert("!")),
        ("He|llo", "Hey|llo", insert("y")),
        ("Hello|", "Hello\n|", pressed(Key::Enter)),
        ("Hello|", "Hell|", pressed(Key::Backspace)),
        ("He|llo", "He|lo", pressed(Key::Delete)),
        ("né|e", "n|e", pressed(Key::Backspace)),
        // The same letter on both sides: the edit is the one that leaves the caret where it is.
        ("aa|a", "aaa|a", insert("a")),
        ("aa|a", "a|a", pressed(Key::Backspace)),
        ("a|aa", "a|a", pressed(Key::Delete)),
        // What was selected is typed over, or deleted.
        ("one |two| three", "one 2| three", insert("2")),
        ("one |two| three", "one | three", pressed(Key::Backspace)),
        // A keyboard mends a word as the space after it is typed, or takes a whole word back.
        ("I saw teh|", "I saw the |", replace(7..9, "he ")),
        ("Hello wrld|", "Hello world|", replace(7..10, "orld")),
        ("naïve wrd |", "naïve word |", replace(8..11, "ord ")),
        ("one two|", "one |", replace(4..7, "")),
        ("Hello|", "Hello|", Reply::Pass),
    ];
    for (held, now, expected) in cases {
        let mut input = holding(held);
        let (value, selection) = now_holds(now);
        assert_eq!(input.event(Dom::Input { value: &value, selection }), expected, "{held:?} to {now:?}");
    }
    // Backspace at the start of a block takes the space the input rests on, which is all there is to take.
    let mut input = holding("|Hello");
    assert_eq!(input.event(Dom::Input { value: "Hello", selection: 0..0 }), pressed(Key::Backspace));
}

/// A step of a composition in an input that holds text: what it holds after an `input` event, or the
/// `data` it ends with and what it holds then. Text is marked as [`marked`] reads it.
enum Step {
    Holds(&'static str),
    Ends(&'static str, &'static str),
}

#[test]
fn a_composition_among_other_text_shows_only_what_is_composed() {
    use Step::{Ends, Holds};
    let commit = |text: &str| InputEvent::Commit(text.into());
    let replaced = |range, text: &str| InputEvent::ReplaceRange { range, text: text.into() };
    let given_up = InputEvent::SetComposition { text: String::new(), selection: None };
    let traces: [(&str, &str, &[Step], &[InputEvent]); 5] = [
        (
            "at the caret",
            "Say | now",
            &[Holds("Say h| now"), Holds("Say hi| now"), Ends("hi", "Say hi| now")],
            &[composition("h"), composition("hi"), commit("hi")],
        ),
        (
            "over what is selected",
            "Say |it| now",
            &[Holds("Say h| now"), Ends("h", "Say h| now")],
            &[composition("h"), commit("h")],
        ),
        (
            // A soft keyboard takes the word before the caret back into its composition, and reports the
            // whole word when it ends. Only the letter is new.
            "a word carried on",
            "hello|",
            &[Holds("hellow|"), Ends("hellow", "hellow|")],
            &[composition("w"), commit("w")],
        ),
        (
            // It may rewrite that word, which no composition at the caret can show.
            "a word rewritten",
            "helo|",
            &[Holds("hello|"), Ends("hello", "hello|")],
            &[replaced(3..4, "lo")],
        ),
        (
            "a word rewritten after the composition began",
            "helo |",
            &[Holds("helo w|"), Holds("hello w|"), Ends("w", "hello w|")],
            &[composition("w"), given_up, replaced(3..5, "lo w")],
        ),
    ];
    for (name, held, steps, expected) in traces {
        let mut input = holding(held);
        let mut told = text_of(&mut input, &[Dom::CompositionStart]);
        for step in steps {
            let (text, data) = match step {
                Holds(text) => (text, None),
                Ends(data, text) => (text, Some(*data)),
            };
            let (value, selection) = now_holds(text);
            let event = match data {
                None => Dom::Input { value: &value, selection },
                Some(data) => Dom::CompositionEnd { data, value: &value },
            };
            assert_eq!(input.hold(&Around::default()), None, "{name}: the input is the input method's");
            told.extend(text_of(&mut input, &[event]));
        }
        assert_eq!(told, expected, "{name}");
        assert!(!input.composing() && input.hold(&Around::default()).is_some(), "{name}");
    }
}

// Fingers

const START: Point = Point { x: 200.0, y: 400.0 };

/// Where a finger is at a time: milliseconds since it went down, then how far it is from [`START`].
type Moves = [(f64, f32, f32)];

/// One finger's travels: down at [`START`], through `moves`, and lifted at `up`.
fn finger(moves: &Moves, up: f64) -> Vec<Finger> {
    let at = |dx: f32, dy: f32| Point { x: START.x + dx, y: START.y + dy };
    let moves = moves.iter().map(|(ms, dx, dy)| Finger::Move { id: 1, at: at(*dx, *dy), ms: *ms });
    [Finger::Down { id: 1, at: START, ms: 0.0 }]
        .into_iter()
        .chain(moves)
        .chain([Finger::Up { id: 1, ms: up }])
        .collect()
}

/// What a run of finger events asks for, with how far it scrolled in all.
fn gestures(touch: &mut Touch, events: &[Finger]) -> (Vec<Point>, Point) {
    let (mut taps, mut scrolled) = (Vec::new(), Point::default());
    for event in events {
        match touch.event(*event) {
            Gesture::None => {}
            Gesture::Tap(at) => taps.push(at),
            Gesture::Scroll { at, by } => {
                assert_eq!(at, START, "what was under the finger when it went down is what scrolls");
                scrolled = Point { x: scrolled.x + by.x, y: scrolled.y + by.y };
            }
        }
    }
    (taps, scrolled)
}

/// How far a glide carries the page before it comes to rest, a frame at a time from `ms`.
fn glide(touch: &mut Touch, mut ms: f64) -> Point {
    let mut carried = Point::default();
    for _ in 0..1000 {
        ms += 16.0;
        let Some((_, by)) = touch.glide(ms) else { break };
        carried = Point { x: carried.x + by.x, y: carried.y + by.y };
    }
    assert!(!touch.gliding(), "a glide comes to rest");
    carried
}

#[test]
fn a_finger_that_stays_put_taps_and_one_that_travels_scrolls() {
    let cases: [(&str, &Moves, f64, bool, f32); 5] = [
        // (what the finger does, its moves, when it lifts, whether it is a tap, how far the page scrolls down)
        ("a tap", &[], 80.0, true, 0.0),
        ("a tap with a wobble", &[(30.0, 3.0, -4.0), (60.0, 5.0, 2.0)], 90.0, true, 0.0),
        ("a long press, which is the browser's menu", &[], 700.0, false, 0.0),
        ("a drag up the screen", &[(50.0, 0.0, -30.0), (400.0, 0.0, -120.0)], 900.0, false, 120.0),
        ("a drag down and back", &[(50.0, 0.0, 40.0), (400.0, 0.0, 0.0)], 900.0, false, 0.0),
    ];
    for (name, moves, up, tap, scrolled) in cases {
        let mut touch = Touch::default();
        let (taps, by) = gestures(&mut touch, &finger(moves, up));
        assert_eq!(taps, if tap { vec![START] } else { vec![] }, "{name}");
        assert_eq!(by, Point { x: 0.0, y: scrolled }, "{name}: the page moves as far as the finger did");
        assert!(!touch.gliding(), "{name}: a finger that stops before it lifts leaves the page where it is");
    }
}

#[test]
fn a_flick_carries_the_page_on_and_a_touch_stops_it() {
    // 300 pixels up the screen in 60 ms, lifted while still moving.
    let flick = finger(&[(20.0, 0.0, -100.0), (40.0, 0.0, -200.0), (60.0, 0.0, -300.0)], 62.0);

    let mut touch = Touch::default();
    gestures(&mut touch, &flick);
    assert!(touch.gliding());
    let carried = glide(&mut touch, 62.0);
    assert!(carried.y > 300.0, "a flick goes further than the finger did: {carried:?}");
    assert_eq!(carried.x, 0.0);

    let mut touch = Touch::default();
    gestures(&mut touch, &flick);
    touch.glide(78.0);
    let (taps, _) =
        gestures(&mut touch, &[Finger::Down { id: 2, at: START, ms: 100.0 }, Finger::Up { id: 2, ms: 150.0 }]);
    assert!(!touch.gliding(), "a finger put down holds the page still");
    assert!(taps.is_empty(), "and presses nothing");
}

#[test]
fn touches_the_browser_takes_ask_for_nothing() {
    let down = |id, ms| Finger::Down { id, at: START, ms };
    let moved = |id, ms| Finger::Move { id, at: Point { x: START.x, y: START.y - 80.0 }, ms };
    let traces: [(&str, &[Finger]); 3] = [
        ("a second finger makes a pinch", &[down(1, 0.0), down(2, 10.0), moved(1, 30.0), moved(2, 30.0)]),
        (
            "a cancelled touch",
            &[down(1, 0.0), Finger::Cancel { id: 1 }, moved(1, 30.0), Finger::Up { id: 1, ms: 40.0 }],
        ),
        ("a finger that was never seen to go down", &[moved(7, 30.0), Finger::Up { id: 7, ms: 40.0 }]),
    ];
    for (name, events) in traces {
        let mut touch = Touch::default();
        assert_eq!(gestures(&mut touch, events), (vec![], Point::default()), "{name}");
        assert!(!touch.gliding(), "{name}");
    }
}
