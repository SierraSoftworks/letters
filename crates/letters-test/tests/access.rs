//! What the person may do with a document: someone who only reads it, or reads and comments, changes
//! nothing in it and is offered nothing that would.

use accesskit::{Action, ActionRequest, NodeId, TreeId};
use letters_editor::{Clip, InputEvent, Picture};
use letters_model::{Anchor, Block, BlockKind, Document, Note, NoteKind, Notes, Run, Version};
use letters_render::image::Image;
use letters_render::{Item, PathEl, Point, Rect};
use letters_test::{Found, Harness, Role};
use letters_ui::{Access, Request};

const ADDRESS: &str = "https://example.org/manual";
const PICTURE: &str = "https://example.org/roof.png";
/// What a server grants short of writing.
const LESS: [Access; 2] = [Access::Comments, Access::Reads];
const ALL: [Access; 3] = [Access::Edits, Access::Comments, Access::Reads];
/// What someone who only reads is told where a comment would be written.
const READS: &str = "You can read this document, but not comment on it.";

/// A plan with one of each thing that is changed where it stands: a heading, a line with a link, a task,
/// code, a table and a callout.
fn plan() -> Harness {
    let markdown = format!(
        "# Roof plan\n\nThe brown fox reads [the manual]({ADDRESS}) twice.\n\n- [ ] Call the roofer\n\n\
         ```rust\nfn main() {{}}\n```\n\n| Name | Count |\n| --- | --- |\n| Apples | 3 |\n\n> [!TIP]\n> Mind the step.\n\nLast line."
    );
    let mut window = Harness::doc(&markdown);
    window.ui().set_author("Sam Okafor");
    window.resize(1100.0, 1400.0);
    window.paint();
    window
}

/// A report with what Markdown has no word for: a table of contents, the preview of a link whose page
/// nothing was fetched of, and a picture linked from a host.
fn report() -> Harness {
    let figure = BlockKind::Figure {
        asset: None,
        src: Some(PICTURE.into()),
        alt: "A slate roof".into(),
        width: 300.0,
        height: 150.0,
        numbered: false,
    };
    let preview = BlockKind::LinkPreview { url: ADDRESS.into(), title: None, description: None, image: None };
    let blocks = vec![
        Block::text("title", BlockKind::Heading { level: 1 }, "Roof report"),
        Block::new("toc", BlockKind::TableOfContents { levels: 3 }),
        Block::text("schedule", BlockKind::Heading { level: 2 }, "Schedule"),
        Block::new("card", preview),
        Block::text("g", figure, "Seen from the garden"),
        Block::text("end", BlockKind::Paragraph, "Last line."),
    ];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.resize(1100.0, 1400.0);
    window.paint();
    window
}

/// The plan as someone with `access` to it has it open.
fn granted(access: Access) -> Harness {
    let mut window = plan();
    window.ui().set_access(access);
    window.paint();
    window
}

fn page(window: &mut Harness) -> Rect {
    window.controls().into_iter().find(|found| found.role == Role::Document).expect("a page").bounds
}

/// A point on the first place `text` stands on the page. The caret is left at the end of the document.
fn on(window: &mut Harness, text: &str) -> Point {
    window.select(text).paint();
    let caret = window.ui().caret_rect().expect("the page shows a caret");
    window.select("Last line").key("Right").paint();
    Point { x: caret.x - 2.0, y: caret.y + caret.height / 2.0 }
}

/// Whether the window has a control of a role by a name.
fn has(window: &mut Harness, role: Role, name: &str) -> bool {
    window.find(role, name).is_some()
}

/// Whether something in the window says a sentence.
fn says(window: &mut Harness, sentence: &str) -> bool {
    let said = |found: &Found| {
        let texts = [Some(found.name.as_str()), found.value.as_deref(), found.description.as_deref()];
        texts.contains(&Some(sentence))
    };
    window.controls().iter().any(said)
}

/// Whether assistive technology is told that a press does something to the control of a role by a name.
fn pressable(window: &mut Harness, role: Role, name: &str) -> bool {
    let tree = window.ui().access();
    // A tick has no name of its own.
    let named = |node: &accesskit::Node| node.role() == role && (name.is_empty() || node.label() == Some(name));
    tree.nodes.iter().any(|(_, node)| named(node) && node.supports_action(Action::Click))
}

/// The commands the palette lists for what is typed into it.
fn offered(window: &mut Harness, typed: &str) -> Vec<String> {
    window.key("Mod+Shift+P").type_text(typed);
    let rows = window.controls().into_iter().filter(|found| found.role == Role::ListBoxOption);
    let names = rows.map(|row| row.name).collect();
    window.key("Escape");
    names
}

/// The rows and buttons the menu about the place of a text brings out, by name.
fn menu_on(window: &mut Harness, text: &str) -> Vec<String> {
    let at = on(window, text);
    menu_at(window, at)
}

/// The rows and buttons the menu about a point brings out, by name.
fn menu_at(window: &mut Harness, at: Point) -> Vec<String> {
    let listed = |found: &Found| matches!(found.role, Role::MenuItem | Role::MenuItemCheckBox | Role::Button);
    let names = |window: &mut Harness| -> Vec<String> {
        window.controls().into_iter().filter(listed).map(|found| found.name).collect()
    };
    let mut before = names(window);
    window.secondary_click_at(at).assert_open(Role::Menu, "Context menu");
    let mut brought = names(window);
    // What was there before the menu is not the menu's, also when the menu has a button of the same name.
    brought.retain(|name| before.iter().position(|was| was == name).map(|at| before.swap_remove(at)).is_none());
    window.key("Escape");
    brought
}

/// The wavy lines under misspelled words: how many the page shows.
fn waves(window: &mut Harness) -> usize {
    let page = page(window);
    let wave = |item: &&Item| match item {
        Item::Path { origin, path, stroke: Some(_), .. } => {
            page.contains(*origin) && path.iter().any(|el| matches!(el, PathEl::QuadTo(..)))
        }
        _ => false,
    };
    window.paint().items.iter().filter(wave).count()
}

/// Lets the window do what it does in idle time, until it has nothing left.
fn rest(window: &mut Harness) {
    for _ in 0..200 {
        window.advance(50).paint();
    }
}

/// How many glyphs the page draws.
fn glyphs(window: &mut Harness) -> usize {
    let page = page(window);
    let drawn = |item: &Item| match item {
        Item::Glyphs { origin, glyphs, .. } if page.contains(*origin) => glyphs.len(),
        _ => 0,
    };
    window.paint().items.iter().map(drawn).sum()
}

/// A comment that Alice wrote on the brown fox.
fn asked() -> Notes {
    let note = Note {
        id: "asked".into(),
        kind: NoteKind::Comment,
        anchor: Anchor::range("b2", 4..13),
        author: "Alice Moreau".into(),
        created: "2026-11-10T08:00:00Z".into(),
        resolved: false,
        content: vec![Run::plain("Which fox?")],
        replies: Vec::new(),
        extra: Default::default(),
    };
    Notes { notes: vec![note], ..Notes::default() }
}

/// A conflict about the last line of the report: what Alice made of it, held back for what the page reads.
fn apart() -> Notes {
    let by = |author: &str, edited: &str, text: &str| Version {
        author: author.into(),
        edited: edited.into(),
        block: Some(Block::text("end", BlockKind::Paragraph, text)),
        extra: Default::default(),
    };
    let versions = [
        by("Alice Moreau", "2026-11-10T08:00:00Z", "First line."),
        by("Sam Okafor", "2026-11-15T01:41:00Z", "Last line."),
    ];
    Notes { notes: vec![Note::conflict("end", versions)], ..Notes::default() }
}

fn png() -> Picture {
    let bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    Picture { bytes, mime: "image/png".into(), width: 400, height: 200 }
}

/// The report with its conflict open.
fn settling() -> Harness {
    let mut window = report();
    window.ui().set_notes(apart());
    window.click(Role::Button, "Next conflict");
    window
}

/// A press on a node, as assistive technology asks for it.
fn press(target_node: NodeId) -> ActionRequest {
    ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node, data: None }
}

/// Presses the first node of a role as assistive technology does, whatever the node says of itself.
fn ask_to_press(window: &mut Harness, role: Role) {
    let tree = window.ui().access();
    let node = tree.nodes.iter().find(|(_, node)| node.role() == role).expect("a node of the role").0;
    window.ask(press(node));
}

/// All that the window shows and has asked of its shell, to tell whether a press did anything.
fn state(window: &mut Harness) -> (String, Vec<(Role, String)>, Vec<Request>) {
    let shown = window.controls().into_iter().map(|found| (found.role, found.name)).collect();
    (window.markdown(), shown, window.ui().take_requests())
}

#[test]
fn nothing_someone_does_changes_a_document_they_may_not_write() {
    type Does = fn(&mut Harness);
    let tries: [(&str, Does); 12] = [
        ("typing", |window| _ = window.select("brown").type_text("x")),
        ("text an input method commits", |window| _ = window.select("brown").input(InputEvent::Commit("x".into()))),
        ("a key that deletes", |window| _ = window.select("brown").key("Backspace")),
        ("a key that breaks the line", |window| _ = window.select("brown").key("Enter")),
        ("a key that formats", |window| _ = window.select("brown").key("Mod+B")),
        ("a cut", |window| _ = window.select("brown").key("Mod+X")),
        ("a paste", |window| {
            window.platform.clipboard = Some(Clip { text: "pasted".into(), ..Clip::default() });
            window.select("brown").key("Mod+V");
        }),
        ("a command", |window| _ = window.select("brown").key("Mod+Shift+P").type_text("bold").key("Enter")),
        ("the key of the link card", |window| _ = window.select("brown").key("Mod+K").type_text("x").key("Enter")),
        ("pictures let go over the page", |window| {
            let at = on(window, "brown");
            window.drop_pictures(Some(at), vec![png()]);
        }),
        ("a press on the tick of a task", |window| ask_to_press(window, Role::CheckBox)),
        ("a replacement for a word that is found", |window| {
            window.key("Mod+H").type_text("fox");
            // The field is there for someone who writes. Anyone else has typed at the page.
            if window.find(Role::TextInput, "Replace with").is_some() {
                window.click(Role::TextInput, "Replace with").type_text("cat").click(Role::Button, "Replace all");
            }
        }),
    ];
    for access in LESS {
        for (what, does) in tries {
            let mut window = granted(access);
            let (before, edits) = (window.markdown(), window.ui().edits());
            does(&mut window);
            assert_eq!(window.markdown(), before, "{access:?}: {what}");
            assert_eq!(window.ui().edits(), edits, "{access:?}: {what} leaves nothing to save");
        }
    }
    // Each of them changes the document of someone who writes it.
    for (what, does) in tries {
        let mut window = granted(Access::Edits);
        let before = window.markdown();
        does(&mut window);
        assert_ne!(window.markdown(), before, "{what}");
    }
}

#[test]
fn a_press_where_something_was_offered_before_the_right_to_write_went_is_not_done() {
    type Opens = fn() -> Harness;
    // What a writer is offered on the page: by role and name, where the caret is for it to show, and
    // whether it is told where it stands, so that the pointer can be put there.
    let offered: [(Opens, Role, &str, &str, bool); 9] = [
        (plan, Role::Button, "Edit callout", "Mind", false),
        (plan, Role::Button, "Language: Rust", "main", false),
        (plan, Role::Button, "Column 1", "Apples", true),
        (plan, Role::CheckBox, "", "Call", false),
        (report, Role::Button, "Table of contents options", "Last", true),
        (report, Role::Button, "Edit link preview", "Last", true),
        (report, Role::Button, "Fetch preview", "Last", true),
        (report, Role::Button, "Alt text", "Last", false),
        (settling, Role::Button, "Use this version", "Last", true),
    ];
    for (opens, role, name, at, placed) in offered {
        // The press comes from assistive technology, which was told of a node, or from the pointer.
        let ways = ALL.into_iter().flat_map(|access| [(access, false), (access, true)]);
        for (access, pointer) in ways.filter(|(_, pointer)| placed || !pointer) {
            let mut window = opens();
            window.select(at).key("Left").paint();
            let named = |found: &Found| found.role == role && (name.is_empty() || found.name == name);
            let place = window.controls().into_iter().find(named).unwrap_or_else(|| panic!("{name} shows")).center();
            let tree = window.ui().access();
            let named = |node: &accesskit::Node| node.role() == role && (name.is_empty() || node.label() == Some(name));
            let node = tree.nodes.iter().find(|(_, node)| named(node)).unwrap_or_else(|| panic!("{name} is told")).0;
            window.ui().set_access(access);
            window.paint();
            // What the window asked on the way is not the press's.
            window.ui().take_requests();
            let before = state(&mut window);
            match pointer {
                true => window.click_at(place),
                false => window.ask(press(node)),
            };
            window.paint();
            let done = state(&mut window) != before;
            assert_eq!(done, access == Access::Edits, "{access:?}: {role:?} {name:?}, by the pointer: {pointer}");
        }
    }
}

#[test]
fn the_right_to_write_comes_and_goes_as_the_shell_says() {
    let mut window = plan();
    window.select("brown").type_text("red");
    let written = window.markdown();
    for access in LESS {
        window.ui().set_access(access);
        window.select("fox").type_text("cat").key("Mod+Z");
        assert_eq!(window.markdown(), written, "{access:?}: nothing is typed, and nothing is taken back");
    }
    window.ui().set_access(Access::Edits);
    window.select("fox").type_text("cat");
    assert!(window.markdown().contains("The red cat reads"), "the document is written again: {}", window.markdown());
    window.key("Mod+Z").key("Mod+Z");
    assert!(window.markdown().contains("The brown fox reads"), "and what was written before can be taken back");
}

#[test]
fn the_bar_says_what_the_person_may_do_where_its_tools_stood() {
    let cases = [
        (Access::Edits, None),
        (Access::Comments, Some(("Commenting", "You can comment on this document, but not change it."))),
        (Access::Reads, Some(("Reading", "You can read this document, but not change it."))),
    ];
    // What changes the document from the bar, and what does not.
    let tools = ["Undo", "Redo", "Bold", "Italic", "Link", "Styles", "History"];
    let stays = ["Notes", "More", "Share"];
    let others: Vec<&str> = cases.iter().filter_map(|(_, says)| Some(says.as_ref()?.0)).collect();
    let mut window = plan();
    // The role can change while the document is open, and back.
    for (access, says) in cases.into_iter().chain(cases.into_iter().rev()) {
        window.ui().set_access(access);
        window.paint();
        let top = page(&mut window).y;
        for tool in tools {
            assert_eq!(has(&mut window, Role::Button, tool), says.is_none(), "{access:?}: {tool}");
        }
        for button in stays {
            assert!(has(&mut window, Role::Button, button), "{access:?}: {button}");
        }
        window.click(Role::Button, "More");
        assert_eq!(has(&mut window, Role::MenuItem, "Page setup"), says.is_none(), "{access:?}: the page's setup");
        window.key("Escape");
        let name = window.find(Role::Button, "Roof plan").filter(|name| name.bounds.y < top);
        assert_eq!(name.is_some(), says.is_none(), "{access:?}: the name is pressed to change it");
        for other in &others {
            let said = window.find(Role::Label, other);
            assert_eq!(said.is_some(), says.is_some_and(|(says, _)| says == *other), "{access:?}: {other}");
        }
        if let Some((says, more)) = says {
            let said = window.find(Role::Label, says).expect("it was found");
            assert!(said.bounds.y < top, "{access:?}: on the bar");
            assert_eq!(said.description.as_deref(), Some(more), "{access:?}");
        }
    }
}

#[test]
fn the_page_offers_nothing_that_changes_it() {
    // What a writer presses on the page: by role and name, and where the caret has to be for it to show.
    let changes = [
        (Role::Button, "Edit callout", "Mind"),
        (Role::Button, "Language: Rust", "main"),
        (Role::Button, "Column 1", "Apples"),
        (Role::Button, "Add row at the end", "Apples"),
        (Role::CheckBox, "", "Call"),
    ];
    for access in ALL {
        let mut window = granted(access);
        for (role, name, at) in changes {
            window.select(at).key("Left").paint();
            let offered = pressable(&mut window, role, name);
            assert_eq!(offered, access == Access::Edits, "{access:?}: {role:?} {name:?}");
        }
        // What only reads stays for everyone.
        assert!(pressable(&mut window, Role::Button, "Copy code"), "{access:?}");
        // Pictures held over the page have nowhere to land.
        let over = on(&mut window, "brown");
        assert_eq!(window.hold_pictures(Some(over)), access == Access::Edits, "{access:?}");
    }
    // The same of what stands on the page whole.
    let whole = ["Table of contents options", "Edit link preview", "Fetch preview", "Alt text"];
    for access in ALL {
        let mut window = report();
        window.ui().set_access(access);
        window.paint();
        for name in whole {
            assert_eq!(pressable(&mut window, Role::Button, name), access == Access::Edits, "{access:?}: {name}");
        }
        // A line of the contents goes to its heading for everyone.
        assert!(pressable(&mut window, Role::Link, "Schedule"), "{access:?}");
    }
}

#[test]
fn someone_listening_is_told_again_of_what_can_no_longer_be_pressed() {
    let ticks = |update: &accesskit::TreeUpdate| -> Vec<bool> {
        let ticks = update.nodes.iter().filter(|(_, node)| node.role() == Role::CheckBox);
        ticks.map(|(_, node)| node.supports_action(Action::Click)).collect()
    };
    let mut window = plan();
    assert_eq!(ticks(&window.ui().access()), [true], "a writer ticks the task");
    assert_eq!(ticks(&window.ui().access_changes()), [false; 0], "and is told so once");
    // What changed and what the tick then says, each way.
    for (access, pressed) in [(Access::Reads, false), (Access::Edits, true)] {
        window.ui().set_access(access);
        assert_eq!(ticks(&window.ui().access_changes()), [pressed], "{access:?}");
    }
}

#[test]
fn the_page_of_a_link_is_fetched_and_the_document_named_by_someone_who_writes() {
    for access in ALL {
        let writes = access == Access::Edits;
        let mut window = granted(access);
        // A link whose page nothing is kept of offers to fetch it, on the card that shows over it.
        let link = on(&mut window, "manual");
        window.hover_at(link).advance(0).advance(1_000);
        assert_eq!(has(&mut window, Role::Button, "Fetch preview"), writes, "{access:?}: what is fetched is kept");
        // A document without a name takes its heading once the caret leaves it.
        window.ui().take_requests();
        window.select("Roof").select("Last");
        let named = window.ui().take_requests().contains(&Request::Named("Roof plan".into()));
        assert_eq!(named, writes, "{access:?}: the name is the document's to keep");
    }
}

#[test]
fn a_linked_picture_is_loaded_by_anyone_and_embedded_by_someone_who_writes() {
    for access in ALL {
        let mut window = report();
        window.ui().set_access(access);
        // Where the picture stands, by what is drawn of it once the shell hands it over.
        window.ui().set_image(PICTURE, Some(Image::from_rgba8(2, 1, vec![255; 8]).expect("two pixels")));
        let drawn = |item: &Item| match item {
            Item::Image { rect, .. } => Some(*rect),
            _ => None,
        };
        let place = window.paint().items.iter().find_map(drawn).expect("a picture");
        window.ui().set_image(PICTURE, None);
        window.paint();
        window.ui().take_requests();
        // The cloud in its corner.
        let cloud = Point { x: place.x + place.width - 16.0, y: place.y + 16.0 };
        window.click_at(cloud).assert_open(Role::Menu, "Linked picture");
        let embeds = has(&mut window, Role::MenuItem, "Embed in the document");
        assert_eq!(embeds, access == Access::Edits, "{access:?}: to embed it changes the document");
        window.click(Role::MenuItem, "Load once");
        assert_eq!(window.ui().take_requests(), [Request::LoadPicture(PICTURE.into())], "{access:?}");
    }
}

#[test]
fn the_markers_of_what_is_typed_show_only_to_someone_who_writes() {
    for access in ALL {
        let mut window = Harness::doc("Plain words.\n\nSome **bold** words.");
        window.ui().set_access(access);
        window.select("Plain").key("Left");
        let plain = glyphs(&mut window);
        window.select("bold").key("Left").key("Right");
        // The markers fade in.
        window.advance(1000);
        let inside = glyphs(&mut window);
        assert_eq!(inside > plain, access == Access::Edits, "{access:?}: {plain} glyphs, then {inside}");
    }
}

#[test]
fn the_source_of_a_formula_is_out_for_someone_who_writes_it() {
    for access in ALL {
        let mut window = Harness::doc("The area is $x^2$ here.");
        window.ui().set_reduced_motion(true);
        window.ui().set_access(access);
        // The caret goes into the formula.
        window.select("^").key("Left").paint();
        let sheet = has(&mut window, Role::Code, "Formula source");
        assert_eq!(sheet, access == Access::Edits, "{access:?}: a formula reads as it is set");
    }
}

#[test]
fn misspelled_words_are_marked_for_someone_who_can_mend_them() {
    let mut window = Harness::doc("The quik brown fox.");
    rest(&mut window);
    assert_eq!(waves(&mut window), 1, "a writer sees the word marked");
    for access in LESS {
        window.ui().set_access(access);
        rest(&mut window);
        assert_eq!(waves(&mut window), 0, "{access:?}");
        window.key("F7").assert_closed(Role::Menu, "Context menu");
        window.ui().set_access(Access::Edits);
        rest(&mut window);
        assert_eq!(waves(&mut window), 1, "the marks come back with the right to write");
    }
}

#[test]
fn the_commands_and_the_menu_are_those_that_read() {
    // What is typed into the palette, the command it finds, and who is offered it.
    let commands = [
        ("bold", "Bold", [true, false, false]),
        ("heading 2", "Heading 2", [true, false, false]),
        ("page setup", "Page setup", [true, false, false]),
        ("history", "History", [true, false, false]),
        ("replace", "Find and replace", [true, false, false]),
        ("comment", "Comment", [true, true, false]),
        ("find", "Find", [true, true, true]),
        ("print view", "Print view", [true, true, true]),
        ("select all", "Select all", [true, true, true]),
    ];
    // What the menu on the link lists, and for whom.
    let rows = [
        ("Edit link", [true, false, false]),
        ("Remove link", [true, false, false]),
        ("Bold", [true, false, false]),
        ("Cut", [true, false, false]),
        ("Turn into", [true, false, false]),
        ("Copy", [true, true, true]),
        ("Open link", [true, true, true]),
        ("Copy link address", [true, true, true]),
    ];
    for (who, access) in ALL.into_iter().enumerate() {
        let mut window = granted(access);
        for (typed, command, offers) in commands {
            window.select("brown");
            let listed = offered(&mut window, typed);
            assert_eq!(listed.iter().any(|name| name == command), offers[who], "{access:?}: {command} in {listed:?}");
        }
        let listed = menu_on(&mut window, "manual");
        for (row, offers) in rows {
            // A row is named with its key, where it has one.
            let named =
                |name: &String| name.strip_prefix(row).is_some_and(|key| key.is_empty() || key.starts_with(" Ctrl"));
            let there = listed.iter().any(named);
            assert_eq!(there, offers[who], "{access:?}: {row} in {listed:?}");
        }
    }
}

#[test]
fn the_menu_about_any_place_lists_only_what_reads() {
    type Opens = fn() -> Harness;
    type Place = fn(&mut Harness) -> Point;
    // Where the menu is asked for: in plain text, a link, a table, a callout, code, a task and a picture.
    let places: [(&str, Opens, Place); 7] = [
        ("text", plan, |window| on(window, "brown")),
        ("a link", plan, |window| on(window, "manual")),
        ("a table", plan, |window| on(window, "Apples")),
        ("a callout", plan, |window| on(window, "Mind")),
        ("code", plan, |window| on(window, "main")),
        ("a task", plan, |window| on(window, "Call")),
        ("a picture", report, |window| {
            let caption = on(window, "Seen");
            Point { x: caption.x, y: caption.y - 60.0 }
        }),
    ];
    // How the names of the rows that read start. A row is named with its key, where it has one.
    let reads = ["Copy", "Comment", "Open link", "Select all", "Find Ctrl", "All commands", "Load", "Always load"];
    let writes = |listed: &[String]| -> Vec<String> {
        let read = |name: &&String| !reads.iter().any(|reads| name.starts_with(reads));
        listed.iter().filter(read).cloned().collect()
    };
    for (place, opens, at) in places {
        for access in ALL {
            let mut window = opens();
            window.ui().set_access(access);
            let at = at(&mut window);
            let listed = menu_at(&mut window, at);
            let writes = writes(&listed);
            assert_eq!(writes.is_empty(), access != Access::Edits, "{access:?} on {place}: {writes:?}");
        }
    }
}

#[test]
fn the_bar_is_fitted_again_when_the_role_changes() {
    // What the bar shows and where, in a window with room for a long name beside a reader's bar and not
    // beside a writer's tools.
    let bar = |window: &mut Harness| -> Vec<(String, Rect)> {
        let top = page(window).y;
        let on_bar = window.controls().into_iter().filter(|found| found.bounds.y < top && found.bounds.height > 0.0);
        on_bar.map(|found| (found.name, found.bounds)).collect()
    };
    let long = || Harness::doc("# A plan for the roof of the cottage on the hill\n\nReply by Friday.");
    // The role comes to a window that has fitted its bar for a writer, and to one that has not.
    let mut came = long();
    came.resize(760.0, 700.0).paint();
    came.ui().set_access(Access::Reads);
    let mut was = long();
    was.ui().set_access(Access::Reads);
    was.resize(760.0, 700.0).paint();
    assert_eq!(bar(&mut came), bar(&mut was), "the bar is fitted again when the role changes");
}

#[test]
fn someone_who_comments_writes_comments_and_someone_who_reads_does_not() {
    // A comment of one's own.
    for access in ALL {
        let mut window = granted(access);
        let (before, edits) = (window.markdown(), window.ui().edits());
        window.select("brown fox").paint();
        let tools = [has(&mut window, Role::Button, "Comment"), has(&mut window, Role::Button, "Link")];
        assert_eq!(tools, [access != Access::Reads, access == Access::Edits], "{access:?}: the tools over a selection");
        if access == Access::Reads {
            continue;
        }
        window.click(Role::Button, "Comment").type_text("Which fox?").key("Enter");
        let kept = window.ui().notes().notes.len();
        assert_eq!((kept, window.ui().edits()), (1, edits + 1), "{access:?}: a comment is kept, to be saved");
        assert_eq!(window.markdown(), before, "{access:?}: and the text is as it was");
    }
    // A comment of someone else's.
    for access in ALL {
        let mut window = granted(access);
        window.ui().set_notes(asked());
        window.click(Role::Button, "1 comment").assert_open(Role::Dialog, "Comments");
        let can = access != Access::Reads;
        assert_eq!(has(&mut window, Role::TextInput, "Reply"), can, "{access:?}: a field for the reply");
        assert_eq!(says(&mut window, READS), !can, "{access:?}: or why there is none");
        assert_eq!(window.find(Role::Button, "Resolve").map(|resolve| !resolve.disabled), Some(can), "{access:?}");
        window.click(Role::Button, "Resolve");
        assert_eq!(window.ui().notes().notes[0].resolved, can, "{access:?}");
    }
}

#[test]
fn a_conflict_is_read_by_anyone_and_settled_by_someone_who_writes() {
    for access in ALL {
        let mut window = report();
        window.ui().set_access(access);
        window.ui().set_notes(apart());
        window.click(Role::Button, "Next conflict");
        let held = |found: &Found| found.role == Role::Note && found.value.as_deref() == Some("First line.");
        assert!(window.controls().iter().any(held), "{access:?}: the version held back is read");
        let chooses = ["Keep this version", "Use this version"].map(|name| has(&mut window, Role::Button, name));
        assert_eq!(chooses, [access == Access::Edits; 2], "{access:?}: to choose one changes the document");
        let commands = offered(&mut window, "version");
        let chooses = commands.iter().any(|name| name == "Keep this version");
        assert_eq!(chooses, access == Access::Edits, "{access:?}: {commands:?}");
    }
}

#[test]
fn someone_who_only_reads_still_finds_copies_and_follows() {
    for access in LESS {
        let mut window = granted(access);
        // Finding, with nothing about replacing.
        window.key("Mod+F").assert_open(Role::Dialog, "Find").type_text("roofer").key("Enter");
        assert!(!has(&mut window, Role::Button, "Show replace"), "{access:?}");
        window.key("Escape").key("Mod+H").assert_closed(Role::Dialog, "Find");
        // What was found is selected, and copied with the key for it.
        window.key("Mod+C");
        let copied = window.platform.clipboard.clone().map(|clip| clip.text);
        assert_eq!(copied.as_deref(), Some("roofer"), "{access:?}");
        // A link is followed as it is by anyone.
        let link = on(&mut window, "manual");
        window.mod_click_at(link);
        let asked = window.ui().take_requests();
        let opens = |request: &Request| matches!(request, Request::OpenLink { href, .. } if href == ADDRESS);
        assert!(asked.iter().any(opens), "{access:?}: {asked:?}");
    }
}

#[test]
fn what_was_out_to_change_the_document_goes_with_the_right_to() {
    type Opens = fn(&mut Harness);
    let cases: [(Role, &str, Opens); 6] = [
        (Role::Dialog, "Link", |window| _ = window.select("brown").key("Mod+K")),
        (Role::Complementary, "Styles", |window| _ = window.click(Role::Button, "Styles")),
        (Role::Complementary, "History", |window| _ = window.click(Role::Button, "History")),
        (Role::Dialog, "Callout", |window| {
            window.select("Mind").key("Mod+Shift+P").type_text("edit callout").key("Enter");
        }),
        (Role::TextInput, "Document name", |window| _ = window.click(Role::Button, "Roof plan")),
        (Role::TextInput, "Replace with", |window| _ = window.key("Mod+H")),
    ];
    for (role, name, opens) in cases {
        let mut window = plan();
        opens(&mut window);
        window.assert_open(role, name);
        window.ui().set_access(Access::Reads);
        window.assert_closed(role, name);
        assert!(has(&mut window, Role::Document, "Roof plan") || page(&mut window).height > 0.0, "the page shows");
    }
}

#[test]
fn print_view_is_looked_at_and_not_set_up() {
    for access in ALL {
        let mut window = granted(access);
        window.key("Mod+P");
        let writes = access == Access::Edits;
        let back = if writes { "Back to editing" } else { "Back to the document" };
        assert!(has(&mut window, Role::Button, back), "{access:?}: {back}");
        let setup = |found: &Found| found.role == Role::Button && found.description.as_deref() == Some("Page setup");
        assert_eq!(window.controls().iter().any(setup), writes, "{access:?}: the page's setup");
        assert!(has(&mut window, Role::Button, "Print"), "{access:?}");
        // The pointer between two blocks of a sheet is offered a page break there.
        let sheet = |found: &Found| found.role == Role::Image && found.name.starts_with("Page ");
        let sheet = window.controls().into_iter().find(sheet).expect("a sheet").bounds;
        window.hover_at(Point { x: sheet.x + sheet.width / 2.0, y: sheet.y + sheet.height * 0.3 });
        assert_eq!(has(&mut window, Role::Button, "Page break here"), writes, "{access:?}");
    }
    // The right to write can go while the view is open.
    let mut window = plan();
    window.key("Mod+P").ui().set_access(Access::Reads);
    assert!(has(&mut window, Role::Button, "Back to the document"), "the view stays, with nothing to set up");
}

#[test]
fn saying_the_same_again_asks_for_no_frame() {
    let mut window = granted(Access::Comments);
    assert!(!window.ui().needs_paint(), "the window is at rest");
    window.ui().set_access(Access::Comments);
    assert!(!window.ui().needs_paint(), "the same again changes nothing");
    window.ui().set_access(Access::Reads);
    assert!(window.ui().needs_paint(), "another role does");
}
