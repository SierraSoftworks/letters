//! The socket a document syncs over, against a server on this machine.

mod common;

use std::net::TcpListener;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use letters::remote::Roots;
use letters::socket::{Closed, Heard, Socket};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::{Message, http};

/// The largest frame the sockets here take, in bytes.
const MOST: usize = 2 * 1024 * 1024;

/// What a client asked the server for: the path, and the sign-in and the name it gave.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Asked {
    path: String,
    authorization: Option<String>,
    agent: Option<String>,
}

/// A server on this machine that takes sockets over TLS as `localhost`.
///
/// - `/echo` sends every frame back. The frame `bye` has it close the socket with a code and a reason,
///   and the frame `drop` has it hang up.
/// - `/greet` sends a frame as soon as the socket is open.
/// - Any other path is refused with 403.
struct Server {
    port: u16,
    certificate: Vec<u8>,
    asked: Arc<Mutex<Vec<Asked>>>,
    /// The sockets that were closed from the other side.
    closed: Arc<Mutex<usize>>,
}

impl Server {
    fn start() -> Self {
        let (config, certificate) = common::tls();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (asked, closed) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(0)));
        let (heard, ended) = (asked.clone(), closed.clone());
        thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                let Ok(connection) = rustls::ServerConnection::new(config.clone()) else { continue };
                let (heard, ended) = (heard.clone(), ended.clone());
                thread::spawn(move || serve(rustls::StreamOwned::new(connection, socket), &heard, &ended));
            }
        });
        Self { port, certificate, asked, closed }
    }

    fn address(&self, path: &str) -> String {
        format!("wss://localhost:{}{path}", self.port)
    }

    fn roots(&self) -> Roots {
        Roots::These(vec![self.certificate.clone()])
    }

    /// Opens a socket at `path`, and gives what it tells.
    fn open(&self, path: &str, most: usize) -> (Socket, mpsc::Receiver<Heard>) {
        let (tell, told) = mpsc::channel();
        let heard = move |heard| drop(tell.send(heard));
        (Socket::open(&self.address(path), || Ok("a-sign-in".to_owned()), &self.roots(), most, heard), told)
    }
}

#[expect(clippy::result_large_err, reason = "what the answer to a request is, is for tungstenite to say")]
fn serve(stream: impl std::io::Read + std::io::Write, asked: &Mutex<Vec<Asked>>, closed: &Mutex<usize>) {
    let mut path = String::new();
    let answer = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        let said = |name: http::HeaderName| Some(request.headers().get(name)?.to_str().ok()?.to_owned());
        path = request.uri().path().to_owned();
        let (authorization, agent) = (said(http::header::AUTHORIZATION), said(http::header::USER_AGENT));
        asked.lock().unwrap().push(Asked { path: path.clone(), authorization, agent });
        match path.as_str() {
            "/echo" | "/greet" => Ok(response),
            _ => Err(http::Response::builder().status(403).body(None).unwrap()),
        }
    };
    // A client that does not believe the certificate, or is refused, has no socket.
    let Ok(mut socket) = tungstenite::accept_hdr(stream, answer) else { return };
    if path == "/greet" {
        socket.send(Message::Binary(b"hello".to_vec().into())).unwrap();
    }
    loop {
        match socket.read() {
            Ok(Message::Binary(frame)) if frame.as_ref() == b"drop" => return,
            Ok(Message::Binary(frame)) if frame.as_ref() == b"bye" => {
                let _ = socket.close(Some(CloseFrame { code: CloseCode::from(4000), reason: "done".into() }));
            }
            Ok(Message::Binary(frame)) => {
                if socket.send(Message::Binary(frame)).is_err() {
                    return;
                }
            }
            Ok(Message::Close(_)) => *closed.lock().unwrap() += 1,
            Ok(_) => {}
            Err(_) => return,
        }
    }
}

/// What a socket tells next, if it tells anything in good time.
fn next(told: &mpsc::Receiver<Heard>) -> Option<Heard> {
    told.recv_timeout(Duration::from_secs(5)).ok()
}

#[test]
fn frames_go_both_ways_for_whoever_signed_in() {
    let server = Server::start();
    let (socket, told) = server.open("/echo", MOST);
    // A frame sent while the socket is opening waits for it.
    socket.send(b"early".to_vec());
    assert_eq!(next(&told), Some(Heard::Open));
    assert_eq!(next(&told), Some(Heard::Frame(b"early".to_vec())));

    // One far longer than the network carries at once, as a document's whole history is.
    let long: Vec<u8> = (0..1024 * 1024u32).map(|n| (n % 251) as u8).collect();
    socket.send(long.clone());
    socket.send(b"after".to_vec());
    assert_eq!(next(&told), Some(Heard::Frame(long)));
    assert_eq!(next(&told), Some(Heard::Frame(b"after".to_vec())));

    let (path, authorization, agent) =
        ("/echo".to_owned(), Some("Bearer a-sign-in".to_owned()), Some("Letters".into()));
    assert_eq!(*server.asked.lock().unwrap(), [Asked { path, authorization, agent }]);

    // A frame the server sends unasked, with the last of its answer, is heard too.
    let (_greeted, told) = server.open("/greet", MOST);
    assert_eq!((next(&told), next(&told)), (Some(Heard::Open), Some(Heard::Frame(b"hello".to_vec()))));
}

#[test]
fn a_socket_that_ends_says_why() {
    let server = Server::start();
    // (the path, the largest frame taken, a frame to send once open, how it ends)
    let lost = Closed::Lost(String::new());
    let cases = [
        ("/echo", MOST, Some(&b"bye"[..]), Closed::There { code: 4000, reason: "done".to_owned() }),
        ("/echo", MOST, Some(b"drop"), lost.clone()),
        ("/echo", 8, Some(b"more than eight bytes"), lost.clone()),
        ("/elsewhere", MOST, None, Closed::Refused(403)),
    ];
    for (path, most, frame, expected) in cases {
        let (socket, told) = server.open(path, most);
        if let Some(frame) = frame {
            assert_eq!(next(&told), Some(Heard::Open), "{path}");
            socket.send(frame.to_vec());
        }
        let ended = std::iter::from_fn(|| next(&told)).find_map(|heard| match heard {
            Heard::Closed(closed) => Some(closed),
            _ => None,
        });
        let ended = ended.map(|closed| match closed {
            // Why the network failed is its own wording.
            Closed::Lost(_) => lost.clone(),
            closed => closed,
        });
        assert_eq!(ended, Some(expected), "{path}, {frame:?}");
        assert_eq!(next(&told), None, "{path}: nothing is told after the end");
    }
}

#[test]
fn a_socket_is_opened_only_to_a_server_that_is_believed_over_tls() {
    let server = Server::start();
    let stranger = common::tls().1;
    let plain = format!("ws://localhost:{}/echo", server.port);
    let page = format!("https://localhost:{}/echo", server.port);
    // (the address, whose word is taken on the certificate, how it ends)
    let cases = [
        (plain, server.roots(), Closed::Address),
        (page, server.roots(), Closed::Address),
        ("no address".to_owned(), server.roots(), Closed::Address),
        (server.address("/echo"), Roots::These(vec![stranger]), Closed::Lost(String::new())),
        (server.address("/echo"), Roots::System, Closed::Lost(String::new())),
    ];
    for (address, roots, expected) in cases {
        let (tell, told) = mpsc::channel();
        let _socket =
            Socket::open(&address, || Ok("a-sign-in".to_owned()), &roots, MOST, move |heard| drop(tell.send(heard)));

        let ended = next(&told).map(|heard| match heard {
            Heard::Closed(Closed::Lost(_)) => Heard::Closed(Closed::Lost(String::new())),
            heard => heard,
        });
        assert_eq!(ended, Some(Heard::Closed(expected)), "{address}");
    }
    assert!(server.asked.lock().unwrap().is_empty(), "the sign-in is shown to no server that is not believed");
}

#[test]
fn dropping_a_socket_closes_it() {
    let server = Server::start();
    let (socket, told) = server.open("/echo", MOST);
    assert_eq!(next(&told), Some(Heard::Open));

    drop(socket);

    assert_eq!(next(&told), Some(Heard::Closed(Closed::Here)));
    let told_server = || *server.closed.lock().unwrap() == 1;
    let waited = (0..100).any(|_| {
        told_server() || {
            thread::sleep(Duration::from_millis(20));
            false
        }
    });
    assert!(waited, "the server is told that the socket is closed");
}
