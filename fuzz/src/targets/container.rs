//! A `.let` file from anywhere: it opens or is refused, and one that opens can be read, edited, saved
//! over itself and merged with itself without anything changing that should not.

use std::io::Cursor;

use letters_format::{Archive, MANIFEST, Save, Saved, WorkingCopy, hash, save, write};
use serde_json::Value;

use crate::archive::{Entries, entries, spelled, unzipped, zipped};
use crate::checks::sound;
use crate::edits::shown;
use crate::folder::Folder;
use crate::input::Script;

const NAME: &str = "a.let";

/// Makes the manifest record the hash of every entry as it now is, the way a writer of the format would,
/// so that changing an entry is not the same as damaging the file.
fn seal(entries: &mut Entries) {
    let hashes: serde_json::Map<String, Value> = (entries.iter())
        .filter(|(name, _)| name != "mimetype" && name != MANIFEST)
        .map(|(name, content)| (name.clone(), hash(content).into()))
        .collect();
    for (_, content) in entries.iter_mut().filter(|(name, _)| name == MANIFEST) {
        if let Ok(Value::Object(mut manifest)) = serde_json::from_slice(content) {
            manifest.insert("hashes".into(), hashes.clone().into());
            *content = Value::Object(manifest).to_string().into_bytes();
        }
    }
}

pub fn run(data: &[u8]) {
    let mut script = Script::new(data);
    let sealed = script.flag();
    let mut entries = entries(&mut script);
    if sealed {
        seal(&mut entries);
    }
    let file = zipped(&entries);
    let Ok(mut archive) = Archive::open(Cursor::new(file.clone())) else { return };
    let names: Vec<String> = archive.names().map(str::to_owned).collect();
    for name in &names {
        let _ = archive.read(name);
    }
    let _ = (archive.style(), archive.notes());
    if let Ok(package) = archive.package() {
        sound(&package.document, "a file's document");
        let mut written = Cursor::new(Vec::new());
        write(&mut written, &package).expect("a package that was read can be written");
        let again = Archive::open(Cursor::new(written.into_inner())).and_then(|mut archive| archive.package());
        assert_eq!(again.as_ref().ok(), Some(&package), "a file is written as another than was read");
    }

    let Ok(mut copy) = WorkingCopy::open(&mut archive, 7.into()) else { return };
    sound(copy.model().doc(), "an opened document");
    // Merging a file into itself adds nothing.
    let before = shown(copy.model());
    if copy.merge_from(&mut archive).is_ok() {
        assert_eq!(shown(copy.model()), before, "a file merged into itself changed the document");
    }

    // Saved over the file it came from, the document is on disk, and a second save finds nothing to write.
    let mut folder = Folder::default();
    folder.put(NAME, file);
    let order = Save { name: NAME, token: "fuzz", label: "fuzz", attempts: 2 };
    let saved = save(&mut folder, &mut copy, &order).expect("a folder that takes every write is saved to");
    if matches!(saved, Saved::Unchanged | Saved::Written) {
        let again = save(&mut folder, &mut copy, &order).expect("a folder that takes every write is saved to");
        assert!(matches!(again, Saved::Unchanged), "a second save found something to write: {again:?}");
        let mut archive = Archive::open(Cursor::new(folder.files[NAME].clone())).expect("a saved file opens");
        let reopened = WorkingCopy::open(&mut archive, 8.into()).expect("a saved file opens");
        assert_eq!(shown(reopened.model()), shown(copy.model()), "a saved file holds another document");
    }
}

pub fn seeds() -> Vec<Vec<u8>> {
    let report = unzipped(include_bytes!("../../../crates/letters-format/corpus/report.let"));
    // Once with its hashes as they are, and once to be sealed again.
    vec![[&[0][..], &spelled(&report)].concat(), [&[1][..], &spelled(&report)].concat()]
}
