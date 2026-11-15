//! Every message survives its frame, and no frame breaks a reader.

mod common;

use common::{CREATED, ID, doc, note, typed};
use letters_model::{Extra, Model, Notes, Pos, ReplicaId, Selection, Style, TextStyle};
use letters_sync::{
    ClientMessage, Hello, MAX_FRAME, PROTOCOL, Passage, Peer, Refusal, Role, ServerMessage, SessionId, Welcome,
    WireError,
};

/// A document with an edit of its own, so that its history, extent and revision are none of them empty.
fn model() -> Model {
    let mut model = Model::new(doc(), ReplicaId::from(7));
    model.apply([typed("p", 5, ",")]).unwrap();
    model
}

fn passage(model: &Model) -> Passage {
    let selection = Selection { anchor: Pos::new("p", 2), head: Pos::new("q", 3) };
    Passage::of(model, &selection).expect("the selection is in the document")
}

fn notes() -> Notes {
    Notes { notes: vec![note("n1", "p", 0..5, "Ada", "Why hello?")], extra: Extra::new() }
}

fn style() -> Style {
    let mut style = Style::default();
    style.types.insert("heading1".into(), TextStyle { size: 31.0, ..style.types["heading1"].clone() });
    style
}

fn peer(model: &Model, session: u32, selection: bool) -> Peer {
    Peer { session: SessionId(session), name: "Zoë".into(), colour: 3, selection: selection.then(|| passage(model)) }
}

/// One of every message a device sends, and each shape of those that have more than one.
fn from_devices() -> Vec<ClientMessage> {
    let model = model();
    let hello =
        |document: &str, extent, revision| Hello { protocol: PROTOCOL, document: document.into(), extent, revision };
    vec![
        ClientMessage::Hello(hello(ID, model.extent(), Some(model.revision()))),
        ClientMessage::Hello(hello("", Default::default(), None)),
        ClientMessage::Edits { seq: 1, updates: model.save() },
        ClientMessage::Edits { seq: u64::MAX, updates: Vec::new() },
        ClientMessage::Notes { seq: 2, notes: notes() },
        ClientMessage::Style { seq: 3, style: style() },
        ClientMessage::Selection(Some(passage(&model))),
        ClientMessage::Selection(None),
        ClientMessage::Ping,
    ]
}

/// One of every message a server sends, and each shape of those that have more than one.
fn from_servers() -> Vec<ServerMessage> {
    let model = model();
    let welcome = |updates, notes, peers| Welcome {
        protocol: PROTOCOL,
        session: SessionId(9),
        role: Role::Commenter,
        document: ID.into(),
        created: CREATED.into(),
        updates,
        extent: model.extent(),
        notes,
        peers,
    };
    vec![
        ServerMessage::Welcome(welcome(model.save(), notes(), vec![peer(&model, 1, true), peer(&model, 2, false)])),
        ServerMessage::Welcome(welcome(Vec::new(), Notes::default(), Vec::new())),
        ServerMessage::Held { seq: 4 },
        ServerMessage::Refused { seq: 5, why: Refusal::ReadOnly },
        ServerMessage::Edits { updates: model.save() },
        ServerMessage::Notes { notes: notes() },
        ServerMessage::Joined(peer(&model, 3, true)),
        ServerMessage::Selection { session: SessionId(3), selection: Some(passage(&model)) },
        ServerMessage::Selection { session: SessionId(3), selection: None },
        ServerMessage::Left { session: SessionId(3) },
        ServerMessage::Pong,
        ServerMessage::Goodbye { why: Refusal::Gone },
    ]
}

/// Every frame of both sides, each with whether a device wrote it.
fn frames() -> Vec<(bool, Vec<u8>)> {
    let devices = from_devices().into_iter().map(|message| (true, message.encode()));
    devices.chain(from_servers().into_iter().map(|message| (false, message.encode()))).collect()
}

/// Whether the side a frame is for reads it at all.
fn reads(from_device: bool, frame: &[u8]) -> Result<bool, WireError> {
    match from_device {
        true => ClientMessage::decode(frame).map(|message| message.is_some()),
        false => ServerMessage::decode(frame).map(|message| message.is_some()),
    }
}

#[test]
fn every_message_reads_back_as_it_was_sent() {
    for message in from_devices() {
        assert_eq!(ClientMessage::decode(&message.encode()), Ok(Some(message)));
    }
    for message in from_servers() {
        assert_eq!(ServerMessage::decode(&message.encode()), Ok(Some(message)));
    }
}

#[test]
fn what_a_later_build_adds_to_a_message_is_ignored() {
    for message in from_devices() {
        let mut frame = message.encode();
        frame.extend_from_slice(b"\x01more");
        assert_eq!(ClientMessage::decode(&frame), Ok(Some(message)));
    }
    for message in from_servers() {
        let mut frame = message.encode();
        frame.extend_from_slice(b"\x01more");
        assert_eq!(ServerMessage::decode(&frame), Ok(Some(message)));
    }
}

#[test]
fn a_kind_of_message_from_a_later_build_is_skipped() {
    let known = |kind: u8, from_device: bool| reads(from_device, &[kind]) != Ok(false);
    let unknown: Vec<u8> = (0..=u8::MAX).filter(|kind| !known(*kind, true) && !known(*kind, false)).collect();
    assert!(unknown.len() > 200, "most kinds are free for later builds: {}", unknown.len());
    for (from_device, frame) in frames() {
        // Neither side takes the other's messages for its own.
        assert_eq!(reads(!from_device, &frame), Ok(false), "kind {}", frame[0]);
        for kind in [unknown[0], unknown[unknown.len() / 2], u8::MAX] {
            let mut later = frame.clone();
            later[0] = kind;
            assert_eq!(reads(from_device, &later), Ok(false), "kind {kind}");
        }
    }
}

#[test]
fn a_frame_that_was_cut_short_is_refused() {
    assert!(ClientMessage::decode(&[]).is_err());
    assert!(ServerMessage::decode(&[]).is_err());
    for (from_device, frame) in frames() {
        for len in 1..frame.len() {
            let cut = reads(from_device, &frame[..len]);
            assert!(matches!(cut, Err(WireError::Malformed(_))), "kind {} cut to {len} bytes: {cut:?}", frame[0]);
        }
    }
}

#[test]
fn a_frame_longer_than_a_frame_may_be_is_refused() {
    for (from_device, frame) in frames() {
        let mut long = frame;
        long.resize(MAX_FRAME + 1, 0);
        assert_eq!(reads(from_device, &long), Err(WireError::TooLong));
    }
}

#[test]
fn a_field_that_does_not_hold_what_it_should_is_refused() {
    let seq = 1u64.to_le_bytes();
    let field = |bytes: &[u8]| [&(bytes.len() as u32).to_le_bytes()[..], bytes].concat();
    let cases: [(&str, bool, Vec<u8>); 6] = [
        ("a document id that is not text", true, [&[1][..], &PROTOCOL.to_le_bytes(), &field(&[0xff, 0xfe])].concat()),
        ("notes that are not notes", true, [&[3][..], &seq, &field(b"{\"notes\": 4}")].concat()),
        ("a style that is not a style", true, [&[4][..], &seq, &field(b"[]")].concat()),
        ("a caret that is nowhere", true, [&[5, 1][..], &field(b"here"), &field(b"there")].concat()),
        ("notes that are not notes", false, [&[69][..], &field(b"not json")].concat()),
        ("a length past the frame's end", false, [&[68][..], &u32::MAX.to_le_bytes(), b"abc"].concat()),
    ];
    for (what, from_device, frame) in cases {
        assert!(matches!(reads(from_device, &frame), Err(WireError::Malformed(_))), "{what}");
    }
}

#[test]
fn a_refusal_keeps_its_number_and_reads_as_a_sentence() {
    let known = [
        Refusal::NoAccess,
        Refusal::SignedOut,
        Refusal::Gone,
        Refusal::TooOld,
        Refusal::TooNew,
        Refusal::OtherDocument,
        Refusal::ReadOnly,
        Refusal::NoComments,
        Refusal::Unreadable,
        Refusal::AccessChanged,
        Refusal::TooLarge,
    ];
    let mut numbers = Vec::new();
    for why in known.into_iter().chain([Refusal::Other(200)]) {
        let frame = ServerMessage::Goodbye { why }.encode();
        assert_eq!(ServerMessage::decode(&frame), Ok(Some(ServerMessage::Goodbye { why })));
        numbers.push(frame[1]);
        let sentence = why.to_string();
        assert!(sentence.ends_with('.') && sentence.len() > 10, "{why:?} reads as {sentence:?}");
    }
    numbers.sort_unstable();
    numbers.dedup();
    assert_eq!(numbers.len(), known.len() + 1, "two refusals share a number");
}

#[test]
fn only_a_refusal_that_trying_again_does_not_mend_is_final() {
    let cases = [
        (Refusal::NoAccess, true),
        (Refusal::SignedOut, true),
        (Refusal::Gone, true),
        (Refusal::TooOld, true),
        (Refusal::TooNew, true),
        (Refusal::OtherDocument, true),
        (Refusal::TooLarge, true),
        (Refusal::ReadOnly, false),
        (Refusal::NoComments, false),
        (Refusal::Unreadable, false),
        (Refusal::AccessChanged, false),
        // A reason from a later build may pass, so the device asks again.
        (Refusal::Other(200), false),
    ];
    for (why, expected) in cases {
        assert_eq!(why.is_final(), expected, "{why:?}");
    }
}

#[test]
fn a_role_from_a_later_build_may_only_read() {
    let welcome = from_servers().remove(0);
    let mut frame = welcome.encode();
    // The role follows the kind, the protocol and the session.
    assert_eq!(frame[9], 1, "the fixture is a commenter");
    frame[9] = 9;
    let Ok(Some(ServerMessage::Welcome(read))) = ServerMessage::decode(&frame) else { panic!("the welcome reads") };
    assert_eq!(read.role, Role::Reader);
}
