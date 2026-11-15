//! The room and its folders: what is kept, when the file is written, and what a stop or a stubborn file
//! costs.

mod common;

use common::{NAME, World, doc, find, note, open_file, save_file, texts, typed};
use letters_format::{Error, Saved, hash};
use letters_model::{Block, BlockKind, Note, NoteKind, Op};
use letters_sync::{Access, LOOK_EVERY, Link, Progress, Role, SAVE_AFTER, SAVE_WITHIN};

const HOME: &str = "/srv/letters/report.let";

fn editors() -> (World, usize, usize) {
    let mut world = World::new();
    let devices = [world.join("Ada", Role::Editor, true), world.join("Ben", Role::Editor, true)];
    world.settle();
    (world, devices[0], devices[1])
}

/// The text of the document's file as it is in the folder now.
fn on_disk(world: &World) -> Vec<String> {
    texts(&world.folder.read_document(NAME).0)
}

fn copies(world: &World) -> Vec<&String> {
    world.folder.files.keys().filter(|name| name.contains("conflicted copy")).collect()
}

fn written(world: &World) -> usize {
    world.saves.iter().filter(|saved| matches!(saved, Saved::Written)).count()
}

#[test]
fn what_the_server_said_it_holds_survives_a_stop_before_any_save() {
    let (mut world, ada, ben) = editors();
    let edit = world.edit(ada, [typed("p", 0, "Kept. ")]);
    let comment = world.annotate(ben, |notes| notes.notes.push(note("n1", "q", 0..6, "Ben", "Kept too?")));
    world.settle();
    assert!(world.devices[ada].holds(edit) && world.devices[ben].holds(comment));

    world.stop_server();
    assert_eq!(on_disk(&world), texts(&doc()), "the file had not been written yet");
    // Nobody comes back to send it again.
    world.cut(ada);
    world.cut(ben);
    world.open_room().unwrap();
    assert_eq!(texts(world.held())[0], "Kept. Hello world");
    assert!(find(&world.room.as_ref().unwrap().copy().notes, "n1").is_some());

    // The room owes it to the file, with nobody in it.
    assert!(!world.room.as_ref().unwrap().is_idle());
    world.advance(SAVE_WITHIN);
    assert_eq!(on_disk(&world)[0], "Kept. Hello world");
    assert!(find(&world.folder.read_document(NAME).1, "n1").is_some());
    assert!(world.room.as_ref().unwrap().is_idle());
}

#[test]
fn an_edit_the_journal_could_not_take_is_held_once_the_file_has_it() {
    let (mut world, ada, ben) = editors();
    world.state.read_only = true;
    let number = world.edit(ada, [typed("p", 0, "Late. ")]);
    world.settle();

    assert!(!world.faults.is_empty(), "the host is told that its disk failed");
    assert!(!world.devices[ada].holds(number), "nothing keeps the edit through a stop yet");
    assert_eq!(world.devices[ada].client.progress(), Progress::Saving);
    assert_eq!(texts(world.devices[ben].document())[0], "Late. Hello world", "the others have it all the same");

    world.advance(SAVE_AFTER);
    assert_eq!(on_disk(&world)[0], "Late. Hello world");
    assert!(world.devices[ada].holds(number));
    assert_eq!(world.devices[ada].client.progress(), Progress::Saved);
}

#[test]
fn the_file_is_written_once_the_edits_pause() {
    let (mut world, ada, _) = editors();
    world.edit(ada, [typed("p", 0, "One. ")]);
    world.advance(SAVE_AFTER - 1);
    // Still typing: the pause starts again.
    world.edit(ada, [typed("p", 0, "Two. ")]);
    world.advance(SAVE_AFTER - 1);
    assert_eq!(on_disk(&world), texts(&doc()));

    world.advance(1);
    assert_eq!(on_disk(&world)[0], "Two. One. Hello world");
    assert_eq!(written(&world), 1);

    // Nothing more is written for it, however long the document stays open.
    world.advance(10 * SAVE_WITHIN);
    assert_eq!(written(&world), 1);
    assert!(copies(&world).is_empty());
}

#[test]
fn edits_that_never_pause_reach_the_file_all_the_same() {
    let (mut world, ada, _) = editors();
    let step = SAVE_AFTER / 4;
    for _ in 0..(5 * SAVE_WITHIN / 2 / step) {
        world.edit(ada, [typed("q", 0, "x")]);
        world.advance(step);
    }
    // Often enough to lose little, and not with every pause for breath.
    assert!((2..=3).contains(&written(&world)), "{} saves in {} ms of typing", written(&world), 5 * SAVE_WITHIN / 2);
    assert!(on_disk(&world)[1].starts_with("xxxxxxxx"));
}

#[test]
fn what_another_writer_puts_in_the_file_reaches_every_session() {
    let (mut world, ada, ben) = editors();
    let mut other = open_file(&world.folder, 900);
    other.apply([typed("q", 6, " opinion")]).unwrap();
    other.add_note(note("n9", "p", 0..5, "Sam", "From the share."));
    assert!(matches!(save_file(&mut world.folder, &mut other), Saved::Written));

    world.advance(LOOK_EVERY);
    for device in [ada, ben] {
        let device = &world.devices[device];
        assert_eq!(texts(device.document()), ["Hello world", "Second opinion"], "{}", device.name);
        assert!(find(&device.notes, "n9").is_some(), "{}", device.name);
        assert_eq!(device.edits_sent(), 0, "{} sent the news back", device.name);
    }
    assert!(copies(&world).is_empty());
    assert_eq!(written(&world), 0, "the room had nothing of its own to write");
}

#[test]
fn two_devices_and_a_writer_on_the_share_lose_nothing() {
    let (mut world, ada, ben) = editors();
    let mut share = open_file(&world.folder, 900);
    let rounds = 8;
    for round in 0..rounds {
        world.edit(ada, [typed("p", 0, &format!("a{round} "))]);
        world.edit(ben, [typed("q", 0, &format!("b{round} "))]);
        // Sometimes long enough for the room to save in between, sometimes not.
        world.advance(600 * (round % 4 + 1));
        let end = share.model().doc().block(&"q".into()).unwrap().plain_text().len();
        share.apply([typed("q", end, &format!(" s{round}"))]).unwrap();
        save_file(&mut world.folder, &mut share);
    }
    world.advance(2 * SAVE_WITHIN);
    save_file(&mut world.folder, &mut share);

    let held = texts(world.held());
    for round in 0..rounds {
        let kept = held[0].contains(&format!("a{round} "))
            && held[1].contains(&format!("b{round} "))
            && held[1].contains(&format!(" s{round}"));
        assert!(kept, "round {round} is missing from {held:?}");
    }
    assert_eq!(on_disk(&world), held, "the file");
    assert_eq!(texts(share.model().doc()), held, "the writer on the share");
    for device in [ada, ben] {
        assert_eq!(texts(world.devices[device].document()), held, "{}", world.devices[device].name);
        assert_eq!(world.devices[device].client.progress(), Progress::Saved);
    }
    assert!(copies(&world).is_empty(), "nobody was pushed aside: {:?}", copies(&world));
}

#[test]
fn a_word_changed_in_session_and_on_the_share_is_settled_the_same_for_everyone() {
    let (mut world, ada, ben) = editors();
    let mut share = open_file(&world.folder, 900);
    share.apply([typed("q", 3, "XX")]).unwrap();
    world.edit(ada, [typed("q", 2, "yy")]);
    world.settle();
    save_file(&mut world.folder, &mut share);
    world.advance(SAVE_WITHIN);
    save_file(&mut world.folder, &mut share);

    // The later change shows whole, and a note keeps both for someone to pick from.
    let held = world.room.as_ref().unwrap().copy();
    let raised: Vec<&Note> =
        held.notes.notes.iter().filter(|note| matches!(note.kind, NoteKind::Conflict { .. })).collect();
    let [conflict] = raised[..] else { panic!("one conflict is raised: {raised:?}") };
    let NoteKind::Conflict { versions } = &conflict.kind else { unreachable!() };
    let mut sides: Vec<String> = versions.iter().map(|side| side.block.as_ref().unwrap().plain_text()).collect();
    assert!(sides.contains(&texts(held.model().doc())[1]), "the document shows neither of {sides:?}");
    sides.sort();
    assert_eq!(sides, ["SecXXond", "Seyycond"]);

    let shown = texts(held.model().doc());
    assert_eq!(on_disk(&world), shown);
    assert_eq!(texts(share.model().doc()), shown, "the writer on the share");
    assert!(find(&share.notes, &conflict.id).is_some(), "the writer on the share has the choice too");
    for device in [ada, ben] {
        let device = &world.devices[device];
        assert_eq!(texts(device.document()), shown, "{}", device.name);
        assert!(find(&device.notes, &conflict.id).is_some(), "{} has the choice too", device.name);
        assert_eq!(device.client.progress(), Progress::Saved);
    }
    assert!(copies(&world).is_empty());
}

#[test]
fn a_folder_that_cannot_be_written_is_tried_again_and_costs_no_edit() {
    let (mut world, ada, _) = editors();
    world.folder.read_only = true;
    let number = world.edit(ada, [typed("p", 0, "Patient. ")]);
    world.advance(SAVE_AFTER);

    assert!(matches!(world.faults.last(), Some(Error::Files(_))), "the host is told: {:?}", world.faults);
    assert!(world.devices[ada].holds(number), "the journal has it");
    assert_eq!(world.devices[ada].client.link(), Link::Live(Role::Editor));

    world.folder.read_only = false;
    world.advance(LOOK_EVERY);
    assert_eq!(on_disk(&world)[0], "Patient. Hello world");
}

#[test]
fn a_file_that_cannot_be_replaced_gets_one_copy_until_the_host_says_where_to_save() {
    let (mut world, ada, _) = editors();
    // Held open by a program that lets nobody rename over it.
    world.folder.locked = Some(NAME.into());
    world.edit(ada, [typed("p", 0, "One. ")]);
    world.advance(SAVE_AFTER);
    assert!(matches!(world.saves.last(), Some(Saved::ConflictCopy { .. })), "{:?}", world.saves);
    let copy = copies(&world)[0].clone();
    assert_eq!(texts(&world.folder.read_document(&copy).0)[0], "One. Hello world");

    // The session goes on, and its edits are kept, but not as a new copy with every pause.
    for round in 0..4 {
        let number = world.edit(ada, [typed("q", 0, "More. ")]);
        world.advance(SAVE_WITHIN);
        assert!(world.devices[ada].holds(number), "round {round}");
    }
    assert_eq!(copies(&world), [&copy]);
    assert_eq!(on_disk(&world), texts(&doc()));

    // The other program lets go, and the host has the room save where it did before.
    world.folder.locked = None;
    world.room.as_mut().unwrap().moved(HOME, &mut world.state).unwrap();
    world.advance(1_000);
    assert_eq!(on_disk(&world), texts(world.held()));
    assert!(on_disk(&world)[1].starts_with("More. More. More. More. "));
}

#[test]
fn edits_made_after_a_conflict_copy_survive_a_stop() {
    let (mut world, ada, _) = editors();
    world.folder.locked = Some(NAME.into());
    world.edit(ada, [typed("p", 0, "One. ")]);
    world.advance(SAVE_AFTER);
    world.edit(ada, [typed("p", 0, "Two. ")]);
    world.settle();

    world.stop_server();
    world.cut(ada);
    world.folder.locked = None;
    world.open_room().unwrap();
    world.advance(SAVE_WITHIN);
    assert_eq!(on_disk(&world)[0], "Two. One. Hello world");
}

#[test]
fn a_picture_is_kept_once_the_document_names_it() {
    let (mut world, ada, _) = editors();
    let bytes = b"\x89PNG, as far as anyone here can tell".to_vec();
    let name = format!("assets/{}.png", hash(&bytes).strip_prefix("sha256:").unwrap());
    let room = world.room.as_mut().unwrap();
    room.add_asset(&name, bytes.clone(), &mut world.state, world.now).unwrap();
    let wrong = room.add_asset("assets/0000.png", bytes.clone(), &mut world.state, world.now);
    assert!(matches!(wrong, Err(Error::HashMismatch(_))), "a picture under another picture's name: {wrong:?}");
    assert_eq!(room.asset(&name), Some(&bytes[..]), "the others can fetch it at once");
    assert_eq!(room.asset("assets/0000.png"), None);

    // Nothing names it yet, so nothing is owed to the file for it.
    world.advance(SAVE_WITHIN);
    assert_eq!(world.folder.read_entry(NAME, &name), None);

    let kind = BlockKind::Figure {
        asset: Some(name.clone()),
        src: None,
        alt: "A chart".into(),
        width: 320.0,
        height: 200.0,
        numbered: false,
    };
    world.edit(ada, [Op::InsertBlock { parent: None, index: 2, block: Block::new("f", kind) }]);
    world.settle();
    // Kept through a stop from here on, like the edit that names it.
    world.stop_server();
    world.cut(ada);
    world.open_room().unwrap();
    assert_eq!(world.room.as_ref().unwrap().asset(&name), Some(&bytes[..]));
    world.advance(SAVE_WITHIN);
    assert_eq!(world.folder.read_entry(NAME, &name), Some(bytes));
}

#[test]
fn a_flush_writes_the_file_at_once() {
    let (mut world, ada, _) = editors();
    world.edit(ada, [typed("p", 0, "Now. ")]);
    world.settle();
    assert_eq!(on_disk(&world), texts(&doc()));

    assert!(matches!(world.flush(), Ok(Saved::Written)));
    assert_eq!(on_disk(&world)[0], "Now. Hello world");
    assert!(matches!(world.flush(), Ok(Saved::Unchanged)));
}

#[test]
fn the_room_tells_who_has_the_document_open() {
    let (mut world, _, ben) = editors();
    // A socket that has not said hello is nobody yet.
    let room = world.room.as_mut().unwrap();
    room.join(Access { role: Role::Reader, name: "Eve".into() }, world.now);
    assert_eq!(room.people().collect::<Vec<_>>(), ["Ada", "Ben"]);
    assert_eq!(room.sessions(), 3);

    world.cut(ben);
    assert_eq!(world.room.as_ref().unwrap().people().collect::<Vec<_>>(), ["Ada"]);
}

#[test]
fn a_room_is_idle_once_nobody_is_in_it_and_the_file_holds_everything() {
    let (mut world, ada, ben) = editors();
    world.cut(ben);
    assert!(!world.room.as_ref().unwrap().is_idle(), "someone is in it");

    world.edit(ada, [typed("p", 0, "Last. ")]);
    world.settle();
    world.cut(ada);
    assert!(!world.room.as_ref().unwrap().is_idle(), "the file is owed an edit");

    world.advance(SAVE_WITHIN);
    let room = world.room.as_ref().unwrap();
    assert!(room.is_idle());
    assert_eq!(room.wake_at(), None);
    assert_eq!(on_disk(&world)[0], "Last. Hello world");

    // The next room starts with nothing owed.
    world.stop_server();
    world.open_room().unwrap();
    assert!(world.room.as_ref().unwrap().is_idle());
}

#[test]
fn a_journal_that_is_another_documents_keeps_the_room_from_opening() {
    let mut world = World::new();
    let ada = world.join("Ada", Role::Editor, true);
    world.settle();
    world.edit(ada, [typed("p", 0, "Mine. ")]);
    world.settle();
    world.stop_server();

    // The state folder holds the journal of one document, and the file is now another.
    let mut other = letters_format::WorkingCopy::new(
        "0d9a1b7c-0000-4000-8000-000000000001",
        common::CREATED,
        doc(),
        Default::default(),
        7.into(),
    );
    world.folder.files.clear();
    letters_format::save(
        &mut world.folder,
        &mut other,
        &letters_format::Save { name: NAME, token: "t", label: "test", attempts: 1 },
    )
    .unwrap();
    let journal = world.state.files.keys().next().unwrap().clone();
    let bytes = world.state.files.remove(&journal).unwrap();
    world.state.files.insert(letters_format::journal_name("0d9a1b7c-0000-4000-8000-000000000001"), bytes);

    assert!(matches!(world.open_room(), Err(Error::OtherDocument(_))));
}
