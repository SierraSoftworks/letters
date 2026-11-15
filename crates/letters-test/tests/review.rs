//! Conflict review: the banner that counts the blocks two writers changed apart and steps through them, the
//! page split at the one that is open, the two versions trading places, and the choice of the one on the page.

use accesskit::{Action, ActionRequest, TreeId};
use letters_model::{Block, BlockKind, Document, Note, NoteKind, Notes, Pos, Selection, Stood, Version};
use letters_render::{Point, Rect};
use letters_test::{Found, Harness, Role};
use letters_ui::Mode;

/// A Sunday, 15 November 2026, at twenty past two in the morning, in seconds since 1970.
const NOW: u64 = 1_794_709_230;

/// What the page reads of the plan and of the roof, and what Alice's versions of them read.
const PLAN: [&str; 2] = [
    "The cottage needs three jobs done before the summer lets begin.",
    "The cottage needs two jobs done before the summer lets begin.",
];
const ROOF: [&str; 2] = [
    "Harland Roofing can start on 10 March, and the scaffold stays up for three weeks.",
    "Harland Roofing can start once the scaffold is up, and it stays up until the chimney is repointed.",
];
const DATES: &str = "Alice is confirming dates with the scaffolders this week.";

fn para(id: &str, text: &str) -> Block {
    Block::text(id, BlockKind::Paragraph, text)
}

/// A conflict about a block of the letter: what Alice made of it on Tuesday, `None` if she took it out,
/// held back for what Sam wrote since, which the page shows.
fn conflict(block: &str, held: Option<&str>, page: Option<&str>) -> Note {
    let by = |author: &str, edited: &str, text: Option<&str>| Version {
        author: author.into(),
        edited: edited.into(),
        block: text.map(|text| para(block, text)),
        extra: Default::default(),
    };
    Note::conflict(
        block,
        [by("Alice Moreau", "2026-11-10T08:00:00Z", held), by("Sam Okafor", "2026-11-15T01:41:00Z", page)],
    )
}

/// The conflicts about the plan and about the roof, in that order down the page.
fn both() -> Vec<Note> {
    vec![conflict("roof", Some(ROOF[1]), Some(ROOF[0])), conflict("plan", Some(PLAN[1]), Some(PLAN[0]))]
}

/// The letter that Sam has open on that Sunday, with `notes` on it.
fn letter(notes: Vec<Note>) -> Harness {
    written(ROOF[0], notes)
}

/// The letter with `notes` on it, where the page reads `roof` of the roof.
fn written(roof: &str, notes: Vec<Note>) -> Harness {
    let blocks = vec![
        Block::text("title", BlockKind::Heading { level: 1 }, "Cottage renovation"),
        para("plan", PLAN[0]),
        para("roof", roof),
        para("dates", DATES),
    ];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.ui().set_author("Sam Okafor");
    window.platform.wall = Some(NOW);
    window.ui().set_notes(Notes { notes, ..Notes::default() });
    window
}

/// The letter with the conflict about the roof open, which is the second of its two.
fn reviewing() -> Harness {
    let mut window = letter(both());
    window.click(Role::Button, "Previous conflict");
    window
}

/// What the banner over the page says, while there is one.
fn banner(window: &mut Harness) -> Option<String> {
    let counts = |found: &Found| found.role == Role::Alert && found.name.contains("changed in two places");
    window.controls().into_iter().find(counts).map(|found| found.name)
}

/// The version that is held back, while a conflict is open: its value is what it reads.
fn held_back(window: &mut Harness) -> Option<Found> {
    window.controls().into_iter().find(|found| found.role == Role::Note && found.name.starts_with("Version held back"))
}

/// The buttons that choose a version: the one on the page, then the one held back.
fn choices(window: &mut Harness) -> [Option<Found>; 2] {
    ["Keep this version", "Use this version"].map(|name| window.find(Role::Button, name))
}

/// Whose the version on the page is said to be and whose the one held back, then what each reads.
fn versions(window: &mut Harness, block: &str) -> [(String, String); 2] {
    let whose = |told: Option<String>| {
        let told = told.unwrap_or_default();
        ["Sam Okafor", "Alice Moreau"].into_iter().find(|name| told.contains(name)).unwrap_or_default().to_owned()
    };
    let [keep, used] = choices(window).map(|choice| whose(choice.and_then(|choice| choice.description)));
    let held = held_back(window).and_then(|held| held.value).unwrap_or_default();
    [(keep, reads(window, block).unwrap_or_default()), (used, held)]
}

/// The two versions of the roof as the conflict has them when it opens: Sam's on the page, Alice's held back.
fn roofs() -> [(String, String); 2] {
    [("Sam Okafor".to_owned(), ROOF[0].to_owned()), ("Alice Moreau".to_owned(), ROOF[1].to_owned())]
}

/// What a block of the page reads, or `None` once the page does not have it.
fn reads(window: &Harness, block: &str) -> Option<String> {
    window.editor().doc().block(&block.into()).map(Block::plain_text)
}

/// How many notes still wait.
fn waiting(window: &mut Harness) -> usize {
    window.ui().notes().notes.iter().filter(|note| !note.resolved).count()
}

/// The part of the window the page is in.
fn area(window: &mut Harness) -> Rect {
    window.controls().into_iter().find(|found| found.role == Role::Document).expect("a page").bounds
}

/// Where the caret shows at a place in a block's text. The caret is left there.
fn caret_at(window: &mut Harness, block: &str, offset: usize) -> Rect {
    window.ui().select(Selection::caret(Pos::new(block, offset)));
    window.paint();
    window.ui().caret_rect().expect("the page shows a caret")
}

/// Whether `inner` shows whole between the left and right edges of `outer`.
fn across(inner: Rect, outer: Rect) -> bool {
    inner.x >= outer.x && inner.x + inner.width <= outer.x + outer.width
}

/// A way of doing something to a window, and one of asking for what a button of that name does.
type Open = fn(&mut Harness);
type Way = fn(&mut Harness, &str);

/// Presses the button of that name with the pointer, as assistive technology does, or takes the command of
/// that name from the palette.
const WAYS: [(&str, Way); 3] = [
    ("a press", |window, name| {
        window.click(Role::Button, name);
    }),
    ("assistive technology", |window, name| {
        let tree = window.ui().access();
        let named = |node: &accesskit::Node| node.role() == Role::Button && node.label() == Some(name);
        let found = tree.nodes.iter().find(|(_, node)| named(node));
        let target_node = found.unwrap_or_else(|| panic!("a button {name}")).0;
        window.ask(ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node, data: None });
    }),
    ("the palette", |window, name| {
        // The palette names the version that the caret is not in from where the caret is.
        let command = if name == "Use this version" { "Use the other version" } else { name };
        window.key("Mod+Shift+P").type_text(command).key("Enter");
    }),
];

#[test]
fn a_banner_counts_the_conflicts_that_wait_and_steps_through_them_down_the_page() {
    assert_eq!(banner(&mut letter(Vec::new())), None, "a letter without conflicts has no banner");
    let settled = both().into_iter().map(|note| Note { resolved: true, ..note }).collect();
    assert_eq!(banner(&mut letter(settled)), None, "nor has one whose conflicts are settled");

    let mut window = letter(both());
    assert!(banner(&mut window).is_some_and(|said| said.starts_with("2 passages")), "it counts them");
    assert_eq!(held_back(&mut window), None, "none is open until it is asked for");
    // A step, the version it shows held back, and where the banner says that conflict stands.
    let steps = [
        ("Next conflict", PLAN[1], "1 of 2"),
        ("Next conflict", ROOF[1], "2 of 2"),
        ("Next conflict", PLAN[1], "1 of 2"),
        ("Previous conflict", ROOF[1], "2 of 2"),
    ];
    for (step, held, stands) in steps {
        window.click(Role::Button, step);
        let shown = held_back(&mut window).and_then(|held| held.value);
        assert_eq!(shown.as_deref(), Some(held), "{step}");
        assert!(window.find(Role::Label, stands).is_some(), "{step}: the banner says {stands}");
        window.assert_closed(Role::Dialog, "Conflict");
    }
}

#[test]
fn a_shell_is_told_while_a_conflict_is_open() {
    assert_eq!(letter(both()).ui().mode(), Mode::Editor);
    assert_eq!(reviewing().ui().mode(), Mode::Review);
}

#[test]
fn a_conflict_opens_in_the_page_with_both_versions_and_whose_they_are() {
    let ways: [(&str, Open); 2] = [
        ("its marker", |window| {
            let roof = caret_at(window, "roof", 0);
            let markers = window.controls().into_iter().filter(|found| found.name == "1 conflict");
            let beside = markers.min_by(|a, b| (a.center().y - roof.y).abs().total_cmp(&(b.center().y - roof.y).abs()));
            window.click_at(beside.expect("a marker beside the roof").center());
        }),
        ("the Notes panel", |window| {
            window.click(Role::Button, "Notes");
            let about = |row: &Found| row.description.as_deref().is_some_and(|told| told.contains("Harland"));
            let row = |found: &Found| found.role == Role::ListBoxOption && about(found);
            let roof = window.controls().into_iter().find(row).expect("a row for the roof");
            window.click_at(roof.center());
        }),
    ];
    for (name, open) in ways {
        let mut window = letter(both());
        open(&mut window);
        window.assert_closed(Role::Dialog, "Conflict");
        let held = held_back(&mut window).unwrap_or_else(|| panic!("by {name}: the version held back shows"));
        assert!(held.name.contains("Alice Moreau"), "by {name}: it says whose it is: {}", held.name);
        assert_eq!(held.value.as_deref(), Some(ROOF[1]), "by {name}");
        let [keep, used] = choices(&mut window).map(|choice| choice.unwrap_or_else(|| panic!("by {name}: a choice")));
        let told = [keep.description.unwrap_or_default(), used.description.unwrap_or_default()];
        assert!(told[0].contains("Sam Okafor") && told[0].contains("on the page"), "by {name}: {told:?}");
        assert!(told[1].contains("Alice Moreau") && told[1].contains("held back"), "by {name}: {told:?}");
        assert_eq!(reads(&window, "roof").as_deref(), Some(ROOF[0]), "by {name}: the page reads as it did");
    }
}

#[test]
fn both_versions_show_side_by_side_while_the_window_has_room_and_stacked_when_it_has_not() {
    // How wide the window is, whether the versions stand side by side, and whether both show whole.
    let cases = [(1440.0, true, true), (1000.0, true, false), (640.0, false, true)];
    for (width, beside, whole) in cases {
        let mut window = letter(both());
        window.resize(width, 800.0);
        let lines = |window: &mut Harness| caret_at(window, "roof", ROOF[0].len()).y - caret_at(window, "roof", 0).y;
        let before = lines(&mut window);
        window.click(Role::Button, "Previous conflict");

        let area = area(&mut window);
        let held = held_back(&mut window).expect("the version held back").bounds;
        let [keep, used] = choices(&mut window).map(|choice| choice.expect("a choice").bounds);
        assert_eq!(keep.y == used.y && used.x < keep.x, beside, "{width}: {used:?}, {keep:?}");
        assert_eq!(keep.y > used.y, !beside, "{width}: the version held back is the upper one of a stack");
        assert!(across(keep, area), "{width}: the version on the page shows whole: {keep:?}");
        assert_eq!(across(held, area), whole, "{width}: {held:?} in {area:?}");
        // The page's version stays text of the page: it is written where it shows, and wraps as it did.
        let (start, end) = (caret_at(&mut window, "roof", 0), caret_at(&mut window, "roof", ROOF[0].len()));
        assert_eq!(end.y - start.y, before, "{width}: its lines break as they did");
        assert!(start.y > keep.y && across(start, area), "{width}: under its head line: {start:?}, {keep:?}");
        if beside {
            assert!(start.x > held.x + held.width, "{width}: beside the version held back: {start:?}, {held:?}");
        }
        let after = caret_at(&mut window, "dates", 0);
        assert!(after.y > held.y + held.height && after.y > end.y, "{width}: the rest of the page is below both");
    }
}

#[test]
fn a_version_the_window_cuts_off_comes_across_when_pressed_and_gives_way_to_the_caret() {
    let mut window = reviewing();
    window.resize(1000.0, 800.0);
    let shows = |window: &mut Harness| {
        let (area, held) = (area(window), held_back(window).expect("the version held back").bounds);
        let keep = choices(window)[0].clone().expect("a choice").bounds;
        [across(held, area), across(keep, area)]
    };
    assert_eq!(shows(&mut window), [false, true], "the page's version shows whole at first");

    // A press on what shows of the version held back.
    let (area, held) = (area(&mut window), held_back(&mut window).expect("the version held back").bounds);
    window.click_at(Point { x: area.x + (held.x + held.width - area.x) / 2.0, y: held.y + held.height / 2.0 });
    window.advance(16).advance(400);
    assert_eq!(shows(&mut window), [true, false], "the pair slid across");
    assert_eq!(reads(&window, "roof").as_deref(), Some(ROOF[0]), "and the press chose nothing");

    caret_at(&mut window, "roof", 4);
    window.advance(16).advance(400);
    assert_eq!(shows(&mut window), [false, true], "the caret is in the page's version, which comes back");
}

#[test]
fn keeping_the_version_on_the_page_settles_the_conflict_and_changes_no_text() {
    for (name, ask) in WAYS {
        let mut window = reviewing();
        let edits = window.ui().edits();
        ask(&mut window, "Keep this version");
        assert_eq!(reads(&window, "roof").as_deref(), Some(ROOF[0]), "by {name}");
        assert_eq!(waiting(&mut window), 1, "by {name}: the conflict is settled");
        assert!(window.ui().edits() > edits, "by {name}: the choice is something to save");
        assert_eq!(held_back(&mut window), None, "by {name}: the page is whole again");
        assert!(banner(&mut window).is_some_and(|said| said.starts_with("1 passage ")), "by {name}: one still waits");
    }
}

#[test]
fn using_the_version_held_back_trades_the_two_and_leaves_the_conflict_open() {
    for (name, ask) in WAYS {
        let mut window = reviewing();
        let (edits, [sams, alices]) = (window.ui().edits(), roofs());
        assert_eq!(versions(&mut window, "roof"), [sams.clone(), alices.clone()], "by {name}: as it opens");
        ask(&mut window, "Use this version");
        assert_eq!(versions(&mut window, "roof"), [alices.clone(), sams.clone()], "by {name}: the two traded places");
        assert_eq!(reads(&window, "plan").as_deref(), Some(PLAN[0]), "by {name}: the rest reads as it did");
        assert_eq!(waiting(&mut window), 2, "by {name}: the conflict is still to be settled");
        assert!(window.ui().edits() > edits, "by {name}: the page changed, which is something to save");

        // One step back takes the trade back, and using the other again trades them again.
        window.key("Mod+Z");
        assert_eq!(versions(&mut window, "roof"), [sams.clone(), alices.clone()], "by {name}: one step back");
        ask(&mut window, "Use this version");
        ask(&mut window, "Use this version");
        assert_eq!(versions(&mut window, "roof"), [sams, alices], "by {name}: twice is as it was");
    }
}

#[test]
fn the_notes_panel_says_which_version_of_each_conflict_the_page_shows() {
    let mut window = reviewing();
    window.click(Role::Button, "Notes");
    // How many of the listed conflicts say that the page shows that version of theirs.
    let showing = |window: &mut Harness, which: &str| {
        let says = format!("The page shows the {which} version.");
        let rows = window.controls().into_iter().filter(|found| found.role == Role::ListBoxOption);
        rows.filter(|row| row.name.ends_with(&says)).count()
    };
    assert_eq!((showing(&mut window, "later"), showing(&mut window, "earlier")), (2, 0));
    window.click(Role::Button, "Use this version");
    assert_eq!((showing(&mut window, "later"), showing(&mut window, "earlier")), (1, 1), "the roof's traded");
}

#[test]
fn either_version_is_written_on_in_the_page_before_one_is_kept() {
    let mut window = reviewing();
    // Sam mends his own version, then has Alice's on the page and writes on that.
    window.select("three weeks").type_text("four weeks");
    let sams = ROOF[0].replace("three weeks", "four weeks");
    window.click(Role::Button, "Use this version");
    window.select("chimney").type_text("north chimney");
    let alices = ROOF[1].replace("chimney", "north chimney");
    let now = versions(&mut window, "roof");
    assert_eq!(now, [("Alice Moreau".into(), alices.clone()), ("Sam Okafor".into(), sams.clone())]);

    // What was written on a version stays with it while the other is on the page.
    window.click(Role::Button, "Use this version");
    assert_eq!(versions(&mut window, "roof"), [("Sam Okafor".into(), sams), ("Alice Moreau".into(), alices.clone())]);
    window.click(Role::Button, "Use this version").click(Role::Button, "Keep this version");
    assert_eq!(reads(&window, "roof"), Some(alices), "the version on the page is the one kept");
    assert_eq!((waiting(&mut window), held_back(&mut window)), (1, None), "which settles the conflict");
}

#[test]
fn a_conflict_that_is_opened_again_shows_its_versions_where_they_were_left() {
    let mut window = reviewing();
    window.select("three weeks").type_text("four weeks");
    let sams = ("Sam Okafor".to_owned(), ROOF[0].replace("three weeks", "four weeks"));
    window.click(Role::Button, "Use this version");
    let left = [("Alice Moreau".to_owned(), ROOF[1].to_owned()), sams];
    assert_eq!(versions(&mut window, "roof"), left);

    let ways: [(&str, Open); 2] = [
        ("another conflict", |window| _ = window.click(Role::Button, "Next conflict")),
        ("closing", |window| _ = window.click(Role::Button, "Close conflict")),
    ];
    for (name, leave) in ways {
        leave(&mut window);
        assert_ne!(versions(&mut window, "roof"), left, "by {name}: the roof's versions are put away");
        window.click(Role::Button, "Previous conflict");
        assert_eq!(versions(&mut window, "roof"), left, "by {name}");
    }
    // A trade that is taken back while the conflict is put away is taken back where it shows again.
    window.click(Role::Button, "Close conflict").key("Mod+Z").click(Role::Button, "Previous conflict");
    assert_eq!(versions(&mut window, "roof"), [left[1].clone(), roofs()[1].clone()]);
}

#[test]
fn a_page_that_reads_as_the_earlier_version_has_the_later_one_held_back() {
    // What the page reads of the roof when the conflict is opened, as after a trade made elsewhere or
    // before the letter was last closed, and which of the two versions the page then stands for.
    let cases = [
        (ROOF[0].to_owned(), 0),
        (ROOF[0].replace("three weeks", "four weeks"), 0),
        (ROOF[1].to_owned(), 1),
        (ROOF[1].replace("chimney", "north chimney"), 1),
    ];
    for (page, stands) in cases {
        let mut window = written(&page, both());
        window.click(Role::Button, "Previous conflict");
        let (on_page, held) = (roofs()[stands].0.clone(), roofs()[1 - stands].clone());
        assert_eq!(versions(&mut window, "roof"), [(on_page, page.clone()), held], "{page}");
    }
}

#[test]
fn a_conflict_is_put_away_without_a_choice() {
    let ways: [(&str, Open); 3] = [
        ("Escape", |window| {
            window.key("Escape");
        }),
        ("the banner", |window| {
            window.click(Role::Button, "Close conflict");
        }),
        ("the palette", |window| {
            window.key("Mod+Shift+P").type_text("Close conflict").key("Enter");
        }),
    ];
    for (name, leave) in ways {
        let mut window = reviewing();
        let (edits, markers) = (window.ui().edits(), |window: &mut Harness| {
            window.controls().into_iter().filter(|found| found.name == "1 conflict").count()
        });
        assert_eq!(markers(&mut window), 1, "the two versions stand where the open conflict's marker did");
        leave(&mut window);
        assert_eq!(held_back(&mut window), None, "by {name}: the page is whole again");
        assert_eq!(choices(&mut window), [None, None], "by {name}");
        assert_eq!((waiting(&mut window), markers(&mut window)), (2, 2), "by {name}: both conflicts still wait");
        assert!(banner(&mut window).is_some_and(|said| said.starts_with("2 passages")), "by {name}");
        assert_eq!(reads(&window, "roof").as_deref(), Some(ROOF[0]), "by {name}");
        assert_eq!(window.ui().edits(), edits, "by {name}: there is nothing to save");
    }
}

#[test]
fn the_version_on_the_page_is_written_on_while_both_show() {
    let mut window = reviewing();
    // A press in its text puts the caret there.
    let spot = caret_at(&mut window, "roof", 8);
    caret_at(&mut window, "title", 0);
    window.click_at(Point { x: spot.x + 1.0, y: spot.y + spot.height / 2.0 });
    let caret = window.editor().selection().map(|selection| selection.head.clone());
    assert_eq!(caret, Some(Pos::new("roof", 8)));

    window.type_text("only ");
    let written = reads(&window, "roof").unwrap_or_default();
    assert_eq!(written, ROOF[0].replacen("Harland ", "Harland only ", 1));
    let held = held_back(&mut window).and_then(|held| held.value);
    assert_eq!(held.as_deref(), Some(ROOF[1]), "the conflict stays open, and what it holds back as it was");

    window.click(Role::Button, "Keep this version");
    assert_eq!(reads(&window, "roof"), Some(written), "what was written is what is kept");
    assert_eq!(waiting(&mut window), 1);
}

#[test]
fn a_press_that_ends_off_its_choice_chooses_nothing() {
    for (pressed, released) in [(0, 1), (1, 0)] {
        let mut window = reviewing();
        let at = choices(&mut window).map(|choice| choice.expect("a choice").center());
        window.press_at(at[pressed]).release_at(at[released]);
        assert_eq!(reads(&window, "roof").as_deref(), Some(ROOF[0]));
        assert_eq!(waiting(&mut window), 2, "both conflicts still wait");
        assert!(held_back(&mut window).is_some(), "and the one that was open still is");
    }
}

#[test]
fn a_document_whose_notes_cannot_be_changed_shows_both_versions_and_offers_no_choice() {
    let mut window = letter(both());
    window.ui().refuse_comments(Some("This copy is read only."));
    window.click(Role::Button, "Previous conflict");
    assert_eq!(held_back(&mut window).and_then(|held| held.value).as_deref(), Some(ROOF[1]));
    assert_eq!(choices(&mut window), [None, None]);
    window.key("Mod+Shift+P").type_text("version");
    let offered = |found: &Found| found.role == Role::ListBoxOption && found.name.contains("version");
    assert!(!window.controls().iter().any(offered), "nor does the palette");
}

#[test]
fn a_version_that_took_the_block_out_says_so_and_using_it_takes_the_block_out() {
    let mut window = letter(vec![conflict("dates", None, Some(DATES))]);
    assert!(banner(&mut window).is_some_and(|said| said.starts_with("1 passage ")));
    window.click(Role::Button, "Next conflict");
    let held = held_back(&mut window).and_then(|held| held.value).unwrap_or_default();
    assert!(held.starts_with("Not in"), "the version held back has nothing of the block: {held}");
    assert_eq!(reads(&window, "dates").as_deref(), Some(DATES), "the page keeps it meanwhile");

    window.click(Role::Button, "Use this version");
    assert_eq!(reads(&window, "dates"), None);
    assert_eq!((waiting(&mut window), banner(&mut window)), (0, None));
    window.key("Mod+Z");
    assert_eq!(reads(&window, "dates").as_deref(), Some(DATES), "one step back");
}

#[test]
fn a_conflict_about_a_block_the_page_no_longer_has_is_read_and_resolved_as_a_note() {
    let mut window = letter(vec![conflict("gone", Some("Skips are booked."), None)]);
    assert_eq!(banner(&mut window), None, "there are no two versions to step to");
    window.click(Role::Button, "Notes");
    let row = window.controls().into_iter().find(|found| found.role == Role::ListBoxOption).expect("it is listed");
    window.click_at(row.center()).assert_open(Role::Dialog, "Conflict");
    let whose = |text: &str| text.contains("Sam Okafor") && text.contains("Alice Moreau");
    let says = |found: &Found| whose(&found.name) || found.value.as_deref().is_some_and(whose);
    assert!(window.controls().iter().any(says), "it says whose changes met");

    window.click(Role::Button, "Resolve").assert_closed(Role::Dialog, "Conflict");
    assert_eq!(waiting(&mut window), 0);
}

/// What Alice made of a paragraph about skips, which Sam then took out.
const SKIPS: &str = "Two skips are booked for the first week of March.";

/// A conflict about that paragraph, which stood behind the block `after`: the note knows unless `after`
/// is `None`.
fn skipped(after: Option<&str>) -> Note {
    let mut note = conflict("skips", Some(SKIPS), None);
    note.anchor.stood = after.map(|after| Stood { after: Some(after.into()), ..Stood::default() });
    // Alice's copy knew the paragraph by another id, as copies do of blocks that share one.
    if let NoteKind::Conflict { versions } = &mut note.kind {
        versions[0].block = Some(para("skips~2", SKIPS));
    }
    note
}

/// The letter with that conflict open as a note.
fn taken_out(after: Option<&str>) -> Harness {
    let mut window = letter(vec![skipped(after)]);
    read_conflict(&mut window);
    window
}

/// Opens the one note from the Notes panel.
fn read_conflict(window: &mut Harness) {
    window.click(Role::Button, "Notes");
    let row = window.controls().into_iter().find(|found| found.role == Role::ListBoxOption).expect("it is listed");
    window.click_at(row.center()).assert_open(Role::Dialog, "Conflict");
}

#[test]
fn a_block_the_later_writer_took_out_is_put_back_where_it_stood_with_both_versions_to_choose_from() {
    let mut window = taken_out(Some("plan"));
    let before = window.markdown();
    let told =
        |found: &Found| [Some(&found.name), found.value.as_ref()].into_iter().flatten().any(|said| said == SKIPS);
    assert!(window.controls().iter().any(told), "the note says how the version reads that still has it");

    window.click(Role::Button, "Use this version").assert_closed(Role::Dialog, "Conflict");
    let order: Vec<&str> = window.editor().doc().blocks.iter().map(|block| block.id.as_str()).collect();
    assert_eq!(order, ["title", "plan", "skips", "roof", "dates"], "it is back behind the plan");
    assert_eq!(reads(&window, "skips").as_deref(), Some(SKIPS));
    // The conflict is still open, now on the page: the version that took the block out is held back.
    assert_eq!(waiting(&mut window), 1);
    assert!(held_back(&mut window).is_some() && choices(&mut window).iter().all(Option::is_some));
    window.key("Mod+Z");
    assert_eq!(window.markdown(), before, "one step back");
}

#[test]
fn a_block_that_was_put_back_stays_or_goes_by_the_version_that_is_chosen() {
    // The choice, and what the page reads of the paragraph once it is made.
    for (choice, left) in [("Keep this version", Some(SKIPS)), ("Use this version", None)] {
        let mut window = taken_out(Some("plan"));
        window.click(Role::Button, "Use this version").click(Role::Button, choice);
        assert_eq!((reads(&window, "skips").as_deref(), waiting(&mut window)), (left, 0), "{choice}");
    }
}

#[test]
fn a_block_is_only_offered_to_be_put_back_where_its_place_is_known_and_a_choice_can_be_kept() {
    // The block it stood behind, if the note knows, and why this copy's notes cannot be changed, if so.
    for (after, refused) in [(None, None), (Some("plan"), Some("This copy is read only."))] {
        let mut window = taken_out(after);
        window.ui().refuse_comments(refused);
        assert_eq!(window.find(Role::Button, "Use this version"), None, "{after:?} {refused:?}");
        assert!(window.find(Role::Dialog, "Conflict").is_some(), "the note is still read");
    }
}

#[test]
fn opening_a_conflict_far_down_the_page_brings_both_versions_into_view() {
    let lines = || (0..80).map(|line| para(&format!("l{line}"), "A line.")).collect::<Vec<Block>>();
    // The block the conflict is about, with the page and the note as they are before it opens, and how
    // it is opened: stepped to, and by putting its block back from the note.
    let stepped: (_, _, _, Open) =
        ("roof", [lines(), vec![para("roof", ROOF[0])]].concat(), both().remove(0), |window| {
            window.click(Role::Button, "Next conflict");
        });
    let put_back: (_, _, _, Open) = ("skips", lines(), skipped(Some("l79")), |window| {
        read_conflict(window);
        window.click(Role::Button, "Use this version");
    });
    for (block, blocks, note, open) in [stepped, put_back] {
        let mut window = Harness::open(Document { blocks, ..Document::default() });
        window.ui().set_notes(Notes { notes: vec![note], ..Notes::default() });
        open(&mut window);

        let area = area(&mut window);
        let within = |rect: Rect| rect.y >= area.y && rect.y + rect.height <= area.y + area.height;
        let held = held_back(&mut window).expect("the version held back").bounds;
        let [keep, used] = choices(&mut window).map(|choice| choice.expect("a choice").bounds);
        assert!(within(held) && within(keep) && within(used), "{block}: {held:?}, {keep:?} and {used:?} in {area:?}");
        let end = reads(&window, block).map_or(0, |text| text.len());
        let caret = caret_at(&mut window, block, end);
        assert!(within(caret), "the page's version of {block} too: {caret:?}");
    }
}
