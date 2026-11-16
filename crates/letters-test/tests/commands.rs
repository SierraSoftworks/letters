//! The command palette: every action by name, from the keyboard.

use letters_editor::Key;
use letters_model::{Block, BlockKind, Change, Document, Note, Notes, Version};
use letters_test::{Found, Harness, Role};
use letters_ui::{Landing, PictureState, Request, SafeMode};

/// One line with a word to select.
fn line() -> Harness {
    let mut window = Harness::doc("Roof first");
    window.select("Roof");
    window
}

/// A table of a header and two rows, with the caret in the first cell of the second row.
fn table() -> Harness {
    let mut window = Harness::doc("| Item | Cost |\n| --- | --- |\n| Slates | 900 |\n| Lead | 200 |");
    window.select("Lead");
    window
}

/// The addresses the pictures of [`pictured`] are linked from.
const LINKS: [&str; 2] = ["https://example.com/roof.png", "https://example.com/gutter.png"];

/// A line and two linked pictures `g1` and `g2`, the first of them selected.
fn pictured() -> Harness {
    let figure = |src: &str| BlockKind::Figure {
        asset: None,
        src: Some(src.into()),
        alt: String::new(),
        width: 300.0,
        height: 150.0,
        numbered: false,
    };
    let blocks = vec![
        Block::text("p", BlockKind::Paragraph, "The roof today:"),
        Block::text("g1", figure(LINKS[0]), "Seen from the garden"),
        Block::text("g2", figure(LINKS[1]), ""),
    ];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    let picked = window.ui().change(|editor| {
        editor.select_picture(&"g1".into());
        Ok::<_, ()>(Change::default())
    });
    picked.expect("a picture to select");
    window.paint();
    window.ui().take_requests();
    window
}

/// A line `a` and a line `b` that two writers each changed apart, with the first `settled` of the two
/// conflicts settled.
fn conflicted(settled: usize) -> Harness {
    let line = |id: &str, text: &str| Block::text(id, BlockKind::Paragraph, text);
    let conflict = |id: &str| {
        let by = |text: &str| Version {
            author: String::new(),
            edited: String::new(),
            block: Some(line(id, text)),
            extra: Default::default(),
        };
        Note::conflict(id, [by("Slates first"), by("Roof first")])
    };
    let blocks = vec![line("a", "Roof first"), line("b", "Roof first")];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.ui().set_notes(Notes { notes: vec![conflict("a"), conflict("b")], ..Notes::default() });
    for _ in 0..settled {
        window.click(Role::Button, "Next conflict").click(Role::Button, "Keep this version");
    }
    window
}

fn palette(window: &mut Harness) -> Option<Found> {
    window.find(Role::Dialog, "Commands")
}

/// The commands the palette lists, in its order.
fn listed(window: &mut Harness) -> Vec<Found> {
    window.controls().into_iter().filter(|found| found.role == Role::ListBoxOption).collect()
}

fn names(window: &mut Harness) -> Vec<String> {
    listed(window).into_iter().map(|found| found.name).collect()
}

/// The command that Enter would take.
fn picked(window: &mut Harness) -> Option<Found> {
    listed(window).into_iter().find(|found| found.selected == Some(true))
}

/// Opens the palette, types and takes what is picked.
fn run(window: &mut Harness, typed: &str) {
    window.key("Mod+Shift+P").type_text(typed).key("Enter");
}

#[test]
fn the_palette_opens_on_its_key_with_the_keyboard_in_its_field_and_goes_away_again() {
    // What puts it away again.
    for away in ["Escape", "Mod+Shift+P"] {
        let mut window = line();
        assert!(palette(&mut window).is_none(), "it waits to be asked for");
        assert!(window.find(Role::Button, "Comment").is_some(), "the tools stand over the selected word");
        window.key("Mod+Shift+P");
        assert!(window.find(Role::Button, "Comment").is_none(), "and step back from under the palette");
        let card = palette(&mut window).expect("the palette").bounds;
        let field = window.focused().expect("something focused");
        assert_eq!((field.role, field.name.as_str()), (Role::TextInput, "Command"));
        assert!(card.y > 0.0 && card.x > 0.0, "the card stands inside the window");
        assert!(!names(&mut window).is_empty(), "with nothing typed it lists what can be done");

        window.key(away);
        assert!(palette(&mut window).is_none(), "{away} puts it away");
        window.type_text("Slates");
        window.assert_markdown("Slates first");
    }
}

#[test]
fn a_press_elsewhere_puts_the_palette_away() {
    let mut window = line();
    window.key("Mod+Shift+P");
    let card = palette(&mut window).expect("the palette").bounds;
    window.click_at(letters_render::Point { x: card.x + card.width / 2.0, y: card.y + card.height + 40.0 });
    assert!(palette(&mut window).is_none());
}

#[test]
fn what_is_typed_finds_a_command_and_enter_does_it() {
    // The words selected, what is typed into the palette, and how the document reads then.
    let cases = [
        ("Roof", "head 2", "## Roof first"),
        ("Roof", "HEADING 1", "# Roof first"),
        ("Roof", "bold", "**Roof** first"),
        ("first", "strike", "Roof ~~first~~"),
        ("Roof", "bullet", "- Roof first"),
        ("Roof", "quote", "> Roof first"),
    ];
    for (selected, typed, reads) in cases {
        let mut window = Harness::doc("Roof first");
        window.select(selected).key("Mod+Shift+P").type_text(typed);
        let first = names(&mut window).into_iter().next();
        assert_eq!(picked(&mut window).map(|found| found.name), first, "{typed}: the first command found is picked");

        window.key("Enter");
        assert!(palette(&mut window).is_none(), "{typed}: the palette goes once a command is taken");
        window.assert_markdown(reads);
        assert_eq!(window.focused().map(|found| found.role), Some(Role::Document), "{typed}: the page has the keys");
    }
}

#[test]
fn the_arrow_keys_walk_the_list_round_its_ends_with_the_pick_in_view() {
    let mut window = line();
    window.key("Mod+Shift+P");
    let all = names(&mut window);
    let list = window.find(Role::ListBox, "Commands").expect("the list").bounds;
    assert!(all.len() > 20, "the list is longer than its card");
    // The key, and the command picked after it.
    let steps = [("Down", 1), ("Down", 2), ("Up", 1), ("Up", 0), ("Up", all.len() - 1), ("Down", 0)];
    for (key, at) in steps {
        window.key(key);
        let pick = picked(&mut window).expect("a pick");
        assert_eq!(pick.name, all[at], "after {key}");
        let (top, bottom) = (pick.bounds.y, pick.bounds.y + pick.bounds.height);
        assert!(top >= list.y && bottom <= list.y + list.height, "{} shows inside the list", pick.name);
        let field = window.focused().expect("something focused");
        assert_eq!(field.role, Role::TextInput, "the keyboard stays in the field");
    }
    window.key("Tab");
    assert_eq!(window.focused().map(|found| found.role), Some(Role::TextInput), "Tab has nowhere else to go");
    window.key("Up").key("Enter");
    assert!(palette(&mut window).is_none(), "Enter takes the pick wherever it is");
}

#[test]
fn a_press_on_a_command_does_it() {
    let mut window = line();
    window.key("Mod+Shift+P").type_text("italic").click(Role::ListBoxOption, "Italic");
    assert!(palette(&mut window).is_none());
    window.assert_markdown("*Roof* first");
}

#[test]
fn a_command_shows_the_key_that_does_the_same() {
    // A command, its key on Linux and Windows, and on a Mac.
    let cases = [
        ("Bold", Some("Ctrl+B"), Some("\u{2318}B")),
        ("Heading 1", Some("Ctrl+Alt+1"), Some("\u{2325}\u{2318}1")),
        ("Link", Some("Ctrl+K"), Some("\u{2318}K")),
        ("Save a copy", Some("Ctrl+Shift+S"), Some("\u{21e7}\u{2318}S")),
        ("Share", None, None),
    ];
    for mac in [false, true] {
        let mut window = if mac { line().mac() } else { line() };
        window.key("Mod+Shift+P");
        let listed = listed(&mut window);
        for (name, standard, on_mac) in cases {
            let command = listed.iter().find(|found| found.name == name).unwrap_or_else(|| panic!("{name} is listed"));
            assert_eq!(command.description.as_deref(), if mac { on_mac } else { standard }, "{name}, mac: {mac}");
        }
    }
}

#[test]
fn a_command_is_listed_only_where_it_has_something_to_act_on() {
    fn held(folder: Option<&str>) -> Harness {
        let mut window = line();
        window.ui().set_safe_mode(Some(SafeMode { folder: folder.map(str::to_owned) }));
        window
    }
    fn typed() -> Harness {
        let mut window = line();
        window.type_text("Slates");
        window
    }
    fn caret() -> Harness {
        let mut window = line();
        window.key("Right");
        window
    }
    // Two conflicts of which `settled` are settled, with the next one open.
    fn reviewing(settled: usize) -> Harness {
        let mut window = conflicted(settled);
        window.click(Role::Button, "Next conflict");
        window
    }
    // A window, a command, and whether the palette lists it there.
    type Open = fn() -> Harness;
    let cases: [(&str, Open, &str, bool); 27] = [
        ("words selected", line, "Comment", true),
        ("a caret", caret, "Comment", false),
        ("nothing done yet", line, "Undo", false),
        ("something typed", typed, "Undo", true),
        ("in a line", line, "Add row below", false),
        ("in a table", table, "Add row below", true),
        ("in the last row", table, "Move row down", false),
        ("in the last row", table, "Move row up", true),
        ("a table with a header", table, "Turn header row off", true),
        ("a table with a header", table, "Turn header row on", false),
        ("in a line", line, "Edit alt text", false),
        ("a picture selected", pictured, "Edit alt text", true),
        ("a picture selected", pictured, "Bold", false),
        ("a picture selected", pictured, "Heading 1", false),
        ("a picture selected", pictured, "Copy", true),
        ("a caret", caret, "Copy", false),
        ("in a line", line, "Tick or untick task", false),
        ("a trusted document", line, "Trust this document", false),
        ("a held document", || held(None), "Trust this folder", false),
        ("a held document in a folder", || held(Some("Downloads")), "Trust this folder", true),
        ("no conflicts", line, "Next conflict", false),
        ("conflicts that wait", || conflicted(0), "Next conflict", true),
        ("conflicts that wait", || conflicted(0), "Keep this version", false),
        ("one conflict of two open", || reviewing(0), "Use the other version", true),
        ("one conflict of two open", || reviewing(0), "Previous conflict", true),
        ("the only conflict open", || reviewing(1), "Close conflict", true),
        ("the only conflict open", || reviewing(1), "Next conflict", false),
    ];
    for (what, open, command, lists) in cases {
        let mut window = open();
        window.key("Mod+Shift+P");
        assert_eq!(names(&mut window).iter().any(|name| name == command), lists, "{what}: {command}");
    }
}

#[test]
fn the_commands_of_a_table_change_the_table_the_caret_is_in() {
    let cell = |window: &Harness| window.editor().cell().expect("a caret in the table");
    let before = cell(&table());
    assert_eq!(((before.rows, before.columns), (before.row, before.column)), ((3, 2), (2, 0)));
    // What is typed, and the table's rows and columns after it.
    let sizes = [
        ("add row below", (4, 2)),
        ("add row above", (4, 2)),
        ("column to the right", (3, 3)),
        ("column to the left", (3, 3)),
        ("remove row", (2, 2)),
        ("remove column", (3, 1)),
    ];
    for (typed, size) in sizes {
        let mut window = table();
        run(&mut window, typed);
        let after = cell(&window);
        assert_eq!((after.rows, after.columns), size, "{typed}");
    }
    // What is typed, and the row and column the caret's cell is in after it.
    let moves = [("move row up", (1, 0)), ("move column right", (2, 1))];
    for (typed, at) in moves {
        let mut window = table();
        run(&mut window, typed);
        let after = cell(&window);
        assert_eq!(((after.rows, after.columns), (after.row, after.column)), ((3, 2), at), "{typed}");
    }

    let mut window = table();
    run(&mut window, "header row off");
    let table = window.editor().doc().blocks.first().map(|block| block.kind.clone());
    assert!(matches!(table, Some(BlockKind::Table { header: false, .. })), "the first row is a row like the others");
}

#[test]
fn the_commands_of_the_window_open_what_its_buttons_open() {
    // What is typed, and the control that shows for it with the keyboard in it or not.
    let cases = [
        ("settings", Role::Dialog, "Settings", false),
        ("save a copy", Role::Dialog, "Save a copy", false),
        ("share", Role::Dialog, "Share", false),
        ("link", Role::TextInput, "Link", true),
        ("comment", Role::TextInput, "Comment", true),
        ("rename", Role::TextInput, "Document name", true),
        ("history", Role::Complementary, "History", false),
    ];
    for (typed, role, name, focused) in cases {
        let mut window = line();
        run(&mut window, typed);
        let shown = window.find(role, name).unwrap_or_else(|| panic!("{typed} shows {name}"));
        assert!(!focused || shown.focused, "{typed}: the keyboard is in {name}");
        assert!(palette(&mut window).is_none(), "{typed}: the palette is out of its way");
    }

    let mut window = line();
    run(&mut window, "notes panel");
    assert!(window.find(Role::Button, "Notes").is_some_and(|button| button.toggled == Some(true)));
    run(&mut window, "new doc");
    assert_eq!(window.ui().take_requests(), [Request::New]);
}

#[test]
fn a_picture_is_described_and_kept_from_the_keyboard() {
    let mut window = pictured();
    run(&mut window, "alt text");
    let field = window.focused().expect("something focused");
    assert_eq!((field.role, field.name.as_str()), (Role::TextInput, "Alt text"));
    window.type_text("A slate roof").key("Enter");
    let alt = window.editor().doc().block(&"g1".into()).map(|figure| figure.kind.clone());
    assert!(matches!(alt, Some(BlockKind::Figure { alt, .. }) if alt == "A slate roof"));

    // What is typed with the first picture selected, and what the shell is asked then.
    let embed = |figure: &str, src: &str| Request::EmbedPicture { figure: figure.into(), src: src.into() };
    let cases = [
        ("embed image in", vec![embed("g1", LINKS[0])]),
        ("embed all", vec![embed("g1", LINKS[0]), embed("g2", LINKS[1])]),
        ("load image once", vec![Request::LoadPicture(LINKS[0].into())]),
    ];
    for (typed, asks) in cases {
        let mut window = pictured();
        run(&mut window, typed);
        assert_eq!(window.ui().take_requests(), asks, "{typed}");
    }
}

#[test]
fn a_held_document_is_trusted_from_the_palette_before_its_pictures_can_be_kept() {
    let mut window = pictured();
    window.ui().set_safe_mode(Some(SafeMode { folder: None }));
    for link in LINKS {
        window.ui().set_picture_state(link, Some(PictureState::Held));
    }
    window.key("Mod+Shift+P");
    let offered = names(&mut window);
    for fetches in ["Embed all remote images", "Embed image in the document", "Load image once"] {
        assert!(!offered.iter().any(|name| name == fetches), "nothing is fetched for a held document: {fetches}");
    }
    window.type_text("trust").key("Enter");
    assert_eq!(window.ui().take_requests(), [Request::TrustDocument]);
}

#[test]
fn words_that_find_nothing_say_so_and_enter_then_does_nothing() {
    let mut window = line();
    window.key("Mod+Shift+P").type_text("zzz");
    assert!(names(&mut window).is_empty());
    let said = window.controls().into_iter().find(|found| found.role == Role::Status).and_then(|said| said.value);
    assert_eq!(said.as_deref(), Some("No command matches."));
    window.key("Enter");
    assert!(palette(&mut window).is_none());
    window.assert_markdown("Roof first");
}

#[test]
fn the_more_menu_opens_the_palette_and_names_its_key_beside_it() {
    let mut window = line();
    window.click(Role::Button, "More");
    window.click(Role::MenuItem, "Commands Ctrl+Shift+P");
    assert!(palette(&mut window).is_some());
    assert_eq!(window.focused().map(|found| found.name), Some("Command".into()));
}

#[test]
fn the_palette_waits_while_a_card_asks_what_to_open() {
    let mut window = line();
    window.ui().set_landing(Some(Landing::default()));
    window.key("Mod+Shift+P");
    assert!(palette(&mut window).is_none(), "there is no document to act on yet");
}

#[test]
fn a_shell_runs_a_command_by_its_name_where_the_palette_offers_it() {
    // The command, whether it is offered with a word selected, and how the document reads once it was asked.
    let cases = [
        ("Heading 2", true, "## Roof first"),
        ("Bold", true, "**Roof** first"),
        ("Remove row", false, "Roof first"),
        ("No such command", false, "Roof first"),
    ];
    for (name, offered, reads) in cases {
        let mut window = line();
        assert_eq!(window.ui().offers(name), offered, "{name:?}");
        let mut platform = letters_test::TestPlatform::default();
        assert_eq!(window.ui().run_command(name, &mut platform), offered, "{name:?}");
        window.assert_markdown(reads);
    }
}

#[test]
fn a_shell_is_told_the_key_of_a_command_as_its_system_writes_it() {
    let chord = |window: &mut Harness, name: &str| {
        let key = window.ui().command_key(name)?;
        Some((key.key, key.mods.primary, key.mods.shift, key.mods.alt))
    };
    let (mut standard, mut mac) = (line(), line().mac());
    for name in letters_ui::command_names() {
        assert_eq!(chord(&mut standard, name).is_some(), chord(&mut mac, name).is_some(), "{name:?}");
    }
    assert_eq!(chord(&mut standard, "Bold"), Some((Key::Char('b'), true, false, false)));
    assert_eq!(chord(&mut standard, "Find and replace"), Some((Key::Char('h'), true, false, false)));
    // A Mac hides the application on Cmd with H.
    assert_eq!(chord(&mut mac, "Find and replace"), Some((Key::Char('f'), true, false, true)));
    assert_eq!(chord(&mut standard, "Share"), None);
    assert_eq!(chord(&mut standard, "No such command"), None);
}
