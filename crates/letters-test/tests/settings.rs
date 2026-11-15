//! The settings dialog, and the accent a window takes from what is set there.

use letters_render::{Color, Item};
use letters_test::{Harness, Role};
use letters_ui::{Appearance, Request, Settings, tokens};

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
