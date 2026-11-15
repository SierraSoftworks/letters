//! The servers the person is signed in to, and signing in to another, in one dialog.
//!
//! The window only asks: the shell finds the server, shows its sign-in page in the system's browser, and
//! says how far that has come with `Window::set_sign_in` and who is signed in where with
//! `Window::set_servers`.

use accesskit::Role;
use letters_editor::{Clip, Platform};

use crate::element::Element;
use crate::icon;
use crate::opening::Request;
use crate::style::{Align, Edges, Justify};
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Button, Popover, TextField};
use crate::window::Message;

const WIDTH: f32 = 520.0;
const MARK: f32 = 32.0;
/// What the field for a server's address is called.
const ADDRESS: &str = "Server address";

/// A server the person is signed in to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Server {
    /// Its address, as the requests about it name it: `https://letters.example.org`.
    pub address: String,
    /// What the person is called there.
    pub account: String,
}

/// How far a sign-in has come.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignIn {
    /// The server is being asked how people sign in to it.
    Finding,
    /// The system's browser shows the sign-in page of the server called `name`, and the app waits for the
    /// person to come back from it. `page` is that page's address, for a browser that did not open.
    Waiting { name: String, page: String },
    /// It did not work, and why, as the person is to read it.
    Failed(String),
}

/// What the Servers dialog asks of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signing {
    Show,
    Close,
    /// The address of a server was typed.
    Address(String),
    /// Sign in to the server whose address is typed.
    In,
    /// Give the sign-in up.
    Cancel,
    /// Put the address of the sign-in page on the clipboard.
    CopyPage,
    /// Sign out of the server at this address.
    Out(String),
}

#[derive(Default)]
pub(crate) struct Servers {
    open: bool,
    /// Whether the dialog came out for a sign-in that was not begun in it. It then goes with the sign-in.
    unasked: bool,
    /// Where the person is signed in, as the shell last said. `None` until a shell says: one that signs in
    /// to no server never does, and its window offers no dialog.
    servers: Option<Vec<Server>>,
    /// What the field for a server's address holds.
    address: String,
    /// How far a sign-in has come. `None` while there is none.
    sign_in: Option<SignIn>,
    /// Whether the sign-in page's address is on the clipboard.
    copied: bool,
}

impl Servers {
    /// Whether the shell signs in to servers, so that the dialog is offered.
    pub fn offered(&self) -> bool {
        self.servers.is_some()
    }

    /// The servers the person is signed in to.
    pub fn signed_in(&self) -> &[Server] {
        self.servers.as_deref().unwrap_or_default()
    }

    /// Puts the dialog away. A sign-in goes on behind it: the browser has it.
    pub fn close(&mut self) {
        (self.open, self.copied) = (false, false);
    }

    /// Whether a sign-in is under way.
    fn busy(&self) -> bool {
        matches!(self.sign_in, Some(SignIn::Finding | SignIn::Waiting { .. }))
    }

    /// Takes in where the person is signed in.
    pub fn listed(&mut self, servers: Vec<Server>) {
        self.servers = Some(servers);
    }

    /// Takes in how far a sign-in has come, or that there is none any more.
    pub fn told(&mut self, state: Option<SignIn>) {
        let under_way = matches!(state, Some(SignIn::Finding | SignIn::Waiting { .. }));
        // A sign-in the shell began, as for a session that ended, is shown while it lasts.
        if under_way && !self.busy() && !self.open {
            (self.open, self.unasked) = (true, true);
        }
        // One that is done leaves the field ready for another server.
        if state.is_none() && self.busy() {
            self.address.clear();
            self.open &= !self.unasked;
        }
        if !matches!(state, Some(SignIn::Waiting { .. })) {
            self.copied = false;
        }
        self.sign_in = state;
    }

    /// Does what the dialog asked.
    pub fn update(&mut self, message: Signing, platform: &mut dyn Platform, requests: &mut Vec<Request>) {
        let busy = self.busy();
        match message {
            Signing::Show => (self.open, self.unasked) = (self.offered(), false),
            Signing::Close => self.close(),
            Signing::Address(_) | Signing::In if busy => {}
            Signing::Address(typed) => {
                // The last failure goes with the next thing that is typed.
                (self.address, self.sign_in) = (typed, None);
            }
            Signing::In => {
                let server = self.address.trim();
                if !server.is_empty() {
                    requests.push(Request::SignIn { server: server.to_owned() });
                    // Said at once, so that a second press asks nothing.
                    (self.sign_in, self.unasked) = (Some(SignIn::Finding), false);
                }
            }
            Signing::Cancel => {
                if busy {
                    (self.sign_in, self.copied) = (None, false);
                    self.open &= !self.unasked;
                    requests.push(Request::CancelSignIn);
                }
            }
            Signing::CopyPage => {
                if let Some(SignIn::Waiting { page, .. }) = &self.sign_in {
                    platform.clipboard_write(Clip { text: page.clone(), ..Clip::default() });
                    self.copied = true;
                }
            }
            Signing::Out(address) => {
                let servers = self.servers.get_or_insert_default();
                if servers.iter().any(|server| server.address == address) {
                    // Gone at once. The shell says who is then signed in where.
                    servers.retain(|server| server.address != address);
                    requests.push(Request::SignOut { server: address });
                }
            }
        }
    }

    /// The dialog over the middle of the window, while it is open: see `settings::Dialog::view`.
    pub fn view(&self) -> Option<Element<Message>> {
        if !self.open {
            return None;
        }
        let head = Element::row()
            .align(Align::Center)
            .justify(Justify::Between)
            .child(Element::text("Servers", TextStyle::title().size(tokens::TEXT_XL)))
            .child(Button::icon_only(icon::X, "Close").subtle().small().on_press(send(Signing::Close)));
        let note = "A Letters server keeps documents that several people write at once. Sign in to one to \
                    open its documents here.";
        let note = Element::paragraph(note, quiet()).fg(tokens::FG_MUTED);
        let servers = self.servers.as_deref().unwrap_or_default();
        let list = (!servers.is_empty()).then(|| {
            Element::column()
                .gap(tokens::SPACE_SM)
                .role(Role::List)
                .label("Signed in")
                .children(servers.iter().map(server))
        });
        let foot =
            Element::row().justify(Justify::End).child(Button::new("Done").primary().on_press(send(Signing::Close)));
        let content = Element::column()
            .width(WIDTH)
            .gap(tokens::SPACE_LG)
            .padding(Edges { top: tokens::SPACE_LG, ..Edges::all(tokens::SPACE_XL) })
            .child(Element::column().gap(tokens::SPACE_XS).child(head).child(note))
            .children(list)
            .child(self.signing())
            .child(foot);
        let dialog = Popover::new(content)
            .flush()
            .label("Servers")
            .max_width(WIDTH)
            .modal()
            .open(true)
            .on_close(send(Signing::Close));
        Some(Element::row().overlay().popover(dialog))
    }

    /// The field for another server, and how far the sign-in to it has come.
    fn signing(&self) -> Element<Message> {
        let busy = self.busy();
        let field = TextField::new(ADDRESS, &self.address)
            .placeholder("letters.example.org")
            .disabled(busy)
            .invalid(matches!(self.sign_in, Some(SignIn::Failed(_))))
            .on_input(|typed| send(Signing::Address(typed)))
            .on_submit(send(Signing::In));
        let sign_in =
            Button::new("Sign in").disabled(busy || self.address.trim().is_empty()).on_press(send(Signing::In));
        let ask = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_XS)
            .child(Element::from(field).key("servers-address").takes_focus().grow(1.0))
            .child(sign_in);
        let cancel = || Element::from(Button::new("Cancel").small().on_press(send(Signing::Cancel)));
        let state = match &self.sign_in {
            None => None,
            Some(SignIn::Finding) => {
                let said = "Asking the server how to sign in.";
                Some(Element::row().align(Align::Center).gap(tokens::SPACE_MD).child(waiting(said)).child(cancel()))
            }
            Some(SignIn::Waiting { name, .. }) => {
                let said = format!(
                    "Your browser shows the sign-in page of {name}. Letters carries on once you have signed in there."
                );
                let (mark, copy) =
                    if self.copied { (icon::CHECK, "Copied") } else { (icon::COPY, "Copy the page\u{2019}s address") };
                let copy = Button::new(copy).icon(mark).small().on_press(send(Signing::CopyPage));
                let ends = Element::row().gap(tokens::SPACE_SM).child(copy).child(cancel());
                Some(Element::column().gap(tokens::SPACE_SM).child(waiting(&said)).child(ends))
            }
            Some(SignIn::Failed(why)) => {
                Some(Element::paragraph(why, quiet()).fg(tokens::DANGER).role(Role::Alert).label(why.as_str()))
            }
        };
        let title = Element::text("Sign in to a server", TextStyle::control().weight(600));
        Element::column().gap(tokens::SPACE_SM).child(title).child(ask).children(state)
    }
}

/// One server the person is signed in to, with the way out of it.
fn server(server: &Server) -> Element<Message> {
    let name = host(&server.address);
    let mark = Element::row()
        .width(MARK)
        .height(MARK)
        .radius(MARK / 2.0)
        .bg(tokens::NEUTRAL_SOFT)
        .fg(tokens::FG_MUTED)
        .align(Align::Center)
        .justify(Justify::Center)
        .child(Element::icon(icon::CLOUD, 16.0))
        .decorative();
    let account = format!("Signed in as {}", server.account);
    let names = Element::column()
        .grow(1.0)
        .child(Element::text(name, TextStyle::control().weight(600)).shrink())
        .child(Element::text(account.as_str(), quiet()).fg(tokens::FG_MUTED).shrink());
    let out = Button::new("Sign out").small().on_press(send(Signing::Out(server.address.clone())));
    Element::row()
        .key(server.address.as_str())
        .align(Align::Center)
        .gap(tokens::SPACE_MD)
        .role(Role::ListItem)
        .label(format!("{name}, signed in as {}", server.account))
        .child(mark)
        .child(names)
        .child(Element::from(out).description(name))
}

/// A sentence about something the person waits for, told to a screen reader as it comes.
fn waiting(said: &str) -> Element<Message> {
    Element::paragraph(said, quiet()).fg(tokens::FG_MUTED).role(Role::Status).label(said).grow(1.0)
}

/// A server as it is called where it is listed: its address without what every address has.
pub(crate) fn host(address: &str) -> &str {
    let address = address.trim_end_matches('/');
    address.strip_prefix("https://").or_else(|| address.strip_prefix("http://")).unwrap_or(address)
}

fn quiet() -> TextStyle {
    TextStyle::control().size(tokens::TEXT_SM)
}

fn send(message: Signing) -> Message {
    Message::Servers(message)
}
