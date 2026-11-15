//! A device with a document open through the server: what a shell holds of it, and its seat in the room.

use axum::http::header::{AUTHORIZATION, COOKIE};
use axum::http::{HeaderMap, Method};
use letters_api::Error;
use letters_model::{Anchor, Block, Extra, Model, Note, NoteKind, Notes, Op, Pos, ReplicaId, Run};
use letters_server::rooms::{Out, Seat};
use letters_server::session::{Reader, Signed};
use letters_sync::{Client as Line, Event, Local, Refusal};
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::error::TryRecvError;

use super::{Browser, Client, Server};

/// The time a desk tells its line. Nothing here waits on the line's own timers.
const NOW: u64 = 1_800_000_000_000;

pub struct Desk {
    pub line: Line,
    pub model: Option<Model>,
    pub notes: Notes,
    seat: Option<(Seat, Receiver<Out>)>,
    /// The numbers the server said it holds what was written up to.
    pub held: Vec<u64>,
    pub refused: Vec<Refusal>,
}

impl Desk {
    /// Opens the document `id` as `who` on a device that holds no copy of it. The desk is in session
    /// once it has settled.
    pub async fn open(server: &Server, who: &Client, id: &str) -> Result<Self, Error> {
        let mut headers = HeaderMap::new();
        let token = who.token.as_deref().unwrap_or_default();
        headers.insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        let signed = Signed::read(&server.app, &Method::GET, &headers).map_err(|refusal| refusal.0)?;
        Self::seated(server, &signed, id).await
    }

    /// Opens the document `id` as the page of a browser would, with whatever its cookies stand for: a
    /// sign-in, or a link that let it in.
    pub async fn paged(server: &Server, browser: &Browser, id: &str) -> Result<Self, Error> {
        let mut headers = HeaderMap::new();
        let cookies: Vec<String> = browser.cookies.iter().map(|(name, value)| format!("{name}={value}")).collect();
        headers.insert(COOKIE, cookies.join("; ").parse().unwrap());
        let Reader(signed) =
            Reader::read(&server.app, &Method::GET, &headers, Some(id)).map_err(|refusal| refusal.0)?;
        Self::seated(server, &signed, id).await
    }

    async fn seated(server: &Server, signed: &Signed, id: &str) -> Result<Self, Error> {
        let seat = server.app.join(signed, id).await.map_err(|refusal| refusal.0)?;
        let mut line = Line::new("");
        line.start(NOW);
        let mut desk = Self {
            line,
            model: None,
            notes: Notes::default(),
            seat: Some(seat),
            held: Vec::new(),
            refused: Vec::new(),
        };
        desk.step().await;
        Ok(desk)
    }

    /// The document's text, block by block.
    pub fn texts(&self) -> Vec<String> {
        self.model.as_ref().expect("the desk holds a copy").doc().walk().map(Block::plain_text).collect()
    }

    /// The person edits. Returns the number the line gave for what the desk holds now.
    pub fn edit(&mut self, ops: impl IntoIterator<Item = Op>) -> u64 {
        let model = self.model.as_mut().expect("the desk holds a copy");
        model.apply(ops).unwrap();
        self.notes.follow(model);
        self.line.changed(Local { model: self.model.as_ref(), notes: &self.notes }, NOW)
    }

    /// The person types `text` at the start of the document.
    pub fn write(&mut self, text: &str) -> u64 {
        let first = self.model.as_ref().expect("the desk holds a copy").doc().blocks[0].id.clone();
        self.edit([Op::InsertText { at: Pos::new(first, 0), text: text.into() }])
    }

    /// The person writes a comment at the start of the document.
    pub fn comment(&mut self, says: &str) -> u64 {
        let model = self.model.as_ref().expect("the desk holds a copy");
        let mut note = Note {
            id: letters_server::ids::block_id(),
            kind: NoteKind::Comment,
            anchor: Anchor::range(model.doc().blocks[0].id.clone(), 0..0),
            author: "Someone".into(),
            created: "2026-11-15T12:00:00Z".into(),
            resolved: false,
            content: vec![Run::plain(says)],
            replies: Vec::new(),
            extra: Extra::new(),
        };
        note.anchor.pin(model);
        self.notes.notes.push(note);
        self.line.changed(Local { model: self.model.as_ref(), notes: &self.notes }, NOW)
    }

    /// Whether the server said it keeps what the desk held when a call returned `number`.
    pub fn holds(&self, number: u64) -> bool {
        self.held.iter().any(|held| *held >= number)
    }

    /// The room of the document fails, as it would at a fault in its work.
    pub async fn fault(&self) {
        self.seat.as_ref().expect("the desk has a seat").0.fault().await;
    }

    /// The person closes the document.
    pub async fn leave(&mut self) {
        if let Some((seat, _)) = self.seat.take() {
            seat.leave().await;
        }
    }

    /// Does what the line asks and hears what the server sent. Says whether there was anything.
    async fn step(&mut self) -> bool {
        let mut busy = false;
        loop {
            while let Some(event) = self.line.poll() {
                busy = true;
                self.handle(event).await;
            }
            let Some((_, from)) = &mut self.seat else { return busy };
            match from.try_recv() {
                Ok(Out::Frame(frame)) => {
                    self.line.received(&frame, Local { model: self.model.as_ref(), notes: &self.notes }, NOW)
                }
                Ok(Out::Close) | Err(TryRecvError::Disconnected) => {
                    self.leave().await;
                    self.line.closed(NOW);
                }
                Err(TryRecvError::Empty) => return busy,
            }
            busy = true;
        }
    }

    async fn handle(&mut self, event: Event) {
        match event {
            Event::Open => match self.seat {
                Some(_) => self.line.opened(Local { model: self.model.as_ref(), notes: &self.notes }, NOW),
                // A socket that was closed is not opened again here: a test opens another desk.
                None => self.line.stop(),
            },
            Event::Send(frame) => {
                if let Some((seat, _)) = &self.seat {
                    seat.received(frame).await;
                }
            }
            Event::Close => self.leave().await,
            Event::Merge(updates) => {
                match &mut self.model {
                    Some(model) => drop(model.merge(&updates).unwrap()),
                    None => {
                        self.model =
                            Some(Model::load(&updates, ReplicaId::new(&letters_server::ids::block_id())).unwrap())
                    }
                }
                self.notes.follow(self.model.as_ref().unwrap());
                self.line.changed(Local { model: self.model.as_ref(), notes: &self.notes }, NOW);
            }
            Event::Notes(notes) => {
                self.notes.merge(&notes);
                if let Some(model) = &self.model {
                    self.notes.follow(model);
                }
                self.line.changed(Local { model: self.model.as_ref(), notes: &self.notes }, NOW);
            }
            Event::Held(number) => self.held.push(number),
            Event::Refused(why) => self.refused.push(why),
            Event::Link | Event::Progress | Event::Peers => {}
        }
    }
}

/// Lets everything that is on its way between these desks and the server arrive.
pub async fn settle<const N: usize>(mut desks: [&mut Desk; N]) {
    for _ in 0..10_000 {
        let mut busy = false;
        for desk in &mut desks {
            busy |= desk.step().await;
        }
        if !busy {
            return;
        }
    }
    panic!("the desks do not settle");
}
