//! A person's session at a Letters server: the token every request carries, renewed before it runs out,
//! and kept in the system's keychain so that the app opens signed in.
//!
//! Every call waits on the network, and one that renews holds the others back until it is done: a renewal
//! works once, so two must never go out together.

use std::fmt;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use letters_api::{
    Access, Code, CreatedLink, Entry, Grant, Limits, Listing, Move, NewDocument, NewFolder, NewLink, Opened, Person,
    Rename, Shares, Space, Token, Trashed, paths,
};

use crate::keychain::{Keychain, Unkept};
use crate::server::{Client, Refused};

/// How long before a token runs out the next is asked for, in seconds: longer than a call may take.
const EARLY: u64 = 60;
/// What a person is told when the session has ended on this side.
const OVER: &str = "Sign in again to carry on.";

/// One person signed in to one server.
pub struct Session {
    server: Client,
    /// `None` where the system has no keychain: the session then lasts as long as the app is open.
    keychain: Option<Keychain>,
    held: Mutex<Held>,
    /// What the server says of itself, once it was asked.
    said: OnceLock<letters_api::Server>,
}

struct Held {
    token: Token,
    /// Why the session will not be there when the app next opens, if it will not.
    unkept: Option<Unkept>,
}

// Written out by hand, so that nothing of the sign-in is.
impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").field("server", &self.server.origin()).finish_non_exhaustive()
    }
}

impl Session {
    /// The session kept for `server` when the app last ran, if the person signed in there and has not
    /// signed out since. Nothing is asked of the server.
    pub fn kept(server: Client, keychain: Keychain) -> Result<Option<Self>, Unkept> {
        let Some(token) = kept(&keychain, &server)? else { return Ok(None) };
        Ok(Some(Self {
            server,
            keychain: Some(keychain),
            held: Mutex::new(Held { token, unkept: None }),
            said: OnceLock::new(),
        }))
    }

    /// The session a sign-in ended in, which is kept from here on.
    pub fn begin(server: Client, keychain: Option<Keychain>, token: Token) -> Self {
        let unkept = keep(keychain.as_ref(), &server, &token).err();
        Self { server, keychain, held: Mutex::new(Held { token, unkept }), said: OnceLock::new() }
    }

    pub fn server(&self) -> &Client {
        &self.server
    }

    /// Who signed in.
    pub fn person(&self) -> Person {
        self.held().token.person.clone()
    }

    /// Why the person will have to sign in again when the app next opens, if they will.
    pub fn unkept(&self) -> Option<Unkept> {
        self.held().unkept.clone()
    }

    /// A token that works for a while yet, as a socket's upgrade needs.
    pub fn token(&self) -> Result<String, Refused> {
        self.fresh(None)
    }

    /// The next token, for when the server would not take the one it was last shown.
    pub fn renewed(&self) -> Result<String, Refused> {
        let mut held = self.held();
        if held.token.ends <= now() {
            return Err(over());
        }
        self.renew(&mut held)?;
        Ok(held.token.token.clone())
    }

    /// Whether the session is over, so that only a new sign-in opens anything. Asks nothing.
    pub fn ended(&self) -> bool {
        self.held().token.ends <= now()
    }

    /// Asks the server something as this session, with a token that is renewed first if it has run out.
    /// A refusal with [`Code::SignedOut`] means the person signs in through the browser again.
    pub fn ask<T>(&self, ask: impl Fn(&Client, &str) -> Result<T, Refused>) -> Result<T, Refused> {
        let token = self.fresh(None)?;
        match ask(&self.server, &token) {
            // The server's clock decides, whatever this device's says.
            Err(refused) if refused.code() == Some(Code::Expired) => ask(&self.server, &self.fresh(Some(&token))?),
            answered => answered,
        }
    }

    /// What the server holds its callers to, asked once. Needs no session, but is asked beside one.
    pub fn limits(&self) -> Result<Limits, Refused> {
        Ok(self.said()?.limits)
    }

    /// Whether the server makes links to its documents. Asked with its limits.
    pub fn links(&self) -> Result<bool, Refused> {
        Ok(self.said()?.links)
    }

    fn said(&self) -> Result<&letters_api::Server, Refused> {
        if let Some(said) = self.said.get() {
            return Ok(said);
        }
        let said = self.server.discover()?;
        Ok(self.said.get_or_init(|| said))
    }

    /// Every space the person reaches.
    pub fn spaces(&self) -> Result<Vec<Space>, Refused> {
        self.ask(|server, token| server.get(token, paths::SPACES))
    }

    /// What `folder` of a space holds, or the space itself for an empty `folder`.
    pub fn entries(&self, space: &str, folder: &str) -> Result<Listing, Refused> {
        self.ask(|server, token| server.get(token, &paths::entries(space, folder)))
    }

    /// The documents the person opened lately, newest first.
    pub fn recent(&self) -> Result<Vec<Entry>, Refused> {
        self.ask(|server, token| server.get(token, paths::RECENT))
    }

    /// The documents shared with the person from beyond their spaces.
    pub fn shared(&self) -> Result<Vec<Entry>, Refused> {
        self.ask(|server, token| server.get(token, paths::SHARED))
    }

    /// A document ready to edit: what the session may do with it, and where its socket is.
    pub fn open(&self, id: &str) -> Result<Opened, Refused> {
        self.ask(|server, token| server.get(token, &paths::document(id)))
    }

    /// The whole `.let` file of a document, of `most` bytes at most.
    pub fn file(&self, id: &str, most: u64) -> Result<Vec<u8>, Refused> {
        self.ask(|server, token| server.file(token, id, most))
    }

    /// One picture of a document, by its name without `assets/`.
    pub fn asset(&self, id: &str, name: &str, most: u64) -> Result<Vec<u8>, Refused> {
        self.ask(|server, token| server.asset(token, id, name, most))
    }

    /// Hands the server a document's whole `.let` file, for work that no frame of its socket carries.
    ///
    /// The server reads the file before it holds the document, which may be moved or thrown away
    /// meanwhile: it then answers [`Code::Stale`], and is asked once more, to find the document where it
    /// is now.
    pub fn keep_file(&self, id: &str, bytes: &[u8]) -> Result<(), Refused> {
        let keep = || self.ask(|server, token| server.keep_file(token, id, bytes));
        match keep() {
            Err(refused) if refused.code() == Some(Code::Stale) => keep(),
            kept => kept,
        }
    }

    /// Says that the person read a document up to `revision`, as its model writes one.
    pub fn saw(&self, id: &str, revision: &str) -> Result<(), Refused> {
        self.ask(|server, token| server.saw(token, id, revision))
    }

    /// The names of the pictures the server holds for a document, each without `assets/`.
    pub fn assets(&self, id: &str) -> Result<Vec<String>, Refused> {
        self.ask(|server, token| server.get(token, &paths::assets(id)))
    }

    /// Hands the server a picture of a document, before the edit that names it is sent.
    pub fn keep_asset(&self, id: &str, name: &str, bytes: &[u8]) -> Result<(), Refused> {
        self.ask(|server, token| server.keep_asset(token, id, name, bytes))
    }

    /// Calls a document `name`. A name another document has in its folder is refused as [`Code::NameTaken`].
    pub fn rename(&self, id: &str, name: &str) -> Result<Entry, Refused> {
        let to = Rename { name: name.to_owned() };
        self.ask(|server, token| server.post(token, &paths::rename(id), &to))
    }

    /// Moves a document to `folder` of `space`, under the name it has.
    pub fn relocate(&self, id: &str, space: &str, folder: &str) -> Result<Entry, Refused> {
        let to = Move { space: space.to_owned(), folder: folder.to_owned() };
        self.ask(|server, token| server.post(token, &paths::relocate(id), &to))
    }

    /// Starts a document in `folder` of a space. It is "Untitled" there until its first heading names it.
    pub fn new_document(&self, space: &str, folder: &str) -> Result<Entry, Refused> {
        let new = NewDocument { folder: folder.to_owned(), name: None };
        self.ask(|server, token| server.post(token, &paths::documents(space), &new))
    }

    /// Makes the folder `name` in `parent` of a space.
    pub fn new_folder(&self, space: &str, parent: &str, name: &str) -> Result<Entry, Refused> {
        let new = NewFolder { parent: parent.to_owned(), name: name.to_owned() };
        self.ask(|server, token| server.post(token, &paths::folders(space), &new))
    }

    /// Makes a document of a Markdown or Word file in `folder` of a space. `name` is the file's own, which
    /// names the document without its ending. A file that is neither is refused as [`Code::Invalid`].
    pub fn import(&self, space: &str, folder: &str, name: &str, file: &[u8]) -> Result<Entry, Refused> {
        self.ask(|server, token| server.hand(token, &paths::import(space, folder, name), file))
    }

    /// Merges into a document every conflict copy a sync tool left beside it on the server, and gives
    /// the names of the copies it took. The server reads the copies before it holds the document, which
    /// may be moved meanwhile: it then answers [`Code::Stale`], and is asked once more.
    pub fn merge_conflicts(&self, id: &str) -> Result<Vec<String>, Refused> {
        let merge = || self.ask(|server, token| server.done(token, &paths::merge_conflicts(id)));
        match merge() {
            Err(refused) if refused.code() == Some(Code::Stale) => merge(),
            merged => merged,
        }
    }

    /// What is in the trash of every space the person reaches, the last thrown away first.
    pub fn trash(&self) -> Result<Vec<Trashed>, Refused> {
        self.ask(|server, token| server.get(token, paths::TRASH))
    }

    /// Takes what a [`Trashed`] names by its `file` out of the trash of `space`, back to where it was.
    pub fn put_back(&self, space: &str, file: &str) -> Result<Entry, Refused> {
        self.ask(|server, token| server.done(token, &paths::put_back(space, file)))
    }

    /// Who a document is shared with, besides those its space lets in.
    pub fn shares(&self, id: &str) -> Result<Shares, Refused> {
        self.ask(|server, token| server.get(token, &paths::shares(id)))
    }

    /// Makes `people` all the people a document is shared with, and gives its shares as they then are.
    pub fn share(&self, id: &str, people: &[Grant]) -> Result<Shares, Refused> {
        let shares = Shares { people: people.to_vec(), links: Vec::new() };
        self.ask(|server, token| server.put(token, &paths::shares(id), &shares))
    }

    /// Who reached a document through a share, newest first.
    /// A page of them: the newest, or with `before` those from before that row's time.
    pub fn access_log(&self, id: &str, before: Option<u64>) -> Result<Vec<Access>, Refused> {
        let page = before.map_or_else(|| paths::access(id), |time| format!("{}?before={time}", paths::access(id)));
        self.ask(|server, token| server.get(token, &page))
    }

    /// How many days the server keeps a row of that log. 0 from one that does not say.
    pub fn log_days(&self) -> Result<u32, Refused> {
        Ok(self.said()?.log_days)
    }

    /// Makes a link to a document. Its address is told this once.
    pub fn make_link(&self, id: &str, new: &NewLink) -> Result<CreatedLink, Refused> {
        self.ask(|server, token| server.post(token, &paths::links(id), new))
    }

    /// Revokes a link to a document, by the link's id.
    pub fn revoke_link(&self, id: &str, link: &str) -> Result<(), Refused> {
        self.ask(|server, token| server.delete(token, &paths::link(id, link)))
    }

    /// Ends the session here and at the server. The server is told if it can be reached, and the sign-in
    /// is forgotten either way: what is left at an unreachable server runs out on its own.
    pub fn sign_out(&self) -> Result<(), Unkept> {
        let mut held = self.held();
        if held.token.ends > now() {
            let _ = self.server.sign_out(&held.token.token);
        }
        end(&mut held.token);
        self.keychain.as_ref().map_or(Ok(()), |keychain| keychain.forget(self.server.origin()))
    }

    fn held(&self) -> MutexGuard<'_, Held> {
        // What a thread that panicked held is still the session.
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A token to ask with. `refused` is one the server just said had run out, which is not given again.
    fn fresh(&self, refused: Option<&str>) -> Result<String, Refused> {
        let mut held = self.held();
        let now = now();
        if held.token.ends <= now {
            return Err(over());
        }
        // Another thread may have renewed while this one waited for its turn.
        if held.token.expires <= now + EARLY || refused == Some(held.token.token.as_str()) {
            self.renew(&mut held)?;
        }
        Ok(held.token.token.clone())
    }

    fn renew(&self, held: &mut Held) -> Result<(), Refused> {
        match self.server.renew(&held.token.renew) {
            Ok(next) => {
                // Kept before it is used: the renewal that bought it is spent.
                held.unkept = keep(self.keychain.as_ref(), &self.server, &next).err();
                held.token = next;
                Ok(())
            }
            Err(refused) if matches!(refused.code(), Some(Code::SignedOut | Code::Expired)) => {
                // Another Letters on this device may have renewed first, and what it kept is the session now.
                let other = self.keychain.as_ref().and_then(|keychain| kept(keychain, &self.server).ok().flatten());
                match other {
                    Some(other) if other.renew != held.token.renew && other.ends > now() => {
                        held.token = other;
                        Ok(())
                    }
                    _ => {
                        end(&mut held.token);
                        if let Some(keychain) = &self.keychain {
                            let _ = keychain.forget(self.server.origin());
                        }
                        Err(over())
                    }
                }
            }
            // A server that cannot be reached has not ended anything.
            Err(refused) => Err(refused),
        }
    }
}

/// Makes a token good for nothing, so that what still holds the session asks no more with it.
fn end(token: &mut Token) {
    token.token.clear();
    token.renew.clear();
    (token.expires, token.ends) = (0, 0);
}

fn over() -> Refused {
    Refused::Said(letters_api::Error::new(Code::SignedOut, OVER))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

/// The token the keychain has for the server. One that cannot be read is as good as none.
fn kept(keychain: &Keychain, server: &Client) -> Result<Option<Token>, Unkept> {
    let kept = keychain.kept(server.origin())?;
    Ok(kept.and_then(|json| serde_json::from_str(&json).ok()))
}

fn keep(keychain: Option<&Keychain>, server: &Client, token: &Token) -> Result<(), Unkept> {
    let keychain = keychain.ok_or(Unkept::NoKeychain)?;
    let json =
        serde_json::to_string(token).map_err(|_| Unkept::Failed("the sign-in could not be written".to_owned()))?;
    keychain.keep(server.origin(), &json)
}
