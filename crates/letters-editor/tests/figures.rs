//! Figures and galleries: pictures coming in and moving about, and what is said of them.

mod common;

use common::Do::{K, T};
use common::{Do, Harness, canon, check, clip, png};
use letters_editor::{Clip, Command, Keymap, Picture, Spot};
use letters_model::{Block, BlockId, BlockKind};

/// How a test names a spot, which the notation has no ids for.
#[derive(Clone, Copy, Debug)]
enum At {
    /// No spot: where the selection is.
    Caret,
    /// Beside the figure that comes so many figures after the first, after it or before.
    Beside(usize, bool),
    /// On a line by the block with this text, below it or above.
    Line(&'static str, bool),
}

fn figures(editor: &Harness) -> Vec<&Block> {
    editor.editor.doc().walk().filter(|block| matches!(block.kind, BlockKind::Figure { .. })).collect()
}

fn spot(editor: &Harness, at: At) -> Option<Spot> {
    let texted = |text: &str| editor.editor.doc().walk().find(|block| block.plain_text() == text && !text.is_empty());
    match at {
        At::Caret => None,
        At::Beside(nth, after) => Some(Spot::Beside { figure: figures(editor)[nth].id.clone(), after }),
        At::Line(text, below) => {
            Some(Spot::Line { block: texted(text).expect("a block with that text").id.clone(), below })
        }
    }
}

/// Puts `count` pictures of 400 by 300 pixels in at a spot.
fn insert(editor: &mut Harness, count: u8, at: At) {
    let pictures = (0..count).map(|seed| png(seed, 400, 300)).collect();
    let at = spot(editor, at);
    editor.run(Command::InsertFigures { pictures, at });
}

/// Checks that one undo takes back what was done to `before`.
fn undone(editor: &mut Harness, before: &str, what: &str) {
    assert!(editor.key("Mod+z"), "{what} on {before:?} is a step to undo");
    assert_eq!(editor.text(), Harness::new(before).text(), "{what} on {before:?}, undone");
}

#[test]
fn pictures_come_in_as_a_figure_or_as_a_gallery() {
    let cases = [
        // One picture is a figure below the caret's block, or in place of an empty line.
        ("p one|\np two", 1, At::Caret, "p one\nfig*\np two"),
        ("p one\np |\np two", 1, At::Caret, "p one\nfig*\np two"),
        ("p o^ne\np tw|o", 1, At::Caret, "p one\np two\nfig*"),
        ("p one\nfig* Boats\np two", 1, At::Caret, "p one\nfig Boats\nfig*\np two"),
        ("r1> td A|\nr1> td B\np two", 1, At::Caret, "r1> td A\nr1> td B\nfig*\np two"),
        ("", 1, At::Caret, "fig*"),
        // Several are a gallery, and with a picture of a gallery selected they join it there.
        ("p one|\np two", 3, At::Caret, "p one\ng> fig\ng> fig\ng> fig*\np two"),
        ("g> fig*\ng> fig\ng> cap Sea", 2, At::Caret, "g> fig\ng> fig\ng> fig*\ng> fig\ng> cap Sea"),
        // Dropped on a line, they stand there.
        ("p one|\np two", 1, At::Line("one", false), "fig*\np one\np two"),
        ("p one|\np two", 2, At::Line("two", true), "p one\np two\ng> fig\ng> fig*"),
        ("p one|\nr1> td A\nr1> td B", 1, At::Line("A", false), "p one\nfig*\nr1> td A\nr1> td B"),
        // Dropped on a figure's edge, they make a gallery with it, which takes its caption and its number.
        ("p one|\nfig# Boats", 1, At::Beside(0, true), "p one\ng#> fig\ng#> fig*\ng#> cap Boats"),
        ("p one|\nfig Boats", 2, At::Beside(0, false), "p one\ng> fig\ng> fig*\ng> fig\ng> cap Boats"),
        ("p one|\ng> fig\ng> fig", 1, At::Beside(1, false), "p one\ng> fig\ng> fig*\ng> fig"),
    ];
    for (before, count, at, after) in cases {
        let mut editor = Harness::new(before);
        insert(&mut editor, count, at);
        assert_eq!(editor.state(), canon(after), "{count} at {at:?} in {before:?}");
        assert!(check(&editor).is_ok(), "{count} at {at:?} in {before:?} leaves a gallery that is not whole");
        assert_eq!(editor.platform.assets.len(), usize::from(count), "each picture is handed over to be kept");
        undone(&mut editor, before, "putting pictures in");
    }
}

#[test]
fn a_picture_is_named_by_its_bytes_and_sized_in_points() {
    let mut editor = Harness::new("p one|");
    let pictures = vec![png(1, 400, 200), png(1, 400, 200), png(2, 4000, 1000)];
    let not = letters_editor::Picture { bytes: b"words".to_vec(), ..png(3, 10, 10) };
    editor.run(Command::InsertFigures { pictures: [pictures, vec![not]].concat(), at: None });

    let shown: Vec<_> = figures(&editor)
        .iter()
        .map(|figure| match &figure.kind {
            BlockKind::Figure { asset: Some(asset), src: None, width, height, .. } => (asset.clone(), *width, *height),
            other => panic!("{other:?} is no figure with an asset"),
        })
        .collect();
    assert_eq!(shown.len(), 3, "what is no picture is left out");
    let (name, width, height) = &shown[0];
    let hash = name.strip_prefix("assets/").and_then(|name| name.strip_suffix(".png")).expect("an asset's name");
    assert!(hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()), "{name} is named by a hash");
    assert_eq!((*width, *height), (300.0, 150.0), "a pixel is three quarters of a point");
    assert_eq!(shown[1].0, *name, "the same bytes have the same name");
    assert_eq!(editor.platform.assets.len(), 2, "and are kept once");
    assert!(editor.platform.assets.contains_key(name));

    // A picture wider than the page's text is brought down to it, at its own shape.
    let page = &editor.editor.model().style().page;
    let across = page.width - page.margin_left - page.margin_right;
    assert!((shown[2].1 - across).abs() < 0.01 && (shown[2].2 - across / 4.0).abs() < 0.01, "{:?}", shown[2]);
}

#[test]
fn a_figure_moves_to_a_line_of_its_own_or_beside_another() {
    let cases = [
        // (the figure selected or its caption under the caret, where it goes, what that leaves)
        ("p one\nfig* Boats\np two", At::Line("one", false), "fig* Boats\np one\np two"),
        ("fig Bo|ats\np one", At::Line("one", true), "p one\nfig* Boats"),
        // Out of a gallery, which is a figure again once it holds one picture.
        (
            "g> fig*\ng> fig\ng> fig\ng> cap Sea\np two",
            At::Line("two", true),
            "g> fig\ng> fig\ng> cap Sea\np two\nfig*",
        ),
        ("g#> fig*\ng#> fig\ng#> cap Sea\np two", At::Line("two", true), "fig# Sea\np two\nfig*"),
        ("g> fig\ng> fig*\np two", At::Line("two", false), "fig\nfig*\np two"),
        // Onto the edge of a figure, which makes a gallery, and into a gallery.
        ("fig* Boats\np one\nfig Ships", At::Beside(1, true), "p one\ng> fig\ng> fig*\ng> cap Ships"),
        ("fig* Boats\np one\nfig", At::Beside(1, false), "p one\ng> fig*\ng> fig\ng> cap Boats"),
        ("fig* Boats\ng> fig\ng> fig\ng> cap Sea", At::Beside(2, true), "g> fig\ng> fig\ng> fig*\ng> cap Sea"),
        ("g> fig*\ng> fig\ng> fig", At::Beside(2, true), "g> fig\ng> fig\ng> fig*"),
    ];
    for (before, at, after) in cases {
        let mut editor = Harness::new(before);
        let moved = editor.editor.selection().expect("a caret").head.block.clone();
        let to = spot(&editor, at).expect("a spot");
        editor.run(Command::MoveFigure(to));
        assert_eq!(editor.state(), canon(after), "to {at:?} in {before:?}");
        assert_eq!(editor.editor.picture(), Some(&moved), "the picture that moved is the one selected");
        assert!(check(&editor).is_ok(), "to {at:?} in {before:?} leaves a gallery that is not whole");
        undone(&mut editor, before, "moving a figure");
    }
    // A figure dropped on itself, or by a block that is gone, stays.
    let mut editor = Harness::new("p one\nfig* Boats");
    for to in [spot(&editor, At::Beside(0, true)), Some(Spot::Line { block: BlockId::new("gone"), below: true })] {
        editor.run(Command::MoveFigure(to.expect("a spot")));
        assert_eq!(editor.state(), canon("p one\nfig* Boats"));
    }
}

#[test]
fn alt_and_an_arrow_move_a_picture_along_its_gallery() {
    let order = |editor: &Harness| -> Vec<BlockId> { figures(editor).iter().map(|figure| figure.id.clone()).collect() };
    let cases: [(&str, &[Do], &str, [usize; 3]); 4] = [
        ("g> fig*\ng> fig\ng> fig", &[K("Alt+Right")], "g> fig\ng> fig*\ng> fig", [1, 0, 2]),
        ("g> fig\ng> fig\ng> fig*", &[K("Alt+Left"), K("Alt+Left")], "g> fig*\ng> fig\ng> fig", [2, 0, 1]),
        // At either end it stays where it is.
        ("g> fig*\ng> fig\ng> fig", &[K("Alt+Left")], "g> fig*\ng> fig\ng> fig", [0, 1, 2]),
        ("g> fig\ng> fig\ng> fig*", &[K("Alt+Right")], "g> fig\ng> fig\ng> fig*", [0, 1, 2]),
    ];
    for (before, script, after, places) in cases {
        let mut editor = Harness::new(before);
        let was = order(&editor);
        editor.play(script);
        assert_eq!(editor.state(), canon(after), "{script:?} on {before:?}");
        assert_eq!(order(&editor), places.map(|place| was[place].clone()), "{script:?} on {before:?}");
    }
    // Anywhere else the keys mean what they did: on a Mac, a word to the left.
    let mut editor = Harness::new("p one two|");
    editor.editor.set_keymap(Keymap::mac());
    editor.key("Alt+Left");
    assert_eq!(editor.state(), canon("p one |two"));
}

#[test]
fn a_label_typed_at_the_start_of_a_caption_turns_the_generated_one_on() {
    let cases: [(&str, &[Do], &str); 9] = [
        ("fig |Boats", &[T("Figure 1: ")], "fig# |Boats"),
        ("fig |", &[T("figure 12. "), T("Boats")], "fig# Boats|"),
        ("g> fig\ng> fig\ng> cap |Sea", &[T("Figure 3: ")], "g#> fig\ng#> fig\ng#> cap |Sea"),
        // Backspace straight after gives the label back as typed. Later it takes the generated one away,
        // and only then steps back onto the picture.
        ("fig |Boats", &[T("Figure 1: "), K("Backspace")], "fig Figure 1: |Boats"),
        ("fig# |Boats", &[K("Backspace")], "fig |Boats"),
        ("fig# |Boats", &[K("Backspace"), K("Backspace")], "fig* Boats"),
        ("g#> fig\ng#> fig\ng#> cap |Sea", &[K("Backspace")], "g> fig\ng> fig\ng> cap |Sea"),
        // Anywhere but the start of a caption the words are only words.
        ("p |", &[T("Figure 1: ")], "p Figure 1: |"),
        ("fig See |", &[T("Figure 1: ")], "fig See Figure 1: |"),
    ];
    for (before, script, after) in cases {
        let mut editor = Harness::new(before);
        editor.play(script);
        assert_eq!(editor.state(), canon(after), "{script:?} on {before:?}");
    }
}

#[test]
fn commands_set_what_is_said_of_the_carets_figure() {
    let kind = |editor: &Harness| figures(editor)[0].kind.clone();
    let cases = [
        ("fig* Boats", Command::SetNumbered(true), "fig#* Boats"),
        ("fig# Bo|ats", Command::SetNumbered(false), "fig Bo|ats"),
        ("g> fig*\ng> fig", Command::SetNumbered(true), "g#> fig*\ng#> fig"),
        ("g#> fig\ng#> fig\ng#> cap Se|a", Command::SetNumbered(false), "g> fig\ng> fig\ng> cap Se|a"),
        // Enter and Esc as commands: into the caption and back to the picture.
        ("fig* Boats", Command::EditCaption, "fig Boats|"),
        ("g> fig*\ng> fig", Command::EditCaption, "g> fig\ng> fig\ng> cap |"),
        ("fig Bo|ats", Command::LeaveCaption, "fig* Boats"),
        ("g> fig\ng> fig\ng> cap Se|a", Command::LeaveCaption, "g> fig\ng> fig*\ng> cap Sea"),
    ];
    for (before, command, after) in cases {
        let mut editor = Harness::new(before);
        editor.run(command.clone());
        assert_eq!(editor.state(), canon(after), "{command:?} on {before:?}");
    }

    let mut editor = Harness::new("fig Bo|ats\np two");
    editor.run(Command::SetAlt("Two boats".into()));
    editor.run(Command::ResizeFigure { width: 120.0, height: 90.0 });
    let BlockKind::Figure { alt, width, height, .. } = kind(&editor) else { panic!("the figure is gone") };
    assert_eq!((alt.as_str(), width, height), ("Two boats", 120.0, 90.0));
    // A size that is no size changes nothing, and neither does a command with no figure under the caret.
    let sized = kind(&editor);
    for (width, height) in [(0.0, 90.0), (120.0, -1.0), (f32::NAN, 90.0), (f32::INFINITY, 90.0)] {
        editor.run(Command::ResizeFigure { width, height });
    }
    editor.play(&[K("Mod+End")]);
    editor.run(Command::SetAlt("Elsewhere".into()));
    assert_eq!(kind(&editor), sized);
}

#[test]
fn esc_and_enter_go_between_a_caption_and_its_picture() {
    let cases: [(&str, &[Do], &str); 3] = [
        ("fig Bo|ats", &[K("Escape")], "fig* Boats"),
        ("fig Bo|ats", &[K("Escape"), K("Enter")], "fig Boats|"),
        ("g> fig\ng> fig\ng> cap Se|a", &[K("Escape")], "g> fig\ng> fig*\ng> cap Sea"),
    ];
    for (before, script, after) in cases {
        let mut editor = Harness::new(before);
        editor.play(script);
        assert_eq!(editor.state(), canon(after), "{script:?} on {before:?}");
    }
}

#[test]
fn pictures_on_the_clipboard_are_pasted_as_figures() {
    let pictured = |text: &str| Clip { pictures: vec![png(1, 40, 30)], ..clip(text, None) };
    let cases = [
        ("p one|\np two", "", "p one\nfig*\np two"),
        ("p o^ne\np tw|o", "", "p oo\nfig*"),
        ("g> fig*\ng> fig", "", "g> fig\ng> fig*\ng> fig"),
        // Beside words a picture is a rendering of the words, and the words are what is pasted.
        ("p one|", " two", "p one two|"),
    ];
    for (before, text, after) in cases {
        let mut editor = Harness::new(before);
        editor.platform.clip = Some(pictured(text));
        editor.key("Mod+v");
        assert_eq!(editor.state(), canon(after), "{text:?} with a picture pasted in {before:?}");
    }
}

#[test]
fn a_linked_picture_is_kept_in_the_document_by_its_bytes() {
    // The caret is elsewhere each time, and stays there. A linked figure is 240 points wide, and its height
    // follows the picture's own shape when the picture tells its size.
    let cases = [
        ("p one|\nfig@ Boats\np two", (400, 300), "p one|\nfig Boats\np two", 180.0),
        ("g> fig@\ng> fig\np one|", (100, 200), "g> fig\ng> fig\np one|", 480.0),
        ("p one|\nfig@# Boats", (0, 0), "p one|\nfig# Boats", 160.0),
    ];
    for (before, (wide, high), after, tall) in cases {
        let mut editor = Harness::new(before);
        let linked = figures(&editor)[0].id.clone();
        let picture = Picture { width: wide, height: high, ..png(1, 400, 300) };
        let command = Command::EmbedFigure { figure: linked, picture: picture.clone() };
        assert!(editor.editor.applies(&command), "a linked picture can be kept, in {before:?}");
        editor.run(command.clone());
        assert_eq!(editor.state(), canon(after), "keeping the picture of {before:?}");

        let BlockKind::Figure { asset: Some(asset), src: None, width, height, .. } = figures(&editor)[0].kind.clone()
        else {
            panic!("the figure names an asset and no address, in {before:?}")
        };
        assert_eq!(editor.platform.assets.get(&asset), Some(&picture.bytes), "the bytes are kept under its name");
        assert_eq!((width, height), (240.0, tall), "a picture of {wide} by {high} pixels in {before:?}");
        assert!(!editor.editor.applies(&command), "a picture that is kept is not kept again");
        undone(&mut editor, before, "keeping a picture");
    }
}

#[test]
fn nothing_is_kept_without_a_linked_figure_and_a_picture() {
    let text = Picture { bytes: b"not a picture".to_vec(), mime: "text/plain".into(), width: 1, height: 1 };
    let cases = [
        // A figure that is kept already, no figure at all, a block that is no figure, and bytes that are no picture.
        ("p one|\nfig Boats", "b2", png(1, 400, 300)),
        ("p one|\nfig@ Boats", "gone", png(1, 400, 300)),
        ("p one|\nfig@ Boats", "b1", png(1, 400, 300)),
        ("p one|\nfig@ Boats", "b2", text),
    ];
    for (state, figure, picture) in cases {
        let mut editor = Harness::new(state);
        let command = Command::EmbedFigure { figure: BlockId::new(figure), picture };
        assert!(!editor.editor.applies(&command), "keeping a picture for {figure} in {state:?}");
        editor.run(command);
        assert_eq!(editor.state(), canon(state), "keeping a picture for {figure} in {state:?}");
        assert!(editor.platform.assets.is_empty(), "nothing is handed over for {figure} in {state:?}");
    }
}
