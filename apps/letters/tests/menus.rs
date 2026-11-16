//! The macOS menu bar: its menus offer only what the window can do, by keys nothing else has.

use std::collections::HashSet;

use letters::menus::{self, BAR, Item, System};
use letters_editor::{Editor, Key, KeyEvent, Modifiers};
use letters_model::{Document, ReplicaId};
use letters_ui::{Keys, Window};

/// A window that follows the key conventions of macOS, as the one the menus ask does.
fn window() -> Window {
    let mut window = Window::new(Editor::new(Document::default(), ReplicaId::from(1)));
    window.set_keys(Keys::Mac);
    window.set_application(true);
    window
}

fn cmd(key: char, shift: bool, alt: bool) -> KeyEvent {
    KeyEvent { key: Key::Char(key), mods: Modifiers { primary: true, shift, alt } }
}

/// Every line of a menu and of the menus inside it.
fn lines(items: &'static [Item]) -> Vec<&'static Item> {
    let inner = |item: &'static Item| match item {
        Item::Menu(_, items) => lines(items),
        _ => Vec::new(),
    };
    items.iter().flat_map(|item| std::iter::once(item).chain(inner(item))).collect()
}

#[test]
fn every_command_of_the_bar_is_one_the_palette_lists_and_is_there_once() {
    let listed: HashSet<_> = letters_ui::command_names().collect();
    let mut seen = HashSet::new();
    for item in menus::items() {
        if let Item::Command(label, name) = item {
            assert!(listed.contains(name), "{label:?} asks for {name:?}, which is no command");
            assert!(seen.insert(name), "{name:?} is in the bar twice");
        }
    }
    assert!(seen.len() > 50, "the bar offers most of what the palette does");
}

#[test]
fn the_bar_has_the_menus_a_mac_expects_with_the_apps_own_first() {
    let titles: Vec<_> = BAR.iter().map(|menu| menu.title).collect();
    assert_eq!(titles.first(), Some(&"Letters"));
    for expected in ["File", "Edit", "View", "Window", "Help"] {
        assert!(titles.contains(&expected), "{expected} is among {titles:?}");
    }
    let system: Vec<_> = BAR.iter().flat_map(|menu| lines(menu.items)).collect();
    for expected in [System::About, System::Quit, System::Hide, System::Minimize, System::FullScreen] {
        let found = system.iter().any(|item| matches!(item, Item::System(system) if *system == expected));
        assert!(found, "{expected:?} is offered");
    }
}

#[test]
fn a_taken_item_is_told_apart_by_its_place() {
    let items = menus::items();
    let theirs = BAR.iter().flat_map(|menu| lines(menu.items));
    let own: Vec<_> = theirs.filter(|item| matches!(item, Item::Command(..) | Item::Key(..))).collect();
    assert_eq!(items.len(), own.len());
    for (item, own) in items.iter().zip(own) {
        assert!(std::ptr::eq(*item, own), "the places follow the bar's order");
    }
}

#[test]
fn the_keys_the_bar_writes_are_held_with_cmd_and_belong_to_one_item_each() {
    let window = window();
    // What macOS and its menus keep: hiding, minimising, quitting, hiding the others, full screen.
    let mut taken: HashSet<_> =
        [cmd('h', false, false), cmd('m', false, false), cmd('q', false, false), cmd('h', false, true)].into();
    for item in menus::items() {
        let Some(key) = menus::key(item, &window) else { continue };
        assert!(key.mods.primary, "{item:?} is done by {key:?}, which a field would take as typing");
        assert!(taken.insert(key.clone()), "{key:?} of {item:?} is another item's too");
    }
    // The key, and whether the bar writes it: a key without Cmd is left to the window.
    let cases = [
        (cmd('n', false, false), true),
        (cmd('b', false, false), true),
        (cmd('p', true, false), true),
        (cmd('f', false, true), true),
        (KeyEvent { key: Key::Tab, mods: Modifiers::default() }, false),
        (KeyEvent { key: Key::F(7), mods: Modifiers::default() }, false),
    ];
    for (key, written) in cases {
        assert_eq!(taken.contains(&key), written, "{key:?}");
    }
}
