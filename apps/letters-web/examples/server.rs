//! The Letters server itself on a port of this machine, with people signed in: for trying the browser's
//! editor against the server it is made for, beside a device that holds a copy as the desktop app does.
//!
//! `LETTERS_WEB_DIST=$PWD/dist cargo run --release -p letters-web --example server -- <folder> [port]`
//!
//! The server keeps its state and its spaces under `<folder>`: each person's own space, and "Family",
//! which the group `family` edits, `reviewers` comment in and `friends` read. Its documents can be shared
//! by link. Built without
//! `LETTERS_WEB_DIST` it has no editor, and shows its documents to read. It prints a line of JSON: its
//! address, and the cookie of a browser for each of ada (family), grace (reviewers) and alan (friends).
//!
//! Each line of input is something for a device to do, signed in as ben (family) with a bearer token:
//! `join <id>` opens a document, `write <text>` writes at its start, and `texts` prints its blocks as a
//! line of JSON once what is on its way has arrived. The server stops when its input ends.

use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{env, fs};

use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method};
use letters_api::Person;
use letters_model::{Block, Model, Notes, Op, Pos, ReplicaId};
use letters_server::config::Config;
use letters_server::rooms::{Out, Seat};
use letters_server::session::{Carrier, Signed};
use letters_server::store::Store;
use letters_server::{App, Shared};
use letters_sync::{Client, Event, Local};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{self, Receiver};

const PEOPLE: [(&str, &str, &str); 3] =
    [("ada", "Ada Lovelace", "family"), ("grace", "Grace Hopper", "reviewers"), ("alan", "Alan Turing", "friends")];

/// Who the device is signed in as.
const BEN: (&str, &str, &str) = ("ben", "Ben", "family");

/// The settings of a server at `url` that keeps everything under `folder`. Nobody signs in through its
/// provider, which is not there.
fn settings(folder: &str, url: &str) -> String {
    format!(
        r#"
[server]
name = "Letters on this machine"
url = "{url}"
state = '{folder}/state'

[auth.oidc]
issuer = "https://id.example.test"
client_id = "letters"

[links]
enabled = true

[[space]]
name = "Home"
path = '{folder}/home/{{username}}'

[[space]]
name = "Family"
path = '{folder}/family'
editors = ["family"]
commenters = ["reviewers"]
readers = ["friends"]
"#
    )
}

/// A document open on a device: its copy, and its seat in the document's room.
struct Desk {
    line: Client,
    model: Option<Model>,
    notes: Notes,
    seat: Seat,
    from: Receiver<Out>,
    started: Instant,
}

impl Desk {
    fn now(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Does what the line asks and hears what the server sent, until nothing has come for a moment.
    async fn settle(&mut self) {
        loop {
            while let Some(event) = self.line.poll() {
                self.handle(event).await;
            }
            let Ok(Some(out)) = tokio::time::timeout(Duration::from_millis(300), self.from.recv()).await else {
                return;
            };
            let (now, local) = (self.now(), Local { model: self.model.as_ref(), notes: &self.notes });
            match out {
                Out::Frame(frame) => self.line.received(&frame, local, now),
                Out::Close => self.line.closed(now),
            }
        }
    }

    async fn handle(&mut self, event: Event) {
        let now = self.now();
        match event {
            Event::Open => self.line.opened(Local { model: self.model.as_ref(), notes: &self.notes }, now),
            Event::Send(frame) => self.seat.received(frame).await,
            Event::Merge(updates) => {
                let merged = match &mut self.model {
                    Some(model) => model.merge(&updates).map(drop).map_err(|error| error.to_string()),
                    None => Model::load(&updates, ReplicaId::new(&format!("desk{}", std::process::id())))
                        .map(|model| self.model = Some(model))
                        .map(|()| self.model.iter_mut().for_each(|model| model.set_author(BEN.1)))
                        .map_err(|error| error.to_string()),
                };
                if let Err(error) = merged {
                    eprintln!("the device could not merge what the server sent: {error}");
                }
                self.told(now);
            }
            Event::Notes(notes) => {
                self.notes.merge(&notes);
                self.told(now);
            }
            Event::Refused(why) => eprintln!("the server refused the device: {why}"),
            Event::Close | Event::Held(_) | Event::Link | Event::Progress | Event::Peers => {}
        }
    }

    /// Tells the line how the copy stands, after the notes have followed the text.
    fn told(&mut self, now: u64) {
        if let Some(model) = &self.model {
            self.notes.follow(model);
        }
        self.line.changed(Local { model: self.model.as_ref(), notes: &self.notes }, now);
    }

    /// The person at the device types `text` at the start of the document.
    fn write(&mut self, text: &str) {
        let Some(model) = &mut self.model else { return eprintln!("the device holds no copy yet") };
        let first = model.doc().blocks[0].id.clone();
        if let Err(error) = model.apply([Op::InsertText { at: Pos::new(first, 0), text: text.into() }]) {
            eprintln!("the device could not write: {error:?}");
        }
        let now = self.now();
        self.told(now);
    }

    fn texts(&self) -> Vec<String> {
        self.model.iter().flat_map(|model| model.doc().walk().map(Block::plain_text)).collect()
    }
}

/// Opens the document `id` on a device whose person carries `token` in a header.
async fn join(app: &Shared, token: &str, id: &str) -> Result<Desk, String> {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, format!("Bearer {token}").parse().map_err(|_| "the token is no header")?);
    let signed = Signed::read(app, &Method::GET, &headers).map_err(|refusal| refusal.0.message)?;
    let (seat, from) = app.join(&signed, id).await.map_err(|refusal| refusal.0.message)?;
    let (mut line, started) = (Client::new(id), Instant::now());
    line.start(0);
    Ok(Desk { line, model: None, notes: Notes::default(), seat, from, started })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut given = env::args().skip(1);
    let folder = PathBuf::from(given.next().ok_or("name a folder for the server to keep its files in")?);
    let port = given.next().map_or(Ok(0), |port| port.parse::<u16>())?;
    fs::create_dir_all(folder.join("family"))?;
    for (username, ..) in PEOPLE.into_iter().chain([BEN]) {
        fs::create_dir_all(folder.join("home").join(username))?;
    }
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let url = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let config = Config::parse(&settings(&folder.display().to_string(), &url)).map_err(|error| error.to_string())?;
    let app: Shared = Arc::new(App::new(config, Store::in_memory()));

    let signed = |username: &str, name: &str, group: &str, carrier| {
        let person = Person { id: format!("sub-{username}"), username: username.into(), name: name.into() };
        app.sign_in(person, vec![group.to_owned()], carrier)
            .map(|token| token.token)
            .map_err(|refusal| refusal.0.message)
    };
    let mut cookies = serde_json::Map::new();
    for (username, name, group) in PEOPLE {
        cookies.insert(username.to_owned(), signed(username, name, group, Carrier::Cookie)?.into());
    }
    let (username, name, group) = BEN;
    let bearer = signed(username, name, group, Carrier::Bearer)?;
    println!("{}", serde_json::json!({ "url": url, "cookies": cookies }));

    // The input is read on a thread of its own: the server answers while the device has nothing to do.
    let (lines, mut asked) = mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || io::stdin().lock().lines().map_while(Result::ok).all(|line| lines.send(line).is_ok()));
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(letters_server::serve(app.clone(), listener, async move { drop(stopped.await) }));

    let mut desk: Option<Desk> = None;
    while let Some(line) = asked.recv().await {
        let (verb, rest) = line.split_once(' ').unwrap_or((&line, ""));
        match (verb, &mut desk) {
            ("join", _) => match join(&app, &bearer, rest).await {
                Ok(joined) => desk = Some(joined),
                Err(error) => eprintln!("the device could not open {rest}: {error}"),
            },
            ("write", Some(desk)) => desk.write(rest),
            ("texts", Some(desk)) => {
                desk.settle().await;
                println!("{}", serde_json::json!(desk.texts()));
            }
            _ => eprintln!("not understood: {line}"),
        }
        if let Some(desk) = &mut desk {
            desk.settle().await;
        }
    }
    drop(desk);
    drop(stop);
    serving.await??;
    Ok(())
}
