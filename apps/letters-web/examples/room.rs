//! A document's room on a server, over standard input and output: the browser tests put it behind a
//! page's socket, so that the editor is tried against what a server runs and not against a likeness.
//!
//! `cargo run -p letters-web --example room -- <opened.json> [document.json]`
//!
//! The files are the ones the `page` example writes the served page from: what the server says of the
//! document, and the document, which is the welcome document if none is named.
//!
//! Each way, a record is a kind in one byte, a connection and a length as four bytes each, least first,
//! and that many bytes. In: 0 joins a connection, as the role its first byte names (0 reads, 1 comments,
//! 2 edits) under the name that follows; 1 is a frame from it; 2 says it closed; 3 asks for the document.
//! Out: 0 is a frame for a connection; 1 closes it; 2 is the room's `document.json`.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::time::Instant;
use std::{env, fs};

use letters_api::Opened;
use letters_format::{Files, Stamp, WorkingCopy};
use letters_model::{Document, ReplicaId, Style};
use letters_sync::{Access, Outbound, Role, Room, SessionId};
use letters_web::page;

/// The document's journal, kept nowhere: the room is gone with the test.
#[derive(Default)]
struct Memory(HashMap<String, Vec<u8>>);

impl Files for Memory {
    fn stamp(&mut self, name: &str) -> io::Result<Option<Stamp>> {
        Ok(self.0.get(name).map(|file| Stamp { len: file.len() as u64, modified: 0 }))
    }

    fn read(&mut self, name: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(self.0.get(name).cloned())
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.0.insert(name.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> io::Result<()> {
        let file = self.0.remove(from).ok_or(io::ErrorKind::NotFound)?;
        self.0.insert(to.to_owned(), file);
        Ok(())
    }

    fn remove(&mut self, name: &str) -> io::Result<()> {
        self.0.remove(name).map(drop).ok_or(io::ErrorKind::NotFound.into())
    }
}

/// Reads the next record. `None` when the tests have closed the pipe.
fn record(from: &mut impl Read) -> io::Result<Option<(u8, u32, Vec<u8>)>> {
    let mut head = [0; 9];
    match from.read_exact(&mut head) {
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        read => read?,
    }
    let [kind, rest @ ..] = head;
    let number = |at: usize| u32::from_le_bytes([rest[at], rest[at + 1], rest[at + 2], rest[at + 3]]);
    let mut bytes = vec![0; number(4) as usize];
    from.read_exact(&mut bytes)?;
    Ok(Some((kind, number(0), bytes)))
}

fn write(to: &mut impl Write, kind: u8, connection: u32, bytes: &[u8]) -> io::Result<()> {
    to.write_all(&[kind])?;
    to.write_all(&connection.to_le_bytes())?;
    to.write_all(&(bytes.len() as u32).to_le_bytes())?;
    to.write_all(bytes)
}

fn main() -> io::Result<()> {
    let said = env::args().nth(1).ok_or_else(|| io::Error::other("name the file that holds what the server says"))?;
    let said: Opened = serde_json::from_str(&fs::read_to_string(said)?)?;
    let mut blocks = 0;
    let document = match env::args().nth(2) {
        Some(path) => Document::from_json(&fs::read_to_string(path)?).map_err(io::Error::other)?,
        // The same welcome the page example writes into a page that was given no document.
        None => page::welcome(&mut || {
            blocks += 1;
            format!("welcome{blocks}")
        }),
    };
    // The style the page was sent is the room's, as it is on a server.
    let style = said.style.and_then(|style| Style::from_json(&style.to_string()).ok()).unwrap_or_default();
    let copy = WorkingCopy::new(&said.id, &said.created, document, style, ReplicaId::new("room"));
    let (mut state, started) = (Memory::default(), Instant::now());
    let now = || started.elapsed().as_millis() as u64;
    let mut room = Room::open(copy, "the tests", &mut state, now()).map_err(io::Error::other)?;

    let mut sessions: Vec<(u32, SessionId)> = Vec::new();
    let (mut from, mut to) = (io::stdin().lock(), io::stdout().lock());
    while let Some((kind, connection, bytes)) = record(&mut from)? {
        let session = sessions.iter().find(|(known, _)| *known == connection).map(|(_, session)| *session);
        match (kind, session) {
            (0, None) => {
                let role = match bytes.first() {
                    Some(0) => Role::Reader,
                    Some(1) => Role::Commenter,
                    _ => Role::Editor,
                };
                let name = String::from_utf8_lossy(bytes.get(1..).unwrap_or_default()).into_owned();
                sessions.push((connection, room.join(Access { role, name }, now())));
            }
            (1, Some(session)) => _ = room.received(session, &bytes, &mut state, now()).map_err(io::Error::other)?,
            (2, Some(session)) => {
                room.leave(session);
                sessions.retain(|(known, _)| *known != connection);
            }
            (3, _) => write(&mut to, 2, connection, room.copy().model().doc().to_json().as_bytes())?,
            _ => {}
        }
        while let Some(asked) = room.poll() {
            let (kind, session, bytes) = match asked {
                Outbound::Send(session, frame) => (0, session, frame),
                Outbound::Close(session) => (1, session, Vec::new()),
            };
            if let Some((connection, _)) = sessions.iter().find(|(_, known)| *known == session) {
                write(&mut to, kind, *connection, &bytes)?;
            }
        }
        to.flush()?;
    }
    Ok(())
}
