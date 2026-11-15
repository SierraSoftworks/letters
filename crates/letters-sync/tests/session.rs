//! Devices and a room in session: what each person ends up with, and what they are told on the way.

mod common;

use common::{CREATED, ID, NAME, World, caret, doc, find, note, texts, typed};
use letters_model::{Extra, Model, NoteKind, Notes, Pos, ReplicaId, Reply, Run, Selection, Style, TextStyle};
use letters_sync::{
    Access, ClientMessage, Hello, Link, Local, PING_AFTER, PROTOCOL, Passage, Progress, Refusal, Role, SAVE_WITHIN,
    SILENT_AFTER, ServerMessage,
};

/// A world with these people in session, each with a copy of the document.
fn world_of<const N: usize>(people: [(&'static str, Role); N]) -> (World, [usize; N]) {
    let mut world = World::new();
    let devices = people.map(|(name, role)| world.join(name, role, true));
    world.settle();
    (world, devices)
}

fn editors() -> (World, usize, usize) {
    let (world, [ada, ben]) = world_of([("Ada", Role::Editor), ("Ben", Role::Editor)]);
    (world, ada, ben)
}

fn reply(author: &str, says: &str) -> Reply {
    Reply { author: author.into(), created: CREATED.into(), content: vec![Run::plain(says)], extra: Extra::new() }
}

/// A frame that carries what `edit` adds to a copy of the document as a device holds it.
fn edits(world: &World, device: usize, seq: u64, edit: letters_model::Op) -> Vec<u8> {
    let mut copy = world.devices[device].model.as_ref().unwrap().fork(ReplicaId::from(77_000 + seq));
    let before = copy.extent();
    copy.apply([edit]).unwrap();
    ClientMessage::Edits { seq, updates: copy.updates_for(&before) }.encode()
}

/// A style that differs from the default one.
fn restyled() -> Style {
    let mut style = Style::default();
    style.types.insert("heading1".into(), TextStyle { size: 31.0, ..style.types["heading1"].clone() });
    style
}

#[test]
fn two_editors_end_on_the_same_document_and_the_server_holds_it() {
    let (mut world, ada, ben) = editors();
    assert_eq!(world.devices[ada].client.link(), Link::Live(Role::Editor));

    let first = world.edit(ada, [typed("p", 0, "Oh. ")]);
    let second = world.edit(ben, [typed("q", 6, " thoughts")]);
    assert_eq!(world.devices[ada].client.progress(), Progress::Saving);
    world.settle();

    let expected = ["Oh. Hello world", "Second thoughts"];
    assert_eq!(texts(world.held()), expected);
    for (device, number) in [(ada, first), (ben, second)] {
        let device = &world.devices[device];
        assert_eq!(texts(device.document()), expected, "{}", device.name);
        assert!(device.holds(number), "{} was told the server holds edit {number}", device.name);
        assert_eq!(device.client.progress(), Progress::Saved, "{}", device.name);
    }
}

#[test]
fn editors_who_type_at_the_same_place_at_the_same_time_agree() {
    let (mut world, ada, ben) = editors();
    world.edit(ada, [typed("p", 5, " there")]);
    world.edit(ben, [typed("p", 5, ", dear")]);
    world.settle();

    let held = texts(world.held());
    assert!(held[0].contains(" there") && held[0].contains(", dear"), "both edits are kept: {held:?}");
    assert_eq!(texts(world.devices[ada].document()), held);
    assert_eq!(texts(world.devices[ben].document()), held);
    // People who see each other type are asked nothing.
    assert!(world.room.as_ref().unwrap().copy().notes.notes.is_empty());
}

#[test]
fn a_device_with_no_copy_is_sent_the_whole_document() {
    let (mut world, ada, _) = editors();
    world.edit(ada, [typed("p", 11, "!")]);
    world.annotate(ada, |notes| notes.notes.push(note("n1", "p", 0..5, "Ada", "Too plain?")));
    world.settle();

    let cy = world.join("Cy", Role::Reader, false);
    world.settle();

    let cy = &world.devices[cy];
    assert_eq!(texts(cy.document()), ["Hello world!", "Second"]);
    assert!(find(&cy.notes, "n1").is_some(), "the notes come with the document");
    assert_eq!(cy.client.document(), Some((ID, CREATED)), "its copy can be saved as this document");
    assert_eq!(cy.client.progress(), Progress::Saved);
}

#[test]
fn edits_written_offline_arrive_once_the_device_is_back() {
    let (mut world, ada, ben) = editors();
    world.cut(ada);
    let offline = world.edit(ada, [typed("p", 11, " again")]);
    world.edit(ben, [typed("q", 0, "The ")]);
    world.settle();
    assert_eq!(world.devices[ada].client.link(), Link::Offline);
    assert_eq!(world.devices[ada].client.progress(), Progress::Edited);
    assert_eq!(texts(world.held()), ["Hello world", "The Second"]);

    world.mend(ada);
    world.advance(5_000);

    let expected = ["Hello world again", "The Second"];
    assert_eq!(texts(world.held()), expected);
    assert_eq!(texts(world.devices[ada].document()), expected);
    assert_eq!(texts(world.devices[ben].document()), expected);
    assert!(world.devices[ada].holds(offline));
    assert_eq!(world.devices[ada].client.progress(), Progress::Saved);
    assert!(world.room.as_ref().unwrap().copy().notes.notes.is_empty(), "edits to other words merge without a word");
}

/// The two versions of each conflict the notes hold, as plain text, in order.
fn conflicts(notes: &Notes) -> Vec<Vec<String>> {
    let sides = |note: &letters_model::Note| match &note.kind {
        NoteKind::Conflict { versions } => {
            let text = |side: &letters_model::Version| side.block.as_ref().map(|block| block.plain_text());
            let mut sides: Vec<String> = versions.iter().filter_map(text).collect();
            sides.sort();
            Some(sides)
        }
        _ => None,
    };
    notes.notes.iter().filter_map(sides).collect()
}

#[test]
fn a_word_changed_offline_and_in_the_room_is_settled_the_same_for_everyone() {
    let (mut world, ada, ben) = editors();
    world.cut(ada);
    world.edit(ada, [typed("q", 2, "yy"), typed("p", 0, "Oh. ")]);
    world.edit(ben, [typed("q", 3, "XX")]);
    world.settle();
    world.mend(ada);
    world.advance(5_000);

    // One of the two shows whole, a note keeps both, and what touched no word of the other's merged.
    let held = world.room.as_ref().unwrap().copy();
    let shown = texts(held.model().doc());
    assert_eq!(conflicts(&held.notes), [["SecXXond", "Seyycond"]]);
    assert!(["SecXXond", "Seyycond"].contains(&shown[1].as_str()), "{shown:?}");
    assert_eq!(shown[0], "Oh. Hello world");
    for device in [ada, ben] {
        let device = &world.devices[device];
        assert_eq!(texts(device.document()), shown, "{}", device.name);
        assert_eq!(device.notes, held.notes, "{}", device.name);
        assert_eq!(device.client.progress(), Progress::Saved, "{}", device.name);
    }

    // Nothing raises it again: not the next socket, and not the file.
    world.cut(ada);
    world.mend(ada);
    world.advance(SAVE_WITHIN);
    assert_eq!(conflicts(&world.room.as_ref().unwrap().copy().notes).len(), 1);
    assert_eq!(conflicts(&world.folder.read_document(NAME).1).len(), 1);
}

#[test]
fn an_edit_that_was_on_its_way_when_the_socket_broke_is_sent_again() {
    let (mut world, ada, _) = editors();
    // The client hands the edit to a socket that is already dead.
    let number = world.edit(ada, [typed("p", 0, "Well. ")]);
    world.cut(ada);
    world.devices[ada].client.poll();
    world.settle();
    assert_eq!(texts(world.held()), ["Hello world", "Second"]);

    world.mend(ada);
    world.advance(5_000);
    assert_eq!(texts(world.held()), ["Well. Hello world", "Second"]);
    assert!(world.devices[ada].holds(number));
}

#[test]
fn a_device_sends_nothing_back_of_what_it_was_sent() {
    let (mut world, ada, ben) = editors();
    for word in ["One ", "two ", "three "] {
        world.edit(ben, [typed("p", 0, word)]);
        world.settle();
    }
    // A new socket tells what each side lacks, which is nothing.
    world.cut(ada);
    world.mend(ada);
    world.advance(5_000);

    assert_eq!(texts(world.devices[ada].document()), texts(world.held()));
    assert_eq!(world.devices[ada].edits_sent(), 0);
    assert_eq!(world.devices[ada].client.progress(), Progress::Saved);
}

#[test]
fn what_a_role_may_not_send_stays_on_its_device() {
    for role in [Role::Reader, Role::Commenter] {
        let (mut world, [ada, guest]) = world_of([("Ada", Role::Editor), ("Guest", role)]);
        assert_eq!(world.devices[guest].client.link(), Link::Live(role));

        world.edit(guest, [typed("p", 0, "Mine. ")]);
        world.restyle(guest, &restyled());
        world.settle();

        let sent = &world.devices[guest].sent;
        let work =
            |message: &&ClientMessage| matches!(message, ClientMessage::Edits { .. } | ClientMessage::Style { .. });
        assert_eq!(sent.iter().filter(work).count(), 0, "a {role:?} sent {sent:?}");
        assert_eq!(texts(world.held()), texts(&doc()), "a {role:?} changed the document");
        assert_eq!(texts(world.devices[ada].document()), texts(&doc()));
        // The window says the work is here and nowhere else.
        assert_eq!(world.devices[guest].client.progress(), Progress::Edited, "{role:?}");
        assert_eq!(texts(world.devices[guest].document())[0], "Mine. Hello world");
    }
}

#[test]
fn the_room_refuses_what_a_role_may_not_do() {
    let import = |id: &str| note(id, "p", 0..5, "Letters import", "A text box became a paragraph.");
    let import = |id: &str| letters_model::Note { kind: NoteKind::Import, ..import(id) };
    let notes =
        |note| ClientMessage::Notes { seq: 7, notes: Notes { notes: vec![note], extra: Extra::new() } }.encode();
    let style = ClientMessage::Style { seq: 7, style: restyled() }.encode();
    let cases = [
        // (who, what they send, how the room answers)
        (Role::Reader, None, Refusal::ReadOnly),
        (Role::Reader, Some(style.clone()), Refusal::ReadOnly),
        (Role::Reader, Some(notes(note("n2", "q", 0..6, "Guest", "Hm."))), Refusal::NoComments),
        (Role::Commenter, None, Refusal::ReadOnly),
        (Role::Commenter, Some(style), Refusal::ReadOnly),
        (Role::Commenter, Some(notes(import("n3"))), Refusal::NoComments),
    ];
    for (role, frame, why) in cases {
        let (mut world, [ada, guest]) = world_of([("Ada", Role::Editor), ("Guest", role)]);
        let frame = frame.unwrap_or_else(|| edits(&world, guest, 7, typed("p", 0, "Mine. ")));
        let before = world.state.files.clone();
        world.forge(guest, &frame);
        world.settle();

        let answered = world.told_to(guest).contains(&&ServerMessage::Refused { seq: 7, why });
        assert!(answered, "a {role:?} was not refused with {why:?}: {:?}", world.told_to(guest).last());
        let room = world.room.as_ref().unwrap();
        assert_eq!(texts(world.held()), texts(&doc()), "{role:?}");
        assert_eq!(room.copy().style(), &Style::default(), "{role:?}");
        assert!(room.copy().notes.notes.is_empty(), "{role:?}");
        assert_eq!(world.state.files, before, "nothing of it is kept");
        assert_eq!(texts(world.devices[ada].document()), texts(&doc()), "nothing of it is passed on");
        assert_eq!(room.sessions(), 2, "the session goes on");
    }
}

#[test]
fn a_comment_reaches_everyone_and_a_commenter_may_answer_it() {
    let (mut world, [ada, ben, cy, dee]) =
        world_of([("Ada", Role::Editor), ("Ben", Role::Editor), ("Cy", Role::Commenter), ("Dee", Role::Reader)]);

    let asked = world.annotate(ada, |notes| notes.notes.push(note("n1", "p", 6..11, "Ada", "Which world?")));
    world.settle();
    assert!(world.devices[ada].holds(asked));
    for device in [ben, cy, dee] {
        let device = &world.devices[device];
        let told = find(&device.notes, "n1").unwrap_or_else(|| panic!("{} has the comment", device.name));
        assert_eq!((told.anchor.block.as_str(), told.anchor.range.clone()), ("p", Some(6..11)));
    }

    // The commenter answers, resolves, and asks something of their own.
    let answered = world.annotate(cy, |notes| {
        let thread = notes.notes.iter_mut().find(|note| note.id == "n1").unwrap();
        thread.replies.push(reply("Cy", "This one."));
        thread.resolved = true;
        notes.notes.push(note("n2", "q", 0..6, "Cy", "Second what?"));
    });
    world.settle();
    assert!(world.devices[cy].holds(answered));
    assert_eq!(world.devices[cy].client.progress(), Progress::Saved);
    let held = &world.room.as_ref().unwrap().copy().notes;
    for notes in [held, &world.devices[ada].notes, &world.devices[ben].notes, &world.devices[dee].notes] {
        let thread = find(notes, "n1").unwrap();
        assert!(thread.resolved);
        assert_eq!(thread.replies, [reply("Cy", "This one.")]);
        assert!(find(notes, "n2").is_some());
    }

    // The reader's comment goes nowhere.
    world.annotate(dee, |notes| notes.notes.push(note("n3", "p", 0..5, "Dee", "Hello?")));
    world.settle();
    assert!(find(&world.room.as_ref().unwrap().copy().notes, "n3").is_none());
    assert_eq!(world.devices[dee].client.progress(), Progress::Edited);
}

#[test]
fn a_commenter_changes_nothing_of_a_note_but_its_thread() {
    let (mut world, [ada, cy]) = world_of([("Ada", Role::Editor), ("Cy", Role::Commenter)]);
    world.annotate(ada, |notes| notes.notes.push(note("n1", "p", 6..11, "Ada", "Which world?")));
    world.settle();

    // A program that does not keep to the rules rewrites the comment, and answers it.
    let mut forged = find(&world.devices[cy].notes, "n1").unwrap().clone();
    forged.content = vec![Run::plain("I agree with everything.")];
    forged.author = "Ada".into();
    forged.replies.push(reply("Cy", "Really?"));
    let frame = ClientMessage::Notes { seq: 3, notes: Notes { notes: vec![forged], extra: Extra::new() } }.encode();
    world.forge(cy, &frame);
    world.settle();

    for notes in [&world.room.as_ref().unwrap().copy().notes, &world.devices[ada].notes] {
        let thread = find(notes, "n1").unwrap();
        assert_eq!(thread.content, [Run::plain("Which world?")], "the comment says what its author wrote");
        assert_eq!(thread.replies, [reply("Cy", "Really?")], "the answer is kept");
    }
}

#[test]
fn a_comment_stays_on_its_words_while_others_edit() {
    let (mut world, ada, ben) = editors();
    // Both at once: the comment is on its way while the text before it grows.
    world.annotate(ada, |notes| notes.notes.push(note("n1", "p", 6..11, "Ada", "Which world?")));
    world.edit(ben, [typed("p", 0, "Oh, ")]);
    world.settle();

    let held = &world.room.as_ref().unwrap().copy().notes;
    for (who, notes) in [("the room", held), ("Ada", &world.devices[ada].notes), ("Ben", &world.devices[ben].notes)] {
        let anchor = &find(notes, "n1").unwrap().anchor;
        assert_eq!((anchor.block.as_str(), anchor.range.clone()), ("p", Some(10..15)), "{who}");
    }
    for device in [ada, ben] {
        assert_eq!(world.devices[device].client.progress(), Progress::Saved);
    }
}

#[test]
fn a_style_set_outside_the_editor_reaches_everyone() {
    let (mut world, ada, ben) = editors();
    let style = restyled();
    let number = world.restyle(ada, &style);
    assert_eq!(world.devices[ada].client.progress(), Progress::Saving);
    world.settle();

    assert_eq!(world.room.as_ref().unwrap().copy().style(), &style);
    for device in [ada, ben] {
        assert_eq!(world.devices[device].model.as_ref().unwrap().style(), &style);
    }
    assert!(world.devices[ada].holds(number));
    assert_eq!(world.devices[ada].client.progress(), Progress::Saved);
}

#[test]
fn a_caret_reaches_the_others_and_stays_with_its_text() {
    let (mut world, [ada, ben, cy]) = world_of([("Ada", Role::Editor), ("Ben", Role::Editor), ("Cy", Role::Reader)]);
    world.select(ada, Some(caret("p", 6)));
    world.select(cy, Some(Selection { anchor: Pos::new("q", 0), head: Pos::new("q", 6) }));
    world.settle();

    let peers = world.devices[ben].client.peers().to_vec();
    assert_eq!(peers.iter().map(|peer| peer.name.as_str()).collect::<Vec<_>>(), ["Ada", "Cy"]);
    assert_ne!(peers[0].colour, peers[1].colour, "two people look different");
    assert!(peers.iter().all(|peer| Some(peer.session) != world.devices[ben].client.session()));
    // Everyone sees a person in the same colour.
    let seen_by_cy = world.devices[cy].client.peers().iter().find(|peer| peer.name == "Ada").unwrap();
    assert_eq!(seen_by_cy.colour, peers[0].colour);

    // Ben types before Ada's caret: it moves with the word it stood at.
    world.edit(ben, [typed("p", 0, "Oh, ")]);
    world.settle();
    let model = world.devices[ben].model.as_ref().unwrap();
    let shown = |peer: usize| peers[peer].selection.as_ref().and_then(|passage| passage.selection(model));
    assert_eq!(shown(0), Some(caret("p", 10)));
    assert_eq!(shown(1), Some(Selection { anchor: Pos::new("q", 0), head: Pos::new("q", 6) }));

    // A caret that left the page, and a person who left the document.
    world.select(cy, None);
    world.advance(100);
    world.cut(ada);
    world.settle();
    let peers = world.devices[ben].client.peers();
    assert_eq!(peers.len(), 1);
    assert_eq!((peers[0].name.as_str(), &peers[0].selection), ("Cy", &None));
}

#[test]
fn someone_who_joins_later_sees_the_carets_that_are_there() {
    let (mut world, ada, _) = editors();
    world.select(ada, Some(caret("q", 3)));
    world.settle();

    let cy = world.join("Cy", Role::Reader, true);
    world.settle();
    let cy = &world.devices[cy];
    let peer = cy.client.peers().iter().find(|peer| peer.name == "Ada").expect("Ada is in the room");
    let selection = peer.selection.as_ref().and_then(|passage| passage.selection(cy.model.as_ref().unwrap()));
    assert_eq!(selection, Some(caret("q", 3)));
}

#[test]
fn a_caret_that_keeps_moving_is_told_now_and_then_and_ends_where_it_stopped() {
    let (mut world, ada, ben) = editors();
    for offset in 0..=10 {
        world.select(ada, Some(caret("p", offset)));
        world.advance(5);
    }
    world.advance(200);

    let told = |message: &&ClientMessage| matches!(message, ClientMessage::Selection(_));
    let told = world.devices[ada].sent.iter().filter(told).count();
    assert!((2..=4).contains(&told), "eleven moves in 50 ms were told {told} times");
    let model = world.devices[ben].model.as_ref().unwrap();
    let peer = &world.devices[ben].client.peers()[0];
    assert_eq!(peer.selection.as_ref().and_then(|passage| passage.selection(model)), Some(caret("p", 10)));
}

#[test]
fn presence_is_passed_on_and_never_kept() {
    let (mut world, ada, _) = editors();
    let (file, state) = (world.folder.files.clone(), world.state.files.clone());
    world.select(ada, Some(caret("p", 3)));
    world.advance(2 * SAVE_WITHIN);

    assert_eq!(world.state.files, state, "the journal took a caret");
    assert_eq!(world.folder.files, file, "the file was written for a caret");
}

#[test]
fn a_quiet_session_stays_open() {
    let (mut world, ada, ben) = editors();
    world.advance(10 * SILENT_AFTER);

    for device in [ada, ben] {
        let device = &world.devices[device];
        assert_eq!(device.client.link(), Link::Live(Role::Editor));
        assert_eq!(device.opens, 1, "{} never lost its socket", device.name);
        let pings = device.sent.iter().filter(|message| matches!(message, ClientMessage::Ping)).count() as u64;
        let most = 10 * SILENT_AFTER / PING_AFTER + 1;
        assert!((10..=most).contains(&pings), "{} said it was there {pings} times", device.name);
    }
}

#[test]
fn a_device_that_went_silent_is_let_go_and_comes_back() {
    let (mut world, ada, ben) = editors();
    world.select(ada, Some(caret("p", 0)));
    world.settle();
    world.devices[ada].frozen = true;
    world.edit(ben, [typed("q", 6, "!")]);
    world.advance(SILENT_AFTER + PING_AFTER);
    assert_eq!(world.room.as_ref().unwrap().sessions(), 1);
    assert!(world.devices[ben].client.peers().is_empty(), "a caret nobody holds is off the page");

    // It wakes up to a socket that has been dead for a while, and finds its way back.
    world.devices[ada].frozen = false;
    world.advance(SILENT_AFTER);
    assert_eq!(world.devices[ada].client.link(), Link::Live(Role::Editor));
    assert_eq!(texts(world.devices[ada].document()), ["Hello world", "Second!"]);
    assert_eq!(world.devices[ben].client.peers().len(), 1);
}

#[test]
fn a_device_keeps_asking_for_a_server_that_is_away_but_ever_less_often() {
    let (mut world, ada, _) = editors();
    world.stop_server();
    let number = world.edit(ada, [typed("p", 0, "Still here. ")]);
    world.advance(1_000);
    assert_eq!(world.devices[ada].client.progress(), Progress::Edited);

    let minutes = 10;
    world.advance(minutes * 60_000);
    let asked = u64::from(world.devices[ada].opens - 1);
    assert!(asked >= minutes, "it asked {asked} times in {minutes} minutes");
    assert!(asked <= 3 * minutes + 6, "it asked {asked} times in {minutes} minutes");

    world.open_room().unwrap();
    world.advance(60_000);
    assert_eq!(world.devices[ada].client.link(), Link::Live(Role::Editor));
    assert_eq!(texts(world.held())[0], "Still here. Hello world");
    assert!(world.devices[ada].holds(number));
}

#[test]
fn a_copy_of_another_document_is_turned_away_and_merged_nowhere() {
    let mut theirs = Model::new(doc(), ReplicaId::from(4_000));
    theirs.apply([typed("p", 0, "Theirs. ")]).unwrap();
    let mut world = World::new();
    let ada = world.join("Ada", Role::Editor, true);
    let eve =
        world.join_with("Eve", Role::Editor, "0d9a1b7c-0000-4000-8000-000000000001", Some(theirs), Notes::default());
    world.settle();
    world.advance(5 * 60_000);

    let eve = &world.devices[eve];
    assert_eq!(eve.client.link(), Link::Refused(Refusal::OtherDocument));
    assert_eq!((eve.opens, eve.client.wake_at()), (1, None), "it does not ask again");
    assert_eq!(texts(eve.document())[0], "Theirs. Hello world", "its copy is as it was");
    assert_eq!(texts(world.held()), texts(&doc()));
    assert_eq!(texts(world.devices[ada].document()), texts(&doc()));
    assert!(world.devices[ada].client.peers().is_empty(), "nobody was told of a session that never began");
}

#[test]
fn a_build_the_server_cannot_talk_to_is_told_which_of_them_is_too_old() {
    for (protocol, why) in [(PROTOCOL - 1, Refusal::TooOld), (PROTOCOL + 1, Refusal::TooNew)] {
        let (mut world, [ada]) = world_of([("Ada", Role::Editor)]);
        let hello = Hello { protocol, document: ID.into(), extent: Default::default(), revision: None };
        let room = world.room.as_mut().unwrap();
        let session = room.join(Access { role: Role::Editor, name: "Other build".into() }, world.now);
        room.received(session, &ClientMessage::Hello(hello).encode(), &mut world.state, world.now).unwrap();
        world.settle();

        assert_eq!(world.told.last(), Some(&(session, ServerMessage::Goodbye { why })));
        assert_eq!(world.room.as_ref().unwrap().sessions(), 1);
        assert!(world.devices[ada].client.peers().is_empty(), "nobody was told of a session that never began");
    }
}

#[test]
fn a_goodbye_ends_the_session_for_good_or_for_now() {
    let cases = [
        // (why the host ends the session, whether the device comes back by itself)
        (Refusal::SignedOut, false),
        (Refusal::NoAccess, false),
        (Refusal::Gone, false),
        (Refusal::Other(200), true),
    ];
    for (why, returns) in cases {
        let (mut world, ada, ben) = editors();
        let session = world.devices[ben].session.unwrap();
        world.room.as_mut().unwrap().dismiss(session, why);
        world.settle();
        assert!(world.devices[ada].client.peers().is_empty(), "{why:?}");

        world.advance(60_000);
        let device = &world.devices[ben];
        match returns {
            true => assert_eq!(device.client.link(), Link::Live(Role::Editor), "{why:?}"),
            false => {
                assert_eq!(device.client.link(), Link::Refused(why));
                assert_eq!((device.opens, device.client.wake_at()), (1, None), "{why:?}: it does not ask again");
                // The person signs in again, or is given access again.
                let now = world.now;
                world.devices[ben].client.start(now);
                world.settle();
                assert_eq!(world.devices[ben].client.link(), Link::Live(Role::Editor), "{why:?}");
            }
        }
    }
}

#[test]
fn a_session_whose_role_changed_joins_again_at_once_in_its_new_role() {
    let (mut world, ada, ben) = editors();
    world.devices[ben].role = Role::Commenter;
    let session = world.devices[ben].session.unwrap();
    world.room.as_mut().unwrap().dismiss(session, Refusal::AccessChanged);
    world.settle();

    assert_eq!(world.devices[ben].client.link(), Link::Live(Role::Commenter));
    assert_eq!(world.devices[ben].opens, 2);
    assert_eq!(world.devices[ada].client.peers().len(), 1);
}

#[test]
fn a_frame_the_room_cannot_read_ends_that_session_alone() {
    let hello = Hello { protocol: PROTOCOL, document: ID.into(), extent: Default::default(), revision: None };
    let cases: [(&str, Vec<u8>); 3] = [
        ("an edit cut short", vec![2, 1, 0, 0]),
        ("a second hello", ClientMessage::Hello(hello).encode()),
        ("notes that are not notes", [&[3][..], &[0; 8], &4u32.to_le_bytes(), b"nope"].concat()),
    ];
    for (what, frame) in cases {
        let (mut world, ada, ben) = editors();
        world.forge(ben, &frame);
        world.settle();
        let goodbye = ServerMessage::Goodbye { why: Refusal::Unreadable };
        assert!(world.told.iter().any(|(_, message)| *message == goodbye), "{what}");
        assert_eq!(world.devices[ada].client.link(), Link::Live(Role::Editor), "{what}");
        assert_eq!(texts(world.held()), texts(&doc()), "{what}");

        // The device behind it is a sound one here, and comes back.
        world.advance(5_000);
        assert_eq!(world.devices[ben].client.link(), Link::Live(Role::Editor), "{what}");
    }
}

#[test]
fn a_socket_that_says_anything_before_hello_is_closed() {
    let mut world = World::new();
    let room = world.room.as_mut().unwrap();
    let session = room.join(Access { role: Role::Editor, name: "Eve".into() }, world.now);
    room.received(session, &ClientMessage::Ping.encode(), &mut world.state, world.now).unwrap();
    world.settle();

    assert_eq!(world.told, [(session, ServerMessage::Goodbye { why: Refusal::Unreadable })]);
    assert_eq!(world.room.as_ref().unwrap().sessions(), 0);
}

#[test]
fn a_frame_a_device_cannot_read_costs_it_the_socket_and_nothing_else() {
    let (mut world, ada, _) = editors();
    let number = world.edit(ada, [typed("p", 0, "Mine. ")]);
    let device = &mut world.devices[ada];
    device.client.received(&[65, 1, 2], Local { model: device.model.as_ref(), notes: &device.notes }, world.now);
    world.settle();
    assert_eq!(world.devices[ada].client.link(), Link::Offline);

    world.advance(5_000);
    assert_eq!(world.devices[ada].client.link(), Link::Live(Role::Editor));
    assert!(world.devices[ada].holds(number));
    assert_eq!(texts(world.devices[ada].document())[0], "Mine. Hello world");
}

/// The update of issue 2: its checksum is sound, and the history library's debug assertions panic on it.
const TRIPS: &[u8] = include_bytes!("../../../fuzz/corpus/history/bytes-after-a-block-header.bin");

#[test]
fn an_update_the_history_library_trips_on_is_refused_and_the_room_goes_on() {
    let (mut world, ada, ben) = editors();
    let before = world.state.files.clone();
    world.forge(ben, &ClientMessage::Edits { seq: 5, updates: TRIPS.to_vec() }.encode());
    world.settle();

    let refused = ServerMessage::Refused { seq: 5, why: Refusal::Unreadable };
    assert!(world.told.iter().any(|(_, message)| *message == refused), "{:?}", world.told.last());
    assert_eq!(world.state.files, before, "nothing of it is kept");
    assert_eq!(texts(world.held()), texts(&doc()));

    // The document goes on, for the device that sent it too once it is back.
    world.advance(5_000);
    let number = world.edit(ada, [typed("p", 0, "After. ")]);
    world.settle();
    assert!(world.devices[ada].holds(number));
    assert_eq!(texts(world.devices[ben].document())[0], "After. Hello world");
}

#[test]
fn edits_that_build_on_some_the_room_never_saw_are_not_held() {
    let (mut world, ada, ben) = editors();
    // Two edits, and only the second is sent.
    let mut copy = world.devices[ada].model.as_ref().unwrap().fork(ReplicaId::from(9_000));
    copy.apply([typed("p", 0, "One. ")]).unwrap();
    let between = copy.extent();
    copy.apply([typed("p", 5, "Two. ")]).unwrap();
    let before = world.state.files.clone();
    world.forge(ada, &ClientMessage::Edits { seq: 5, updates: copy.updates_for(&between) }.encode());
    world.settle();

    let refused = ServerMessage::Refused { seq: 5, why: Refusal::Unreadable };
    assert!(world.told.iter().any(|(_, message)| *message == refused), "{:?}", world.told.last());
    assert_eq!(world.state.files, before, "a journal with half an edit in it could not be replayed");
    assert_eq!(texts(world.devices[ben].document()), texts(&doc()));

    // The device asks what the server holds and sends all of it.
    world.devices[ada].model.as_mut().unwrap().merge(&copy.save()).unwrap();
    world.advance(5_000);
    assert_eq!(world.devices[ada].refused, [Refusal::Unreadable]);
    assert_eq!(world.devices[ada].opens, 2);
    assert_eq!(texts(world.held())[0], "One. Two. Hello world");
    assert_eq!(texts(world.devices[ben].document())[0], "One. Two. Hello world");
    assert_eq!(world.devices[ada].client.progress(), Progress::Saved);
}

#[test]
fn the_window_is_told_when_there_is_something_new_to_show() {
    use letters_sync::Event;
    let mut world = World::new();
    let ada = world.join("Ada", Role::Editor, true);
    let mut events = Vec::new();
    // By hand, to see what a shell sees.
    let device = &mut world.devices[ada];
    assert_eq!(device.client.poll(), Some(Event::Open));
    let room = world.room.as_mut().unwrap();
    let session = room.join(Access { role: Role::Editor, name: "Ada".into() }, world.now);
    let local = Local { model: device.model.as_ref(), notes: &device.notes };
    device.client.opened(local, world.now);
    while let Some(event) = device.client.poll() {
        if let Event::Send(frame) = &event {
            room.received(session, frame, &mut world.state, world.now).unwrap();
        }
        events.push(event);
    }
    while let Some(letters_sync::Outbound::Send(_, frame)) = room.poll() {
        device.client.received(&frame, local, world.now);
    }
    events.extend(std::iter::from_fn(|| device.client.poll()));

    assert_eq!(events.iter().filter(|event| **event == Event::Link).count(), 2, "connecting, then live: {events:?}");
    assert!(events.contains(&Event::Peers));
    assert_eq!(device.client.link(), Link::Live(Role::Editor));
    assert_eq!(world.folder.files.keys().collect::<Vec<_>>(), [NAME]);
}

#[test]
fn a_passage_is_none_where_its_text_is_not() {
    let model = Model::new(doc(), ReplicaId::from(1));
    assert_eq!(Passage::of(&model, &caret("nowhere", 0)), None);

    let mut other = model.fork(ReplicaId::from(2));
    other
        .apply([letters_model::Op::InsertBlock {
            parent: None,
            index: 2,
            block: letters_model::Block::text("r", letters_model::BlockKind::Paragraph, "Third"),
        }])
        .unwrap();
    let passage = Passage::of(&other, &caret("r", 2)).expect("the caret is in the document");
    assert_eq!(passage.selection(&model), None, "a copy that has not seen the block cannot show the caret");
    assert_eq!(passage.selection(&other), Some(caret("r", 2)));
}
