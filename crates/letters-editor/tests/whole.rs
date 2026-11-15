mod common;

use common::{Harness, canon, png};
use letters_editor::{Command, Picture, PreviewEdit};
use letters_model::{Block, BlockId, BlockKind, Pos, Selection};

const BARE: &str = "card https://a.example";
const FILLED: &str = "card https://a.example ~ A page ~ About it ~ assets/old.png";

fn said(text: &str) -> Option<String> {
    Some(text.to_owned())
}

/// A change to the card that is the second block of a document.
fn card(edit: PreviewEdit) -> Command {
    Command::Preview { block: "b2".into(), edit }
}

/// The link previews of a document, as what each is made of.
fn cards(editor: &Harness) -> Vec<BlockKind> {
    let preview = |block: &Block| matches!(block.kind, BlockKind::LinkPreview { .. }).then(|| block.kind.clone());
    editor.editor.doc().walk().filter_map(preview).collect()
}

/// The asset that the one card of a document shows.
fn image(editor: &Harness) -> Option<String> {
    match cards(editor).as_slice() {
        [BlockKind::LinkPreview { image, .. }] => image.clone(),
        other => panic!("one card, not {other:?}"),
    }
}

#[test]
fn a_table_of_contents_and_a_link_preview_go_in_as_a_divider_does() {
    let preview = |url: &str| Command::InsertPreview(url.into());
    let cases = [
        // Below the caret's block, with the caret in the text that follows.
        ("p one|\np two", Command::InsertContents, "p one\ntoc:3\np |two"),
        ("p one|\np two", preview("https://a.example"), "p one\ncard https://a.example\np |two"),
        // A line is made to write on where no text follows, and an empty line gives way.
        ("p one|", Command::InsertContents, "p one\ntoc:3\np |"),
        ("p one\np |", preview("https://a.example"), "p one\ncard https://a.example\np |"),
        // It follows the table that the caret is in a cell of, and stays in the quote the caret is in.
        ("r1> td on|e\np two", Command::InsertContents, "r1> td one\ntoc:3\np |two"),
        ("> p one|", Command::InsertContents, "> p one\n> toc:3\n> p |"),
        // A document with no text yet starts with it.
        ("", Command::InsertContents, "toc:3\np |"),
        // An address is kept on one line, with no space around it.
        ("p |", preview("  https://a.example\n"), "card https://a.example\np |"),
    ];
    for (before, command, after) in cases {
        let mut editor = Harness::new(before);
        assert!(editor.editor.applies(&command), "{command:?} from {before:?}");
        editor.run(command.clone());
        assert_eq!(editor.state(), canon(after), "{command:?} from {before:?}");
        editor.key("Mod+z");
        assert_eq!(editor.state(), canon(before), "undoing {command:?} from {before:?}");
    }
}

#[test]
fn an_address_with_nothing_in_it_makes_no_card() {
    for url in ["", "  ", "\n"] {
        let mut editor = Harness::new("p one|");
        let command = Command::InsertPreview(url.into());
        assert!(!editor.editor.applies(&command), "{url:?}");
        editor.run(command);
        assert_eq!(editor.state(), canon("p one|"), "{url:?}");
    }
}

#[test]
fn the_caret_steps_over_a_block_with_no_text_and_what_deletes_beside_it_takes_it_whole() {
    for whole in ["toc:3", FILLED] {
        let around = |one: &str, two: &str| format!("p {one}\n{whole}\np {two}");
        // The key, the document before and after, and whether the key changed the document.
        let cases = [
            ("Right", around("one|", "two"), around("one", "|two"), false),
            ("Left", around("one", "|two"), around("one|", "two"), false),
            ("Down", around("on|e", "two"), around("one", "tw|o"), false),
            ("Backspace", around("one", "|two"), "p one\np |two".to_owned(), true),
            ("Delete", around("one|", "two"), "p one|\np two".to_owned(), true),
            // A selection across it takes it along.
            ("Backspace", around("o^ne", "t|wo"), "p o|wo".to_owned(), true),
            // The document does not end in it for the caret: an arrow makes a line past it.
            ("Down", format!("p on|e\n{whole}"), format!("p one\n{whole}\np |"), true),
        ];
        for (key, before, after, edits) in cases {
            let mut editor = Harness::new(&before);
            editor.key(key);
            assert_eq!(editor.state(), canon(&after), "{key} from {before:?}");
            if edits {
                editor.key("Mod+z");
                assert_eq!(editor.state(), canon(&before), "undoing {key} from {before:?}");
            }
        }
    }
}

#[test]
fn a_table_of_contents_lists_headings_down_to_the_level_it_is_told() {
    let before = "p one|\ntoc:3\nh2 two";
    // The level asked for, and whether it is taken: it is from 1 to 4, and not what the table has.
    for (levels, taken) in [(1, true), (2, true), (4, true), (3, false), (0, false), (5, false)] {
        let mut editor = Harness::new(before);
        let command = Command::SetContentsLevels { table: "b2".into(), levels };
        assert_eq!(editor.editor.applies(&command), taken, "{levels} levels");
        editor.run(command);
        let after = if taken { format!("p one|\ntoc:{levels}\nh2 two") } else { before.to_owned() };
        assert_eq!(editor.state(), canon(&after), "{levels} levels");
        editor.key("Mod+z");
        assert_eq!(editor.state(), canon(before), "{levels} levels are one undo step");
    }
    // Only a table of contents has levels.
    let editor = Harness::new(before);
    for block in ["b1", "b3", "gone"] {
        let command = Command::SetContentsLevels { table: block.into(), levels: 2 };
        assert!(!editor.editor.applies(&command), "{block}");
    }
}

#[test]
fn a_link_preview_takes_what_its_inspector_and_a_fetch_say() {
    let fetched = |url: &str, title: Option<&str>, description: Option<&str>| PreviewEdit::Fetched {
        url: url.to_owned(),
        title: title.map(str::to_owned),
        description: description.map(str::to_owned),
        picture: None,
    };
    let found = |title, description| fetched("https://a.example", title, description);
    // The card, the change, and the card after it: `None` for a change that is not taken.
    let cases = [
        (BARE, PreviewEdit::Url("https://b.example".into()), Some("card https://b.example")),
        (BARE, PreviewEdit::Url(" https://b.example\n".into()), Some("card https://b.example")),
        (BARE, PreviewEdit::Title(said("A page")), Some("card https://a.example ~ A page")),
        (BARE, PreviewEdit::Title(said("A\npage")), Some("card https://a.example ~ A page")),
        (BARE, PreviewEdit::Description(said("About it")), Some("card https://a.example ~ - ~ About it")),
        // The address alone changes: the card keeps what it shows.
        (
            FILLED,
            PreviewEdit::Url("https://b.example".into()),
            Some("card https://b.example ~ A page ~ About it ~ assets/old.png"),
        ),
        // What is left empty is none, and so is what is taken away.
        (FILLED, PreviewEdit::Title(said("")), Some("card https://a.example ~ - ~ About it ~ assets/old.png")),
        (FILLED, PreviewEdit::Title(None), Some("card https://a.example ~ - ~ About it ~ assets/old.png")),
        (FILLED, PreviewEdit::Description(said("")), Some("card https://a.example ~ A page ~ - ~ assets/old.png")),
        (FILLED, PreviewEdit::Picture(None), Some("card https://a.example ~ A page ~ About it")),
        // A fetch says all the card shows of the page: what it did not find is gone.
        (BARE, found(Some(" A\n  page "), Some("About\nit")), Some("card https://a.example ~ A page ~ About it")),
        (FILLED, found(Some("Another"), None), Some("card https://a.example ~ Another")),
        (FILLED, found(Some(" "), None), Some("card https://a.example")),
        // Not taken: an address with nothing in it, what the card already says, and what is no picture.
        (FILLED, PreviewEdit::Url(" \n".into()), None),
        (FILLED, PreviewEdit::Title(said("A page")), None),
        (BARE, found(None, None), None),
        // Nor what was fetched for an address that the card has left since.
        (BARE, fetched("https://b.example", Some("A page"), None), None),
        (BARE, PreviewEdit::Picture(Some(Picture { bytes: b"not a picture".to_vec(), ..png(1, 4, 3) })), None),
    ];
    for (before, edit, after) in cases {
        let before = format!("p one|\n{before}");
        let mut editor = Harness::new(&before);
        let command = card(edit);
        assert_eq!(editor.editor.applies(&command), after.is_some(), "{command:?} on {before:?}");
        editor.run(command.clone());
        let after = after.map_or(before.clone(), |after| format!("p one|\n{after}"));
        assert_eq!(editor.state(), canon(&after), "{command:?} on {before:?}");
        editor.key("Mod+z");
        assert_eq!(editor.state(), canon(&before), "{command:?} on {before:?} is one undo step");
    }
    // Only a link preview takes these.
    let editor = Harness::new("p one|\ntoc:3");
    for block in ["b1", "b2", "gone"] {
        let command = Command::Preview { block: block.into(), edit: PreviewEdit::Title(said("A page")) };
        assert!(!editor.editor.applies(&command), "{block}");
    }
}

#[test]
fn a_cards_picture_is_kept_among_the_files_assets() {
    let mut editor = Harness::new(&format!("p one|\n{BARE}"));
    let first = png(7, 400, 300);
    editor.run(card(PreviewEdit::Picture(Some(first.clone()))));
    let kept = image(&editor).expect("the card shows a picture");
    assert!(kept.starts_with("assets/") && kept.ends_with(".png"), "{kept}");
    assert_eq!(editor.platform.assets.get(&kept), Some(&first.bytes), "the shell was handed the bytes");

    // A fetch brings the title, the description and the picture at once, and is undone at once.
    let second = png(8, 400, 300);
    let fetched = |description: Option<&str>, picture: Picture| PreviewEdit::Fetched {
        url: "https://a.example".into(),
        title: said("A page"),
        description: description.map(str::to_owned),
        picture: Some(picture),
    };
    editor.run(card(fetched(Some("About it"), second.clone())));
    let found = image(&editor).expect("the card shows the picture that was found");
    assert_ne!(found, kept, "another picture has another name");
    assert_eq!(editor.platform.assets.get(&found), Some(&second.bytes));
    assert_eq!(editor.state(), canon(&format!("p one|\n{BARE} ~ A page ~ About it ~ {found}")));
    editor.key("Mod+z");
    assert_eq!(editor.state(), canon(&format!("p one|\n{BARE} ~ - ~ - ~ {kept}")));

    // A fetch that found no picture Letters can show leaves the card without one.
    let unshown = Picture { bytes: b"not a picture".to_vec(), ..png(1, 4, 3) };
    editor.run(card(fetched(None, unshown)));
    assert_eq!(editor.state(), canon(&format!("p one|\n{BARE} ~ A page")));
}

#[test]
fn the_caret_goes_to_the_heading_that_a_line_of_the_table_lists() {
    let mut editor = Harness::new("h1 Title|\ntoc:2\nh2 One\np text\nh3 Deep\nh2 Two");
    let table = BlockId::from("b2");
    let lines: Vec<BlockId> = editor.editor.doc().contents(&table).iter().map(|heading| heading.id.clone()).collect();
    assert_eq!(lines.len(), 2, "the table lists the headings after it, down to its level");
    editor.editor.select(Selection::caret(Pos::new(lines[1].clone(), 0)));
    assert_eq!(editor.state(), canon("h1 Title\ntoc:2\nh2 One\np text\nh3 Deep\nh2 |Two"));
}

#[test]
fn a_card_that_is_copied_is_pasted_as_a_card_of_its_own() {
    let mut editor = Harness::new(&format!("p ^one\n{FILLED}\np two|"));
    for key in ["Mod+c", "Right", "Mod+v"] {
        editor.key(key);
    }
    let cards = cards(&editor);
    assert_eq!(cards.len(), 2, "{}", editor.state());
    assert_eq!(cards[0], cards[1], "the copy shows what the card shows");
}
