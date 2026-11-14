mod common;

use common::{Folder, ID, bytes, open, package, paragraph, rezip};
use letters_format::{DiskCopy, Error, Package, Replica, Save, Saved, conflict_copy_of, save};

const NAME: &str = "report.let";
const SAVE: Save = Save { name: NAME, token: "t1", label: "2026-11-14 desk", attempts: 3 };

/// A document in memory whose merge is a plain union: blocks by id, and history entries by name. Assets are
/// left on disk unread, as the app leaves them.
struct Memory(Package);

impl Replica for Memory {
    fn id(&self) -> &str {
        &self.0.id
    }

    fn merge(&mut self, disk: &mut DiskCopy) -> Result<(), Error> {
        for block in disk.document()?.blocks {
            if self.0.document.block(&block.id).is_none() {
                self.0.document.blocks.push(block);
            }
        }
        let history: Vec<String> = disk.names().filter(|name| name.starts_with("history/")).map(Into::into).collect();
        for name in history {
            let bytes = disk.read(&name)?.unwrap();
            self.0.entries.entry(name).or_insert(bytes);
        }
        Ok(())
    }

    fn package(&self) -> Package {
        self.0.clone()
    }
}

/// The same document as [`package`], as another writer has since changed it.
fn theirs() -> Package {
    let mut package = package();
    package.document.blocks.push(paragraph("theirs", "From the server"));
    package.entries.insert("assets/theirs.png".into(), vec![9; 32]);
    package.entries.insert("history/theirs".into(), vec![4, 5]);
    package
}

fn ours() -> Memory {
    let mut package = package();
    package.document.blocks.push(paragraph("ours", "From the desk"));
    package.entries.remove("assets/0a1b.png");
    Memory(package)
}

fn block_ids(package: &Package) -> Vec<&str> {
    package.document.blocks.iter().map(|block| block.id.as_str()).collect()
}

fn on_disk(folder: &Folder, name: &str) -> Package {
    open(&folder.files[name]).package().unwrap()
}

#[test]
fn a_first_save_writes_the_file_and_leaves_nothing_else() {
    let mut folder = Folder::default();
    let mut memory = Memory(package());
    let saved = save(&mut folder, &mut memory, &SAVE).unwrap();
    assert!(matches!(saved, Saved::Written), "{saved:?}");
    assert_eq!(folder.names(), [NAME]);
    assert_eq!(on_disk(&folder, NAME), package());
}

#[test]
fn a_save_that_adds_nothing_writes_nothing() {
    let mut folder = Folder::default();
    folder.put(NAME, bytes(&package()));
    let before = folder.changes();
    let saved = save(&mut folder, &mut Memory(package()), &SAVE).unwrap();
    assert!(matches!(saved, Saved::Unchanged), "{saved:?}");
    assert_eq!(folder.changes(), before);
}

#[test]
fn what_another_writer_saved_is_merged_and_kept() {
    let mut folder = Folder::default();
    folder.put(NAME, bytes(&theirs()));
    let mut memory = ours();
    let saved = save(&mut folder, &mut memory, &SAVE).unwrap();
    assert!(matches!(saved, Saved::Written), "{saved:?}");

    let file = on_disk(&folder, NAME);
    assert_eq!(block_ids(&file), ["p", "ours", "theirs"]);
    assert_eq!(block_ids(&memory.0), ["p", "ours", "theirs"]);
    // Their asset was never loaded into memory, and the one memory dropped is still on disk: both survive.
    assert_eq!(file.entries, theirs().entries);
    assert_eq!(folder.names(), [NAME]);
}

#[test]
fn a_writer_that_slips_in_mid_save_is_merged_on_the_retry() {
    let mut folder = Folder::default();
    folder.put(NAME, bytes(&package()));
    folder.after_write = Some(Box::new(|folder| folder.put(NAME, bytes(&theirs()))));
    let saved = save(&mut folder, &mut ours(), &SAVE).unwrap();
    assert!(matches!(saved, Saved::Written), "{saved:?}");
    assert_eq!(block_ids(&on_disk(&folder, NAME)), ["p", "ours", "theirs"]);
    assert_eq!(folder.names(), [NAME]);
}

#[test]
fn a_file_that_cannot_be_replaced_gets_a_conflict_copy() {
    let mut folder = Folder::default();
    let original = bytes(&theirs());
    folder.put(NAME, original.clone());
    folder.locked = Some(NAME.into());

    let mut copies = Vec::new();
    for _ in 0..2 {
        match save(&mut folder, &mut ours(), &SAVE).unwrap() {
            Saved::ConflictCopy { name, cause: Error::Files(_) } => copies.push(name),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(folder.files[NAME], original, "the original is left alone");
    assert_ne!(copies[0], copies[1], "a second copy never replaces the first");
    assert_eq!(folder.names().len(), 3, "no temporary files are left: {:?}", folder.names());
    for copy in &copies {
        assert_eq!(conflict_copy_of(copy).as_deref(), Some(NAME), "{copy}");
        assert_eq!(block_ids(&on_disk(&folder, copy)), ["p", "ours", "theirs"]);
    }
    assert!(folder.waits > 0, "the shell gets to wait between attempts");
}

#[test]
fn a_file_that_is_not_ours_to_replace_is_left_alone() {
    let other = Package { id: "another-document".into(), ..package() };
    let newer = rezip(&bytes(&package()), |name, content| {
        Some(if name == "manifest.json" { br#"{"format": 2}"#.to_vec() } else { content })
    });
    type Expected = fn(&Error) -> bool;
    let cases: [(Vec<u8>, Expected); 2] = [
        (bytes(&other), |cause| matches!(cause, Error::OtherDocument(id) if id == "another-document")),
        (newer, |cause| matches!(cause, Error::NewerFormat(2))),
    ];
    for (original, expected) in cases {
        let mut folder = Folder::default();
        folder.put(NAME, original.clone());
        let saved = save(&mut folder, &mut ours(), &SAVE).unwrap();
        let Saved::ConflictCopy { name, cause } = saved else { panic!("{saved:?}") };
        assert!(expected(&cause), "{cause:?}");
        assert_eq!(folder.files[NAME], original);
        assert_eq!(block_ids(&on_disk(&folder, &name)), ["p", "ours"]);
    }
}

/// A writer with no way to merge, as one without the file's history is.
struct Unmergeable(Package);

impl Replica for Unmergeable {
    fn id(&self) -> &str {
        &self.0.id
    }

    fn merge(&mut self, _disk: &mut DiskCopy) -> Result<(), Error> {
        Err(Error::Unmerged)
    }

    fn package(&self) -> Package {
        self.0.clone()
    }
}

#[test]
fn a_file_that_cannot_be_merged_is_left_alone() {
    let mut folder = Folder::default();
    let original = bytes(&theirs());
    folder.put(NAME, original.clone());

    let saved = save(&mut folder, &mut Unmergeable(ours().0), &SAVE).unwrap();
    let Saved::ConflictCopy { name, cause: Error::Unmerged } = saved else { panic!("{saved:?}") };
    assert_eq!(folder.files[NAME], original);
    let copy = on_disk(&folder, &name);
    assert_eq!(block_ids(&copy), ["p", "ours"]);
    assert!(copy.entries.contains_key("assets/theirs.png"), "the copy is whole: it takes the file's assets along");
    assert_eq!(folder.waits, 0, "waiting would not make it mergeable");
}

#[test]
fn a_damaged_file_is_set_aside_and_restored_from_memory() {
    let good = bytes(&package());
    let cases = [
        ("cut short", good[..good.len() / 2].to_vec()),
        ("not a zip", b"plain text".to_vec()),
        (
            "an entry that does not match its hash",
            rezip(&good, |name, content| Some(if name == "document.json" { b"{}".to_vec() } else { content })),
        ),
    ];
    for (why, damaged) in cases {
        let mut folder = Folder::default();
        folder.put(NAME, damaged.clone());
        let mut memory = Memory(package());
        let saved = save(&mut folder, &mut memory, &SAVE).unwrap();
        let Saved::Restored { aside } = saved else { panic!("{why}: {saved:?}") };
        assert_eq!(folder.files[&aside], damaged, "{why}: the damaged file is kept");
        assert_eq!(on_disk(&folder, NAME), package(), "{why}");
        assert_eq!(folder.names().len(), 2, "{why}: {:?}", folder.names());
    }
}

#[test]
fn a_file_that_is_still_syncing_is_waited_for() {
    let mut folder = Folder::default();
    let arriving = bytes(&theirs());
    folder.put(NAME, arriving[..arriving.len() / 2].to_vec());
    folder.at_wait = Some(Box::new(move |folder| folder.put(NAME, arriving)));
    let saved = save(&mut folder, &mut ours(), &SAVE).unwrap();
    assert!(matches!(saved, Saved::Written), "{saved:?}");
    assert_eq!(block_ids(&on_disk(&folder, NAME)), ["p", "ours", "theirs"]);
    assert_eq!(folder.names(), [NAME]);
}

#[test]
fn a_folder_that_takes_no_writes_fails_and_is_left_as_it_was() {
    let mut folder = Folder::default();
    folder.put(NAME, bytes(&theirs()));
    folder.read_only = true;
    let before = folder.changes();
    assert!(matches!(save(&mut folder, &mut ours(), &SAVE), Err(Error::Files(_))));
    assert_eq!(folder.changes(), before);
}

#[test]
fn conflict_copies_are_recognised_by_name() {
    let cases = [
        ("report (conflicted copy 2026-11-14 desk).let", Some("report.let")),
        ("report (conflicted copy 2026-11-14 desk 2).let", Some("report.let")),
        ("report (Ben's conflicted copy 2026-11-14).let", Some("report.let")),
        ("Q3 (final) (conflicted copy 2026-11-14 203000).let", Some("Q3 (final).let")),
        ("report.sync-conflict-20261114-203000-ABCDEFG.let", Some("report.let")),
        ("report.let", None),
        ("report (final).let", None),
        ("conflicted copy.let", None),
        (ID, None),
    ];
    for (name, original) in cases {
        assert_eq!(conflict_copy_of(name).as_deref(), original, "{name}");
    }
}
