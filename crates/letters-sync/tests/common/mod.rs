//! A server with one room, its folders and the devices on it: all in memory, and on one clock.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{self, Cursor};
use std::ops::Range;

use letters_format::{Archive, Error, Files, Save, Saved, Stamp, WorkingCopy};
use letters_model::{
    Anchor, Block, BlockKind, Document, Extra, Model, Note, NoteKind, Notes, Op, Pos, ReplicaId, Run, Selection, Style,
};
use letters_sync::{
    Access, Client, ClientMessage, Event, Local, Outbound, Refusal, Role, Room, ServerMessage, SessionId,
};

pub const ID: &str = "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f00";
pub const CREATED: &str = "2026-11-15T09:00:00Z";
/// The document's file in the server's folder.
pub const NAME: &str = "report.let";

/// A folder in memory, with the ways a folder fails.
#[derive(Default)]
pub struct Folder {
    pub files: BTreeMap<String, Vec<u8>>,
    /// When each file last changed, on a clock that ticks once per change.
    modified: BTreeMap<String, u64>,
    clock: u64,
    /// How often a file was read whole.
    pub reads: usize,
    /// Refuses every write and rename.
    pub read_only: bool,
    /// Renames onto this name are refused, as a file held open on SMB refuses them.
    pub locked: Option<String>,
}

impl Folder {
    fn put(&mut self, name: &str, bytes: Vec<u8>) {
        self.clock += 1;
        self.modified.insert(name.into(), self.clock);
        self.files.insert(name.into(), bytes);
    }

    fn refuse<T>(&self) -> io::Result<T> {
        Err(io::ErrorKind::PermissionDenied.into())
    }

    /// One entry of a `.let` file, such as a picture. `None` when the file has none by that name.
    pub fn read_entry(&self, name: &str, entry: &str) -> Option<Vec<u8>> {
        Archive::open(Cursor::new(self.files[name].clone())).unwrap().read(entry).unwrap()
    }

    /// The document a file holds, and its notes.
    pub fn read_document(&self, name: &str) -> (Document, Notes) {
        let mut file = Archive::open(Cursor::new(self.files[name].clone())).unwrap();
        (file.document().unwrap(), file.notes().unwrap())
    }
}

impl Files for Folder {
    fn stamp(&mut self, name: &str) -> io::Result<Option<Stamp>> {
        Ok(self.files.get(name).map(|bytes| Stamp { len: bytes.len() as u64, modified: self.modified[name] }))
    }

    fn read(&mut self, name: &str) -> io::Result<Option<Vec<u8>>> {
        self.reads += 1;
        Ok(self.files.get(name).cloned())
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        if self.read_only {
            return self.refuse();
        }
        self.put(name, bytes.to_vec());
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> io::Result<()> {
        if self.read_only || self.locked.as_deref() == Some(to) {
            return self.refuse();
        }
        let bytes = self.files.remove(from).ok_or(io::ErrorKind::NotFound)?;
        self.modified.remove(from);
        self.put(to, bytes);
        Ok(())
    }

    fn remove(&mut self, name: &str) -> io::Result<()> {
        self.files.remove(name).map(drop).ok_or(io::ErrorKind::NotFound.into())
    }
}

/// Two paragraphs: `p` reading "Hello world" and `q` reading "Second".
pub fn doc() -> Document {
    let paragraph = |id: &str, text: &str| Block::text(id, BlockKind::Paragraph, text);
    Document { blocks: vec![paragraph("p", "Hello world"), paragraph("q", "Second")], ..Document::default() }
}

pub fn typed(block: &str, offset: usize, text: &str) -> Op {
    Op::InsertText { at: Pos::new(block, offset), text: text.into() }
}

pub fn caret(block: &str, offset: usize) -> Selection {
    Selection::caret(Pos::new(block, offset))
}

pub fn texts(document: &Document) -> Vec<String> {
    document.walk().map(Block::plain_text).collect()
}

/// A note on a passage of a block. A comment unless the test says otherwise.
pub fn note(id: &str, block: &str, range: Range<usize>, author: &str, says: &str) -> Note {
    Note {
        id: id.into(),
        kind: NoteKind::Comment,
        anchor: Anchor::range(block, range),
        author: author.into(),
        created: CREATED.into(),
        resolved: false,
        content: vec![Run::plain(says)],
        replies: Vec::new(),
        extra: Extra::new(),
    }
}

/// The note with this id, wherever its text is now.
pub fn find<'a>(notes: &'a Notes, id: &str) -> Option<&'a Note> {
    notes.notes.iter().find(|note| note.id == id)
}

fn save_as<'a>(token: &'a str) -> Save<'a> {
    Save { name: NAME, token, label: "test", attempts: 3 }
}

/// The file as a writer on the folder holds it: another program on the same share.
pub fn open_file(folder: &Folder, replica: u64) -> WorkingCopy {
    let mut file = Archive::open(Cursor::new(folder.files[NAME].clone())).unwrap();
    WorkingCopy::open(&mut file, ReplicaId::from(replica)).unwrap()
}

/// Saves as every writer of a shared folder does.
pub fn save_file(folder: &mut Folder, copy: &mut WorkingCopy) -> Saved {
    letters_format::save(folder, copy, &save_as("other")).unwrap()
}

/// A document open on one device, and what its shell would hold beside it.
pub struct Device {
    pub name: &'static str,
    pub role: Role,
    pub model: Option<Model>,
    pub notes: Notes,
    pub client: Client,
    /// The session the server gave this device's socket, while it has one.
    pub session: Option<SessionId>,
    /// Whether the network reaches the server.
    pub reachable: bool,
    /// A device that froze: it neither reads nor ticks, and its socket stays as it is.
    pub frozen: bool,
    replica: u64,
    /// What the client handed out, for the tests to ask about.
    pub sent: Vec<ClientMessage>,
    pub held: Vec<u64>,
    pub refused: Vec<Refusal>,
    pub opens: u32,
}

impl Device {
    pub fn local(&self) -> Local<'_> {
        Local { model: self.model.as_ref(), notes: &self.notes }
    }

    pub fn document(&self) -> &Document {
        self.model.as_ref().expect("the device holds a copy").doc()
    }

    fn merge(&mut self, updates: &[u8]) {
        match &mut self.model {
            Some(model) => _ = model.merge(updates).unwrap(),
            None => self.model = Some(Model::load(updates, ReplicaId::from(self.replica)).unwrap()),
        }
        self.notes.follow(self.model.as_ref().unwrap());
    }

    /// Whether the device was told that the server keeps what it held when a call returned `number`.
    pub fn holds(&self, number: u64) -> bool {
        self.held.iter().any(|held| *held >= number)
    }

    /// The edits this device sent, leaving out everything else.
    pub fn edits_sent(&self) -> usize {
        self.sent.iter().filter(|message| matches!(message, ClientMessage::Edits { .. })).count()
    }
}

/// One server, one document, and the devices that have it open.
pub struct World {
    pub room: Option<Room>,
    /// The folder the document's file is in, and the server's own folder for the room's journal.
    pub folder: Folder,
    pub state: Folder,
    pub devices: Vec<Device>,
    /// Milliseconds since 1970.
    pub now: u64,
    /// How each save ended, and what went wrong between the room and its folders.
    pub saves: Vec<Saved>,
    pub faults: Vec<Error>,
    /// Every frame the room sent, with the session it was for.
    pub told: Vec<(SessionId, ServerMessage)>,
    replicas: u64,
}

impl World {
    /// A server whose folder holds [`doc`] as a file, with the room open and nobody in it.
    pub fn new() -> Self {
        let mut folder = Folder::default();
        let replica = ReplicaId::from(1);
        let mut first = WorkingCopy::new(ID, CREATED, doc(), Style::default(), replica);
        letters_format::save(&mut folder, &mut first, &save_as("first")).unwrap();
        let mut world = Self {
            room: None,
            folder,
            state: Folder::default(),
            devices: Vec::new(),
            now: 1_800_000_000_000,
            saves: Vec::new(),
            faults: Vec::new(),
            told: Vec::new(),
            replicas: 100,
        };
        world.open_room().unwrap();
        world
    }

    /// Opens the room over the file as it is, as a server does when the first session comes, and when it
    /// starts again.
    pub fn open_room(&mut self) -> Result<(), Error> {
        self.replicas += 1;
        let stamp = self.folder.stamp(NAME).unwrap().expect("the document has a file");
        let copy = open_file(&self.folder, self.replicas);
        let mut room = Room::open(copy, "/srv/letters/report.let", &mut self.state, self.now)?;
        room.read_as(stamp);
        self.room = Some(room);
        Ok(())
    }

    /// A device opens the document: with a copy of the file as it is now, or with none. It is online
    /// once the world has settled.
    pub fn join(&mut self, name: &'static str, role: Role, copy: bool) -> usize {
        self.replicas += 1;
        let held = copy.then(|| open_file(&self.folder, self.replicas));
        let notes = held.as_ref().map(|held| held.notes.clone()).unwrap_or_default();
        let model = held.map(|held| held.model().fork(ReplicaId::from(self.replicas + 1000)));
        self.join_with(name, role, if copy { ID } else { "" }, model, notes)
    }

    /// A device opens whatever it holds under `id` against this server's document.
    pub fn join_with(&mut self, name: &'static str, role: Role, id: &str, model: Option<Model>, notes: Notes) -> usize {
        self.replicas += 1;
        let mut device = Device {
            name,
            role,
            notes,
            model,
            client: Client::new(id),
            session: None,
            reachable: true,
            frozen: false,
            replica: self.replicas + 1000,
            sent: Vec::new(),
            held: Vec::new(),
            refused: Vec::new(),
            opens: 0,
        };
        device.client.start(self.now);
        self.devices.push(device);
        self.devices.len() - 1
    }

    /// The person at a device edits. Returns the number the client gave for what the device holds now.
    pub fn edit(&mut self, device: usize, ops: impl IntoIterator<Item = Op>) -> u64 {
        let device = &mut self.devices[device];
        device.model.as_mut().expect("the device holds a copy").apply(ops).unwrap();
        device.notes.follow(device.model.as_ref().unwrap());
        device.client.changed(Local { model: device.model.as_ref(), notes: &device.notes }, self.now)
    }

    /// The person at a device writes a note or changes one. Returns the number as [`World::edit`] does.
    pub fn annotate(&mut self, device: usize, change: impl FnOnce(&mut Notes)) -> u64 {
        let device = &mut self.devices[device];
        change(&mut device.notes);
        if let Some(model) = &device.model {
            device.notes.pin(model);
        }
        device.client.changed(Local { model: device.model.as_ref(), notes: &device.notes }, self.now)
    }

    /// The person at a device sets the style outside the editor.
    pub fn restyle(&mut self, device: usize, style: &Style) -> u64 {
        self.devices[device].client.restyled(style, self.now)
    }

    /// The person's caret moves, or leaves the page.
    pub fn select(&mut self, device: usize, selection: Option<Selection>) {
        let device = &mut self.devices[device];
        let model = device.model.as_ref().expect("the device holds a copy");
        device.client.selected(model, selection.as_ref(), self.now);
    }

    /// Sends a frame on a device's socket past its client, as a program that does not keep to the rules
    /// would.
    pub fn forge(&mut self, device: usize, frame: &[u8]) {
        let session = self.devices[device].session.expect("the device is in session");
        let room = self.room.as_mut().expect("the room is open");
        if let Err(error) = room.received(session, frame, &mut self.state, self.now) {
            self.faults.push(error);
        }
    }

    /// The server stops without saving and without a word, and every socket goes with it.
    pub fn stop_server(&mut self) {
        self.room = None;
        for device in &mut self.devices {
            if device.session.take().is_some() && !device.frozen {
                device.client.closed(self.now);
            }
        }
    }

    /// The server stops as it was asked to: the room writes what its file lacks and lets every socket
    /// go, and is gone.
    pub fn shut_down(&mut self) -> Result<Option<Saved>, Error> {
        let mut room = self.room.take().expect("the room is open");
        let saved = room.stop(&mut self.folder, &mut self.state, &save_as("stop"), self.now);
        while let Some(outbound) = room.poll() {
            self.deliver(outbound);
        }
        self.settle();
        saved
    }

    /// The network between a device and the server goes down.
    pub fn cut(&mut self, device: usize) {
        let (now, device) = (self.now, &mut self.devices[device]);
        device.reachable = false;
        if let (Some(session), Some(room)) = (device.session.take(), &mut self.room) {
            room.leave(session);
            device.client.closed(now);
        }
    }

    pub fn mend(&mut self, device: usize) {
        self.devices[device].reachable = true;
    }

    /// Lets everything that is on its way arrive, without the clock moving.
    pub fn settle(&mut self) {
        for _ in 0..10_000 {
            let mut busy = false;
            for device in 0..self.devices.len() {
                busy |= self.drain(device);
            }
            while let Some(outbound) = self.room.as_mut().and_then(Room::poll) {
                busy = true;
                self.deliver(outbound);
            }
            if !busy {
                return;
            }
        }
        panic!("the world does not settle");
    }

    /// Does what the room asked of its host: a frame goes to its device, or the device's socket closes.
    fn deliver(&mut self, outbound: Outbound) {
        let now = self.now;
        let (session, frame) = match outbound {
            Outbound::Send(session, frame) => (session, Some(frame)),
            Outbound::Close(session) => (session, None),
        };
        if let Some(frame) = &frame {
            self.told.push((session, ServerMessage::decode(frame).unwrap().unwrap()));
        }
        let to = self.devices.iter().position(|device| device.session == Some(session) && !device.frozen);
        let Some(to) = to else { return };
        let device = &mut self.devices[to];
        match frame {
            Some(frame) => {
                device.client.received(&frame, Local { model: device.model.as_ref(), notes: &device.notes }, now);
            }
            None => {
                device.session = None;
                device.client.closed(now);
            }
        }
        // A shell drains its client after every call, so a socket the client gave up is closed before the
        // server's own close of it could be told.
        self.drain(to);
    }

    /// Does what a device's client asks for. Returns whether it asked for anything.
    fn drain(&mut self, device: usize) -> bool {
        let mut busy = false;
        while !self.devices[device].frozen
            && let Some(event) = self.devices[device].client.poll()
        {
            busy = true;
            self.handle(device, event);
        }
        busy
    }

    fn handle(&mut self, device: usize, event: Event) {
        let now = self.now;
        let device = &mut self.devices[device];
        match event {
            Event::Open => {
                device.opens += 1;
                match self.room.as_mut().filter(|_| device.reachable) {
                    Some(room) => {
                        device.session = Some(room.join(Access { role: device.role, name: device.name.into() }, now));
                        device.client.opened(Local { model: device.model.as_ref(), notes: &device.notes }, now);
                    }
                    None => device.client.closed(now),
                }
            }
            Event::Send(frame) => {
                device.sent.push(ClientMessage::decode(&frame).unwrap().unwrap());
                if let (Some(room), Some(session)) = (&mut self.room, device.session)
                    && let Err(error) = room.received(session, &frame, &mut self.state, now)
                {
                    self.faults.push(error);
                }
            }
            Event::Close => {
                if let (Some(room), Some(session)) = (&mut self.room, device.session.take()) {
                    room.leave(session);
                }
            }
            Event::Merge(updates) => {
                device.merge(&updates);
                device.client.changed(Local { model: device.model.as_ref(), notes: &device.notes }, now);
            }
            Event::Notes(notes) => {
                device.notes.merge(&notes);
                if let Some(model) = &device.model {
                    device.notes.follow(model);
                }
                device.client.changed(Local { model: device.model.as_ref(), notes: &device.notes }, now);
            }
            Event::Held(number) => device.held.push(number),
            Event::Refused(why) => device.refused.push(why),
            Event::Link | Event::Progress | Event::Peers => {}
        }
    }

    /// Lets time pass, with every timer that falls due on the way.
    pub fn advance(&mut self, ms: u64) {
        let until = self.now + ms;
        for _ in 0..10_000 {
            self.settle();
            let clients =
                self.devices.iter().filter(|device| !device.frozen).filter_map(|device| device.client.wake_at());
            let next = clients.chain(self.room.as_ref().and_then(Room::wake_at)).min();
            let Some(next) = next.filter(|next| *next <= until) else {
                self.now = until;
                return self.settle();
            };
            self.now = self.now.max(next);
            self.tick();
        }
        panic!("the clock does not move");
    }

    fn tick(&mut self) {
        let now = self.now;
        for device in self.devices.iter_mut().filter(|device| !device.frozen) {
            if device.client.wake_at().is_some_and(|at| at <= now) {
                device.client.tick(now);
            }
        }
        let Some(room) = &mut self.room else { return };
        room.tick(now);
        if room.due(now) {
            let token = format!("save-{now}");
            match room.exchange(&mut self.folder, &mut self.state, &save_as(&token), now) {
                Ok(saved) => self.saves.extend(saved),
                Err(error) => self.faults.push(error),
            }
        }
    }

    /// Saves now, as a server does before it stops.
    pub fn flush(&mut self) -> Result<Saved, Error> {
        let room = self.room.as_mut().expect("the room is open");
        room.flush(&mut self.folder, &mut self.state, &save_as("flush"), self.now)
    }

    /// The document as the room holds it.
    pub fn held(&self) -> &Document {
        self.room.as_ref().expect("the room is open").copy().model().doc()
    }

    /// What the room has sent to a device's session so far.
    pub fn told_to(&self, device: usize) -> Vec<&ServerMessage> {
        let session = self.devices[device].session;
        self.told.iter().filter(|(to, _)| Some(*to) == session).map(|(_, message)| message).collect()
    }
}
