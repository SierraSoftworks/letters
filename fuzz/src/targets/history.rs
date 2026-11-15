//! A history from anywhere: it is checked before a document reads it, as a server checks every history
//! and a shell those it does not trust. One that passes loads and merges, a model that takes it in still
//! tells one document, and a model that refuses it is left as it was.

use letters_model::Model;

use crate::checks::sound;
use crate::edits::{ops, reread, sample, shown};
use crate::input::{Script, bounded};

/// Where a history keeps the checksum of everything after it.
const SUM: std::ops::Range<usize> = 16..20;

/// Makes a history's checksum fit what follows it, as any writer of the format would. The fuzzer then
/// changes what a history says, where otherwise it would only ever find the checksum.
pub fn sealed(history: &[u8]) -> Vec<u8> {
    let mut history = history.to_vec();
    if history.len() > SUM.end && history.starts_with(b"loro") {
        let sum = xxhash_rust::xxh32::xxh32(&history[SUM.end..], u32::from_le_bytes(*b"LORO"));
        history[SUM].copy_from_slice(&sum.to_le_bytes());
    }
    history
}

pub fn run(data: &[u8]) {
    let history = sealed(bounded(data));
    // Loro takes in histories that it cannot keep, and panics on some. The check refuses those.
    let loaded = Model::check(&history).and_then(|()| Model::load(&history, 2.into()));
    if let Ok(model) = loaded {
        sound(model.doc(), "a loaded history");
        assert_eq!(shown(&model), reread(&model), "a loaded history is saved as another");
    }

    let mut model = Model::new(sample(), 1.into());
    let before = shown(&model);
    let (compared, related) = (model.compare(&history), model.related(&history));
    match model.admit(&history).and_then(|()| model.merge(&history)) {
        Ok(_) => {
            sound(model.doc(), "a merged history");
            assert_eq!(shown(&model), reread(&model), "a merged history is saved as another");
            assert!(compared.is_ok() && related.is_ok(), "a history that merges could not be compared");
        }
        Err(_) => assert_eq!(shown(&model), before, "a history that was refused changed the document"),
    }
}

/// Histories as they are saved: whole, and as what one writer holds beyond another.
pub fn seeds() -> Vec<Vec<u8>> {
    let mut first = Model::new(sample(), 1.into());
    let mut second = first.fork(2.into());
    let (base, mut seeds) = (first.revision(), vec![first.save()]);
    let script: Vec<u8> = (0..=255).collect();
    let mut script = Script::new(&script);
    for turn in 0..3 {
        let model = if turn == 1 { &mut second } else { &mut first };
        for _ in 0..6 {
            let _ = model.apply(ops(&mut script, &shown(model), 1));
        }
        seeds.push(model.updates_since(&base));
    }
    first.merge(&second.save()).expect("a replica reads another's history");
    seeds.push(first.save());
    seeds
}
