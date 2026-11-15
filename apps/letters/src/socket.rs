//! One WebSocket to a Letters server, for a document that is open in connected mode.
//!
//! The socket carries frames and knows nothing of what is in them: `letters-sync` says what to send and
//! what a frame means, and when to try again. Two threads serve each socket, so nothing waits on the UI
//! thread and nothing polls: one waits on the network, and the other owns the connection and waits for
//! whichever comes first, bytes from the server or a frame to send.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore};
use rustls_platform_verifier::BuilderVerifierExt;
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::HandshakeError;
use tungstenite::http::{HeaderValue, Uri, header};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

use crate::remote::{LIMIT, Roots};

/// All a server is told about what asks, beside who signed in.
const CLIENT: &str = "Letters";
/// How long a write waits on a server that has stopped reading.
const STALLED: Duration = Duration::from_secs(30);

/// What a socket tells its document, in the order it happened. `Closed` is the last of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Heard {
    /// The server took the connection: what was sent goes out, and frames will come.
    Open,
    Frame(Vec<u8>),
    Closed(Closed),
}

/// Why a socket is open no longer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Closed {
    /// This side closed it.
    Here,
    /// The server closed it, with the code and the reason it gave.
    There { code: u16, reason: String },
    /// The server answered the request for a socket with this status, such as 401 for a sign-in it does
    /// not take.
    Refused(u16),
    /// The address is none a socket is opened at: only `wss` is spoken.
    Address,
    /// The server took longer than [`LIMIT`] to take the connection.
    Slow,
    /// The server could not be reached, or the connection broke. The reason is the network's own
    /// wording, which is kept for whoever debugs and not shown.
    Lost(String),
}

impl fmt::Display for Closed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Here => write!(f, "the connection was closed"),
            Self::There { code, .. } => write!(f, "the server closed the connection ({code})"),
            Self::Refused(status) => write!(f, "the server answered {status}"),
            Self::Address => write!(f, "the address is not one of a Letters server"),
            Self::Slow => write!(f, "the server took longer than {} seconds", LIMIT.as_secs()),
            Self::Lost(_) => write!(f, "the server could not be reached"),
        }
    }
}

/// An open socket, or one on its way to being open. Dropping it closes the connection.
pub struct Socket {
    to: mpsc::Sender<Turn>,
}

/// What the thread that owns the connection is woken for.
enum Turn {
    Send(Vec<u8>),
    /// Bytes the server sent, as they came off the network.
    Arrived(Vec<u8>),
    /// The network gave no more.
    Gone(Option<io::Error>),
    Close,
}

impl Socket {
    /// Opens a socket at `address`, a `wss` address, for whoever holds the token that `token` gives.
    /// Nothing waits here: `token` is asked on another thread, since a session may have to renew it
    /// first, and all that happens is told to `heard` there, ending with one [`Heard::Closed`]. A frame
    /// of more than `most` bytes is taken from no server.
    ///
    /// The token goes to the server in a header and nowhere else.
    pub fn open(
        address: &str,
        token: impl FnOnce() -> Result<String, Closed> + Send + 'static,
        roots: &Roots,
        most: usize,
        mut heard: impl FnMut(Heard) + Send + 'static,
    ) -> Self {
        let (to, turns) = mpsc::channel();
        let (address, roots, arrived) = (address.to_owned(), roots.clone(), to.clone());
        thread::spawn(move || {
            let closed = match token().and_then(|token| connect(&address, &token, &roots, most)) {
                Ok(socket) => serve(socket, &turns, arrived, &mut heard),
                Err(closed) => closed,
            };
            heard(Heard::Closed(closed));
        });
        Self { to }
    }

    /// Sends a frame. One sent before the socket is open goes out once it is, and one sent after it
    /// closed goes nowhere.
    pub fn send(&self, frame: Vec<u8>) {
        let _ = self.to.send(Turn::Send(frame));
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        let _ = self.to.send(Turn::Close);
    }
}

/// The TLS connection, as the WebSocket reads and writes it.
struct Tunnel {
    tls: ClientConnection,
    tcp: TcpStream,
    /// What the server sent that the connection has not been given yet, once another thread waits on
    /// the network. `None` while the socket is being opened, when a read waits on the network itself.
    waiting: Option<Vec<u8>>,
}

impl Tunnel {
    /// Sends what the connection has ready for the server.
    fn pass(&mut self) -> io::Result<()> {
        while self.tls.wants_write() {
            self.tls.write_tls(&mut self.tcp)?;
        }
        Ok(())
    }

    /// Gives the connection more of what the server sent. Returns whether there was any for now.
    ///
    /// A little at a time: the connection holds only so much that has not been read from it yet.
    fn more(&mut self) -> io::Result<bool> {
        let read = match &mut self.waiting {
            Some(waiting) if waiting.is_empty() => return Ok(false),
            Some(waiting) => {
                let mut rest = waiting.as_slice();
                let read = self.tls.read_tls(&mut rest)?;
                waiting.drain(..read);
                read
            }
            None => self.tls.read_tls(&mut self.tcp)?,
        };
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.tls.process_new_packets().map_err(io::Error::other)?;
        self.pass()?;
        Ok(true)
    }
}

impl Read for Tunnel {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.tls.reader().read(into) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if !self.more()? {
                        return Err(error);
                    }
                }
                read => return read,
            }
        }
    }
}

impl Write for Tunnel {
    fn write(&mut self, from: &[u8]) -> io::Result<usize> {
        let written = self.tls.writer().write(from)?;
        self.pass()?;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.tls.writer().flush()?;
        self.pass()
    }
}

/// Whose word is taken that the server is the one it says it is.
fn config(roots: &Roots) -> Result<ClientConfig, rustls::Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions()?;
    Ok(match roots {
        Roots::System => builder.with_platform_verifier()?.with_no_client_auth(),
        Roots::These(certificates) => {
            let mut store = RootCertStore::empty();
            for certificate in certificates {
                store.add(CertificateDer::from(certificate.clone()))?;
            }
            builder.with_root_certificates(store).with_no_client_auth()
        }
    })
}

/// Opens the connection and asks the server for a socket on it, all within [`LIMIT`].
fn connect(address: &str, token: &str, roots: &Roots, most: usize) -> Result<WebSocket<Tunnel>, Closed> {
    let started = Instant::now();
    let left = || LIMIT.checked_sub(started.elapsed()).filter(|left| !left.is_zero()).ok_or(Closed::Slow);
    let lost = |error: &dyn fmt::Display| Closed::Lost(error.to_string());

    let uri: Uri = address.parse().map_err(|_| Closed::Address)?;
    let host = uri.host().filter(|_| uri.scheme_str() == Some("wss")).ok_or(Closed::Address)?;
    let host = host.trim_start_matches('[').trim_end_matches(']').to_owned();
    let port = uri.port_u16().unwrap_or(443);
    let mut request = uri.into_client_request().map_err(|_| Closed::Address)?;
    // A sign-in that cannot be written in a header is one no server takes.
    let mut bearer = HeaderValue::try_from(format!("Bearer {token}")).map_err(|_| Closed::Refused(401))?;
    bearer.set_sensitive(true);
    request.headers_mut().insert(header::AUTHORIZATION, bearer);
    request.headers_mut().insert(header::USER_AGENT, HeaderValue::from_static(CLIENT));

    let name = ServerName::try_from(host.clone()).map_err(|_| Closed::Address)?;
    let tls = ClientConnection::new(Arc::new(config(roots).map_err(|error| lost(&error))?), name);
    let tls = tls.map_err(|error| lost(&error))?;
    let mut tcp = Err(Closed::Lost("the name leads nowhere".to_owned()));
    for found in (host.as_str(), port).to_socket_addrs().map_err(|error| lost(&error))? {
        tcp = TcpStream::connect_timeout(&found, left()?).map_err(|error| match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Closed::Slow,
            _ => lost(&error),
        });
        if tcp.is_ok() {
            break;
        }
    }
    let tcp = tcp?;
    let within = Some(left()?);
    let timed = tcp.set_read_timeout(within).and_then(|()| tcp.set_write_timeout(within));
    timed.and_then(|()| tcp.set_nodelay(true)).map_err(|error| lost(&error))?;

    let limits = WebSocketConfig::default().max_message_size(Some(most)).max_frame_size(Some(most));
    let tunnel = Tunnel { tls, tcp, waiting: None };
    match tungstenite::client::client_with_config(request, tunnel, Some(limits)) {
        Ok((socket, _)) => Ok(socket),
        Err(HandshakeError::Failure(tungstenite::Error::Http(response))) => {
            Err(Closed::Refused(response.status().as_u16()))
        }
        // A read that waited out its time comes back as one to try again.
        Err(HandshakeError::Interrupted(_)) => Err(Closed::Slow),
        Err(HandshakeError::Failure(tungstenite::Error::Io(error))) if error.kind() == io::ErrorKind::TimedOut => {
            Err(Closed::Slow)
        }
        Err(error) => Err(lost(&error)),
    }
}

/// Carries frames both ways over an open socket until either side closes it or the connection breaks.
fn serve(
    mut socket: WebSocket<Tunnel>,
    turns: &mpsc::Receiver<Turn>,
    arrived: mpsc::Sender<Turn>,
    heard: &mut dyn FnMut(Heard),
) -> Closed {
    let lost = |error: &dyn fmt::Display| Closed::Lost(error.to_string());
    // From here on another thread waits on the network, and this one for whatever comes first.
    let tunnel = socket.get_mut();
    tunnel.waiting = Some(Vec::new());
    let timed = tunnel.tcp.set_read_timeout(None).and_then(|()| tunnel.tcp.set_write_timeout(Some(STALLED)));
    let mut reading = match timed.and_then(|()| tunnel.tcp.try_clone()) {
        Ok(reading) => reading,
        Err(error) => return lost(&error),
    };
    thread::spawn(move || {
        let mut piece = vec![0; 64 * 1024];
        loop {
            let turn = match reading.read(&mut piece) {
                Ok(0) => Turn::Gone(None),
                Ok(read) => Turn::Arrived(piece[..read].to_vec()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => Turn::Gone(Some(error)),
            };
            let last = matches!(turn, Turn::Gone(_));
            if arrived.send(turn).is_err() || last {
                break;
            }
        }
    });
    heard(Heard::Open);

    let closed = 'open: loop {
        // All that can be read is, before the next turn is waited for: a frame may have come with the
        // last of the server's answer.
        loop {
            match socket.read() {
                Ok(Message::Binary(frame)) => heard(Heard::Frame(frame.to_vec())),
                Ok(Message::Close(frame)) => {
                    // The answer to it is queued, and goes out before the connection does.
                    let _ = socket.flush();
                    let said = frame.map(|frame| (u16::from(frame.code), frame.reason.to_string()));
                    let (code, reason) = said.unwrap_or((1005, String::new()));
                    break 'open Closed::There { code, reason };
                }
                // Text is not spoken here, and a ping is answered by the socket itself.
                Ok(_) => {}
                Err(tungstenite::Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => break 'open lost(&error),
            }
        }
        // An answer to a ping was queued by the read.
        if let Err(error) = socket.flush() {
            break lost(&error);
        }
        match turns.recv() {
            Ok(Turn::Send(frame)) => {
                if let Err(error) = socket.send(Message::Binary(frame.into())) {
                    break lost(&error);
                }
            }
            Ok(Turn::Arrived(bytes)) => socket.get_mut().waiting.get_or_insert_default().extend(bytes),
            Ok(Turn::Gone(error)) => {
                break lost(&error.map_or_else(|| "the server hung up".to_owned(), |error| error.to_string()));
            }
            Ok(Turn::Close) | Err(_) => {
                let _ = socket.close(None).and_then(|()| socket.flush());
                break Closed::Here;
            }
        }
    };
    // The thread that waits on the network is let go.
    let _ = socket.get_mut().tcp.shutdown(Shutdown::Both);
    closed
}
