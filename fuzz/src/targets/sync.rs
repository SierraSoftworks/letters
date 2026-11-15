//! Realtime sync with anything on the socket: each side reads a frame or refuses it, a room holds every
//! session to its role, and what the room holds is there after a stop and in the file it saves.

use letters_format::{Archive, Save, WorkingCopy};
use letters_model::{Extent, Model, Notes, Selection, Style};
use letters_sync::{
    Access, Client, ClientMessage, Hello, Local, Outbound, PROTOCOL, Passage, Peer, Refusal, Role, Room, ServerMessage,
    SessionId, Welcome,
};

use super::history::sealed;
use crate::edits::{ops, sample, shown};
use crate::folder::Folder;
use crate::input::{Script, bounded, put};

const ID: &str = "0b5b1f0e-6d5c-4c58-9a51-2f3f0a1d7c10";
const CREATED: &str = "2026-01-01T00:00:00Z";
const NAME: &str = "notice.let";
const HOME: &str = "/srv/notice.let";
const STEPS: usize = 24;
const PAYLOAD: usize = 8 * 1024;
const ROLES: [Role; 3] = [Role::Editor, Role::Commenter, Role::Reader];

// The kinds of frame a script writes with a payload of its own, as letters-sync numbers them.
const EDITS: u8 = 2;
const NOTES: u8 = 3;
const STYLE: u8 = 4;
const SELECTION: u8 = 5;

fn save(token: &str) -> Save<'_> {
    Save { name: NAME, token, label: "fuzz", attempts: 2 }
}

fn open(folder: &Folder, replica: u64) -> WorkingCopy {
    let mut file = Archive::open(std::io::Cursor::new(folder.files[NAME].clone())).expect("the file opens");
    WorkingCopy::open(&mut file, replica.into()).expect("the file holds the document")
}

/// A length and then the bytes, as a frame holds text and bytes.
fn field(bytes: &[u8]) -> Vec<u8> {
    [&(bytes.len() as u32).to_le_bytes()[..], bytes].concat()
}

/// A message that carries a number and one payload.
fn numbered(kind: u8, seq: u8, payload: &[u8]) -> Vec<u8> {
    [&[kind][..], &u64::from(seq).to_le_bytes(), &field(payload)].concat()
}

/// The room's sockets, one of each role, and what they said hello with.
struct Sockets {
    sessions: [SessionId; 3],
}

impl Sockets {
    fn hello(room: &mut Room, state: &mut Folder, role: Role, now: u64) -> SessionId {
        let session = room.join(Access { role, name: format!("{role:?}") }, now);
        let hello = Hello { protocol: PROTOCOL, document: ID.into(), extent: Extent::default(), revision: None };
        let taken = room.received(session, &ClientMessage::Hello(hello).encode(), state, now);
        taken.expect("a folder that takes every write is journalled to");
        session
    }

    /// Reads everything the room has to say, and opens a socket again for each one it closed.
    fn drain(&mut self, room: &mut Room, state: &mut Folder, now: u64) {
        while let Some(outbound) = room.poll() {
            match outbound {
                Outbound::Send(_, frame) => {
                    let read = ServerMessage::decode(&frame);
                    assert!(matches!(read, Ok(Some(_))), "the room sent a frame that does not read: {read:?}");
                }
                Outbound::Close(closed) => {
                    if let Some(nth) = self.sessions.iter().position(|session| *session == closed) {
                        self.sessions[nth] = Self::hello(room, state, ROLES[nth], now);
                    }
                }
            }
        }
    }
}

/// Either side reads the frame or refuses it, and what it reads is written back the same.
fn read(frame: &[u8]) {
    if let Ok(Some(message)) = ClientMessage::decode(frame) {
        assert_eq!(
            ClientMessage::decode(&message.encode()),
            Ok(Some(message)),
            "a device's message reads back differently"
        );
    }
    if let Ok(Some(message)) = ServerMessage::decode(frame) {
        assert_eq!(
            ServerMessage::decode(&message.encode()),
            Ok(Some(message)),
            "a server's message reads back differently"
        );
    }
    if let Ok(extent) = Extent::from_bytes(frame) {
        assert!(Extent::from_bytes(&extent.to_bytes()).is_ok(), "an extent does not read back");
    }
}

/// A device takes the frame from its server, before the welcome and after it, and goes on.
fn hear(frame: &[u8]) {
    let (model, notes) = (Model::new(sample(), 3.into()), Notes::default());
    let local = Local { model: Some(&model), notes: &notes };
    for welcomed in [false, true] {
        let mut client = Client::new(ID);
        client.start(0);
        client.opened(local, 0);
        if welcomed {
            client.received(&welcome(&model).encode(), local, 0);
        }
        client.received(frame, local, 1);
        client.changed(local, 2);
        client.tick(60_000);
        // Whatever it asks for, it asks a bounded number of times.
        let asked = std::iter::from_fn(|| client.poll()).take(1_000).count();
        assert!(asked < 1_000, "a device does not stop asking");
        let _ = (client.link(), client.progress(), client.peers(), client.wake_at());
    }
}

fn welcome(model: &Model) -> ServerMessage {
    ServerMessage::Welcome(Welcome {
        protocol: PROTOCOL,
        session: SessionId(1),
        role: Role::Editor,
        document: ID.into(),
        created: CREATED.into(),
        updates: Vec::new(),
        extent: model.extent(),
        notes: Notes::default(),
        peers: Vec::new(),
    })
}

pub fn run(data: &[u8]) {
    read(bounded(data));
    hear(bounded(data));

    let mut script = Script::new(data);
    let (mut folder, mut state) = (Folder::default(), Folder::default());
    let mut first = WorkingCopy::new(ID, CREATED, sample(), Style::default(), 1.into());
    letters_format::save(&mut folder, &mut first, &save("first")).expect("a new document is saved");
    let mut room = Room::open(open(&folder, 2), HOME, &mut state, 0).expect("a room opens over a new file");
    let mut sockets = Sockets { sessions: ROLES.map(|role| Sockets::hello(&mut room, &mut state, role, 0)) };
    sockets.drain(&mut room, &mut state, 0);
    // A sound device, for edits that fit the document.
    let mut device = open(&folder, 4).model().fork(5.into());

    let mut now = 0;
    for _ in 0..STEPS {
        if script.done() {
            break;
        }
        now += 1_000;
        let (who, seq) = (script.pick(ROLES.len()), script.byte());
        let frame = match script.pick(6) {
            0 => script.bytes(PAYLOAD).to_vec(),
            1 => numbered(EDITS, seq, &sealed(script.bytes(PAYLOAD))),
            2 => {
                let _ = device.apply(ops(&mut script, &shown(&device), 3));
                numbered(EDITS, seq, &device.updates_for(&room.copy().model().extent()))
            }
            3 => numbered(NOTES, seq, script.bytes(PAYLOAD)),
            4 => numbered(STYLE, seq, script.bytes(PAYLOAD)),
            _ => [&[SELECTION, 1][..], &field(script.bytes(64)), &field(script.bytes(64))].concat(),
        };

        let before = (shown(room.copy().model()), room.copy().notes.clone());
        let taken = room.received(sockets.sessions[who], &frame, &mut state, now);
        let changed = taken.expect("a folder that takes every write is journalled to");
        let after = (shown(room.copy().model()), room.copy().notes.clone());
        match ROLES[who] {
            Role::Editor => {}
            Role::Commenter => assert_eq!(after.0, before.0, "a commenter changed the document"),
            Role::Reader => assert!(after == before, "a reader changed the document or its notes"),
        }
        assert!(changed || after == before, "the room changed and did not say so");
        sockets.drain(&mut room, &mut state, now);
    }

    // A room that is opened after a stop holds what this one does, with no save in between.
    let held = shown(room.copy().model());
    let again = Room::open(open(&folder, 6), HOME, &mut state, now);
    let again = again.expect("a room opens over what the last one journalled");
    assert_eq!(shown(again.copy().model()), held, "a room that stopped came back with another document");
    assert_eq!(again.copy().notes, room.copy().notes, "a room that stopped came back with other notes");

    room.flush(&mut folder, &mut state, &save("last"), now).expect("a folder that takes every write is saved to");
    assert_eq!(shown(open(&folder, 7).model()), held, "the file holds another document than the room");
}

/// One script that sends each kind of frame from each role, and a frame of each kind of message.
pub fn seeds() -> Vec<Vec<u8>> {
    let notes = include_str!("../../../crates/letters-format/corpus/notes/report.json");
    let style = include_str!("../../../crates/letters-format/corpus/style/report.json");
    let mut writer = Model::new(sample(), 8.into());
    let start = writer.extent();
    let first = sample().blocks[0].id.clone();
    let caret = Selection::caret(letters_model::Pos::new(first, 0));
    let passage = Passage::of(&writer, &caret).expect("the sample's first block is in it");
    let _ = writer.apply(ops(&mut Script::new(&[0, 1, 2, 3, 4, 5, 6, 7]), &shown(&writer), 1));
    let updates = writer.updates_for(&start);

    let mut script = Vec::new();
    for who in 0..ROLES.len() as u8 {
        script.extend([who, 1, 0]);
        put(&mut script, PAYLOAD, &ClientMessage::Ping.encode());
        script.extend([who, 2, 1]);
        put(&mut script, PAYLOAD, &updates);
        script.extend([who, 3, 2, 1, 7, 0, 3, 1, 4]);
        script.extend([who, 4, 3]);
        put(&mut script, PAYLOAD, notes.as_bytes());
        script.extend([who, 5, 4]);
        put(&mut script, PAYLOAD, style.as_bytes());
        script.extend([who, 6, 5]);
        put(&mut script, 64, passage.anchor.to_string().as_bytes());
        put(&mut script, 64, passage.head.to_string().as_bytes());
    }

    let peer = Peer { session: SessionId(2), name: "Ada".into(), colour: 1, selection: Some(passage.clone()) };
    let hello =
        Hello { protocol: PROTOCOL, document: ID.into(), extent: writer.extent(), revision: Some(writer.revision()) };
    let notes = Notes::from_json(notes).expect("the corpus holds notes");
    let style = Style::from_json(style).expect("the corpus holds a style");
    let devices = [
        ClientMessage::Hello(hello),
        ClientMessage::Edits { seq: 1, updates: updates.clone() },
        ClientMessage::Notes { seq: 2, notes: notes.clone() },
        ClientMessage::Style { seq: 3, style },
        ClientMessage::Selection(Some(passage)),
    ];
    let mut greeting = welcome(&writer);
    if let ServerMessage::Welcome(welcome) = &mut greeting {
        (welcome.updates, welcome.notes, welcome.peers) = (writer.save(), notes.clone(), vec![peer.clone()]);
    }
    let servers = [
        greeting,
        ServerMessage::Held { seq: 1 },
        ServerMessage::Refused { seq: 2, why: Refusal::ReadOnly },
        ServerMessage::Edits { updates },
        ServerMessage::Notes { notes },
        ServerMessage::Joined(peer),
        ServerMessage::Goodbye { why: Refusal::AccessChanged },
    ];
    let frames = devices.iter().map(ClientMessage::encode).chain(servers.iter().map(ServerMessage::encode));
    [script].into_iter().chain(frames).collect()
}
