//! Sharing a document a server keeps: one dialog lists the people and groups it is shared with, each with
//! what they may do, and the links made for people without an account. Its owner also reads there who
//! reached it: see `activity`.
//!
//! The window only asks: the shell talks to the server, and answers with the shares as they then stand or
//! with why something could not be done. What the person changes shows at once, and is asked for again
//! when the server refuses it.

use accesskit::Role;
use letters_api::{Access as Reach, Allows, CreatedLink, Grant, Grantee, Link, NewLink, Role as Grade, Shares};
use letters_editor::{Clip, Platform};

use crate::activity::{self, Earlier};
use crate::color::fade;
use crate::element::Element;
use crate::icon::{self, Icon};
use crate::opening::{Request, date_called, day};
use crate::style::{Align, Dir, Edges, Justify, Look, Ring};
use crate::text::TextStyle;
use crate::theme::accent;
use crate::threads::initials;
use crate::tokens::{self, Color as Ink};
use crate::widgets::{Button, Menu, MenuItem, Popover, Segmented, TextField};
use crate::window::Message;

const WIDTH: f32 = 700.0;
/// What the dialog takes of the window's height besides the people and the links, with the room it keeps
/// clear of the window's edges, and the least those are given before they scroll.
const AROUND: f32 = 220.0;
const SHORTEST: f32 = 200.0;
const FACE: f32 = 32.0;
/// What the field for a group is called.
pub(crate) const GROUP: &str = "Add a group";
/// How wide the control of a role is at least: the longest role fits, so the rows' controls line up.
const ROLE: f32 = 128.0;
/// How long a link lasts, in days, as the form offers it.
const LASTS: [(&str, Option<u64>); 4] =
    [("1 day", Some(1)), ("7 days", Some(7)), ("30 days", Some(30)), ("No expiry", None)];
/// The roles someone can be given. An owner is not made here.
const GRADES: [Grade; 3] = [Grade::Reader, Grade::Commenter, Grade::Editor];

/// Who a document on a server is shared with, for the Share dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shared {
    /// The people and groups that reach the document besides those its space lets in, and its links.
    pub shares: Shares,
    /// Whether the person says who else may reach it, as its owner does. Anyone else only reads the list.
    pub owner: bool,
    /// Whether the server makes links.
    pub links: bool,
}

/// Something a link lets its reader take away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Take {
    Download,
    Copy,
    Print,
}

/// The two lists of the dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    /// Who the document is shared with.
    #[default]
    Access,
    /// Who reached it through a share.
    Activity,
}

/// What the Share dialog sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sharing {
    Open,
    Close,
    /// Shows one of the dialog's lists.
    Tab(Tab),
    /// Asks for the rows of the log from before those listed.
    Earlier,
    /// What the field for a group's name holds.
    Group(String),
    /// Shares the document with the group that is typed.
    Add(Grade),
    /// Gives the row at this place another role.
    Grant(usize, Grade),
    /// Takes the row at this place away.
    Remove(usize),
    /// Shows the form that makes a link, or puts it away.
    Form(bool),
    /// Who the link is for.
    Label(String),
    LinkRole(Grade),
    /// How many days the link lasts. `None` is for ever.
    Lasts(Option<u64>),
    /// Allows or withholds something the link's reader could take away.
    Allow(Take, bool),
    Passphrase(String),
    /// Asks the shell for the link.
    Make,
    /// Puts the address of a link made here on the clipboard.
    CopyLink(String),
    /// Revokes the link with this id.
    Revoke(String),
}

/// The form that makes a link. What it sets is fixed once the link is made.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Form {
    label: String,
    role: Grade,
    lasts: Option<u64>,
    allows: Allows,
    passphrase: String,
    /// Whether the shell has been asked and has not answered.
    asked: bool,
}

impl Default for Form {
    fn default() -> Self {
        let lasts = Some(7);
        Self {
            label: String::new(),
            role: Grade::Reader,
            lasts,
            allows: Allows::default(),
            passphrase: String::new(),
            asked: false,
        }
    }
}

/// The dialog, and what the window knows of the document's shares while it is closed.
#[derive(Clone, Debug, Default)]
pub(crate) struct Share {
    open: bool,
    /// What the server last said. `None` until it has.
    shared: Option<Shared>,
    /// Why the last thing asked could not be done.
    notice: Option<String>,
    /// What the field for a group's name holds.
    group: String,
    form: Option<Form>,
    /// The addresses of the links made from this window, by the link's id. A server tells an address once.
    made: Vec<(String, String)>,
    /// The link whose address was last put on the clipboard.
    copied: Option<String>,
    /// The time when the dialog opened, in seconds since 1970, and the person's zone in minutes.
    now: Option<u64>,
    zone: i32,
    /// Whether the dialog shows a change that the server has not answered for.
    unsure: bool,
    /// Which of its lists the dialog shows.
    tab: Tab,
    /// Who reached the document through a share, as the server last said. `None` until it has.
    log: Option<Vec<Reach>>,
    /// Whether the server may have rows from before those.
    earlier: Earlier,
    /// How many days the server keeps a row of the log. 0 until a shell says.
    kept: u32,
}

impl Share {
    /// Puts the dialog away. What was typed and not sent goes with it.
    pub fn close(&mut self) {
        (self.open, self.form, self.notice, self.copied) = (false, None, None, None);
        self.tab = Tab::Access;
        self.group.clear();
    }

    /// Takes in who reached the document through a share, newest first.
    pub fn logged(&mut self, log: Vec<Reach>) {
        self.earlier = paged(&log);
        self.log = Some(log);
    }

    /// Takes in the rows from before those held, which were asked for. Others come too late.
    pub fn logged_earlier(&mut self, rows: Vec<Reach>) {
        if let (Some(log), Earlier::Asked) = (&mut self.log, self.earlier) {
            self.earlier = paged(&rows);
            log.extend(rows);
        }
    }

    /// Takes in how many days the server keeps a row of the log.
    pub fn keeps(&mut self, days: u32) {
        self.kept = days;
    }

    /// Takes in the shares as the server says they stand.
    pub fn told(&mut self, shared: Shared) {
        (self.shared, self.unsure) = (Some(shared), false);
    }

    /// Takes in why the last thing asked could not be done. A change the dialog shows already may not be
    /// so: the shares are then asked for again, once, so that a server out of reach is not asked for ever.
    pub fn refused(&mut self, why: &str, requests: &mut Vec<Request>) {
        self.notice = Some(why.to_owned());
        if let Some(form) = &mut self.form {
            form.asked = false;
        }
        // The earlier rows can be asked for again.
        if self.earlier == Earlier::Asked {
            self.earlier = Earlier::Offered;
        }
        if std::mem::take(&mut self.unsure) {
            requests.push(Request::Shares);
        }
    }

    /// Takes in a link the server made: the one time its address is told.
    pub fn made(&mut self, created: CreatedLink) {
        self.made.push((created.link.id.clone(), created.url));
        self.form = None;
        if let Some(shared) = &mut self.shared {
            shared.shares.links.retain(|link| link.id != created.link.id);
            shared.shares.links.insert(0, created.link);
        }
    }

    /// Does what the dialog asked. `served` says whether a server keeps the document.
    pub fn update(
        &mut self,
        message: Sharing,
        (served, zone): (bool, i32),
        platform: &mut dyn Platform,
        requests: &mut Vec<Request>,
    ) {
        // Whatever is done next leaves the last refusal behind.
        if !matches!(message, Sharing::Group(_) | Sharing::Label(_) | Sharing::Passphrase(_)) {
            self.notice = None;
        }
        let owner = self.shared.as_ref().is_some_and(|shared| shared.owner);
        let links = self.shared.as_ref().is_some_and(|shared| shared.links);
        match message {
            Sharing::Open => {
                (self.open, self.now, self.zone) = (true, platform.wall_time(), zone);
                if served {
                    requests.push(Request::Shares);
                }
            }
            Sharing::Close => self.close(),
            Sharing::Group(typed) => self.group = typed,
            Sharing::Label(typed) => _ = self.form.as_mut().map(|form| form.label = typed),
            Sharing::Passphrase(typed) => _ = self.form.as_mut().map(|form| form.passphrase = typed),
            Sharing::LinkRole(role) => _ = self.form.as_mut().map(|form| form.role = role),
            Sharing::Lasts(days) => _ = self.form.as_mut().map(|form| form.lasts = days),
            Sharing::Allow(take, allowed) => {
                if let Some(form) = &mut self.form {
                    match take {
                        Take::Download => form.allows.download = allowed,
                        Take::Copy => form.allows.copy = allowed,
                        Take::Print => form.allows.print = allowed,
                    }
                }
            }
            Sharing::CopyLink(id) => {
                if let Some((_, url)) = self.made.iter().find(|(made, _)| *made == id) {
                    platform.clipboard_write(Clip { text: url.clone(), ..Clip::default() });
                    self.copied = Some(id);
                }
            }
            // The rest changes who reaches the document, or tells who did, which is for whoever may share it.
            _ if !owner => {}
            Sharing::Tab(tab) => {
                self.tab = tab;
                if tab == Tab::Activity && served {
                    requests.push(Request::AccessLog);
                }
            }
            Sharing::Earlier => {
                let last = self.log.as_ref().and_then(|log| log.last());
                if let (Some(last), Earlier::Offered) = (last, self.earlier) {
                    requests.push(Request::AccessLogBefore(last.time));
                    self.earlier = Earlier::Asked;
                }
            }
            Sharing::Add(role) => {
                let name = std::mem::take(&mut self.group).trim().to_owned();
                if name.is_empty() || role.shares() {
                    return;
                }
                let group = Grantee::Group { name };
                self.people(requests, |people| match people.iter_mut().find(|grant| grant.to == group) {
                    Some(grant) => grant.role = role,
                    None => people.push(Grant { to: group, role }),
                });
            }
            Sharing::Grant(at, role) => self.people(requests, |people| {
                // An owner's row is not another's to change, and nobody is made an owner here.
                if let Some(grant) = people.get_mut(at).filter(|grant| !grant.role.shares() && !role.shares()) {
                    grant.role = role;
                }
            }),
            Sharing::Remove(at) => self.people(requests, |people| {
                if people.get(at).is_some_and(|grant| !grant.role.shares()) {
                    people.remove(at);
                }
            }),
            Sharing::Form(shown) => self.form = (shown && links).then(Form::default),
            Sharing::Make => {
                let Some(form) = self.form.as_mut().filter(|form| !form.asked && !form.label.trim().is_empty()) else {
                    return;
                };
                form.asked = true;
                let expires = form.lasts.zip(self.now).map(|(days, now)| now + days * 86_400);
                let passphrase = Some(form.passphrase.clone()).filter(|typed| !typed.is_empty());
                let (label, role, allows) = (form.label.trim().to_owned(), form.role, form.allows);
                requests.push(Request::MakeLink(NewLink { label, role, expires, passphrase, allows }));
            }
            Sharing::Revoke(id) => {
                if let Some(shared) = &mut self.shared {
                    shared.shares.links.retain(|link| link.id != id);
                }
                self.made.retain(|(made, _)| *made != id);
                self.unsure = true;
                requests.push(Request::RevokeLink(id));
            }
        }
    }

    /// Changes the people the document is shared with, and asks the shell for the list as it then stands.
    fn people(&mut self, requests: &mut Vec<Request>, change: impl FnOnce(&mut Vec<Grant>)) {
        let Some(shared) = &mut self.shared else { return };
        let before = shared.shares.people.clone();
        change(&mut shared.shares.people);
        if shared.shares.people != before {
            self.unsure = true;
            requests.push(Request::SharePeople(shared.shares.people.clone()));
        }
    }

    /// The dialog over the middle of the window, while it is open: see `settings::Dialog::view`. `name` is
    /// what the document is called, and `room` how tall the window is.
    pub fn view(&self, name: &str, served: bool, room: f32) -> Option<Element<Message>> {
        if !self.open {
            return None;
        }
        let title = format!("Share \u{201c}{name}\u{201d}");
        let head = Element::row()
            .align(Align::Center)
            .justify(Justify::Between)
            .gap(tokens::SPACE_MD)
            .child(Element::text(title, TextStyle::title().size(tokens::TEXT_XL)).shrink())
            .child(Button::icon_only(icon::X, "Close").subtle().small().on_press(send(Sharing::Close)));
        let notice = self.notice.as_ref().map(|notice| {
            Element::paragraph(notice, quiet()).fg(tokens::DANGER).role(Role::Alert).label(notice.as_str())
        });
        let done = || Element::from(Button::new("Done").primary().on_press(send(Sharing::Close)));
        // While a link is made its form stands alone: what it sets is read once, with nothing beside it.
        let (body, ends) = match (&self.shared, &self.form, served) {
            (_, _, false) => {
                let local = "Sharing is for documents a Letters server keeps. This one is kept on this device, and \
                             nobody reaches it but you.";
                (said(local), vec![done()])
            }
            (None, _, true) if self.notice.is_some() => (Element::column(), vec![done()]),
            (None, _, true) => (said("Asking the server who this document is shared with."), vec![done()]),
            (Some(_), Some(form), true) => (form.card(self.now.is_some()), form.ends()),
            (Some(shared), None, true) if shared.owner && self.tab == Tab::Activity => (self.activity(), vec![done()]),
            (Some(shared), None, true) => (self.shares(shared), vec![done()]),
        };
        // Who reached the document is told to whoever may share it.
        let owner = self.shared.as_ref().is_some_and(|shared| shared.owner);
        let tabs = (served && owner && self.form.is_none()).then(|| self.tabs());
        // What ends the dialog stands on a band across its foot.
        let foot = Element::row()
            .justify(Justify::End)
            .gap(tokens::SPACE_SM)
            .padding(Edges::xy(tokens::SPACE_XL, tokens::SPACE_MD))
            .bg(tokens::TONE_1)
            .children(ends);
        // What scrolls runs to the dialog's edges, so that its thumb stands clear of what it scrolls.
        let side = Edges::xy(tokens::SPACE_XL, 0.0);
        let within = |part: Element<Message>| Element::column().padding(side).child(part);
        let body = within(body).max_height((room - AROUND).max(SHORTEST)).scroll();
        // The band runs to the card's edges, so the content is as wide as the surface is inside its line, and
        // is cut to the surface's corners there.
        let content = Element::column()
            .width(WIDTH - 2.0)
            .gap(tokens::SPACE_LG)
            .padding(Edges::top(tokens::SPACE_LG))
            .radius(tokens::RADIUS_CARD - 1.0)
            .clip()
            .child(within(head))
            .children(tabs.map(within))
            .children(notice.map(within))
            .child(body)
            .child(foot);
        let dialog = Popover::new(content)
            .flush()
            .label("Share")
            .max_width(WIDTH)
            .modal()
            .open(true)
            .on_close(send(Sharing::Close));
        Some(Element::row().overlay().popover(dialog))
    }

    /// The two lists as tabs: who may reach the document, and who did.
    fn tabs(&self) -> Element<Message> {
        let tab = |said: &'static str, tab: Tab| {
            let shown = self.tab == tab;
            let (ink, under) = match shown {
                true => (tokens::FG, accent().base),
                false => (tokens::FG_MUTED, fade(tokens::LINE, 0.0)),
            };
            Element::row()
                .padding(Edges::xy(tokens::SPACE_MD, tokens::SPACE_SM))
                .border(Edges::bottom(2.0), under)
                .fg(ink)
                .hover(Look::bg(tokens::NEUTRAL_SOFT))
                .ring(Ring::Outline(2.0))
                .role(Role::Tab)
                .label(said)
                .selected(shown)
                .focusable()
                .on_press(send(Sharing::Tab(tab)))
                .child(Element::text(said, TextStyle::control().weight(if shown { 600 } else { 400 })))
        };
        Element::row()
            .role(Role::TabList)
            .label("Share")
            .border(Edges::bottom(1.0), tokens::LINE)
            .arrows(Dir::Row)
            .one_stop()
            .child(tab("Access", Tab::Access))
            .child(tab("Activity", Tab::Activity))
    }

    /// Who reached the document through a share, once the server has said.
    fn activity(&self) -> Element<Message> {
        match &self.log {
            Some(log) => activity::view(log, self.now, self.zone, self.kept, self.earlier),
            None if self.notice.is_some() => Element::column(),
            None => said("Asking the server who has reached this document."),
        }
    }

    /// The people and the links.
    fn shares(&self, shared: &Shared) -> Element<Message> {
        let people = shared.shares.people.iter().enumerate().map(|(at, grant)| person(at, grant, shared.owner));
        let people = Element::column().gap(tokens::SPACE_SM).role(Role::List).label("Shared with").children(people);
        let mut body = Element::column().gap(tokens::SPACE_LG);
        if shared.owner {
            body = body.child(self.adding());
        }
        body = body.child(people);
        // Revoked links stay with the server for its log. Here they are gone.
        let links: Vec<&Link> = shared.shares.links.iter().filter(|link| link.revoked.is_none()).collect();
        if shared.links || !links.is_empty() {
            body =
                body.child(Element::row().height(1.0).bg(tokens::LINE).decorative()).child(self.links(shared, &links));
        }
        body
    }

    /// The field that takes a group's name, with the button that adds it as a reader and the menu that adds
    /// it as something else: one control in one frame.
    fn adding(&self) -> Element<Message> {
        let nothing = self.group.trim().is_empty();
        let field = TextField::new(GROUP, &self.group)
            .placeholder("Add a group by its name")
            .on_input(|typed| send(Sharing::Group(typed)))
            .on_submit(send(Sharing::Add(Grade::Reader)));
        let add =
            Button::new("Add reader").icon(icon::EYE).disabled(nothing).on_press(send(Sharing::Add(Grade::Reader)));
        let mut others = Menu::new().label("Add as");
        for role in GRADES {
            let (mark, ink, _) = marks(role);
            let item =
                MenuItem::new(format!("Add {}", named(role).to_lowercase())).icon(mark).ink(ink).aside(does(role));
            others = others.item(item.disabled(nothing).on_select(send(Sharing::Add(role))));
        }
        // Both menus hang from the dialog's right edge, and open inwards from it.
        let others =
            Button::icon_only(icon::CHEVRON_DOWN, "Add with another role").popover(others.popover().align(Align::End));
        // Each part gives up its own line for the frame's and stands a line from the next. The parts at the
        // ends keep the frame's corners, so the field's ring lies on the frame, and a button's ring is drawn
        // inside it, clear of its neighbours.
        let inside = tokens::RADIUS_CONTROL - 1.0;
        let part = |part: Element<Message>, corner: f32| {
            let mut part = part.height(tokens::CONTROL_HEIGHT - 2.0).radius(corner);
            part.style.border = Edges::ZERO;
            part
        };
        let line = || Element::row().width(1.0).bg(tokens::LINE_STRONG).decorative();
        Element::row()
            .border(Edges::all(1.0), tokens::LINE_STRONG)
            .radius(tokens::RADIUS_CONTROL)
            .child(part(Element::from(field).grow(1.0), inside))
            .child(line())
            .child(part(add.into(), 0.0).ring(Ring::Outline(-3.0)))
            .child(line())
            .child(part(others.into(), inside).ring(Ring::Outline(-3.0)))
    }

    /// The links made for the document, newest first.
    fn links(&self, shared: &Shared, links: &[&Link]) -> Element<Message> {
        let about = "For people without an account. A link\u{2019}s limits are set once, when it is made.";
        let about = Element::column()
            .grow(1.0)
            .child(Element::text("Links", TextStyle::control().weight(600)))
            .child(Element::paragraph(about, quiet()).fg(tokens::FG_MUTED));
        let new = (shared.owner && shared.links)
            .then(|| Button::new("New link").icon(icon::PLUS).small().on_press(send(Sharing::Form(true))));
        let head =
            Element::row().align(Align::Start).gap(tokens::SPACE_MD).child(about).children(new.map(Element::from));
        let cards = links.iter().map(|link| self.link(link, shared.owner));
        Element::column().gap(tokens::SPACE_MD).child(head).children(cards)
    }

    /// A link's card. Its limits were fixed when it was made, so they read as labels and not as controls.
    fn link(&self, link: &Link, owner: bool) -> Element<Message> {
        // A label too long for the card is cut short: the day and the buttons keep their room.
        let label = Element::text(link.label.as_str(), TextStyle::control().size(tokens::TEXT_SM).weight(600));
        let tag = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_XS)
            .padding(Edges::xy(tokens::SPACE_SM, 2.0))
            .border(Edges::all(1.0), tokens::LINE_STRONG)
            .radius(tokens::RADIUS_SM)
            .shrink()
            .child(Element::icon(icon::LINK, 14.0).fg(tokens::FG_MUTED))
            .child(label.shrink());
        let made = format!("Made {}", dated(link.created, self.now, self.zone));
        let mut top = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .child(tag)
            .child(Element::text(made, quiet()).fg(tokens::FG_MUTED))
            .child(Element::spacer());
        // Only a link made from this window still has its address here.
        if self.made.iter().any(|(made, _)| *made == link.id) {
            let (said, mark) = match self.copied.as_ref() == Some(&link.id) {
                true => ("Copied", icon::CHECK),
                false => ("Copy link", icon::COPY),
            };
            let copy = Button::new(said).icon(mark).small().on_press(send(Sharing::CopyLink(link.id.clone())));
            top = top.child(Element::from(copy).description(format!("The link for {}", link.label)));
        }
        if owner {
            let revoke =
                Button::new("Revoke").icon(icon::TRASH_2).small().on_press(send(Sharing::Revoke(link.id.clone())));
            top = top.child(Element::from(revoke).description(format!("The link for {}", link.label)));
        }
        let (lasts, ended) = expiry(link.expires, self.now, self.zone);
        let lasts_mark = if link.expires.is_some() { icon::HOURGLASS } else { icon::INFINITY };
        let (takes, none) = taken(link.allows);
        let key = if link.passphrase { "Passphrase" } else { "No passphrase" };
        let limits = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_MD)
            .child(cap(link.role))
            .child(limit(lasts_mark, &lasts, ended))
            .child(limit(icon::DOWNLOAD, &takes, none))
            .child(limit(icon::KEY_ROUND, key, !link.passphrase));
        let mut card = Element::column()
            .gap(tokens::SPACE_SM)
            .padding(Edges::all(tokens::SPACE_MD))
            .border(Edges::all(1.0), tokens::CARD_LINE)
            .radius(tokens::RADIUS_CONTROL)
            .role(Role::Group)
            .label(format!("Link for {}", link.label))
            .child(top)
            .child(limits);
        if self.made.iter().any(|(made, _)| *made == link.id) {
            let once = "Copy the link now. Its address is not kept, and is not shown again once this window closes.";
            card = card.child(Element::paragraph(once, quiet()).fg(tokens::FG_MUTED));
        }
        card
    }
}

impl Form {
    /// The form. `clock` says whether the shell tells the time, without which no link can be told when to
    /// end.
    fn card(&self, clock: bool) -> Element<Message> {
        let label = TextField::new("Who the link is for", &self.label)
            .placeholder("Who it is for, such as Accountant")
            .on_input(|typed| send(Sharing::Label(typed)))
            .on_submit(send(Sharing::Make));
        let mut role = Segmented::new("Role").small();
        for grade in GRADES {
            role = role.segment(named(grade), self.role == grade, send(Sharing::LinkRole(grade)));
        }
        let mut lasts = Segmented::new("Expires").small();
        for (said, days) in LASTS {
            lasts = lasts.segment(said, self.lasts == days, send(Sharing::Lasts(days)));
        }
        let takes = [("Download", Take::Download, self.allows.download), ("Copy", Take::Copy, self.allows.copy)];
        let takes =
            takes.into_iter().chain([("Print", Take::Print, self.allows.print)]).map(|(said, take, allowed)| {
                Element::from(Button::new(said).small().pressed(allowed).on_press(send(Sharing::Allow(take, !allowed))))
            });
        let takes = Element::row().gap(tokens::SPACE_XS).role(Role::Group).label("Allow").children(takes);
        let passphrase = TextField::new("Passphrase", &self.passphrase)
            .placeholder("No passphrase")
            .on_input(|typed| send(Sharing::Passphrase(typed)))
            .on_submit(send(Sharing::Make));
        let about = "For someone without an account. What is set here is fixed once the link is made: another \
                     setting is another link.";
        let about = Element::column()
            .child(Element::text("New link", TextStyle::control().weight(600)))
            .child(Element::paragraph(about, quiet()).fg(tokens::FG_MUTED));
        let mut card = Element::column()
            .gap(tokens::SPACE_MD)
            .role(Role::Group)
            .label("New link")
            .child(about)
            .child(set("For", Element::from(label).key("sharing-link").takes_focus().grow(1.0)))
            .child(set("Role", role.into()));
        if clock {
            card = card.child(set("Expires", lasts.into()));
        }
        card.child(set("Allow", takes)).child(set("Passphrase", Element::from(passphrase).grow(1.0)))
    }

    /// What the dialog ends with while the form shows: the way back, and the button that makes the link.
    fn ends(&self) -> Vec<Element<Message>> {
        let make = Button::new("Make link")
            .primary()
            .disabled(self.asked || self.label.trim().is_empty())
            .on_press(send(Sharing::Make));
        vec![Button::new("Cancel").on_press(send(Sharing::Form(false))).into(), make.into()]
    }
}

/// One thing the form sets: its name and the control that sets it, which names itself to assistive
/// technology.
fn set(name: &str, control: Element<Message>) -> Element<Message> {
    let name = Element::row().width(96.0).child(Element::text(name, quiet().weight(500)).fg(tokens::FG_MUTED));
    Element::row().align(Align::Center).gap(tokens::SPACE_MD).child(name.decorative()).child(control)
}

/// One row of the people: a face, the name, and the role, which the owner changes.
fn person(at: usize, grant: &Grant, owner: bool) -> Element<Message> {
    let (name, group) = match &grant.to {
        Grantee::Person { name, .. } => (name.as_str(), false),
        Grantee::Group { name } => (name.as_str(), true),
    };
    let face = Element::row()
        .width(FACE)
        .height(FACE)
        .radius(FACE / 2.0)
        .bg(tokens::NEUTRAL_SOFT)
        .align(Align::Center)
        .justify(Justify::Center)
        .child(Element::text(initials(name), TextStyle::control().size(tokens::TEXT_SM).weight(600)))
        .decorative();
    let detail = group.then(|| Element::text("Group", quiet()).fg(tokens::FG_MUTED));
    // A name too long for the row is cut short, so that its role stays in line with the others.
    let said = Element::text(name, TextStyle::control().weight(500)).shrink();
    let who = Element::column().grow(1.0).shrink().child(said).children(detail);
    // An owner's row stays as it is, and only the owner changes another's.
    let role = match owner && !grant.role.shares() {
        true => choose(at, name, grant.role),
        false => cap(grant.role),
    };
    Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_MD)
        .role(Role::ListItem)
        .label(format!("{name}, {}", named(grant.role)))
        .child(face)
        .child(who)
        .child(role)
}

/// The control that gives someone another role, or takes their row away.
fn choose(at: usize, name: &str, role: Grade) -> Element<Message> {
    let mut menu = Menu::new().label("Role");
    for grade in GRADES {
        let (mark, ink, _) = marks(grade);
        let item = MenuItem::new(named(grade)).icon(mark).ink(ink).aside(does(grade)).checked(grade == role);
        menu = menu.item(item.on_select(send(Sharing::Grant(at, grade))));
    }
    menu = menu.separator().item(MenuItem::new("Remove").icon(icon::X).danger().on_select(send(Sharing::Remove(at))));
    let (mark, ink, soft) = marks(role);
    Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_XS)
        .height(tokens::CONTROL_HEIGHT_SM)
        .padding(Edges::xy(tokens::SPACE_SM, 0.0))
        .border(Edges::all(1.0), fade(ink, 0.45))
        .radius(tokens::RADIUS_CONTROL)
        .bg(soft)
        .fg(ink)
        .hover(Look::bg(fade(ink, 0.18)))
        .ring(Ring::Outline(2.0))
        .role(Role::Button)
        .label(format!("Role of {name}"))
        .description(named(role))
        .min_width(ROLE)
        .child(Element::icon(mark, 14.0))
        .child(Element::text(named(role), TextStyle::control().size(tokens::TEXT_SM).weight(500)).grow(1.0))
        .child(Element::icon(icon::CHEVRON_DOWN, 14.0))
        .popover(menu.popover().align(Align::End))
}

/// A role as a label, in its colour: sharing is where a role's colour shows.
pub(crate) fn cap(role: Grade) -> Element<Message> {
    let (mark, ink, soft) = marks(role);
    Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_XS)
        .height(tokens::CONTROL_HEIGHT_SM - 4.0)
        .padding(Edges::xy(tokens::SPACE_SM, 0.0))
        .radius((tokens::CONTROL_HEIGHT_SM - 4.0) / 2.0)
        .bg(soft)
        .fg(ink)
        .child(Element::icon(mark, 14.0))
        .child(Element::text(named(role), TextStyle::control().size(tokens::TEXT_SM).weight(500)))
}

/// One limit of a link. `off` strikes the icon: the thing is not allowed, or is over.
fn limit(mark: Icon, said: &str, off: bool) -> Element<Message> {
    let strike = off.then(|| Element::icon(icon::SLASH, 14.0).overlay());
    let mark = Element::row().child(Element::icon(mark, 14.0)).children(strike);
    let ink = if off { tokens::FG_MUTED } else { tokens::FG };
    Element::row().align(Align::Center).gap(tokens::SPACE_XS).fg(ink).child(mark).child(Element::text(said, quiet()))
}

pub(crate) fn named(role: Grade) -> &'static str {
    match role {
        Grade::Reader => "Reader",
        Grade::Commenter => "Commenter",
        Grade::Editor => "Editor",
        Grade::Owner => "Owner",
    }
}

/// What a role may do, in a few words.
fn does(role: Grade) -> &'static str {
    match role {
        Grade::Reader => "Reads",
        Grade::Commenter => "Reads and comments",
        Grade::Editor => "Reads, comments and edits",
        Grade::Owner => "Edits, and says who else may",
    }
}

/// A role's icon, its colour and the ground it stands on: an eye in grey, a speech bubble in the blue of
/// comments, a pencil in green, and a crown in the same green for the owner.
pub(crate) fn marks(role: Grade) -> (Icon, Ink, Ink) {
    match role {
        Grade::Reader => (icon::EYE, tokens::FG_MUTED, tokens::NEUTRAL_SOFT),
        Grade::Commenter => (icon::MESSAGE_SQUARE, tokens::INFO, tokens::INFO_SOFT),
        Grade::Editor => (icon::PENCIL, tokens::OK, tokens::OK_SOFT),
        Grade::Owner => (icon::CROWN, tokens::OK, tokens::OK_SOFT),
    }
}

/// What a link lets its reader take away, in words, and whether that is nothing.
fn taken(allows: Allows) -> (String, bool) {
    let takes = [("download", allows.download), ("copy", allows.copy), ("print", allows.print)];
    let allowed: Vec<&str> = takes.iter().filter(|(_, allowed)| *allowed).map(|(take, _)| *take).collect();
    let said = match allowed[..] {
        [] => return ("No download, copy or print".into(), true),
        [one] => format!("{one} allowed"),
        [one, other] => format!("{one} and {other} allowed"),
        _ => "download, copy and print allowed".into(),
    };
    let mut letters = said.chars();
    let first = letters.next().map(|first| first.to_ascii_uppercase());
    (first.into_iter().chain(letters).collect(), false)
}

/// When a link ends, in words, and whether it has.
fn expiry(expires: Option<u64>, now: Option<u64>, zone: i32) -> (String, bool) {
    match (expires, now) {
        (None, _) => ("No expiry".into(), false),
        (Some(at), Some(now)) if at <= now => (format!("Expired {}", dated(at, Some(now), zone)), true),
        (Some(at), Some(now)) => match (at - now).div_ceil(86_400) {
            ..=1 => ("Expires within a day".into(), false),
            days => (format!("Expires in {days} days"), false),
        },
        (Some(at), None) => (format!("Expires {}", dated(at, None, zone)), false),
    }
}

/// A day as the dialog says it: "4 Nov", with its year where that is not plain.
fn dated(seconds: u64, now: Option<u64>, zone: i32) -> String {
    date_called(day(seconds, zone), now.map(|now| day(now, zone)))
}

/// A sentence that stands where a list would.
/// Whether a page of the log may have rows before it: a server tells a full page while it has more.
fn paged(rows: &[Reach]) -> Earlier {
    if rows.len() >= activity::PAGE { Earlier::Offered } else { Earlier::None }
}

pub(crate) fn said(sentence: &str) -> Element<Message> {
    Element::paragraph(sentence, TextStyle::body()).fg(tokens::FG_MUTED)
}

pub(crate) fn quiet() -> TextStyle {
    TextStyle::control().size(tokens::TEXT_SM)
}

fn send(message: Sharing) -> Message {
    Message::Sharing(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_says_what_it_allows_and_when_it_ends() {
        let allows = |download, copy, print| Allows { download, copy, print };
        let cases = [
            (allows(false, false, false), "No download, copy or print", true),
            (allows(true, false, false), "Download allowed", false),
            (allows(false, true, true), "Copy and print allowed", false),
            (allows(true, true, true), "Download, copy and print allowed", false),
        ];
        for (allows, said, none) in cases {
            assert_eq!(taken(allows), (said.to_owned(), none), "{allows:?}");
        }

        // Noon on 15 November 2026.
        let now = 1_794_744_000;
        let day = 86_400;
        let cases = [
            (None, Some(now), "No expiry", false),
            (Some(now + 12 * day), Some(now), "Expires in 12 days", false),
            (Some(now + 3_600), Some(now), "Expires within a day", false),
            (Some(now - 11 * day), Some(now), "Expired 4 Nov", true),
            (Some(now + 30 * day), None, "Expires 15 Dec 2026", false),
        ];
        for (expires, now, said, ended) in cases {
            assert_eq!(expiry(expires, now, 0), (said.to_owned(), ended), "{expires:?} at {now:?}");
        }
    }
}
