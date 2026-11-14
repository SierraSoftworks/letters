//! Two devices on one document: the failure table of the design, played out on a folder in memory.

mod common;

use std::cmp::Ordering;

use common::{Folder, ID, bytes, open, package, paragraph};
use letters_format::{Error, OPLOG, Package, Save, Saved, WorkingCopy, conflict_copy_of, save};
use letters_model::{Anchor, BlockId, Document, Note, NoteKind, Op, Pos, ReplicaId, Run, Style};

const NAME: &str = "report.let";

fn document() -> Document {
    Document { blocks: vec![paragraph("p", "Hello world")], ..Document::default() }
}

/// Saves as the named device would.
fn save_from(folder: &mut Folder, copy: &mut WorkingCopy, device: &str) -> Saved {
    save(folder, copy, &Save { name: NAME, token: device, label: device, attempts: 3 }).unwrap()
}

fn open_at(folder: &Folder, name: &str, replica: u64) -> WorkingCopy {
    WorkingCopy::open(&mut open(&folder.files[name]), ReplicaId::from(replica)).unwrap()
}

/// A folder that holds `file`, and two devices that have it open.
fn opened(file: Vec<u8>) -> (Folder, WorkingCopy, WorkingCopy) {
    let mut folder = Folder::default();
    folder.put(NAME, file);
    let (desk, laptop) = (open_at(&folder, NAME, 2), open_at(&folder, NAME, 3));
    (folder, desk, laptop)
}

/// A folder that holds a saved document reading "Hello world", and two devices that have it open.
fn shared() -> (Folder, WorkingCopy, WorkingCopy) {
    let mut folder = Folder::default();
    let mut first = WorkingCopy::new(ID, "2026-11-14T20:00:00Z", document(), ReplicaId::from(1));
    save_from(&mut folder, &mut first, "first");
    opened(folder.files[NAME].clone())
}

/// Saves from the desk and the laptop in turn, `turns` times in all.
fn in_turn(folder: &mut Folder, desk: &mut WorkingCopy, laptop: &mut WorkingCopy, turns: usize) -> Vec<Saved> {
    let turn = |turn| match turn % 2 {
        0 => save_from(folder, desk, "desk"),
        _ => save_from(folder, laptop, "laptop"),
    };
    (0..turns).map(turn).collect()
}

fn typed(copy: &mut WorkingCopy, offset: usize, text: &str) {
    copy.apply([Op::InsertText { at: Pos::new("p", offset), text: text.into() }]).unwrap();
}

/// Adds a comment on `range` of the paragraph.
fn comment(copy: &mut WorkingCopy, id: &str, range: std::ops::Range<usize>) {
    copy.add_note(Note {
        id: id.into(),
        kind: NoteKind::Comment,
        anchor: Anchor::range("p", range),
        author: "Ana".into(),
        created: "2026-11-14T20:00:00Z".into(),
        resolved: false,
        content: vec![Run::plain("Is this right?")],
        replies: Vec::new(),
        extra: Default::default(),
    });
}

fn text(document: &Document) -> String {
    document.block(&BlockId::from("p")).unwrap().plain_text()
}

fn note_ids(copy: &WorkingCopy) -> Vec<&str> {
    copy.notes.notes.iter().map(|note| note.id.as_str()).collect()
}

/// The text a note of `copy` covers.
fn covered(copy: &WorkingCopy, note: &str) -> String {
    let anchor = &copy.notes.notes.iter().find(|held| held.id == note).unwrap().anchor;
    text(copy.model().doc())[anchor.range.clone().unwrap()].to_owned()
}

/// Whether two copies hold the same history.
fn level(a: &WorkingCopy, b: &WorkingCopy) -> bool {
    a.model().compare(&b.model().save()).unwrap() == Some(Ordering::Equal)
}

#[test]
fn two_writers_on_one_file_keep_everything_and_then_settle() {
    let (mut folder, mut desk, mut laptop) = shared();
    comment(&mut desk, "desk", 6..11);
    typed(&mut desk, 0, "Oh. ");
    desk.entries.insert("assets/desk.png".into(), vec![1; 16]);
    typed(&mut laptop, 11, "!");
    comment(&mut laptop, "laptop", 0..5);
    laptop.entries.insert("assets/laptop.png".into(), vec![2; 16]);

    let saves = in_turn(&mut folder, &mut desk, &mut laptop, 3);
    assert!(saves.iter().all(|saved| matches!(saved, Saved::Written | Saved::Unchanged)), "{saves:?}");

    let file = open_at(&folder, NAME, 4);
    for (who, copy) in [("the file", &file), ("the desk", &desk), ("the laptop", &laptop)] {
        assert_eq!(text(copy.model().doc()), "Oh. Hello world!", "{who}");
        assert_eq!(note_ids(copy), ["desk", "laptop"], "{who}");
        assert_eq!(covered(copy, "desk"), "world", "{who}: a note stays on its text");
        assert_eq!(covered(copy, "laptop"), "Hello", "{who}: a note stays on its text");
    }
    let mut on_disk = open(&folder.files[NAME]);
    for asset in ["assets/desk.png", "assets/laptop.png"] {
        assert!(on_disk.read(asset).unwrap().is_some(), "{asset}");
    }
    assert_eq!(folder.names(), [NAME]);

    // Writers with nothing new must leave the file alone, or two idle devices would rewrite it forever.
    let settled = folder.changes();
    let saves = in_turn(&mut folder, &mut desk, &mut laptop, 4);
    assert!(saves.iter().all(|saved| matches!(saved, Saved::Unchanged)), "{saves:?}");
    assert_eq!(folder.changes(), settled);
}

#[test]
fn a_save_that_lost_a_race_is_healed_by_the_next() {
    let (mut folder, mut desk, mut laptop) = shared();
    let stale = folder.files[NAME].clone();
    typed(&mut desk, 0, "Oh. ");
    desk.entries.insert("assets/desk.png".into(), vec![1; 16]);
    assert!(matches!(save_from(&mut folder, &mut desk, "desk"), Saved::Written));

    // The laptop read the file before the desk's save and replaces it just after: the desk's save is gone.
    let mut elsewhere = Folder::default();
    elsewhere.put(NAME, stale);
    typed(&mut laptop, 11, "!");
    save_from(&mut elsewhere, &mut laptop, "laptop");
    folder.put(NAME, elsewhere.files[NAME].clone());
    assert_eq!(text(open_at(&folder, NAME, 4).model().doc()), "Hello world!");

    let saved = save_from(&mut folder, &mut desk, "desk");
    assert!(matches!(saved, Saved::Written), "{saved:?}");
    assert_eq!(text(open_at(&folder, NAME, 4).model().doc()), "Oh. Hello world!");
    assert!(open(&folder.files[NAME]).read("assets/desk.png").unwrap().is_some(), "the new asset is written again");
}

#[test]
fn a_conflict_copy_merges_back_with_nothing_lost() {
    // Who left the copy: Letters, when the file could not be replaced, or a sync tool that saw two versions.
    for by in ["Letters", "a sync tool"] {
        let (mut folder, mut desk, mut laptop) = shared();
        typed(&mut laptop, 11, "!");
        assert!(matches!(save_from(&mut folder, &mut laptop, "laptop"), Saved::Written), "{by}");
        let original = folder.files[NAME].clone();

        comment(&mut desk, "desk", 6..11);
        typed(&mut desk, 0, "Oh. ");
        desk.entries.insert("assets/desk.png".into(), vec![1; 16]);
        let copy = if by == "Letters" {
            folder.locked = Some(NAME.into());
            let saved = save_from(&mut folder, &mut desk, "desk");
            let Saved::ConflictCopy { name, cause: Error::Files(_) } = saved else { panic!("{saved:?}") };
            folder.locked = None;
            name
        } else {
            let mut offline = Folder::default();
            offline.put(NAME, opened_bytes());
            save_from(&mut offline, &mut desk, "desk");
            let name = "report (desk's conflicted copy 2026-11-14).let";
            folder.put(name, offline.files[NAME].clone());
            name.to_owned()
        };
        assert_eq!(folder.files[NAME], original, "{by}: the original is left alone");
        assert_eq!(conflict_copy_of(&copy).as_deref(), Some(NAME), "{by}");
        assert_eq!(open(&folder.files[&copy]).manifest().id, laptop.id, "{by}: the copy is of this document");

        laptop.merge_from(&mut open(&folder.files[&copy])).unwrap();
        assert_eq!(text(laptop.model().doc()), "Oh. Hello world!", "{by}");
        assert_eq!(covered(&laptop, "desk"), "world", "{by}");
        assert!(matches!(save_from(&mut folder, &mut laptop, "laptop"), Saved::Written), "{by}");
        assert!(open(&folder.files[NAME]).read("assets/desk.png").unwrap().is_some(), "{by}");

        // The copy holds nothing more now, so merging it again changes nothing, and neither does the desk.
        laptop.merge_from(&mut open(&folder.files[&copy])).unwrap();
        for (copy, device) in [(&mut laptop, "laptop"), (&mut desk, "desk")] {
            let saved = save_from(&mut folder, copy, device);
            assert!(matches!(saved, Saved::Unchanged), "{by}, {device}: {saved:?}");
        }
        assert!(level(&desk, &laptop), "{by}");
    }
}

/// The file [`shared`] starts from.
fn opened_bytes() -> Vec<u8> {
    let mut folder = Folder::default();
    let mut first = WorkingCopy::new(ID, "2026-11-14T20:00:00Z", document(), ReplicaId::from(1));
    save_from(&mut folder, &mut first, "first");
    folder.files.remove(NAME).unwrap()
}

#[test]
fn a_file_without_a_usable_history_still_merges() {
    // `package` holds a history no build can read.
    let mut bare = package();
    bare.entries.remove(OPLOG);
    let cases = [("no history", bytes(&bare)), ("an unreadable history", bytes(&package()))];
    for (why, file) in cases {
        let (mut folder, mut desk, mut laptop) = opened(file);
        typed(&mut desk, 0, "Oh. ");
        typed(&mut laptop, 5, "!");
        let saves = in_turn(&mut folder, &mut desk, &mut laptop, 3);
        assert!(saves.iter().all(|saved| matches!(saved, Saved::Written | Saved::Unchanged)), "{why}: {saves:?}");
        assert_eq!(text(open_at(&folder, NAME, 4).model().doc()), "Oh. Hello!", "{why}");
        assert!(level(&desk, &laptop), "{why}");
        assert_eq!(folder.names(), [NAME], "{why}");
    }
}

#[test]
fn a_saved_file_opens_with_its_history() {
    let (mut folder, mut desk, _) = shared();
    typed(&mut desk, 0, "Oh. ");
    // A damaged file is replaced from memory, history and all.
    for damaged in [false, true] {
        if damaged {
            folder.put(NAME, b"cut short".to_vec());
        }
        let saved = save_from(&mut folder, &mut desk, "desk");
        assert!(matches!((damaged, &saved), (false, Saved::Written) | (true, Saved::Restored { .. })), "{saved:?}");
        let reopened = open_at(&folder, NAME, 4);
        assert!(level(&reopened, &desk), "damaged: {damaged}");
        assert_eq!(text(reopened.model().doc()), "Oh. Hello world");
    }
}

#[test]
fn a_file_that_cannot_be_merged_is_left_alone() {
    let rewritten = |edit: fn(&mut Package)| {
        let mut package = open(&opened_bytes()).package().unwrap();
        edit(&mut package);
        bytes(&package)
    };
    let cases = [
        // Its history no longer leads to its document.
        ("changed by another tool", rewritten(|package| package.document.blocks.push(paragraph("q", "Added")))),
        (
            "another document without a history",
            rewritten(|package| {
                package.document.blocks = vec![paragraph("x", "Something else")];
                package.entries.remove(OPLOG);
            }),
        ),
    ];
    for (why, theirs) in cases {
        let (mut folder, mut desk, _) = shared();
        typed(&mut desk, 0, "Oh. ");
        folder.put(NAME, theirs.clone());

        let saved = save_from(&mut folder, &mut desk, "desk");
        let Saved::ConflictCopy { name, cause: Error::Unmerged } = saved else { panic!("{why}: {saved:?}") };
        assert_eq!(folder.files[NAME], theirs, "{why}: the file is left alone");
        assert_eq!(text(open_at(&folder, &name, 4).model().doc()), "Oh. Hello world", "{why}");
    }
}

#[test]
fn a_style_changed_on_one_device_reaches_the_other() {
    fn wide(style: &mut Style) {
        style.page.margin_left = 99.0;
    }
    fn tall(style: &mut Style) {
        style.page.margin_top = 77.0;
    }
    type Edit = Option<fn(&mut Style)>;
    // What each device does to the style, and the change both must end up with. When both change it there is
    // no telling which was last, so they only have to agree.
    let cases: [(Edit, Edit, Edit); 3] =
        [(Some(wide), None, Some(wide)), (None, Some(tall), Some(tall)), (Some(wide), Some(tall), None)];
    for (case, (at_desk, at_laptop, expected)) in cases.into_iter().enumerate() {
        let (mut folder, mut desk, mut laptop) = shared();
        let mut changed = desk.style.clone();
        at_desk.inspect(|edit| edit(&mut desk.style));
        at_laptop.inspect(|edit| edit(&mut laptop.style));
        in_turn(&mut folder, &mut desk, &mut laptop, 4);

        assert_eq!(desk.style, laptop.style, "case {case}");
        assert_eq!(open(&folder.files[NAME]).style().unwrap(), desk.style, "case {case}");
        if let Some(edit) = expected {
            edit(&mut changed);
            assert_eq!(desk.style, changed, "case {case}");
        }
    }
}

#[test]
fn a_document_changed_by_another_tool_opens_as_written_with_its_history_kept() {
    let (folder, desk, _) = shared();
    let mut package = open(&folder.files[NAME]).package().unwrap();
    package.document.blocks[0] = paragraph("p", "Hello there world");
    package.document.blocks.push(paragraph("q", "Added"));
    let mut edited = Folder::default();
    edited.put(NAME, bytes(&package));

    let mut reopened = open_at(&edited, NAME, 4);
    assert_eq!(reopened.model().doc(), &package.document, "the document is what the file says");
    let held = reopened.model().compare(&desk.model().save()).unwrap();
    assert_eq!(held, Some(Ordering::Greater), "the change is added to the history the file had");

    assert!(matches!(save_from(&mut edited, &mut reopened, "desk"), Saved::Written));
    assert!(level(&open_at(&edited, NAME, 5), &reopened), "and the next open finds nothing more to take in");
}
