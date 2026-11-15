//! Two devices on one document: the failure table of the design, played out on a folder in memory.

mod common;

use std::cmp::Ordering;

use common::{Folder, ID, bytes, open, package, paragraph};
use letters_format::{Error, NOTES, OPLOG, Reading, STYLE, Save, Saved, WorkingCopy, conflict_copy_of, save};
use letters_model::{
    Anchor, Block, BlockId, BlockKind, Body, Document, Note, NoteKind, Op, Pos, ReplicaId, Run, Style, Version,
};

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
    sharing(document())
}

/// A folder that holds `document` as saved, and two devices that have it open.
fn sharing(document: Document) -> (Folder, WorkingCopy, WorkingCopy) {
    let mut folder = Folder::default();
    let mut first = WorkingCopy::new(ID, "2026-11-14T20:00:00Z", document, Style::default(), ReplicaId::from(1));
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

fn restyle(copy: &mut WorkingCopy, edit: impl Fn(&mut Style)) {
    let mut style = copy.style().clone();
    edit(&mut style);
    copy.set_style(style).unwrap();
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

/// The two versions each open conflict note of `copy` holds, as who wrote what. A side that took the block
/// away wrote nothing.
fn clashes(copy: &WorkingCopy) -> Vec<[(String, Option<String>); 2]> {
    let side = |version: &Version| (version.author.clone(), version.block.as_ref().map(|block| block.plain_text()));
    let versions = |note: &Note| match &note.kind {
        NoteKind::Conflict { versions } if !note.resolved => Some([side(&versions[0]), side(&versions[1])]),
        _ => None,
    };
    copy.notes.notes.iter().filter_map(versions).collect()
}

fn wrote(author: &str, text: &str) -> (String, Option<String>) {
    (author.to_owned(), Some(text.to_owned()))
}

/// The time on a writer's clock, so many minutes into the evening.
fn at(minutes: u64) -> u64 {
    1_794_000_000 + minutes * 60
}

/// The desk and the laptop of [`shared`], after Ana and then Ben each changed "world" on their own.
fn clashing() -> (Folder, WorkingCopy, WorkingCopy) {
    let (folder, mut desk, mut laptop) = shared();
    desk.sign("Ana", at(0));
    typed(&mut desk, 11, "s");
    laptop.sign("Ben", at(10));
    typed(&mut laptop, 6, "under");
    (folder, desk, laptop)
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
fn one_word_changed_by_two_writers_reads_as_the_later_edit_with_a_note_of_both() {
    let (mut folder, mut desk, mut laptop) = clashing();
    typed(&mut laptop, 0, "Oh. ");
    let saves = in_turn(&mut folder, &mut desk, &mut laptop, 3);
    assert!(saves.iter().all(|saved| matches!(saved, Saved::Written | Saved::Unchanged)), "{saves:?}");

    let file = open_at(&folder, NAME, 4);
    for (who, copy) in [("the file", &file), ("the desk", &desk), ("the laptop", &laptop)] {
        assert_eq!(text(copy.model().doc()), "Oh. Hello underworld", "{who}: the later edit stands whole");
        let found = clashes(copy);
        assert_eq!(found, [[wrote("Ana", "Hello worlds"), wrote("Ben", "Oh. Hello underworld")]], "{who}");
        assert_eq!(copy.notes.notes[0].anchor.block.as_str(), "p", "{who}");
    }

    // A note that is picked on one device stays picked, and nothing raises it again.
    desk.notes.notes[0].resolved = true;
    let saves = in_turn(&mut folder, &mut desk, &mut laptop, 6);
    assert!(matches!(saves[..], [.., Saved::Unchanged, Saved::Unchanged]), "{saves:?}");
    for (who, copy) in [("the desk", &desk), ("the laptop", &laptop)] {
        assert!(clashes(copy).is_empty(), "{who}: {:?}", copy.notes.notes);
        assert_eq!(copy.notes.notes.len(), 1, "{who}");
    }
}

#[test]
fn devices_that_find_the_same_clash_apart_settle_it_alike() {
    let (_, mut desk, mut laptop) = clashing();
    let (mut hers, mut his) = (Folder::default(), Folder::default());
    save_from(&mut hers, &mut desk, "desk");
    save_from(&mut his, &mut laptop, "laptop");
    desk.merge_from(&mut open(&his.files[NAME])).unwrap();
    laptop.merge_from(&mut open(&hers.files[NAME])).unwrap();

    assert_eq!(clashes(&desk).len(), 1, "{:?}", desk.notes.notes);
    assert_eq!(desk.notes, laptop.notes);
    assert!(level(&desk, &laptop), "each wrote the same change to settle it");
    // Each then meets the other's note, which must not become a second one.
    let (mut folder, _, _) = shared();
    let saves = in_turn(&mut folder, &mut desk, &mut laptop, 4);
    assert!(matches!(saves[..], [.., Saved::Unchanged, Saved::Unchanged]), "{saves:?}");
    assert_eq!(clashes(&laptop).len(), 1);
    assert_eq!(desk.notes, laptop.notes);
}

#[test]
fn edits_that_arrive_without_their_file_settle_as_two_files_do() {
    // The same clash twice: met through a file, and through the edits alone.
    let (_, mut by_file, mut laptop) = clashing();
    let mut his = Folder::default();
    save_from(&mut his, &mut laptop, "laptop");
    by_file.merge_from(&mut open(&his.files[NAME])).unwrap();

    let (_, mut by_edits, laptop) = clashing();
    let (ours, theirs) = (by_edits.model().revision(), laptop.model().revision());
    by_edits.edit(&laptop.model().save()).unwrap();
    assert!(clashes(&by_edits).is_empty(), "edits alone merge without a word");
    by_edits.settle(&ours, &theirs).unwrap();

    assert_eq!(clashes(&by_edits), [[wrote("Ana", "Hello worlds"), wrote("Ben", "Hello underworld")]]);
    assert_eq!(text(by_edits.model().doc()), "Hello underworld");
    assert_eq!(by_edits.notes, by_file.notes);
    assert!(level(&by_edits, &by_file), "each wrote the same change to settle it");
}

#[test]
fn a_file_read_ahead_merges_as_one_read_at_the_merge() {
    // The same clash twice, with a picture only the laptop's file holds. The desk writes on before the two
    // meet: once after the file was read, as a host does that reads a file before it holds the document.
    let meet = |ahead: bool| {
        let (_, mut desk, mut laptop) = clashing();
        laptop.entries.insert("assets/laptop.png".into(), vec![1; 16]);
        let mut his = Folder::default();
        save_from(&mut his, &mut laptop, "laptop");
        let mut file = open(&his.files[NAME]);
        if ahead {
            let read = Reading::of(&mut file, ReplicaId::from(9)).unwrap();
            typed(&mut desk, 0, "Oh. ");
            desk.merge_read(read).unwrap();
        } else {
            typed(&mut desk, 0, "Oh. ");
            desk.merge_from(&mut file).unwrap();
        }
        desk
    };
    let (ahead, at_once) = (meet(true), meet(false));

    assert_eq!(clashes(&ahead), [[wrote("Ana", "Oh. Hello worlds"), wrote("Ben", "Hello underworld")]]);
    assert_eq!(clashes(&ahead), clashes(&at_once));
    assert_eq!(text(ahead.model().doc()), text(at_once.model().doc()));
    assert!(ahead.entries.contains_key("assets/laptop.png"), "{:?}", ahead.entries.keys());
}

#[test]
fn a_block_that_clashes_again_keeps_only_the_newer_note_open() {
    let (mut folder, mut desk, mut laptop) = clashing();
    in_turn(&mut folder, &mut desk, &mut laptop, 3);
    // Both go on in the same word without picking.
    desk.sign("Ana", at(20));
    typed(&mut desk, 16, "s");
    laptop.sign("Ben", at(30));
    typed(&mut laptop, 6, "the ");
    typed(&mut laptop, 15, "-");
    let saves = in_turn(&mut folder, &mut desk, &mut laptop, 5);
    assert!(matches!(saves[..], [.., Saved::Unchanged, Saved::Unchanged]), "{saves:?}");

    for (who, copy) in [("the desk", &desk), ("the laptop", &laptop)] {
        assert_eq!(text(copy.model().doc()), "Hello the under-world", "{who}");
        let found = clashes(copy);
        assert_eq!(found, [[wrote("Ana", "Hello underworlds"), wrote("Ben", "Hello the under-world")]], "{who}");
        assert_eq!(copy.notes.notes.len(), 2, "{who}: the older note is kept, resolved");
    }
}

#[test]
fn devices_that_merged_at_different_points_keep_one_note_open() {
    let (_, mut desk, mut laptop) = clashing();
    let (mut hers, mut his) = (Folder::default(), Folder::default());
    save_from(&mut hers, &mut desk, "desk");
    save_from(&mut his, &mut laptop, "laptop");
    // The desk meets what the laptop saved. The laptop writes on in the word before it meets the desk.
    desk.merge_from(&mut open(&his.files[NAME])).unwrap();
    laptop.sign("Ben", at(20));
    typed(&mut laptop, 16, "ly");
    laptop.merge_from(&mut open(&hers.files[NAME])).unwrap();

    let (mut folder, _, _) = shared();
    let saves = in_turn(&mut folder, &mut desk, &mut laptop, 6);
    assert!(matches!(saves[..], [.., Saved::Unchanged, Saved::Unchanged]), "{saves:?}");
    for (who, copy) in [("the desk", &desk), ("the laptop", &laptop)] {
        assert_eq!(text(copy.model().doc()), "Hello underworldly", "{who}");
        assert_eq!(clashes(copy), [[wrote("Ana", "Hello worlds"), wrote("Ben", "Hello underworldly")]], "{who}");
    }
}

#[test]
fn a_block_one_writer_removed_and_the_other_changed_goes_the_later_way_with_a_note() {
    // When Ana removes the paragraph, against Ben changing its text ten minutes in.
    for (removed, stands) in [(0, true), (20, false)] {
        let (mut folder, mut desk, mut laptop) = shared();
        desk.sign("Ana", at(removed));
        desk.apply([Op::RemoveBlock { block: "p".into() }]).unwrap();
        laptop.sign("Ben", at(10));
        typed(&mut laptop, 6, "under");
        let saves = in_turn(&mut folder, &mut desk, &mut laptop, 5);
        assert!(matches!(saves[..], [.., Saved::Unchanged, Saved::Unchanged]), "{saves:?}");

        let (gone, kept) = (("Ana".to_owned(), None), wrote("Ben", "Hello underworld"));
        let expected = if stands { [gone, kept] } else { [kept, gone] };
        for (who, copy) in [("the desk", &desk), ("the laptop", &laptop)] {
            let block = copy.model().doc().block(&BlockId::from("p"));
            assert_eq!(block.is_some(), stands, "{who}, removed at {removed}");
            assert_eq!(clashes(copy), std::slice::from_ref(&expected), "{who}, removed at {removed}");
        }
    }
}

#[test]
fn a_note_on_a_block_that_went_says_where_it_stood() {
    let quote = |blocks| Block { body: Body::Blocks(blocks), ..Block::new("q", BlockKind::Quote) };
    let (above, below) = (paragraph("a", "Above"), paragraph("z", "Below"));
    let p = paragraph("p", "Hello world");
    type Place = (Option<&'static str>, Option<&'static str>);
    // The document, what Ana removes from it after Ben changed "p", and the block before and around "p".
    let cases: [(&str, Vec<Block>, &[&str], Place); 4] = [
        ("between two", vec![above.clone(), p.clone(), below.clone()], &["p"], (Some("a"), None)),
        ("with the one before", vec![above.clone(), p.clone(), below.clone()], &["a", "p"], (None, None)),
        ("in a quote", vec![above.clone(), quote(vec![below.clone(), p.clone()])], &["p"], (Some("z"), Some("q"))),
        ("with its quote", vec![above.clone(), quote(vec![below.clone(), p.clone()])], &["q"], (Some("a"), None)),
    ];
    for (why, blocks, removed, (after, parent)) in cases {
        let (mut folder, mut desk, mut laptop) = sharing(Document { blocks, ..Document::default() });
        laptop.sign("Ben", at(0));
        typed(&mut laptop, 6, "under");
        desk.sign("Ana", at(10));
        desk.apply(removed.iter().map(|block| Op::RemoveBlock { block: (*block).into() })).unwrap();
        let saves = in_turn(&mut folder, &mut desk, &mut laptop, 5);
        assert!(matches!(saves[..], [.., Saved::Unchanged, Saved::Unchanged]), "{why}: {saves:?}");

        for (who, copy) in [("the desk", &desk), ("the laptop", &laptop)] {
            let [note] = &copy.notes.notes[..] else { panic!("{why}, {who}: {:?}", copy.notes.notes) };
            assert_eq!(note.anchor.block.as_str(), "p", "{why}, {who}");
            let stood = note.anchor.stood.as_ref().unwrap_or_else(|| panic!("{why}, {who}: no place"));
            let ids = [&stood.after, &stood.parent].map(|id| id.as_ref().map(BlockId::as_str));
            assert_eq!(ids, [after, parent], "{why}, {who}");
        }
    }
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
    let mut first = WorkingCopy::new(ID, "2026-11-14T20:00:00Z", document(), Style::default(), ReplicaId::from(1));
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
fn a_file_that_shares_no_history_is_left_alone() {
    let other = Document { blocks: vec![paragraph("x", "Something else")], ..Document::default() };
    let mut bare = open(&opened_bytes()).package().unwrap();
    bare.document = other.clone();
    bare.entries.remove(OPLOG);
    let mut elsewhere = Folder::default();
    let mut unrelated = WorkingCopy::new(ID, "2026-11-14T20:00:00Z", other, Style::default(), ReplicaId::from(9));
    save_from(&mut elsewhere, &mut unrelated, "elsewhere");

    // Merging these would put one document after the other.
    let cases = [("without a history", bytes(&bare)), ("with a history of its own", elsewhere.files[NAME].clone())];
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
fn a_change_by_another_tool_is_merged_once_on_every_device() {
    let (mut folder, mut desk, mut laptop) = shared();
    // Another tool rewrites the document and the style, and leaves the history as it was.
    let mut package = open(&folder.files[NAME]).package().unwrap();
    package.document.blocks[0] = paragraph("p", "Hello there world");
    package.document.blocks.push(paragraph("q", "Added"));
    let mut style = Style::default();
    style.page.margin_left = 99.0;
    package.set_style(&style);
    folder.put(NAME, bytes(&package));

    typed(&mut desk, 0, "Oh. ");
    typed(&mut laptop, 11, "!");
    let saves = in_turn(&mut folder, &mut desk, &mut laptop, 3);
    assert!(saves.iter().all(|saved| matches!(saved, Saved::Written | Saved::Unchanged)), "{saves:?}");

    let file = open_at(&folder, NAME, 4);
    let blocks: Vec<String> = file.model().doc().walk().map(|block| block.plain_text()).collect();
    assert_eq!(blocks, ["Oh. Hello there world!", "Added"], "both devices took the change in, and it is there once");
    assert!(note_ids(&file).is_empty(), "nothing clashed");
    assert_eq!(file.style(), &style);
    assert!(level(&desk, &laptop));
    assert_eq!(folder.names(), [NAME]);
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
        let mut changed = desk.style().clone();
        at_desk.inspect(|edit| restyle(&mut desk, *edit));
        at_laptop.inspect(|edit| restyle(&mut laptop, *edit));
        in_turn(&mut folder, &mut desk, &mut laptop, 4);

        assert_eq!(desk.style(), laptop.style(), "case {case}");
        assert_eq!(&open(&folder.files[NAME]).style().unwrap(), desk.style(), "case {case}");
        if let Some(edit) = expected {
            edit(&mut changed);
            assert_eq!(desk.style(), &changed, "case {case}");
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

/// The file [`shared`] starts from, with one entry replaced by something no build reads.
fn with_unreadable(entry: &str) -> Vec<u8> {
    let mut package = open(&opened_bytes()).package().unwrap();
    package.entries.insert(entry.into(), b"from a newer build".to_vec());
    bytes(&package)
}

#[test]
fn an_entry_this_build_cannot_read_is_carried_over() {
    // The entry, and whether the file was opened before it became unreadable.
    for (entry, opened_before) in [(STYLE, false), (STYLE, true), (NOTES, false), (NOTES, true)] {
        let why = format!("{entry}, opened before: {opened_before}");
        let unreadable = with_unreadable(entry);
        let (mut folder, mut desk, _) = if opened_before { shared() } else { opened(unreadable.clone()) };
        folder.put(NAME, unreadable);
        assert_eq!(desk.style(), &Style::default(), "{why}: the built-in style stands in");

        typed(&mut desk, 0, "Oh. ");
        let saved = save_from(&mut folder, &mut desk, "desk");
        assert!(matches!(saved, Saved::Written), "{why}: {saved:?}");
        let mut file = open(&folder.files[NAME]);
        assert_eq!(text(&file.document().unwrap()), "Oh. Hello world", "{why}");
        assert_eq!(file.read(entry).unwrap().unwrap(), b"from a newer build", "{why}");
        assert_eq!(folder.names(), [NAME], "{why}");
    }
}

#[test]
fn a_style_changed_here_replaces_one_that_cannot_be_read() {
    let (mut folder, mut desk, _) = opened(with_unreadable(STYLE));
    restyle(&mut desk, |style| style.page.margin_left = 99.0);
    let saved = save_from(&mut folder, &mut desk, "desk");
    assert!(matches!(saved, Saved::Written), "{saved:?}");
    assert_eq!(&open(&folder.files[NAME]).style().unwrap(), desk.style());
}

#[test]
fn a_note_added_here_never_replaces_notes_that_cannot_be_read() {
    let original = with_unreadable(NOTES);
    let (mut folder, mut desk, _) = opened(original.clone());
    comment(&mut desk, "desk", 6..11);
    let saved = save_from(&mut folder, &mut desk, "desk");
    let Saved::ConflictCopy { name, cause: Error::Unmerged } = saved else { panic!("{saved:?}") };
    assert_eq!(folder.files[NAME], original, "the file is left alone");
    assert_eq!(note_ids(&open_at(&folder, &name, 4)), ["desk"]);
}

#[test]
fn a_file_copied_by_hand_becomes_its_own_document_with_its_first_edit() {
    const COPY: &str = "report copy.let";
    let copied = Save { name: COPY, token: "desk", label: "desk", attempts: 3 };
    let (mut folder, mut original, _) = shared();
    let before = folder.files[NAME].clone();
    folder.put(COPY, before.clone());

    let mut copy = open_at(&folder, COPY, 5);
    copy.fork("a-new-id");
    typed(&mut copy, 11, "!");
    let saved = save(&mut folder, &mut copy, &copied).unwrap();
    assert!(matches!(saved, Saved::Written), "{saved:?}");
    assert_eq!(folder.files[NAME], before, "the original is left alone");
    assert_eq!(folder.names(), [COPY, NAME], "and no conflict copy is written");

    let reopened = open_at(&folder, COPY, 6);
    assert_eq!((reopened.id.as_str(), reopened.created.as_str()), ("a-new-id", original.created.as_str()));
    assert_eq!(text(reopened.model().doc()), "Hello world!");
    assert!(matches!(save(&mut folder, &mut copy, &copied).unwrap(), Saved::Unchanged));

    // The history is kept up to the fork, so the two still merge when asked to.
    typed(&mut original, 0, "Oh. ");
    original.merge_from(&mut open(&folder.files[COPY])).unwrap();
    assert_eq!(text(original.model().doc()), "Oh. Hello world!");
    assert_eq!(original.id, ID);

    // From here on a file with the old id in the copy's place is another document's.
    folder.put(COPY, before.clone());
    typed(&mut copy, 0, "So. ");
    let saved = save(&mut folder, &mut copy, &copied).unwrap();
    let Saved::ConflictCopy { cause: Error::OtherDocument(id), .. } = saved else { panic!("{saved:?}") };
    assert_eq!((id.as_str(), &folder.files[COPY]), (ID, &before));
}

/// Histories with sound checksums that Loro cannot be trusted with, from the fuzzer's findings.
const PANICS: &[u8] = include_bytes!("../../../fuzz/corpus/history/a-history-loro-panics-on.bin");
const UNWRITABLE: &[u8] = include_bytes!("../../../fuzz/corpus/history/a-history-loro-cannot-write-out.bin");

/// The shared file as a writer left it with `history` for its history, and every hash sound.
fn with_history(history: &[u8]) -> Vec<u8> {
    let mut package = open(&opened_bytes()).package().unwrap();
    package.document.blocks[0] = paragraph("p", "Hello from elsewhere");
    package.entries.insert(OPLOG.into(), history.to_vec());
    bytes(&package)
}

#[test]
fn a_file_whose_history_cannot_be_read_is_set_aside_and_never_merged() {
    // A person's own document refuses what is no history. One that takes every file for a stranger's
    // also refuses what only a trial in a scratch document shows.
    let cases: [(&str, bool, &[u8]); 4] = [
        ("no history, own", false, b"no history at all"),
        ("no history, wary", true, b"no history at all"),
        ("Loro panics on it", true, PANICS),
        ("Loro cannot write it out", true, UNWRITABLE),
    ];
    for (why, wary, history) in cases {
        let (mut folder, mut desk, _) = shared();
        desk.wary = wary;
        typed(&mut desk, 0, "Oh. ");
        let broken = with_history(history);
        folder.put(NAME, broken.clone());

        let saved = save_from(&mut folder, &mut desk, "desk");
        let Saved::Restored { aside } = saved else { panic!("{why}: {saved:?}") };
        assert_eq!(folder.files[&aside], broken, "{why}: the file is kept as it was, under another name");
        assert_eq!(text(open_at(&folder, NAME, 4).model().doc()), "Oh. Hello world", "{why}");
        assert_eq!(text(desk.model().doc()), "Oh. Hello world", "{why}: nothing of it was merged");
    }
}

#[test]
fn wary_copies_merge_sound_files_and_edits_as_any_do() {
    let (mut folder, mut desk, mut laptop) = shared();
    (desk.wary, laptop.wary) = (true, true);
    typed(&mut laptop, 0, "Oh. ");
    // Edits from an editor of its own reach a copy as bytes too.
    let mut editor = letters_model::Model::load(&desk.model().save(), ReplicaId::from(12)).unwrap();
    let stood = editor.revision();
    editor.apply([Op::InsertText { at: Pos::new("p", 5), text: ",".into() }]).unwrap();
    desk.edit(&editor.updates_since(&stood)).unwrap();

    let saves = in_turn(&mut folder, &mut desk, &mut laptop, 5);
    assert!(matches!(saves[..], [.., Saved::Unchanged, Saved::Unchanged]), "{saves:?}");
    for (who, copy) in [("the desk", &desk), ("the laptop", &laptop)] {
        assert_eq!(text(copy.model().doc()), "Oh. Hello, world", "{who}");
    }

    // A word both then change is settled as between any two copies.
    desk.sign("Ana", at(0));
    typed(&mut desk, 16, "s");
    laptop.sign("Ben", at(10));
    typed(&mut laptop, 11, "under");
    let saves = in_turn(&mut folder, &mut desk, &mut laptop, 5);
    assert!(matches!(saves[..], [.., Saved::Unchanged, Saved::Unchanged]), "{saves:?}");
    for (who, copy) in [("the desk", &desk), ("the laptop", &laptop)] {
        assert_eq!(text(copy.model().doc()), "Oh. Hello, underworld", "{who}");
        assert_eq!(clashes(copy).len(), 1, "{who}");
    }
}

#[test]
fn bytes_from_a_stranger_that_cannot_be_read_are_refused_and_change_nothing() {
    let (folder, _, _) = shared();
    let mut folder = folder;
    let mut copy = WorkingCopy::open_wary(&mut open(&folder.files[NAME]), ReplicaId::from(7)).unwrap();
    assert!(copy.wary, "a copy opened from a stranger stays wary of what joins it");
    let before = copy.model().save();

    assert!(matches!(copy.edit(b"no edits at all"), Err(Error::History(_))));
    for (why, history) in [("no history", &b"no history at all"[..]), ("Loro panics on it", PANICS)] {
        folder.put("other.let", with_history(history));
        let merged = copy.merge_from(&mut open(&folder.files["other.let"]));
        assert!(matches!(merged, Err(Error::History(_))), "{why}: {merged:?}");
        let mut other = open(&folder.files["other.let"]);
        let read = Reading::of(&mut other, ReplicaId::from(9)).err();
        assert!(matches!(read, Some(Error::History(_))), "{why}, read ahead: {read:?}");
        assert_eq!(copy.model().compare(&before).unwrap(), Some(Ordering::Equal), "{why}");
        assert_eq!(text(copy.model().doc()), "Hello world", "{why}");

        // Opened for editing, such a file still gives its document: the history starts over.
        let opened = WorkingCopy::open_wary(&mut open(&folder.files["other.let"]), ReplicaId::from(8)).unwrap();
        assert_eq!(text(opened.model().doc()), "Hello from elsewhere", "{why}");
    }
}
