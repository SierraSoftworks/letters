//! A journal file with anything in its records: it is read or refused, what it kept is replayed or
//! refused, and a journal that opens on it goes on as any other.

use letters_format::{Journal, Saved, Unsaved, WorkingCopy, journal_name};
use letters_model::{Model, Style};
use sha2::{Digest, Sha256};

use super::history::sealed;
use crate::checks::sound;
use crate::edits::{ops, sample, shown};
use crate::folder::Folder;
use crate::input::{Script, put};

const RECORDS: usize = 12;
const PAYLOAD: usize = 16 * 1024;
const KINDS: [u8; 6] = *b"HUNAUX";

/// A record as a journal writes one: its kind, its length, its payload, and a sum of the three. The fuzzer
/// changes what a record says, where otherwise it would only ever find the sum.
fn record(kind: u8, payload: &[u8]) -> Vec<u8> {
    let length = (payload.len() as u32).to_le_bytes();
    let sum = Sha256::new().chain_update([kind]).chain_update(length).chain_update(payload).finalize();
    [&[kind][..], &length, payload, &sum[..8]].concat()
}

pub fn run(data: &[u8]) {
    let mut script = Script::new(data);
    let mut bytes = b"LETJ1\n".to_vec();
    for _ in 0..RECORDS {
        if script.done() {
            break;
        }
        let (kind, payload) = (script.one(&KINDS), script.bytes(PAYLOAD));
        // Edits are a history, sealed as every history is.
        let payload = if kind == b'U' { sealed(payload) } else { payload.to_vec() };
        bytes.extend(record(kind, &payload));
    }
    // A record cut off as it was written.
    bytes.extend_from_slice(script.rest());

    let mut state = Folder::default();
    state.put("found.journal", bytes.clone());
    let Ok(Some(unsaved)) = Unsaved::read(&mut state, "found.journal") else { return };
    let _ = WorkingCopy::recover(&unsaved, 5.into());
    let mut copy = WorkingCopy::new(&*unsaved.id, &*unsaved.created, sample(), Style::default(), 6.into());
    if let Ok(replayed) = copy.replay(&unsaved) {
        // A replay that stops part way says why, and what came back before it is a document like any other.
        assert_eq!(replayed.refused.is_some(), replayed.edits < replayed.of, "a replay stopped unsaid: {replayed:?}");
        sound(copy.model().doc(), "a document a journal was replayed onto");
    }

    // The same file under its document's name is the journal of that document.
    let name = journal_name(&unsaved.id);
    state.put(&name, bytes);
    let opened = Journal::open(&mut state, &unsaved.id, &unsaved.created, unsaved.home.as_deref());
    let (mut journal, kept) = opened.expect("a journal that was read opens for its own document");
    assert_eq!(kept.as_ref(), Some(&unsaved), "a journal that opens tells of other work than one that is read");
    journal.edits(&mut state, b"more").expect("a folder that takes every write is journalled to");
    let longer = Unsaved::read(&mut state, &name).expect("a journal that was added to reads");
    assert!(longer.is_some_and(|longer| longer != unsaved), "what was added to a journal is not read back");
    let mark = journal.mark();
    journal.saved(&mut state, mark, &Saved::Unchanged).expect("a folder that takes every write is journalled to");
    assert!(journal.is_empty() && !state.files.contains_key(&name), "a journal outlived the save that held it all");
}

/// Journals as a session writes them, spelled out as their records.
pub fn seeds() -> Vec<Vec<u8>> {
    let mut model = Model::new(sample(), 1.into());
    let script: Vec<u8> = (0..=255).collect();
    let mut script = Script::new(&script);
    let header =
        br#"{"format":1,"id":"8f14e45f","created":"2026-11-15T00:00:00Z","home":"/home/ana/a.let","whole":true}"#;
    let mut seed = vec![0];
    put(&mut seed, PAYLOAD, header);
    for kind in [1, 1, 2, 3, 0] {
        let payload = match kind {
            0 => br#"{"format":1,"id":"8f14e45f","created":"2026-11-15T00:00:00Z","whole":false}"#.to_vec(),
            1 => {
                let since = model.revision();
                let whole = model.save();
                let _ = model.apply(ops(&mut script, &shown(&model), 2));
                if seed.len() < 200 { whole } else { model.updates_since(&since) }
            }
            2 => include_bytes!("../../../crates/letters-format/corpus/notes/report.json").to_vec(),
            _ => [&7u16.to_le_bytes()[..], b"a/b.png", b"\x89PNG"].concat(),
        };
        seed.push(kind);
        put(&mut seed, PAYLOAD, &payload);
    }
    vec![seed]
}
