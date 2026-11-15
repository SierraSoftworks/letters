//! A document's socket as a device opens it: who is let in, and that frames travel both ways.

mod common;

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use common::{Server, URL, texts};
use letters_api::paths;
use letters_model::{Extent, Model, Op, Pos, ReplicaId};
use letters_server::session::COOKIE_NAME;
use letters_sync::{ClientMessage, Hello, PROTOCOL, Refusal, ServerMessage};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::header::{AUTHORIZATION, COOKIE, ORIGIN};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error, Message, WebSocket};

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

/// Asks for the socket of the document `id` with these headers. `Err` with the status of a refusal.
fn dial(at: SocketAddr, id: &str, headers: &[(tungstenite::http::HeaderName, String)]) -> Result<Socket, u16> {
    let mut request = format!("ws://{at}{}", paths::socket(id)).into_client_request().unwrap();
    for (name, value) in headers {
        request.headers_mut().insert(name, value.parse().unwrap());
    }
    match tungstenite::connect(request) {
        Ok((socket, _)) => {
            if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
                stream.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
            }
            Ok(socket)
        }
        Err(Error::Http(refused)) => Err(refused.status().as_u16()),
        Err(error) => panic!("the server did not answer: {error}"),
    }
}

/// The next frame the server sends, read as the room wrote it. `None` once the server has closed.
fn heard(socket: &mut Socket) -> Option<ServerMessage> {
    loop {
        match socket.read().expect("the server says something in time") {
            Message::Binary(frame) => return Some(ServerMessage::decode(&frame).unwrap().expect("a known message")),
            Message::Close(_) => return None,
            _ => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_socket_is_refused_with_a_status_before_it_opens() {
    let server = Server::new("socket-refused");
    let ada = server.person("ada", &["family"]);
    let id = ada.start("family", "", "Plan").await.id.unwrap();
    let at = server.listen().await;
    let bearer = |who: &common::Client| (AUTHORIZATION, format!("Bearer {}", who.token.as_deref().unwrap()));
    let browser = server.browser("ada", &["family"]);
    let cookie = (COOKIE, format!("{COOKIE_NAME}={}", browser.token.as_deref().unwrap()));
    let cases = [
        (vec![], id.as_str(), Some(401), "nobody is signed in"),
        (vec![bearer(&ada)], "no-such-document", Some(404), "there is no such document"),
        (vec![bearer(&server.person("eve", &[]))], id.as_str(), Some(404), "the document is out of reach"),
        (vec![cookie.clone()], id.as_str(), Some(403), "a browser was not on the server's page"),
        (vec![cookie.clone(), (ORIGIN, "https://elsewhere.example".into())], id.as_str(), Some(403), "another site"),
        (vec![cookie, (ORIGIN, URL.into())], id.as_str(), None, "the server's own page"),
        (vec![bearer(&ada)], id.as_str(), None, "the desktop app"),
    ];
    for (headers, id, refused, what) in cases {
        assert_eq!(dial(at, id, &headers).err(), refused, "{what}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn frames_travel_both_ways_and_the_server_says_why_it_closes() {
    let server = Server::new("socket-frames");
    let ada = server.person("ada", &["family"]);
    let id = ada.start("family", "", "Plan").await.id.unwrap();
    let at = server.listen().await;
    let mut socket = dial(at, &id, &[(AUTHORIZATION, format!("Bearer {}", ada.token.as_deref().unwrap()))]).unwrap();

    let hello = Hello { protocol: PROTOCOL, document: String::new(), extent: Extent::default(), revision: None };
    socket.send(Message::Binary(ClientMessage::Hello(hello).encode().into())).unwrap();
    let Some(ServerMessage::Welcome(welcome)) = heard(&mut socket) else { panic!("the server answers a hello") };
    assert_eq!(welcome.document, id);
    assert!(!welcome.updates.is_empty(), "a device with no copy is sent the document");

    ada.post::<()>(&paths::trash(&id), &()).await.unwrap();
    assert_eq!(heard(&mut socket), Some(ServerMessage::Goodbye { why: Refusal::Gone }));
    assert_eq!(heard(&mut socket), None, "and then the socket is closed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_is_told_to_stop_saves_what_is_open_first() {
    let server = Server::new("socket-stop");
    let ada = server.person("ada", &["family"]);
    let id = ada.start("family", "", "Plan").await.id.unwrap();
    let (at, stop, served) = server.serving().await;
    let mut socket = dial(at, &id, &[(AUTHORIZATION, format!("Bearer {}", ada.token.as_deref().unwrap()))]).unwrap();
    let hello = Hello { protocol: PROTOCOL, document: String::new(), extent: Extent::default(), revision: None };
    socket.send(Message::Binary(ClientMessage::Hello(hello).encode().into())).unwrap();
    let Some(ServerMessage::Welcome(welcome)) = heard(&mut socket) else { panic!("the server answers a hello") };

    let mut model = Model::load(&welcome.updates, ReplicaId::from(7)).unwrap();
    let (before, first) = (model.extent(), model.doc().blocks[0].id.clone());
    model.apply([Op::InsertText { at: Pos::new(first, 0), text: "Written just before the stop. ".into() }]).unwrap();
    let edits = ClientMessage::Edits { seq: 1, updates: model.updates_for(&before) };
    socket.send(Message::Binary(edits.encode().into())).unwrap();
    assert_eq!(heard(&mut socket), Some(ServerMessage::Held { seq: 1 }));

    stop.send(()).unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(20), served).await;
    assert!(matches!(ended, Ok(Ok(Ok(())))), "the server stops though a socket is open: {ended:?}");

    // The file itself is read, as whatever starts next will find it.
    let file = std::fs::read(server.scratch.0.join("family/Plan.let")).unwrap();
    assert!(texts(file).join("\n").contains("Written just before the stop."));
    assert!(server.scratch.names("state/rooms").is_empty(), "nothing is left for the next start to recover");
    assert_eq!(heard(&mut socket), None, "the socket is closed, with nothing said that would keep the device away");
}
