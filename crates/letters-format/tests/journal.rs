//! A writer's journal: what comes back in the session after a crash, and when the journal lets go of it.

mod common;

use common::{Folder, ID, open, paragraph};
use letters_format::{Error, Journal, Replayed, Save, Saved, Unsaved, WorkingCopy, is_journal, journal_name, save};
use letters_model::{
    Anchor, BlockId, Document, Model, Note, NoteKind, Notes, Op, Pos, ReplicaId, Revision, Run, Style,
};

const NAME: &str = "report.let";
const CREATED: &str = "2026-11-14T20:00:00Z";
const SAVE: Save = Save { name: NAME, token: "t1", label: "desk", attempts: 3 };

fn document() -> Document {
    Document { blocks: vec![paragraph("p", "Hello world")], ..Document::default() }
}

/// A shared folder that holds the document, saved once.
fn shared() -> Folder {
    let mut folder = Folder::default();
    let mut first = WorkingCopy::new(ID, CREATED, document(), Style::default(), ReplicaId::from(1));
    save(&mut folder, &mut first, &SAVE).unwrap();
    folder
}

fn text(document: &Document) -> String {
    document.block(&BlockId::from("p")).unwrap().plain_text()
}

fn note(id: &str) -> Notes {
    let note = Note {
        id: id.into(),
        kind: NoteKind::Comment,
        anchor: Anchor::block("p"),
        author: "Ana".into(),
        created: CREATED.into(),
        resolved: false,
        content: vec![Run::plain("Is this right?")],
        replies: Vec::new(),
        extra: Default::default(),
    };
    Notes { notes: vec![note], ..Notes::default() }
}

/// A document open in a window, as the shell holds it: the editor's model, the copy saves are made from,
/// and the journal in the writer's own state folder.
struct Session {
    editor: Model,
    copy: WorkingCopy,
    journal: Journal,
    /// What the editor had when it last wrote to the journal.
    journalled: Revision,
    /// Whether the journal held something the file lacked when the document was opened.
    recovered: bool,
}

impl Session {
    /// Opens the document in `folder`, taking in whatever an earlier session left in `state`.
    fn open(folder: &Folder, state: &mut Folder, replica: u64) -> Self {
        let mut copy = WorkingCopy::open(&mut open(&folder.files[NAME]), ReplicaId::from(replica)).unwrap();
        let (journal, unsaved) = Journal::open(state, &copy.id, &copy.created, Some(NAME)).unwrap();
        let recovered = unsaved.is_some_and(|unsaved| copy.replay(&unsaved).unwrap().whole().unwrap());
        let editor = Model::load(&copy.model().save(), ReplicaId::from(replica + 100)).unwrap();
        Self { journalled: editor.revision(), editor, copy, journal, recovered }
    }

    /// Types at the start of the paragraph, and journals it as the shell does a moment later.
    fn write(&mut self, state: &mut Folder, text: &str) {
        self.editor.apply([Op::InsertText { at: Pos::new("p", 0), text: text.into() }]).unwrap();
        self.journal.edits(state, &self.editor.updates_since(&self.journalled)).unwrap();
        self.journalled = self.editor.revision();
    }

    /// Saves as an autosave does, and tells the journal how it went.
    fn save(&mut self, folder: &mut Folder, state: &mut Folder) -> Saved {
        let mark = self.journal.mark();
        self.copy.edit(&self.editor.save()).unwrap();
        let saved = save(folder, &mut self.copy, &SAVE).unwrap();
        self.journal.saved(state, mark, &saved).unwrap();
        saved
    }
}

/// The document of a journal that holds one whole.
fn recovered(unsaved: &Unsaved) -> (WorkingCopy, Replayed) {
    WorkingCopy::recover(unsaved, ReplicaId::from(3)).unwrap().expect("the journal stands alone")
}

/// Edits the model refuses, by what is wrong with them. The histories are ones fuzzing found.
const REFUSED: [(&str, &[u8]); 3] = [
    ("no history", b"not a history"),
    (
        "edits Loro panics on as they land",
        include_bytes!("../../../fuzz/corpus/history/a-first-edit-that-builds-on-one-before-it.bin"),
    ),
    ("a history Loro overflows on", include_bytes!("../../../fuzz/corpus/history/a-history-whose-mark-overflows.bin")),
];

#[test]
fn a_document_that_never_reached_a_file_comes_back_from_its_journal() {
    let mut state = Folder::default();
    let mut editor = Model::new(document(), ReplicaId::from(2));
    let (mut journal, unsaved) = Journal::open(&mut state, ID, CREATED, None).unwrap();
    assert_eq!(unsaved, None);
    assert!(state.names().is_empty(), "nothing is written before there is something to record");

    // With no file to build on, the first edits are the whole history.
    editor.apply([Op::InsertText { at: Pos::new("p", 0), text: "A title. ".into() }]).unwrap();
    journal.edits(&mut state, &editor.save()).unwrap();
    let journalled = editor.revision();
    editor.apply([Op::InsertText { at: Pos::new("p", 0), text: "Draft: ".into() }]).unwrap();
    journal.edits(&mut state, &editor.updates_since(&journalled)).unwrap();
    journal.asset(&mut state, "assets/a.png", &[1; 8]).unwrap();
    journal.notes(&mut state, &note("n")).unwrap();

    // The session ends here, and all that is left is the state folder.
    let names: Vec<String> = state.names().into_iter().map(str::to_owned).collect();
    assert!(matches!(names.as_slice(), [name] if is_journal(name)), "{names:?}");
    let unsaved = Unsaved::read(&mut state, &names[0]).unwrap().unwrap();
    assert_eq!((unsaved.id.as_str(), unsaved.created.as_str(), unsaved.home.as_deref()), (ID, CREATED, None));

    let (copy, replayed) = recovered(&unsaved);
    assert_eq!(replayed.refused, None);
    assert_eq!(text(copy.model().doc()), "Draft: A title. Hello world");
    assert_eq!(copy.entries.get("assets/a.png"), Some(&vec![1; 8]));
    assert_eq!(copy.notes.notes.len(), 1);
}

#[test]
fn a_home_chosen_before_the_first_write_keeps_the_journal_whole() {
    let mut state = Folder::default();
    let mut editor = Model::new(document(), ReplicaId::from(2));
    let (mut journal, _) = Journal::open(&mut state, ID, CREATED, None).unwrap();
    journal.moved(&mut state, NAME).unwrap();
    editor.apply([Op::InsertText { at: Pos::new("p", 0), text: "A title. ".into() }]).unwrap();
    journal.edits(&mut state, &editor.save()).unwrap();

    // The crash comes before the first save, so there is no file at that home to add to.
    let unsaved = Unsaved::read(&mut state, &journal_name(ID)).unwrap().unwrap();
    assert_eq!(unsaved.home.as_deref(), Some(NAME));
    let (copy, _) = recovered(&unsaved);
    assert_eq!(text(copy.model().doc()), "A title. Hello world");
}

#[test]
fn edits_made_since_the_last_save_come_back_and_leave_once_saved() {
    let (mut folder, mut state) = (shared(), Folder::default());
    let mut session = Session::open(&folder, &mut state, 2);
    assert!(!session.recovered);
    session.write(&mut state, "Oh. ");
    drop(session);

    let mut session = Session::open(&folder, &mut state, 3);
    assert!(session.recovered, "the journal held edits the file lacked");
    assert_eq!(text(session.copy.model().doc()), "Oh. Hello world");

    // A save that writes has not read the file back, so the journal lets go at the next one.
    let saves = [session.save(&mut folder, &mut state), session.save(&mut folder, &mut state)];
    assert!(matches!(saves, [Saved::Written, Saved::Unchanged]), "{saves:?}");
    assert!(session.journal.is_empty());
    assert!(state.names().is_empty(), "{:?}", state.names());
    assert!(!Session::open(&folder, &mut state, 4).recovered);
}

#[test]
fn a_save_that_lost_a_race_is_still_in_the_journal_after_a_crash() {
    let (mut folder, mut state) = (shared(), Folder::default());
    let stale = folder.files[NAME].clone();
    let mut session = Session::open(&folder, &mut state, 2);
    session.write(&mut state, "Oh. ");
    assert!(matches!(session.save(&mut folder, &mut state), Saved::Written));
    assert!(!session.journal.is_empty(), "written is not yet read back");

    // Another writer, who read the file before that save, replaces it. Then this session ends badly.
    folder.put(NAME, stale);
    drop(session);

    let mut session = Session::open(&folder, &mut state, 3);
    assert!(session.recovered);
    assert!(matches!(session.save(&mut folder, &mut state), Saved::Written));
    assert_eq!(text(&open(&folder.files[NAME]).document().unwrap()), "Oh. Hello world");
}

#[test]
fn edits_journalled_while_a_save_runs_outlive_it() {
    let (mut folder, mut state) = (shared(), Folder::default());
    let mut session = Session::open(&folder, &mut state, 2);
    session.write(&mut state, "one ");
    session.save(&mut folder, &mut state);

    // The confirming save is handed its edits, and typing goes on while it runs.
    let mark = session.journal.mark();
    session.write(&mut state, "two ");
    let saved = save(&mut folder, &mut session.copy, &SAVE).unwrap();
    assert!(matches!(saved, Saved::Unchanged), "{saved:?}");
    session.journal.saved(&mut state, mark, &saved).unwrap();
    assert!(!session.journal.is_empty());
    drop(session);

    let session = Session::open(&folder, &mut state, 3);
    assert!(session.recovered);
    assert_eq!(text(session.copy.model().doc()), "two one Hello world");
}

#[test]
fn a_torn_last_record_loses_only_itself() {
    let (folder, mut state) = (shared(), Folder::default());
    let mut session = Session::open(&folder, &mut state, 2);
    session.write(&mut state, "one ");
    session.write(&mut state, "two ");
    drop(session);
    // The crash came while the second record was being written.
    let name = journal_name(ID);
    let whole = state.files[&name].clone();
    state.put(&name, whole[..whole.len() - 3].to_vec());

    let mut session = Session::open(&folder, &mut state, 3);
    assert!(session.recovered);
    assert_eq!(text(session.copy.model().doc()), "one Hello world");

    // What is journalled after the tear can be read, so the tear is not left in the way.
    session.write(&mut state, "three ");
    drop(session);
    let session = Session::open(&folder, &mut state, 4);
    assert_eq!(text(session.copy.model().doc()), "three one Hello world");
}

#[test]
fn a_journal_is_only_replayed_into_the_document_it_was_kept_for() {
    let (folder, mut state) = (shared(), Folder::default());
    let mut session = Session::open(&folder, &mut state, 2);
    session.write(&mut state, "Oh. ");
    drop(session);
    let unsaved = Unsaved::read(&mut state, &journal_name(ID)).unwrap().unwrap();
    assert_eq!((unsaved.home.as_deref(), unsaved.whole), (Some(NAME), false));
    assert!(WorkingCopy::recover(&unsaved, ReplicaId::from(3)).unwrap().is_none());

    let other = Document { blocks: vec![paragraph("p", "Something else")], ..Document::default() };
    type Expected = fn(&Error) -> bool;
    let cases: [(&str, Expected); 2] = [
        ("another-document", |error| matches!(error, Error::OtherDocument(_))),
        // The same id, with a history the journal's edits do not build on.
        (ID, |error| matches!(error, Error::Unmerged)),
    ];
    for (id, expected) in cases {
        let mut copy = WorkingCopy::new(id, CREATED, other.clone(), Style::default(), ReplicaId::from(5));
        let error = copy.replay(&unsaved).expect_err(id);
        assert!(expected(&error), "{id}: {error:?}");
    }
}

#[test]
fn journal_names_are_safe_whatever_the_id() {
    let ids = [ID, "", "../../etc/passwd", "a/b\\c:d", "Zoë ✉️"];
    let names: Vec<String> = ids.iter().map(|id| journal_name(id)).collect();
    for (id, name) in ids.iter().zip(&names) {
        assert!(is_journal(name), "{id}: {name}");
        assert!(name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'.'), "{id}: {name}");
        assert_eq!(names.iter().filter(|other| *other == name).count(), 1, "{id}: each id has its own journal");
    }
    for name in ["report.let", ".journal", "notes.journal"] {
        assert!(!is_journal(name), "{name}");
    }
}

#[test]
fn a_copy_that_became_its_own_document_comes_back_under_its_new_id() {
    let (folder, mut state) = (shared(), Folder::default());
    // The file is a copy made by hand. Its first edit makes it a document of its own, before the journal
    // hears of the edit: the original's journal goes by the id the two share.
    let opened = WorkingCopy::open(&mut open(&folder.files[NAME]), ReplicaId::from(2)).unwrap();
    let mut editor = Model::load(&opened.model().save(), ReplicaId::from(102)).unwrap();
    let stood = editor.revision();
    let (mut journal, _) = Journal::open(&mut state, "a-new-id", CREATED, Some(NAME)).unwrap();
    editor.apply([Op::InsertText { at: Pos::new("p", 0), text: "Mine. ".into() }]).unwrap();
    journal.edits(&mut state, &editor.updates_since(&stood)).unwrap();

    // The session ends before a save. The file still carries the old id.
    let unsaved = Unsaved::read(&mut state, &journal_name("a-new-id")).unwrap().unwrap();
    let mut copy = WorkingCopy::open(&mut open(&folder.files[NAME]), ReplicaId::from(3)).unwrap();
    assert!(matches!(copy.replay(&unsaved), Err(Error::OtherDocument(_))), "not while it goes by the old id");
    copy.fork(unsaved.id.clone());
    assert!(copy.replay(&unsaved).unwrap().added);
    assert_eq!(text(copy.model().doc()), "Mine. Hello world");

    let mut folder = folder;
    assert!(matches!(save(&mut folder, &mut copy, &SAVE).unwrap(), Saved::Written));
    assert_eq!(open(&folder.files[NAME]).manifest().id, "a-new-id");
}

#[test]
fn a_journal_comes_back_up_to_the_first_edit_that_does_not_stand() {
    let cases = REFUSED.iter().flat_map(|refused| [(refused, false), (refused, true)]);
    for ((why, refused), wary) in cases {
        let (folder, mut state) = (shared(), Folder::default());
        let mut session = Session::open(&folder, &mut state, 2);
        session.write(&mut state, "one ");
        session.journal.edits(&mut state, refused).unwrap();
        // These build on nothing of the refused edits, so they would stand if they were tried.
        session.write(&mut state, "two ");
        session.journal.notes(&mut state, &note("n")).unwrap();
        drop(session);

        let unsaved = Unsaved::read(&mut state, &journal_name(ID)).unwrap().unwrap();
        let mut copy = WorkingCopy::open(&mut open(&folder.files[NAME]), ReplicaId::from(3)).unwrap();
        copy.wary = wary;
        let replayed = copy.replay(&unsaved).expect(why);
        assert_eq!(text(copy.model().doc()), "one Hello world", "{why}: nothing after it is taken in");
        assert_eq!((replayed.edits, replayed.of), (1, 3), "{why}");
        assert!(replayed.added && replayed.refused.is_some(), "{why}: {replayed:?}");
        assert_eq!(copy.notes.notes.len(), 1, "{why}: the notes come back");
        assert!(matches!(replayed.whole(), Err(Error::History(_))), "{why}");
    }
}

#[test]
fn a_document_with_no_file_comes_back_up_to_the_first_edit_that_does_not_stand() {
    for (why, refused) in REFUSED {
        let mut state = Folder::default();
        let mut editor = Model::new(document(), ReplicaId::from(2));
        let (mut journal, _) = Journal::open(&mut state, ID, CREATED, None).unwrap();
        editor.apply([Op::InsertText { at: Pos::new("p", 0), text: "A title. ".into() }]).unwrap();
        journal.edits(&mut state, &editor.save()).unwrap();
        journal.edits(&mut state, refused).unwrap();
        let journalled = editor.revision();
        editor.apply([Op::InsertText { at: Pos::new("p", 0), text: "Draft: ".into() }]).unwrap();
        journal.edits(&mut state, &editor.updates_since(&journalled)).unwrap();

        let unsaved = Unsaved::read(&mut state, &journal_name(ID)).unwrap().unwrap();
        let (copy, replayed) = recovered(&unsaved);
        assert_eq!(text(copy.model().doc()), "A title. Hello world", "{why}");
        assert_eq!((replayed.edits, replayed.of), (1, 3), "{why}");
        assert!(replayed.refused.is_some(), "{why}");
    }
}
