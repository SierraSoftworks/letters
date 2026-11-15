//! Figures, galleries and tables: blocks made of parts, which editing never breaks or joins to other text.

mod common;

use common::{Do, Do::K, Do::T, Harness, canon, clip};
use letters_editor::Command;
use letters_model::{Block, BlockKind, Body, Document, Model, Pos, ReplicaId, Selection, Tone};

const FIGURE: &str = "p one\nfig Boats\np two";
const GALLERY: &str = "p one\ng> fig\ng> fig\ng> cap At sea\np two";
const TABLE: &str = "p one\nr1> th A\nr1> th B\nr2> td C\nr2> td D\np two";

/// Puts the selection the notation describes into one of the documents above.
fn at(doc: &str, from: &str, to: &str) -> String {
    assert!(doc.contains(from), "{doc:?} has no {from:?}");
    doc.replacen(from, to, 1)
}

/// Runs each script on its document and compares what it leaves.
fn play(cases: &[(String, &[Do], &str)]) {
    for (before, script, after) in cases {
        let mut editor = Harness::new(before);
        editor.play(script);
        assert_eq!(editor.state(), canon(after), "{script:?} on {before:?}");
    }
}

/// The kinds of a document's blocks, with what each holds in brackets after it.
fn outline(doc: &Document) -> String {
    fn walk(blocks: &[Block]) -> String {
        let name = |kind: &BlockKind| format!("{kind:?}").split([' ', '{']).next().unwrap_or_default().to_owned();
        let one = |block: &Block| match &block.body {
            Body::Blocks(parts) => format!("{}({})", name(&block.kind), walk(parts)),
            _ => name(&block.kind),
        };
        blocks.iter().map(one).collect::<Vec<_>>().join(" ")
    }
    walk(&doc.blocks)
}

#[test]
fn a_picture_is_a_stop_before_its_caption() {
    play(&[
        (at(FIGURE, "one", "one|"), &[K("Right")], "p one\nfig* Boats\np two"),
        (at(FIGURE, "one", "one|"), &[K("Right"), K("Right")], "p one\nfig |Boats\np two"),
        (at(FIGURE, "fig", "fig*"), &[K("Left")], "p one|\nfig Boats\np two"),
        (at(FIGURE, "Boats", "|Boats"), &[K("Left")], "p one\nfig* Boats\np two"),
        (at(FIGURE, "two", "|two"), &[K("Left")], "p one\nfig Boats|\np two"),
        // Without a caption there is only the picture to stop on.
        ("p one\nfig\np |two".into(), &[K("Left")], "p one\nfig*\np two"),
        ("p one\nfig*\np two".into(), &[K("Right")], "p one\nfig\np |two"),
        // Down comes to the picture and then into the caption, and up goes back the same way.
        (at(FIGURE, "one", "o|ne"), &[K("Down")], "p one\nfig* Boats\np two"),
        (at(FIGURE, "one", "o|ne"), &[K("Down"), K("Down")], "p one\nfig |Boats\np two"),
        (at(FIGURE, "two", "t|wo"), &[K("Up")], "p one\nfig B|oats\np two"),
        (at(FIGURE, "two", "t|wo"), &[K("Up"), K("Up")], "p one\nfig* Boats\np two"),
        (at(FIGURE, "two", "t|wo"), &[K("Up"), K("Up"), K("Up")], "p o|ne\nfig Boats\np two"),
        ("p one\nfig\np t|wo".into(), &[K("Up")], "p one\nfig*\np two"),
        // The ends of the document, and of the line a picture has to itself.
        ("fig Boats\np tw|o".into(), &[K("Mod+Home")], "fig* Boats\np two"),
        ("p o|ne\nfig".into(), &[K("Mod+End")], "p one\nfig*"),
        ("p one\nfig* Boats".into(), &[K("Home")], "p one\nfig* Boats"),
    ]);
}

#[test]
fn the_pictures_of_a_gallery_stand_side_by_side_above_its_caption() {
    let first = "p one\ng> fig*\ng> fig\ng> cap At sea\np two";
    let second = "p one\ng> fig\ng> fig*\ng> cap At sea\np two";
    let caption = "p one\ng> fig\ng> fig\ng> cap |At sea\np two";
    play(&[
        (at(GALLERY, "one", "one|"), &[K("Right")], first),
        (first.into(), &[K("Right")], second),
        (second.into(), &[K("Right")], caption),
        (caption.into(), &[K("Left")], second),
        (first.into(), &[K("Left")], "p one|\ng> fig\ng> fig\ng> cap At sea\np two"),
        (at(GALLERY, "one", "one|"), &[K("Down")], first),
        (first.into(), &[K("Down")], caption),
        (second.into(), &[K("Down")], caption),
        (caption.into(), &[K("Up")], second),
        (second.into(), &[K("Up")], "p one|\ng> fig\ng> fig\ng> cap At sea\np two"),
        // With no caption, below the pictures is the text after them.
        ("g> fig*\ng> fig\np two".into(), &[K("Down")], "g> fig\ng> fig\np |two"),
        ("g> fig\ng> fig\np tw|o".into(), &[K("Up")], "g> fig\ng> fig*\np two"),
    ]);
}

#[test]
fn arrows_and_tab_go_from_cell_to_cell() {
    play(&[
        (at(TABLE, "A", "A|"), &[K("Right")], "p one\nr1> th A\nr1> th |B\nr2> td C\nr2> td D\np two"),
        (at(TABLE, "A", "A|"), &[K("Down")], "p one\nr1> th A\nr1> th B\nr2> td C|\nr2> td D\np two"),
        (at(TABLE, "D", "|D"), &[K("Up")], "p one\nr1> th A\nr1> th |B\nr2> td C\nr2> td D\np two"),
        (at(TABLE, "C", "C|"), &[K("Down")], "p one\nr1> th A\nr1> th B\nr2> td C\nr2> td D\np |two"),
        (at(TABLE, "B", "|B"), &[K("Up")], "p one|\nr1> th A\nr1> th B\nr2> td C\nr2> td D\np two"),
        (at(TABLE, "A", "|A"), &[K("Tab")], "p one\nr1> th A\nr1> th B|\nr2> td C\nr2> td D\np two"),
        (at(TABLE, "B", "|B"), &[K("Tab")], "p one\nr1> th A\nr1> th B\nr2> td C|\nr2> td D\np two"),
        (at(TABLE, "C", "|C"), &[K("Shift+Tab")], "p one\nr1> th A\nr1> th B|\nr2> td C\nr2> td D\np two"),
        (at(TABLE, "A", "|A"), &[K("Shift+Tab")], "p one\nr1> th |A\nr1> th B\nr2> td C\nr2> td D\np two"),
        // Past the last cell, Tab adds a row.
        (
            at(TABLE, "D", "|D"),
            &[K("Tab"), T("E")],
            "p one\nr1> th A\nr1> th B\nr2> td C\nr2> td D\nr3> td E|\nr3> td\np two",
        ),
    ]);
}

#[test]
fn enter_never_breaks_a_part_in_two() {
    play(&[
        // On a picture it starts the caption, and in a caption it leaves for a new line.
        (at(FIGURE, "fig", "fig*"), &[K("Enter")], "p one\nfig Boats|\np two"),
        (at(FIGURE, "Boats", "Bo|ats"), &[K("Enter")], "p one\nfig Boats\np |\np two"),
        ("g> fig*\ng> fig".into(), &[K("Enter"), T("Sea")], "g> fig\ng> fig\ng> cap Sea|"),
        ("g> fig\ng> fig*\ng> cap At sea".into(), &[K("Enter")], "g> fig\ng> fig\ng> cap At sea|"),
        (at(GALLERY, "At sea", "At| sea"), &[K("Enter")], "p one\ng> fig\ng> fig\ng> cap At sea\np |\np two"),
        // In a cell it goes down the column, and from the last row out of the table.
        (at(TABLE, "B", "|B"), &[K("Enter")], "p one\nr1> th A\nr1> th B\nr2> td C\nr2> td D|\np two"),
        (at(TABLE, "D", "|D"), &[K("Enter")], "p one\nr1> th A\nr1> th B\nr2> td C\nr2> td D\np |\np two"),
        // A line break stays inside, and pasted lines run on.
        (at(TABLE, "C", "C|"), &[K("Shift+Enter")], "p one\nr1> th A\nr1> th B\nr2> td C⏎|\nr2> td D\np two"),
    ]);
}

#[test]
fn backspace_and_delete_never_join_a_part_to_other_text() {
    play(&[
        // From the start of a caption the caret steps onto the picture, and the next press takes the figure.
        (at(FIGURE, "Boats", "|Boats"), &[K("Backspace")], "p one\nfig* Boats\np two"),
        (at(FIGURE, "fig", "fig*"), &[K("Backspace")], "p one\np |two"),
        (at(FIGURE, "fig", "fig*"), &[K("Delete")], "p one\np |two"),
        ("p one\nfig* Boats".into(), &[K("Backspace")], "p one|"),
        (at(FIGURE, "two", "|two"), &[K("Backspace")], "p one\nfig Boats|\np two"),
        (at(FIGURE, "Boats", "Boats|"), &[K("Delete")], "p one\nfig Boats|\np two"),
        (at(FIGURE, "one", "one|"), &[K("Delete")], "p one\nfig* Boats\np two"),
        // An empty line beside a part gives way.
        ("fig\np |".into(), &[K("Backspace")], "fig*"),
        ("p |\nfig Boats".into(), &[K("Delete")], "fig* Boats"),
        ("r1> td A\np |".into(), &[K("Backspace")], "r1> td A|"),
        // A cell keeps to itself.
        (at(TABLE, "B", "|B"), &[K("Backspace")], "p one\nr1> th A\nr1> th |B\nr2> td C\nr2> td D\np two"),
        (at(TABLE, "A", "A|"), &[K("Delete")], "p one\nr1> th A|\nr1> th B\nr2> td C\nr2> td D\np two"),
        (at(TABLE, "two", "|two"), &[K("Backspace")], "p one\nr1> th A\nr1> th B\nr2> td C\nr2> td D|\np two"),
        (at(TABLE, "one", "one|"), &[K("Delete")], "p one|\nr1> th A\nr1> th B\nr2> td C\nr2> td D\np two"),
        // A gallery's caption goes when it is empty, and a gallery left with one picture is a figure.
        ("g> fig\ng> fig\ng> cap |".into(), &[K("Backspace")], "g> fig\ng> fig*"),
        ("g> fig\ng> fig\ng> cap |Sea".into(), &[K("Backspace")], "g> fig\ng> fig*\ng> cap Sea"),
        ("g> fig*\ng> fig\ng> cap Sea".into(), &[K("Backspace")], "fig* Sea"),
        ("g#> fig\ng#> fig*".into(), &[K("Delete")], "fig#*"),
        ("g> fig\ng> fig*\ng> fig".into(), &[K("Backspace")], "g> fig\ng> fig*"),
    ]);
}

#[test]
fn deleting_a_range_takes_whole_figures_and_keeps_a_cut_table_in_shape() {
    play(&[
        (at(&at(FIGURE, "one", "o^ne"), "two", "tw|o"), &[K("Backspace")], "p o|o"),
        // A caption that is only partly selected stays with its picture.
        (at(&at(FIGURE, "one", "o^ne"), "Boats", "Bo|ats"), &[K("Backspace")], "p o|\nfig ats\np two"),
        (at(&at(FIGURE, "Boats", "Bo^ats"), "two", "tw|o"), &[T("x")], "p one\nfig Box|\np o"),
        // From its start, or to its end from text before, the figure is taken whole.
        (at(&at(FIGURE, "Boats", "^Boats"), "two", "tw|o"), &[K("Backspace")], "p one\np |o"),
        (at(&at(FIGURE, "one", "o^ne"), "Boats", "Boats|"), &[K("Backspace")], "p o|\np two"),
        ("p one^\nfig |\np two".into(), &[K("Backspace")], "p one|\np two"),
        (at(&at(GALLERY, "one", "o^ne"), "two", "tw|o"), &[K("Backspace")], "p o|o"),
        ("g> fig ^\ng> fig |\ng> fig\ng> cap Sea".into(), &[K("Backspace")], "fig* Sea"),
        // A gallery that loses pictures to a range is what is left of it: a figure, or nothing.
        ("g> fig\ng> fig ^\ng> cap Se|a".into(), &[T("x")], "fig x|a"),
        ("g> fig ^\ng> fig\ng> cap Sea|\np two".into(), &[K("Backspace")], "p |two"),
        // A table inside the range goes. One that the range only reaches into is emptied there.
        (at(&at(TABLE, "one", "o^ne"), "two", "tw|o"), &[K("Backspace")], "p o|o"),
        (at(&at(TABLE, "one", "o^ne"), "C", "C|"), &[K("Backspace")], "p o|\nr1> th\nr1> th\nr2> td\nr2> td D\np two"),
        (at(&at(TABLE, "A", "^A"), "C", "C|"), &[T("x")], "p one\nr1> th x|\nr1> th\nr2> td\nr2> td D\np two"),
        // Covered from its first cell or to the end of its last, it goes whole.
        (at(&at(TABLE, "one", "o^ne"), "D", "D|"), &[K("Backspace")], "p o|\np two"),
        (at(&at(TABLE, "A", "^A"), "two", "tw|o"), &[K("Backspace")], "p one\np |o"),
        ("r1> td ^A\nr1> td B\np two|".into(), &[K("Backspace")], "p |"),
    ]);
}

#[test]
fn commands_for_blocks_leave_parts_as_they_are() {
    let heading = Command::SetBlock(BlockKind::Heading { level: 2 });
    let quote = Command::SetBlock(BlockKind::Quote);
    let tip = Command::SetBlock(BlockKind::Callout { tone: Tone::Tip });
    let divider = Command::SetBlock(BlockKind::Divider);
    let all = "Paragraph Figure Paragraph";
    let cases = [
        // A caption and a cell take no other kind, and the text around them does.
        (at(FIGURE, "Boats", "Bo|ats"), heading.clone(), all),
        (
            at(TABLE, "C", "|C"),
            heading.clone(),
            "Paragraph Table(TableRow(TableCell TableCell) TableRow(TableCell TableCell)) Paragraph",
        ),
        (at(&at(FIGURE, "one", "o^ne"), "two", "tw|o"), heading, "Heading Figure Heading"),
        // What wraps or follows a part wraps or follows the whole it belongs to.
        (at(GALLERY, "At sea", "At| sea"), quote.clone(), "Paragraph Quote(Gallery(Figure Figure Caption)) Paragraph"),
        (
            at(TABLE, "C", "|C"),
            tip,
            "Paragraph Callout(Table(TableRow(TableCell TableCell) TableRow(TableCell TableCell))) Paragraph",
        ),
        (at(FIGURE, "fig", "fig*"), quote, "Paragraph Quote(Figure) Paragraph"),
        (
            at(TABLE, "A", "|A"),
            divider.clone(),
            "Paragraph Table(TableRow(TableCell TableCell) TableRow(TableCell TableCell)) Divider Paragraph",
        ),
        (at(GALLERY, "fig", "fig*"), divider, "Paragraph Gallery(Figure Figure Caption) Divider Paragraph"),
    ];
    for (before, command, after) in cases {
        let mut editor = Harness::new(&before);
        editor.run(command.clone());
        assert_eq!(outline(editor.editor.doc()), after, "{command:?} on {before:?}");
    }
}

#[test]
fn shorthand_and_typing_meet_a_part_as_they_should() {
    play(&[
        // Nothing is typed onto a picture.
        (at(FIGURE, "fig", "fig*"), &[T("x")], "p one\nfig* Boats\np two"),
        // A caption and a cell take marks, and none of the shorthand that makes blocks.
        ("fig |".into(), &[T("**b** # - > 1. ")], "fig **b** # - > 1. |"),
        ("r1> td |".into(), &[T("- [!TIP] *i*")], "r1> td - \\[!TIP\\] *i*|"),
    ]);
}

#[test]
fn an_arrow_at_the_edge_of_the_document_leaves_a_part_by_a_new_line() {
    play(&[
        ("fig* Boats".into(), &[K("Up")], "p |\nfig Boats"),
        ("fig Boats|".into(), &[K("Down")], "fig Boats\np |"),
        ("fig Boats|".into(), &[K("Right")], "fig Boats\np |"),
        ("fig*".into(), &[K("Left")], "p |\nfig"),
        ("g> fig\ng> fig*".into(), &[K("Right")], "g> fig\ng> fig\np |"),
        ("g> fig\ng> fig*".into(), &[K("Down")], "g> fig\ng> fig\np |"),
        ("r1> td A\nr2> td B|".into(), &[K("Down")], "r1> td A\nr2> td B\np |"),
        ("r1> td |A\nr2> td B".into(), &[K("Up")], "p |\nr1> td A\nr2> td B"),
        // The click below everything.
        ("fig Boats|".into(), &[], "fig Boats|"),
    ]);
    for state in ["fig Bo|ats", "g> fig*\ng> fig", "r1> td A|"] {
        let mut editor = Harness::new(state);
        editor.run(Command::LineBelow);
        assert!(editor.state().ends_with("\np |"), "{state:?} gave {:?}", editor.state());
    }
}

#[test]
fn copies_keep_figures_and_rows_whole() {
    // What is selected, the key that takes it, where it is pasted, and what the paste leaves there.
    let cases = [
        // A selected picture is its figure.
        (at(FIGURE, "fig", "fig*"), "Mod+c", "p |", "fig Boats|"),
        (at(FIGURE, "fig", "fig*"), "Mod+c", "p a|b", "p a\nfig Boats\np |b"),
        // The text of a caption or a cell is only text.
        (at(FIGURE, "Boats", "^Boats|"), "Mod+c", "p |", "p Boats|"),
        (at(TABLE, "C", "^C|"), "Mod+c", "p |", "p C|"),
        // A figure taken whole comes whole, and one that is not gives its text.
        (at(&at(FIGURE, "one", "o^ne"), "two", "t|wo"), "Mod+c", "p |", "p ne\nfig Boats\np t|"),
        (at(&at(FIGURE, "one", "o^ne"), "Boats", "Bo|ats"), "Mod+c", "p |", "p ne\np Bo|"),
        // A row comes with all of its cells, and heads the copy only when it headed the table.
        (at(&at(TABLE, "C", "^C"), "D", "D|"), "Mod+c", "p |", "r1> td C\nr1> td D|"),
        (at(&at(TABLE, "one", "o^ne"), "A", "A|"), "Mod+c", "p |", "p ne\nr1> th A\nr1> th B|"),
        // One picture of a gallery is a figure of its own, and several are a gallery.
        ("g> fig*\ng> fig\ng> cap Sea".into(), "Mod+c", "p |", "fig*"),
        ("g> fig ^\ng> fig |\ng> fig\ng> cap Sea".into(), "Mod+c", "p |", "g> fig\ng> fig*"),
        // Into a caption or a cell, what is pasted goes as one stretch of text.
        (at(&at(FIGURE, "one", "o^ne"), "two", "t|wo"), "Mod+c", "r1> td x|", "r1> td xne Boats t|"),
        (at(&at(FIGURE, "one", "o^ne"), "two", "t|wo"), "Mod+c", "fig x|", "fig xne Boats t|"),
        // With a picture selected, it goes on a line after the figure.
        (at(FIGURE, "one", "^one|"), "Mod+c", "fig* Boats", "fig Boats\np one|"),
        // A cut takes the picture away.
        ("g> fig*\ng> fig\ng> cap Sea\np".into(), "Mod+x", "p |", "fig*"),
    ];
    for (from, key, into, after) in cases {
        let mut source = Harness::new(&from);
        assert!(source.key(key), "{key} on {from:?}");
        let mut target = Harness::new(into);
        target.platform.clip = source.platform.clip.clone();
        target.key("Mod+v");
        assert_eq!(target.state(), canon(after), "{from:?} into {into:?}");
    }

    let mut editor = Harness::new("g> fig*\ng> fig\ng> cap Sea\np");
    editor.key("Mod+x");
    assert_eq!(editor.state(), canon("fig* Sea\np"), "a cut leaves a gallery of one as a figure");
    editor.platform.clip = Some(clip("one\n\ntwo", None));
    editor.key("Right");
    editor.key("Mod+v");
    assert_eq!(editor.state(), canon("fig one two|Sea\np"), "lines pasted into a caption run on");
}

#[test]
fn a_gallery_that_becomes_a_figure_is_one_undo_step() {
    let before = "p one\ng#> fig\ng#> fig*\ng#> cap At sea\np two";
    let mut editor = Harness::new(before);
    editor.key("Backspace");
    assert_eq!(editor.state(), canon("p one\nfig#* At sea\np two"));
    assert!(editor.key("Mod+z"));
    assert_eq!(editor.state(), canon(before));
    assert!(!editor.editor.can_undo());
    editor.key("Mod+Shift+z");
    assert_eq!(editor.state(), canon("p one\nfig#* At sea\np two"));
}

#[test]
fn the_selected_figures_are_the_one_under_the_caret_or_those_a_range_takes() {
    let cases = [
        (at(FIGURE, "fig", "fig*"), 1),
        (at(FIGURE, "Boats", "Bo|ats"), 0),
        (at(FIGURE, "Boats", "^Boats|"), 0),
        (at(&at(FIGURE, "one", "o^ne"), "two", "tw|o"), 1),
        (at(&at(FIGURE, "one", "o^ne"), "Boats", "Bo|ats"), 0),
        (at(&at(GALLERY, "one", "o^ne"), "At sea", "At| sea"), 2),
        ("g> fig ^\ng> fig |\ng> fig".into(), 2),
    ];
    for (state, count) in cases {
        let editor = Harness::new(&state);
        assert_eq!(editor.editor.figures().len(), count, "{state:?}");
        assert_eq!(editor.editor.picture().is_some(), state.contains("fig*"), "{state:?}");
    }
}

/// Picks the blocks to take out of a document.
type Gone = fn(&Block) -> bool;

/// An editor on `state` with every block that `gone` picks taken out. Only a merge leaves a gallery, a
/// table or a row with nothing in it, so the notation has no way to write one.
fn left_without(state: &str, gone: Gone) -> Harness {
    fn strip(blocks: &mut Vec<Block>, gone: Gone) {
        blocks.retain(|block| !gone(block));
        for block in blocks {
            if let Body::Blocks(parts) = &mut block.body {
                strip(parts, gone);
            }
        }
    }
    let (mut doc, selection) = common::parse(state);
    strip(&mut doc.blocks, gone);
    let mut editor = Harness::on(Model::new(doc, ReplicaId::from(1)), 1);
    if let Some(selection) = selection {
        editor.editor.select(selection);
    }
    editor
}

#[test]
fn an_empty_gallery_or_table_holds_no_caret_and_goes_as_a_divider_does() {
    let rows: Gone = |block| block.kind == BlockKind::TableRow;
    let pictures: Gone = |block| matches!(block.kind, BlockKind::Figure { .. });
    let emptied = [("p one\nr1> td x\np two", rows), ("p one\ng> fig\ng> fig\np two", pictures)];
    let cases = [
        // (the caret, the key, the text afterwards, whether the empty block is still there)
        (("one", "one|"), "Right", "p one\np |two", true),
        (("two", "|two"), "Left", "p one|\np two", true),
        (("two", "|two"), "Backspace", "p one\np |two", false),
        (("one", "one|"), "Delete", "p one|\np two", false),
        (("two", "t|wo"), "Shift+Mod+Home", "p |one\np t^wo", true),
    ];
    for (doc, gone) in emptied {
        for ((from, to), key, after, kept) in cases {
            let mut editor = left_without(&at(doc, from, to), gone);
            assert_eq!(editor.editor.doc().blocks.len(), 3, "{doc:?} has an empty block in the middle");
            editor.key(key);
            assert_eq!(editor.state(), canon(after), "{key} at {to:?} in {doc:?}");
            assert_eq!(editor.editor.doc().blocks.len() == 3, kept, "{key} at {to:?} in {doc:?}");
        }
        // A range over it takes it away, and a click on it lands in the text after it.
        let mut editor = left_without(&at(&at(doc, "one", "o^ne"), "two", "tw|o"), gone);
        editor.key("Backspace");
        assert_eq!(editor.state(), canon("p o|o"), "{doc:?}");
        let mut editor = left_without(&at(doc, "one", "o|ne"), gone);
        let empty = editor.editor.doc().blocks[1].id.clone();
        editor.editor.select(Selection::caret(Pos::new(empty, 0)));
        assert_eq!(editor.state(), canon("p one\np |two"), "a click on the empty block of {doc:?}");
    }
}

#[test]
fn a_row_with_no_cells_is_passed_over() {
    let table = "r1> td A\nr1> td B\nr2> td x\nr3> td C\nr3> td D";
    let cases = [
        // The third row keeps its number, since the second is still there with nothing in it.
        (("A", "A|"), "Down", "r1> td A\nr1> td B\nr3> td C|\nr3> td D"),
        (("D", "D|"), "Up", "r1> td A\nr1> td B|\nr3> td C\nr3> td D"),
        (("A", "A|"), "Enter", "r1> td A\nr1> td B\nr3> td C|\nr3> td D"),
        (("B", "B|"), "Tab", "r1> td A\nr1> td B\nr3> td C|\nr3> td D"),
        (("C", "C|"), "Shift+Tab", "r1> td A\nr1> td B|\nr3> td C\nr3> td D"),
    ];
    for ((from, to), key, after) in cases {
        let mut editor = left_without(&at(table, from, to), |block| block.plain_text() == "x");
        editor.key(key);
        assert_eq!(editor.state(), after, "{key} at {to:?}");
    }
}

#[test]
fn either_side_of_a_picture_in_a_gallery_is_the_picture() {
    for side in [0, 1, 7] {
        let mut editor = Harness::new(&at(GALLERY, "one", "one|"));
        let second = editor.editor.doc().blocks[1].body.blocks()[1].id.clone();
        editor.editor.select(Selection::caret(Pos::new(second.clone(), side)));
        assert_eq!(editor.editor.picture(), Some(&second), "offset {side}");
        assert_eq!(editor.state(), canon("p one\ng> fig\ng> fig*\ng> cap At sea\np two"), "offset {side}");
    }
}
