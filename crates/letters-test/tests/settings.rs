//! The settings dialog, and the accent a window takes from what is set there.

use letters_render::{Color, Item};
use letters_test::{Harness, Role};
use letters_ui::{Appearance, Request, Settings, TrustedFolder, TrustedSource, tokens};

const GREEN: [u8; 3] = [0x1f, 0x8a, 0x5b];

/// What a person does in the open dialog.
type Act = fn(&mut Harness);

fn letter() -> Harness {
    Harness::doc("# Roof repairs\n\nHarland Roofing sent a quote.\n")
}

/// A letter with the settings open over it.
fn settings() -> Harness {
    let mut window = letter();
    window.key("Mod+,").assert_open(Role::Dialog, "Settings");
    window
}

const FOLDERS: [&str; 2] = ["Letters", "Work"];
const MINE: &str = "https://img.example.org/";
const FIXED: &str = "https://intranet.example.org/charts/";
const WORDS: [&str; 2] = ["Zorvex", "soffit"];

fn folders(names: &[&str]) -> Vec<TrustedFolder> {
    let folder = |name: &&str| TrustedFolder { path: format!("/home/ada/{name}"), shown: format!("~/{name}") };
    names.iter().map(folder).collect()
}

/// The person's own sources, after the one an administrator set.
fn sources(mine: &[&str]) -> Vec<TrustedSource> {
    let fixed = TrustedSource { source: FIXED.into(), fixed: true };
    let mine = mine.iter().map(|source| TrustedSource { source: (*source).into(), fixed: false });
    [fixed].into_iter().chain(mine).collect()
}

/// The settings of a shell that keeps trust and a dictionary, over a letter the person trusts, in a window
/// tall enough to show all of them.
fn kept() -> Harness {
    let mut window = letter();
    window.resize(1280.0, 1600.0);
    window.ui().set_trusted_folders(folders(&FOLDERS));
    window.ui().set_trusted_sources(sources(&[MINE]));
    window.ui().set_document_trusted(true);
    window.ui().set_user_words(WORDS);
    window.key("Mod+,").assert_open(Role::Dialog, "Settings");
    window
}

/// The rows the dialog's lists show, top to bottom.
fn rows(window: &mut Harness) -> Vec<String> {
    window.controls().into_iter().filter(|found| found.role == Role::ListItem).map(|found| found.name).collect()
}

/// Presses Tab until the control of this name has the keyboard.
fn tab_to(window: &mut Harness, name: &str) {
    for _ in 0..80 {
        if window.focused().is_some_and(|found| found.name == name) {
            return;
        }
        window.key("Tab");
    }
    panic!("Tab never reached {name}");
}

/// The colours a frame is drawn in, without how strongly each is drawn.
fn colours(window: &mut Harness) -> Vec<[u8; 3]> {
    let colour = |item: &Item| match item {
        Item::Fill { color, .. }
        | Item::Stroke { color, .. }
        | Item::Glyphs { color, .. }
        | Item::Path { color, .. } => Some(*color),
        _ => None,
    };
    window.paint().items.iter().filter_map(colour).map(|Color { r, g, b, .. }| [r, g, b]).collect()
}

#[test]
fn the_settings_open_from_the_menu_or_the_keyboard_and_change_nothing_by_being_looked_at() {
    for route in ["the menu", "the keyboard"] {
        let mut window = letter();
        match route {
            "the menu" => window.click(Role::Button, "More").click(Role::MenuItem, "Settings"),
            _ => window.key("Mod+,"),
        };
        window.assert_open(Role::Dialog, "Settings");
        window.key("Escape").assert_closed(Role::Dialog, "Settings");
        assert_eq!(window.ui().take_requests(), [], "{route}");
        assert_eq!(*window.ui().settings(), Settings::default(), "{route}");
    }
}

#[test]
fn each_setting_is_handed_to_the_shell_as_it_is_made() {
    let nothing = Settings::default();
    // What the person does in the dialog, and what they have then set.
    let cases: [(&str, Act, Settings); 6] = [
        (
            "a theme",
            |window| _ = window.click(Role::Button, "Dark"),
            Settings { appearance: Appearance::Dark, ..nothing.clone() },
        ),
        (
            "an accent",
            |window| _ = window.click(Role::Button, "Green"),
            Settings { accent: Some(GREEN), ..nothing.clone() },
        ),
        (
            "a colour of their own",
            |window| _ = window.click(Role::TextInput, "Accent colour").key("Mod+A").type_text("6D4AFF").key("Enter"),
            Settings { accent: Some([0x6d, 0x4a, 0xff]), ..nothing.clone() },
        ),
        (
            "a name",
            |window| _ = window.click(Role::TextInput, "Your name").type_text(" Ada Lovelace").key("Enter"),
            Settings { name: Some("Ada Lovelace".into()), ..nothing.clone() },
        ),
        (
            "a name, left by moving on",
            |window| _ = window.click(Role::TextInput, "Your name").type_text("Ada").key("Tab"),
            Settings { name: Some("Ada".into()), ..nothing.clone() },
        ),
        (
            "a language",
            |window| {
                _ = window.click(Role::Button, "Language").click(Role::MenuItemCheckBox, "English (United Kingdom)")
            },
            Settings { language: Some("en-GB".into()), ..nothing.clone() },
        ),
    ];
    for (name, set, expected) in cases {
        let mut window = settings();
        set(&mut window);
        assert_eq!(window.ui().take_requests(), [Request::Settings(expected.clone())], "{name}");
        assert_eq!(*window.ui().settings(), expected, "{name}");
        window.assert_open(Role::Dialog, "Settings");
    }
}

#[test]
fn what_is_typed_but_makes_no_sense_or_is_dropped_sets_nothing() {
    let cases: [(&str, Act); 3] = [
        ("no colour", |window| {
            _ = window.click(Role::TextInput, "Accent colour").key("Mod+A").type_text("green").key("Enter")
        }),
        ("a colour, dropped", |window| {
            _ = window.click(Role::TextInput, "Accent colour").key("Mod+A").type_text("#1f8a5b").key("Escape")
        }),
        ("a name, dropped", |window| _ = window.click(Role::TextInput, "Your name").type_text("Ada").key("Escape")),
    ];
    for (name, typed) in cases {
        let mut window = settings();
        typed(&mut window);
        assert_eq!(window.ui().take_requests(), [], "{name}");
        assert_eq!(*window.ui().settings(), Settings::default(), "{name}");
    }
}

#[test]
fn the_languages_offered_are_the_kinds_of_english_and_those_the_shell_has() {
    let mut window = letter();
    window.ui().set_languages(["fr", "de-CH", "en-GB"]);
    window.ui().set_settings(Settings { language: Some("de-CH".into()), ..Settings::default() });
    window.key("Mod+,").click(Role::Button, "Language");
    let offered: Vec<_> = window.controls().into_iter().filter(|found| found.role == Role::MenuItemCheckBox).collect();
    let names: Vec<&str> = offered.iter().map(|found| found.name.as_str()).collect();
    let english = ["English (United States)", "English (United Kingdom)", "English (Australia)", "English (Canada)"];
    assert_eq!(names[..4], english, "English is built in, and comes first");
    assert_eq!(names[4..], ["French", "German (Switzerland)"]);
    let picked: Vec<&str> =
        offered.iter().filter(|found| found.toggled == Some(true)).map(|found| &*found.name).collect();
    assert_eq!(picked, ["German (Switzerland)"]);
}

#[test]
fn a_window_is_drawn_in_the_accent_that_was_set() {
    let yewi = tokens::ACCENT.light;
    let yewi = [yewi.r, yewi.g, yewi.b];
    // How the accent comes to be set: in this window's own dialog, or by the shell, as when another
    // window set it or the app starts.
    for route in ["the dialog", "the shell"] {
        let mut window = letter();
        // A selection on the page and a toggle that is down both show the accent.
        window.select("Harland").click(Role::Button, "Bold");
        let before = colours(&mut window);
        assert!(before.contains(&yewi), "{route}: yewi's accent shows until another is set");
        assert!(!before.contains(&GREEN));

        match route {
            "the dialog" => _ = window.key("Mod+,").click(Role::Button, "Green").key("Escape"),
            _ => window.ui().set_settings(Settings { accent: Some(GREEN), ..Settings::default() }),
        }
        let after = colours(&mut window);
        assert!(after.contains(&GREEN), "{route}: the accent shows");
        let token = |ink: tokens::Color| [ink.light.r, ink.light.g, ink.light.b];
        for old in [tokens::ACCENT, tokens::ACCENT_SOLID, tokens::ACCENT_SOLID_HOVER, tokens::ACCENT_TEXT] {
            assert!(!after.contains(&token(old)), "{route}: {old:?} of yewi's accent is still drawn");
        }

        window.ui().set_settings(Settings::default());
        assert_eq!(colours(&mut window), before, "{route}: yewi's accent comes back when the choice is undone");
    }
}

#[test]
fn usage_reports_are_switched_only_where_a_shell_sends_them() {
    const USAGE: &str = "Share anonymous usage reports";
    // A shell that sends none never says, and its window has no switch.
    let mut window = settings();
    assert!(window.find(Role::CheckBox, USAGE).is_none());

    let on = |window: &mut Harness| window.find(Role::CheckBox, USAGE).and_then(|found| found.toggled);
    for said in [true, false] {
        let mut window = letter();
        window.ui().set_usage_reports(said);
        window.key("Mod+,");
        assert_eq!(on(&mut window), Some(said));
        assert_eq!(window.ui().take_requests(), [], "{said}: the shell's word asks for nothing");
        // The switch shows at once, and the shell is asked to keep it.
        window.click(Role::CheckBox, USAGE);
        assert_eq!(on(&mut window), Some(!said));
        assert_eq!(window.ui().take_requests(), [Request::UsageReports(!said)]);
        assert_eq!(*window.ui().settings(), Settings::default(), "it is no setting of the documents");
        // Another window turned them back: the switch follows, and asks nothing.
        window.ui().set_usage_reports(said);
        assert_eq!(on(&mut window), Some(said));
        assert_eq!(window.ui().take_requests(), []);
    }
}

#[test]
fn what_a_shell_keeps_is_listed_only_once_it_has_said_it() {
    // What stands for each part of the dialog.
    let parts = [
        (Role::Button, "Add a folder"),
        (Role::TextInput, "Trust a source"),
        (Role::Button, "Stop trusting this document"),
        (Role::Label, "DICTIONARY"),
    ];
    // What a shell says, and the one part the dialog then has.
    let cases: [(Act, Option<usize>); 6] = [
        (|_| {}, None),
        (|window| window.ui().set_trusted_folders(folders(&[])), Some(0)),
        (|window| window.ui().set_trusted_sources(sources(&[])), Some(1)),
        (|window| window.ui().set_document_trusted(true), Some(2)),
        (|window| window.ui().set_document_trusted(false), None),
        (|window| window.ui().set_user_words([""; 0]), Some(3)),
    ];
    for (at, (says, expected)) in cases.into_iter().enumerate() {
        let mut window = letter();
        says(&mut window);
        window.key("Mod+,").assert_open(Role::Dialog, "Settings");
        let shown: Vec<_> =
            (0..parts.len()).filter(|&part| window.find(parts[part].0, parts[part].1).is_some()).collect();
        assert_eq!(shown, Vec::from_iter(expected), "case {at}");
        assert_eq!(window.ui().take_requests(), [], "case {at}: the shell's word asks for nothing");
    }

    let mut window = kept();
    // Words stand as a person reads a list of them, whatever their capitals.
    let expected = ["~/Letters", "~/Work", &format!("{FIXED}, set by your administrator"), MINE, "soffit", "Zorvex"];
    assert_eq!(rows(&mut window), expected);

    // The empty lines of a dictionary file are no words.
    let mut window = letter();
    window.ui().set_user_words(["", " soffit ", "  "]);
    window.key("Mod+,");
    assert_eq!(rows(&mut window), ["soffit"]);
}

#[test]
fn what_a_shell_keeps_is_asked_of_it_and_changes_when_it_answers() {
    // What the person does, what the shell is asked, and the row that is gone once the shell answered.
    let cases: [(Act, Request, Option<&str>); 6] = [
        (|window| _ = window.click(Role::Button, "Add a folder"), Request::TrustFolderDialog, None),
        (
            |window| _ = window.click(Role::Button, "Stop trusting ~/Work"),
            Request::ForgetFolder("/home/ada/Work".into()),
            Some("~/Work"),
        ),
        (
            |window| _ = window.click(Role::Button, &format!("Stop trusting {MINE}")),
            Request::ForgetSource(MINE.into()),
            Some(MINE),
        ),
        (|window| _ = window.click(Role::Button, "Stop trusting this document"), Request::ForgetDocument, None),
        (
            |window| _ = window.click(Role::TextInput, "Trust a source").type_text(" cdn.example.net ").key("Enter"),
            Request::TrustSource("cdn.example.net".into()),
            None,
        ),
        (
            |window| {
                _ = window
                    .click(Role::TextInput, "Trust a source")
                    .type_text("cdn.example.net")
                    .click(Role::Button, "Trust")
            },
            Request::TrustSource("cdn.example.net".into()),
            None,
        ),
    ];
    for (act, request, gone) in cases {
        let mut window = kept();
        let before = rows(&mut window);
        act(&mut window);
        assert_eq!(window.ui().take_requests(), std::slice::from_ref(&request));
        assert_eq!(rows(&mut window), before, "{request:?}: trust is the shell's to keep, and nothing changed yet");
        assert_eq!(*window.ui().settings(), Settings::default(), "{request:?}: it is no setting");

        window.ui().set_trusted_folders(folders(&FOLDERS[..1]));
        window.ui().set_trusted_sources(sources(&[]));
        if let Some(gone) = gone {
            assert!(!rows(&mut window).iter().any(|row| row == gone), "{request:?}");
        }
        assert_eq!(window.ui().take_requests(), [], "{request:?}: the answer asks for nothing");
    }

    // The way out of a document's trust goes once the shell says the document is held again.
    let mut window = kept();
    window.ui().set_document_trusted(false);
    assert!(window.find(Role::Button, "Stop trusting this document").is_none());
}

#[test]
fn something_too_long_for_its_list_is_cut_short_and_can_still_be_removed() {
    const LONG: &str = "~/Work/Harland Roofing/Quotes and invoices/2026/Autumn/Sent to the insurer";
    let mut window = letter();
    window.resize(1280.0, 1600.0);
    window.ui().set_trusted_folders(vec![TrustedFolder { path: "/long".into(), shown: LONG.into() }]);
    window.ui().set_trusted_sources(vec![TrustedSource {
        source: format!("https://{}.example.org/", "a".repeat(90)),
        fixed: true,
    }]);
    window.key("Mod+,");

    let dialog = window.find(Role::Dialog, "Settings").expect("the dialog").bounds;
    for list in ["Trusted folders", "Trusted sources"] {
        let list = window.find(Role::List, list).expect(list).bounds;
        assert!(list.x >= dialog.x && list.x + list.width <= dialog.x + dialog.width, "{list:?} in {dialog:?}");
    }
    window.click(Role::Button, &format!("Stop trusting {LONG}"));
    assert_eq!(window.ui().take_requests(), [Request::ForgetFolder("/long".into())]);
}

#[test]
fn a_source_an_administrator_set_has_nothing_to_remove_it_with() {
    let mut window = kept();
    assert!(window.find(Role::Button, &format!("Stop trusting {MINE}")).is_some());
    assert!(window.find(Role::Button, &format!("Stop trusting {FIXED}")).is_none());
    let row = window.find(Role::ListItem, &format!("{FIXED}, set by your administrator"));
    assert!(row.is_some(), "and its row says why");
}

#[test]
fn a_source_that_was_asked_for_is_answered_under_its_field() {
    const WHY: &str = "\u{201c}img example\u{201d} is not an address Letters can load pictures from.";
    let typed = |window: &mut Harness| window.find(Role::TextInput, "Trust a source").and_then(|field| field.value);
    let mut window = kept();

    // Nothing typed is nothing to ask.
    assert!(window.find(Role::Button, "Trust").is_some_and(|button| button.disabled));
    window.click(Role::TextInput, "Trust a source").key("Enter");
    assert_eq!(window.ui().take_requests(), []);

    // One question at a time.
    window.type_text("img example").key("Enter").key("Enter");
    assert_eq!(window.ui().take_requests(), [Request::TrustSource("img example".into())]);

    // The shell says no: what was typed stays, with why it will not do, until it is mended.
    window.ui().refuse_source(WHY);
    assert!(window.find(Role::Alert, WHY).is_some());
    assert_eq!(typed(&mut window).as_deref(), Some("img example"));
    window.key("Mod+A").type_text("img.example.net");
    assert!(window.find(Role::Alert, WHY).is_none(), "the reason was about what stood there before");

    // The shell says yes with its new list, and the field is free for another.
    window.key("Enter");
    assert_eq!(window.ui().take_requests(), [Request::TrustSource("img.example.net".into())]);
    window.ui().set_trusted_sources(sources(&[MINE, "https://img.example.net/"]));
    assert!(rows(&mut window).iter().any(|row| row == "https://img.example.net/"));
    assert_eq!(typed(&mut window).as_deref().unwrap_or_default(), "");

    // A list the shell says for another reason leaves alone what is being typed.
    window.type_text("cdn.example");
    window.ui().set_trusted_sources(sources(&[MINE]));
    assert_eq!(typed(&mut window).as_deref(), Some("cdn.example"));
    // What was typed and never asked for goes with the dialog.
    window.key("Escape").key("Mod+,");
    assert_eq!(typed(&mut window).as_deref().unwrap_or_default(), "");
}

#[test]
fn a_word_of_the_persons_own_is_taken_out_at_once_and_the_shell_is_told() {
    let mut window = kept();
    window.click(Role::Button, "Remove soffit");
    assert_eq!(window.ui().take_requests(), [Request::ForgetWord("soffit".into())]);
    assert!(!rows(&mut window).iter().any(|row| row == "soffit"));
    assert!(rows(&mut window).iter().any(|row| row == "Zorvex"), "the others stay");
    assert!(window.platform.learned.is_empty(), "nothing is learned by it");

    // By the keyboard the next word is in reach at once, with no way back through the dialog.
    let mut window = kept();
    tab_to(&mut window, "Remove soffit");
    window.key("Enter");
    assert_eq!(window.ui().take_requests(), [Request::ForgetWord("soffit".into())]);
    assert!(window.focused().is_some_and(|found| found.name == "Remove Zorvex"));
}

#[test]
fn settings_taller_than_the_window_scroll_to_what_the_keyboard_reaches() {
    let mut window = kept();
    window.resize(1280.0, 600.0);
    let dialog = window.find(Role::Dialog, "Settings").expect("the dialog").bounds;
    assert!(dialog.y >= 0.0 && dialog.y + dialog.height <= 600.0, "the dialog stays inside the window");
    let last = |window: &mut Harness| window.find(Role::Button, "Remove Zorvex").expect("the last word").bounds;
    assert!(last(&mut window).y > dialog.y + dialog.height, "the last of it starts out of view");

    tab_to(&mut window, "Remove Zorvex");
    let last = last(&mut window);
    assert!(last.y >= dialog.y && last.y + last.height <= dialog.y + dialog.height, "{last:?} in {dialog:?}");
}
