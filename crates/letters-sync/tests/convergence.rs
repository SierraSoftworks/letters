//! Whatever the people, the network and the server do, everyone ends on one document, and nothing that
//! was written is lost.

mod common;

use common::{NAME, World, find, note, open_file, save_file, typed};
use letters_model::NoteKind;
use letters_sync::{Link, Progress, Role};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

/// Ada and Ben edit, and Cy comments.
const PEOPLE: [(&str, Role); 3] = [("Ada", Role::Editor), ("Ben", Role::Editor), ("Cy", Role::Commenter)];
const BLOCKS: [&str; 2] = ["p", "q"];

/// The cases a run draws: the same ones every time, so that a fault in a rare schedule fails the change
/// that made it. `PROPTEST_CASES=1500` draws that many afresh, which is how new faults are looked for.
fn cases(cases: u32) -> ProptestConfig {
    match std::env::var_os("PROPTEST_CASES") {
        Some(_) => ProptestConfig::default(),
        None => ProptestConfig { cases, rng_seed: RngSeed::Fixed(1_800_000_000), ..ProptestConfig::default() },
    }
}

#[derive(Clone, Debug)]
enum Step {
    /// An editor types a word that no other step types.
    Type {
        who: usize,
        block: usize,
        at: usize,
    },
    /// Anyone writes a comment.
    Comment {
        who: usize,
    },
    /// A device loses the network, or finds it again.
    Cut {
        who: usize,
    },
    Mend {
        who: usize,
    },
    /// Another program opens the file in its folder, types, and saves.
    Share {
        block: usize,
        at: usize,
    },
    /// The server stops without saving and starts again.
    Stop,
    Wait {
        ms: u64,
    },
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        6 => (0..2usize, 0..2usize, 0..64usize).prop_map(|(who, block, at)| Step::Type { who, block, at }),
        2 => (0..3usize).prop_map(|who| Step::Comment { who }),
        2 => (0..3usize).prop_map(|who| Step::Cut { who }),
        2 => (0..3usize).prop_map(|who| Step::Mend { who }),
        1 => (0..2usize, 0..64usize).prop_map(|(block, at)| Step::Share { block, at }),
        1 => Just(Step::Stop),
        4 => (0..4_000u64).prop_map(|ms| Step::Wait { ms }),
    ]
}

/// A place to type at that is in no word of another step, so that each word can be looked for whole.
/// Every text here is ASCII.
fn place(text: &str, at: usize) -> usize {
    let (mut depth, mut free) = (0, vec![0]);
    for (offset, byte) in text.bytes().enumerate() {
        depth += i32::from(byte == b'[') - i32::from(byte == b']');
        if depth == 0 {
            free.push(offset + 1);
        }
    }
    free[at % free.len()]
}

/// Everything a run wrote, to look for at its end.
#[derive(Default)]
struct Written {
    words: Vec<String>,
    comments: Vec<String>,
}

/// Plays the steps, and then gives everyone the network and the time to catch up.
fn play(steps: &[Step]) -> (World, Written) {
    let mut world = World::new();
    for (name, role) in PEOPLE {
        world.join(name, role, true);
    }
    world.settle();
    let mut written = Written::default();
    for (index, step) in steps.iter().enumerate() {
        let word = format!("[{index}]");
        match *step {
            Step::Type { who, block, at } => {
                let text = world.devices[who].document().block(&BLOCKS[block].into()).unwrap().plain_text();
                world.edit(who, [typed(BLOCKS[block], place(&text, at), &word)]);
                written.words.push(word);
            }
            Step::Comment { who } => {
                let id = format!("n{index}");
                world.annotate(who, |notes| notes.notes.push(note(&id, "p", 0..1, PEOPLE[who].0, "Hm.")));
                written.comments.push(id);
            }
            Step::Cut { who } => world.cut(who),
            Step::Mend { who } => world.mend(who),
            Step::Share { block, at } => {
                let mut file = open_file(&world.folder, 5_000 + index as u64);
                let text = file.model().doc().block(&BLOCKS[block].into()).unwrap().plain_text();
                file.apply([typed(BLOCKS[block], place(&text, at), &word)]).unwrap();
                save_file(&mut world.folder, &mut file);
                written.words.push(word);
            }
            Step::Stop => {
                world.stop_server();
                world.open_room().expect("the room opens over what the last one left");
            }
            Step::Wait { ms } => world.advance(ms),
        }
        world.settle();
    }
    for who in 0..PEOPLE.len() {
        world.mend(who);
    }
    world.advance(180_000);
    (world, written)
}

proptest! {
    #![proptest_config(cases(48))]

    #[test]
    fn everyone_ends_on_one_document_with_nothing_lost(steps in vec(step(), 1..40)) {
        let (world, written) = play(&steps);
        let room = world.room.as_ref().unwrap();
        let held = room.copy().model().doc();
        // A word is in the text, or in a conflict for someone to pick: where a session and the writer on
        // the share changed one word apart, the later change shows and a note keeps both.
        let mut kept: Vec<String> = held.walk().map(|block| block.plain_text()).collect();
        for note in &room.copy().notes.notes {
            if let NoteKind::Conflict { versions } = &note.kind {
                kept.extend(versions.iter().filter_map(|version| version.block.as_ref()).map(|block| block.plain_text()));
            }
        }
        for word in &written.words {
            prop_assert!(kept.iter().any(|text| text.contains(word.as_str())), "{} is missing from {:?}", word, kept);
        }
        for id in &written.comments {
            prop_assert!(find(&room.copy().notes, id).is_some(), "comment {} is missing", id);
        }

        let (on_disk, notes_on_disk) = world.folder.read_document(NAME);
        prop_assert_eq!(&on_disk, held, "the file");
        prop_assert_eq!(&notes_on_disk, &room.copy().notes, "the file's notes");
        prop_assert_eq!(world.folder.files.len(), 1, "a copy was put beside the file");
        for (device, (name, role)) in world.devices.iter().zip(PEOPLE) {
            prop_assert_eq!(device.client.link(), Link::Live(role), "{}", name);
            prop_assert_eq!(device.document(), held, "{}", name);
            prop_assert_eq!(&device.notes, &room.copy().notes, "{}'s notes", name);
            prop_assert_eq!(device.client.progress(), Progress::Saved, "{}", name);
        }
        prop_assert!(room.wake_at().is_some_and(|at| at > world.now), "the room has nothing left to do but look");
        prop_assert!(world.faults.is_empty(), "{:?}", world.faults);
    }
}
