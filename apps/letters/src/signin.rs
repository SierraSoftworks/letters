//! Signing in to a Letters server through the system's browser.
//!
//! The app never sees a password. It sends the browser to the server with the hash of a secret, listens on
//! this machine for the browser to come back with a code, and trades the code and the secret for a
//! session. Only what began the sign-in knows the secret, so a code read on its way is of no use to
//! anything else. The waiting is for a thread of its own: a person may take minutes at the browser.

use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use letters_api::{Token, paths};
use sha2::{Digest, Sha256};

use crate::server::{Client, Refused};

/// How long the browser has to say what it came with, once it has come.
const SAID: Duration = Duration::from_secs(2);
/// The most the browser may say, in bytes. A code and a state are a few hundred.
const MOST: u64 = 16 * 1024;
/// How often the wait looks whether it was given up.
const LOOK: Duration = Duration::from_millis(100);

/// What the browser shows once the app has what it needs.
const DONE: &str = "Go back to Letters to carry on. You can close this tab.";
/// What it shows to a request that is not the one waited for.
const STRAY: &str = "This is not the sign-in Letters is waiting for.";

/// Why a sign-in did not end in a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsigned {
    /// The app could not listen on this machine for the browser to come back. The reason is the system's
    /// own wording, for whoever debugs.
    Deaf(String),
    /// The person gave up, or closed what was waiting.
    GivenUp,
    /// Nobody came back from the browser in time.
    Waited,
    /// The browser came back, and the server would not trade what it brought for a session.
    Refused(Refused),
}

impl fmt::Display for Unsigned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deaf(_) => write!(f, "this device would not let Letters wait for the browser"),
            Self::GivenUp => write!(f, "the sign-in was given up"),
            Self::Waited => write!(f, "nobody signed in at the browser in time"),
            Self::Refused(refused) => refused.fmt(f),
        }
    }
}

impl std::error::Error for Unsigned {}

/// One sign-in, from the address for the browser to the session.
pub struct SignIn {
    /// Where the browser comes back to: this machine only, at a port the system chose.
    listener: TcpListener,
    /// The secret the server is told only the hash of, until the code is traded.
    verifier: String,
    /// What the browser must bring back, so that a page cannot hand the app a code of its own.
    state: String,
}

// Written out by hand, so that the secret is not.
impl fmt::Debug for SignIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignIn").field("listener", &self.listener).finish_non_exhaustive()
    }
}

impl SignIn {
    /// Starts to listen, and makes up the secret. Nothing is asked of anyone yet.
    pub fn begin() -> Result<Self, Unsigned> {
        let deaf = |error: io::Error| Unsigned::Deaf(error.to_string());
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(deaf)?;
        listener.set_nonblocking(true).map_err(deaf)?;
        Ok(Self { listener, verifier: secret(), state: secret() })
    }

    /// The page of `server` to send the system's browser to.
    pub fn page(&self, server: &Client) -> Result<String, Unsigned> {
        let port = self.listener.local_addr().map_err(|error| Unsigned::Deaf(error.to_string()))?.port();
        let challenge = base64(&Sha256::digest(self.verifier.as_bytes()));
        Ok(format!("{}{}", server.origin(), paths::desktop_sign_in(&challenge, port, &self.state)))
    }

    /// Waits for the browser to come back, for `within` at most and until `given_up` is set, and trades
    /// what it brings for a session at `server`.
    pub fn finish(self, server: &Client, within: Duration, given_up: &AtomicBool) -> Result<Token, Unsigned> {
        let until = Instant::now() + within;
        let code = loop {
            if given_up.load(Ordering::Relaxed) {
                return Err(Unsigned::GivenUp);
            }
            if Instant::now() >= until {
                return Err(Unsigned::Waited);
            }
            match self.listener.accept() {
                Ok((browser, _)) => {
                    // Anything on this machine can knock. Only the browser that was sent has the state.
                    if let Some(code) = self.hear(browser) {
                        break code;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(LOOK),
                Err(error) => return Err(Unsigned::Deaf(error.to_string())),
            }
        };
        server.exchange(&code, &self.verifier).map_err(Unsigned::Refused)
    }

    /// Reads what one caller asks, answers it, and gives the code if it is the browser coming back.
    fn hear(&self, mut browser: TcpStream) -> Option<String> {
        browser.set_nonblocking(false).ok()?;
        browser.set_read_timeout(Some(SAID)).ok()?;
        browser.set_write_timeout(Some(SAID)).ok()?;
        let code = asked(&mut browser).and_then(|target| self.code(&target));
        let (status, words) = if code.is_some() { ("200 OK", DONE) } else { ("400 Bad Request", STRAY) };
        let page = format!(
            "<!doctype html><meta charset=\"utf-8\"><title>Letters</title>\
             <body style=\"font: 16px system-ui, sans-serif; margin: 4em auto; max-width: 30em\"><p>{words}</p>"
        );
        // The code is in the address the browser has on show, so it is told to keep and pass on nothing.
        let head = format!(
            "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\n\
             cache-control: no-store\r\nreferrer-policy: no-referrer\r\nconnection: close\r\n\r\n",
            page.len()
        );
        let _ = browser.write_all(head.as_bytes()).and_then(|()| browser.write_all(page.as_bytes()));
        code
    }

    /// The code in what was asked for, if it is `/?code=…&state=…` with the state this sign-in made up.
    fn code(&self, target: &str) -> Option<String> {
        let query = target.strip_prefix("/?")?;
        let part = |name: &str| {
            let mut parts = query.split('&').filter_map(|part| part.split_once('='));
            parts.find(|(key, _)| *key == name).and_then(|(_, value)| unescaped(value))
        };
        let code = part("code").filter(|code| !code.is_empty())?;
        (part("state")? == self.state).then_some(code)
    }
}

/// What a caller asks for with `GET`, once it has said all it came to say.
fn asked(browser: &mut TcpStream) -> Option<String> {
    let mut said = BufReader::new(browser.take(MOST));
    let mut first = Vec::new();
    said.read_until(b'\n', &mut first).ok()?;
    // The rest is heard out before anything is answered: a caller cut off as it speaks may never see
    // the answer.
    let mut line = Vec::new();
    while said.read_until(b'\n', &mut line).ok()? > 2 {
        line.clear();
    }
    let first = String::from_utf8(first).ok()?;
    let mut words = first.split_ascii_whitespace();
    (words.next()? == "GET").then(|| words.next().map(str::to_owned))?
}

/// A secret nobody can guess, in letters an address takes as they are: 256 bits.
fn secret() -> String {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).expect("the system has a source of randomness");
    base64(&bytes)
}

/// `bytes` in base64 for addresses, without padding.
fn base64(bytes: &[u8]) -> String {
    const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = chunk.iter().fold(0u32, |group, byte| group << 8 | u32::from(*byte)) << (8 * (3 - chunk.len()));
        for place in 0..=chunk.len() {
            out.push(char::from(LETTERS[(group >> (18 - 6 * place) & 63) as usize]));
        }
    }
    out
}

/// A part of an address as it was before its bytes were escaped.
fn unescaped(part: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(part.len());
    let mut rest = part.bytes();
    while let Some(byte) = rest.next() {
        bytes.push(match byte {
            b'%' => {
                let digits = [rest.next()?, rest.next()?];
                u8::from_str_radix(std::str::from_utf8(&digits).ok()?, 16).ok()?
            }
            b'+' => b' ',
            byte => byte,
        });
    }
    String::from_utf8(bytes).ok()
}
