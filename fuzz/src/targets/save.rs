//! Several writers saving one file to a shared folder that misbehaves: no save fails, no one's work is
//! lost, and once the folder is quiet they all end on the document and the notes the file holds.

use std::io::Cursor;

use letters_format::{Archive, Replica, Save, Saved, WorkingCopy, save, write};
use letters_model::{Anchor, Model, Note, NoteKind, Notes, Run, Style};

use crate::edits::{ops, reread, sample, shown};
use crate::folder::Folder;
use crate::input::Script;

const NAME: &str = "doc.let";
const ID: &str = "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f00";
const WRITERS: usize = 3;
const STEPS: usize = 40;

fn archive(file: &[u8]) -> Option<Archive<Cursor<Vec<u8>>>> {
    Archive::open(Cursor::new(file.to_vec())).ok()
}

fn file_of(copy: &WorkingCopy) -> Vec<u8> {
    let mut file = Cursor::new(Vec::new());
    write(&mut file, &copy.package()).expect("a package can be written");
    file.into_inner()
}

fn store(folder: &mut Folder, copy: &mut WorkingCopy, token: &str, attempts: u32) -> Saved {
    let order = Save { name: NAME, token, label: token, attempts };
    save(folder, copy, &order).expect("a folder that takes every write is saved to")
}

fn comment(script: &mut Script, copy: &WorkingCopy, id: String) -> Option<Note> {
    let blocks: Vec<_> = copy.model().doc().walk().collect();
    let block = blocks.get(script.pick(blocks.len()))?;
    let text = block.plain_text();
    let ends: Vec<usize> = text.char_indices().map(|(at, _)| at).chain([text.len()]).collect();
    let (from, to) = (script.pick(ends.len()), script.pick(ends.len()));
    let anchor = match from < to {
        true => Anchor::range(block.id.clone(), ends[from]..ends[to]),
        false => Anchor::block(block.id.clone()),
    };
    Some(Note {
        id,
        kind: NoteKind::Comment,
        anchor,
        author: "Ana".into(),
        created: "2026-11-15T00:00:00Z".into(),
        resolved: script.flag(),
        content: vec![Run::plain("why?")],
        replies: Vec::new(),
        extra: Default::default(),
    })
}

pub fn run(data: &[u8]) {
    let mut script = Script::new(data);
    let mut folder = Folder::default();
    let mut first = WorkingCopy::new(ID, "2026-11-15T00:00:00Z", sample(), Style::default(), 1.into());
    assert!(matches!(store(&mut folder, &mut first, "new", 1), Saved::Written), "a new file was not written");
    let open = |folder: &Folder, replica: u64| {
        let mut file = archive(&folder.files[NAME]).expect("a saved file opens");
        WorkingCopy::open(&mut file, replica.into()).expect("a saved file opens")
    };
    let mut writers = [first, open(&folder, 2), open(&folder, 3)];
    // Whether the file has only ever held what these writers saved.
    let mut theirs = true;
    let (mut copies, mut comments) = (Vec::new(), Vec::new());

    for nth in 0..STEPS {
        if script.done() {
            break;
        }
        let (step, who) = (script.pick(16), script.pick(WRITERS));
        let token = format!("w{who}n{nth}");
        match step {
            0..=6 => {
                let copy = &mut writers[who];
                copy.sign(["Ana", "Ben", ""][who], 1_800_000_000 + (nth * (40 + who)) as u64);
                let _ = copy.apply(ops(&mut script, &shown(copy.model()), 2));
            }
            7 => {
                if let Some(note) = comment(&mut script, &writers[who], token.clone()) {
                    writers[who].add_note(note);
                    comments.push(token);
                }
            }
            8..=11 => {
                match script.pick(6) {
                    0 => folder.locked = Some(NAME.into()),
                    // Another writer's save lands between this one's read and its rename.
                    1 => folder.racing = Some((NAME.into(), file_of(&writers[(who + 1) % WRITERS]))),
                    _ => {}
                }
                let attempts = 1 + script.pick(3) as u32;
                if let Saved::ConflictCopy { name, .. } = store(&mut folder, &mut writers[who], &token, attempts) {
                    copies.push(name);
                }
                (folder.locked, folder.racing) = (None, None);
            }
            // Another tool rewrites the document in the file, and knows nothing of its history.
            12 => {
                let Some(mut package) = archive(&folder.files[NAME]).and_then(|mut file| file.package().ok()) else {
                    continue;
                };
                let mut outside = Model::new(package.document, 98.into());
                let _ = outside.apply(ops(&mut script, &shown(&outside), 3));
                package.document = outside.doc().clone();
                let mut file = Cursor::new(Vec::new());
                write(&mut file, &package).expect("a package can be written");
                folder.put(NAME, file.into_inner());
                theirs = false;
            }
            // The file is cut short, as a sync tool leaves one it has not finished.
            13 => {
                let file = folder.files[NAME].clone();
                folder.put(NAME, file[..script.pick(file.len())].to_vec());
                theirs = false;
            }
            // A conflict copy is merged back in.
            _ => {
                let copy = copies.get(script.pick(copies.len())).and_then(|name| archive(&folder.files[name]));
                if let Some(mut copy) = copy {
                    writers[who].merge_from(&mut copy).expect("a conflict copy of this document merges");
                }
            }
        }
    }

    // The folder is quiet from here on. What was set aside is merged back, and everyone saves until
    // no save finds anything new.
    for name in &copies {
        let mut copy = archive(&folder.files[name]).expect("a conflict copy opens");
        writers[0].merge_from(&mut copy).expect("a conflict copy of this document merges");
    }
    let mut all = Model::load(&writers[0].model().save(), 90.into()).expect("a saved history loads");
    for writer in &writers[1..] {
        all.merge(&writer.model().save()).expect("a replica reads another's history");
    }
    let mut rounds = 0;
    loop {
        let mut quiet = true;
        for (who, writer) in writers.iter_mut().enumerate() {
            match store(&mut folder, writer, &format!("w{who}r{rounds}"), 3) {
                Saved::Unchanged => {}
                Saved::Written | Saved::Restored { .. } => quiet = false,
                saved => panic!("a save to a quiet folder ended as {saved:?}"),
            }
        }
        if quiet {
            break;
        }
        rounds += 1;
        assert!(rounds < 6, "the writers never stop finding something new to save");
    }

    let [first, second, third] = &writers;
    assert_eq!(shown(first.model()), shown(second.model()), "two writers ended on different documents");
    assert_eq!(shown(first.model()), shown(third.model()), "two writers ended on different documents");
    assert_eq!(shown(first.model()), reread(first.model()), "the document drifted from its history");
    // A block two writers changed in the same words is settled as the later left it, and a note says so.
    // Where no save had to do that, the file holds what the writers wrote and nothing else.
    let settled = first.notes.notes.iter().any(|note| matches!(note.kind, NoteKind::Conflict { .. }));
    if theirs && !settled {
        assert_eq!(shown(first.model()), shown(&all), "saving lost or added something the writers did not write");
    }
    // Notes are compared as they are stored, since each writer ties them to its own history.
    let notes = first.notes.to_json();
    assert_eq!(notes, second.notes.to_json(), "two writers ended on different notes");
    assert_eq!(notes, third.notes.to_json(), "two writers ended on different notes");
    for id in &comments {
        assert!(first.notes.notes.iter().any(|note| note.id == *id), "the comment {id} was lost");
    }
    let stored = Notes::from_json(&notes).expect("the notes that were written are read");
    assert_eq!(stored.to_json(), notes, "the notes read back differently than they were written");
    let reopened = open(&folder, 91);
    assert_eq!(shown(reopened.model()), shown(first.model()), "the file holds another document than its writers");
    assert_eq!(reopened.notes.to_json(), notes, "the file holds other notes than its writers");
}

pub fn seeds() -> Vec<Vec<u8>> {
    let edit = [0, 0, 1, 0, 2, 1, 3];
    let mut seeds: Vec<Vec<u8>> = (7..16u8)
        .map(|step| [&edit[..], &[8, 1, 5, 0], &[step, 1, 2, 3, 4, 5, 6, 7], &[8, 0, 5, 0]].concat())
        .collect();
    seeds.push((0..=255).rev().collect());
    seeds
}
