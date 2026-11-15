//! History: the document as it was, version by version.
//!
//! The window swaps its bar's content for this view's, lists the versions beside the page and shows the
//! picked one where the page was. A version is read there and not written on: what changes the document
//! from here is restoring one, which is an edit that undo takes back.
//!
//! The model's history has a step for each minute of a writer's typing. A version is a run of steps with
//! no long rest in it and no night between: what was written at one sitting. A step that was marked ends
//! its version, so that a named version, an import, a merge or a restore is listed on its own.

use std::rc::Rc;

use accesskit::{Node, NodeId, Role};
use letters_editor::{Key, KeyEvent};
use letters_layout::{DocLayout, Fonts, Palette};
use letters_model::{BlockKind, Document, Point as Step, Revision, Style, Tag, TagKind};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Item, PathEl, Point, Rect, Size};

use crate::color::resolve;
use crate::describe::{self, Told};
use crate::element::{Element, Paint, Pointer};
use crate::highlight;
use crate::icon;
use crate::notes;
use crate::opening::{MONTHS, WEEKDAYS};
use crate::paint::scrollbar;
use crate::pictures;
use crate::style::{Align, Edges, Look};
use crate::text::TextStyle;
use crate::theme::accent;
use crate::threads;
use crate::tokens::{self, Color as Ink, Scheme};
use crate::widgets::{Button, SidePanel, TopBar};
use crate::window::{GUTTER, Message, PT, TOP};

/// How long writing rests before what comes after it is another version, in seconds.
const PAUSE: u64 = 600;
/// The keys of the bar's two forms, which are none of the editing bar's or Print view's.
const FORMS: (usize, usize) = (12, 13);
/// How far an arrow key scrolls.
const STEP: f32 = 48.0;
/// How much text one idle step lays out, in bytes. Small enough to fit between two frames.
const SETTLE_BYTES: usize = 4096;
/// The rail a day's versions hang on: the room it has across, the size of a version's marker on it, how
/// thick the rail and a marker's outline are, and the room a marker keeps clear of the rail.
const TRACK: f32 = 20.0;
const DOT: f32 = 10.0;
const LINE: f32 = 2.0;
const CLEAR: f32 = 3.0;
/// How tall the line of a version's title is, which its marker stands in the middle of.
const HEAD: f32 = 20.0;
/// The sign of a version that was given a name.
const SIGN: f32 = 12.0;

/// What is asked of the history.
#[derive(Clone, Debug, PartialEq)]
pub enum Historying {
    /// Shows the document's versions, the newest picked.
    Show,
    /// Goes back to editing.
    Back,
    /// Picks a version, counted from the oldest.
    Pick(usize),
    /// Picks the version before the one picked, or the one after it.
    Step { back: bool },
    /// The wheel turned over the version.
    Scroll(Point),
    /// Makes the document read as the picked version again.
    Restore,
}

/// A version the history lists.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Version {
    /// The revision that shows it.
    pub at: Revision,
    /// When it was last written, in seconds since 1970. 0 when nobody told the model the time.
    time: u64,
    /// Who wrote it, each with how many of its steps are theirs. Whoever wrote most comes first.
    authors: Vec<(String, usize)>,
    /// About how many characters it put in and took out.
    size: usize,
    /// Why it was marked, when it was.
    tag: Option<Tag>,
}

impl Version {
    fn kind(&self) -> Option<TagKind> {
        self.tag.as_ref().map(|tag| tag.kind)
    }

    /// The name a person gave it.
    fn name(&self) -> Option<&str> {
        let tag = self.tag.as_ref().filter(|tag| tag.kind == TagKind::Named)?;
        tag.name.as_deref().map(str::trim).filter(|name| !name.is_empty())
    }

    /// The time of day it was last written at, on the person's clock.
    fn clock(&self, zone: i32) -> Option<String> {
        let of_day = local(self.time, zone).rem_euclid(86_400);
        (self.time > 0).then(|| format!("{:02}:{:02}", of_day / 3_600, of_day / 60 % 60))
    }

    /// What it is listed as: its name, what made it a version, or else the time it was written at. `first`
    /// and `newest` say whether it is the oldest and the newest of the history.
    fn title(&self, (first, newest): (bool, bool), zone: i32) -> String {
        let import = |tag: &Tag| match tag.name.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
            Some(file) => format!("Imported from {file}"),
            None => "Imported".to_owned(),
        };
        match (self.name(), &self.tag, self.clock(zone)) {
            (Some(name), ..) => name.to_owned(),
            _ if newest => "Current version".to_owned(),
            (_, Some(tag), _) if tag.kind == TagKind::Import => import(tag),
            (_, Some(tag), _) if tag.kind == TagKind::Merge => "Merged edits".to_owned(),
            (_, Some(tag), _) if tag.kind == TagKind::Restore => "Restored an earlier version".to_owned(),
            (.., Some(clock)) => clock,
            // What a document began with was written before the model was told the time.
            _ if first => "First version".to_owned(),
            _ => "Undated edits".to_owned(),
        }
    }

    /// Who wrote it, in a few words. `None` when nobody put a name to it.
    fn who(&self) -> Option<String> {
        let name = |at: usize| threads::named(&self.authors[at].0);
        match self.authors.len() {
            0 => None,
            1 => Some(name(0).to_owned()),
            2 => Some(format!("{} and {}", name(0), name(1))),
            many => Some(format!("{} and {} others", name(0), many - 1)),
        }
    }

    /// What the day it was written on is called on `today`: "Today", "Yesterday", the weekday for a week
    /// back, and the date before that.
    fn day(&self, today: Option<i64>, zone: i32) -> String {
        if self.time == 0 {
            return "Undated".to_owned();
        }
        let day = day(self.time, zone);
        let ago = today.map(|today| today - day);
        let (year, month, date) = notes::date(u64::try_from(day).unwrap_or_default());
        let month = MONTHS[(month.clamp(1, 12) - 1) as usize];
        match ago {
            Some(0) => "Today".to_owned(),
            Some(1) => "Yesterday".to_owned(),
            Some(2..7) => WEEKDAYS[(day + 4).rem_euclid(7) as usize].to_owned(),
            // A date without its year is plain for most of a year back.
            Some(7..300) => format!("{date} {month}"),
            _ => format!("{date} {month} {year}"),
        }
    }
}

/// A moment on the person's clock, which is `zone` minutes ahead of UTC.
fn local(seconds: u64, zone: i32) -> i64 {
    i64::try_from(seconds).unwrap_or(i64::MAX).saturating_add(i64::from(zone) * 60)
}

/// The day a moment falls on where the person is, counted from 1970.
fn day(seconds: u64, zone: i32) -> i64 {
    local(seconds, zone).div_euclid(86_400)
}

/// How much a version changed, in words.
fn characters(size: usize) -> String {
    match size {
        1 => "1 character".to_owned(),
        ..10_000 => format!("{size} characters"),
        _ => format!("{}k characters", size / 1_000),
    }
}

/// The versions a history of `steps` lists, oldest first. Where a day ends is told by `zone`, which is how
/// far the person's clock is ahead of UTC in minutes.
fn versions(steps: &[Step], zone: i32) -> Vec<Version> {
    let mut versions: Vec<Version> = Vec::new();
    let mut open: Option<Version> = None;
    for step in steps {
        // A long rest or a night ends a version, and what was written after it is the next.
        let goes_on =
            |open: &Version| step.time.abs_diff(open.time) <= PAUSE && day(step.time, zone) == day(open.time, zone);
        if !open.as_ref().is_some_and(goes_on) {
            versions.extend(open.take());
        }
        let version = open.get_or_insert_with(|| Version {
            at: step.after.clone(),
            time: step.time,
            authors: Vec::new(),
            size: 0,
            tag: None,
        });
        (version.at, version.time) = (step.after.clone(), step.time);
        version.size += step.size;
        for (name, edits) in &step.authors {
            match version.authors.iter_mut().find(|(known, _)| known == name) {
                Some((_, count)) => *count += edits,
                None => version.authors.push((name.clone(), *edits)),
            }
        }
        // A step that was marked ends its version too. Each mark is a version, shown at its own revision,
        // and the first of them stands for what the run wrote.
        if let Some(run) = open.take_if(|_| !step.tags.is_empty()) {
            for (index, tag) in step.tags.iter().enumerate() {
                let size = if index == 0 { run.size } else { 0 };
                versions.push(Version { at: tag.at.clone(), size, tag: Some(tag.clone()), ..run.clone() });
            }
        }
    }
    versions.extend(open);
    for version in &mut versions {
        version.authors.sort_by_key(|(_, edits)| std::cmp::Reverse(*edits));
    }
    versions
}

/// The version that is picked, read and laid out.
struct Leaf {
    at: Revision,
    doc: Document,
    style: Style,
    layout: DocLayout,
    /// The width it is laid out at, once it is.
    laid: Option<f32>,
    /// Whether some of its heights are still estimates.
    estimated: bool,
}

/// What the history keeps while it shows.
pub(crate) struct History {
    /// Oldest first, and never empty.
    versions: Vec<Version>,
    /// The version picked.
    at: usize,
    /// What the page area shows of it, once the window has read it.
    leaf: Option<Leaf>,
    /// How far down the version is scrolled, which stays as it is from one version to the next: the same
    /// part of the document is looked at in each.
    scroll: f32,
    /// The room the last frame had, how tall what scrolls was, and how many device pixels a pixel is.
    room: Size,
    tall: f32,
    scale: f32,
    /// The time when the history was shown, when the shell has a clock, and how far the person's clock is
    /// ahead of UTC in minutes.
    now: Option<u64>,
    zone: i32,
}

impl History {
    /// The history of a document that reads as `current`, with its newest version picked.
    pub fn new(steps: &[Step], current: Revision, now: Option<u64>, zone: i32) -> Self {
        let mut history = Self {
            versions: Vec::new(),
            at: 0,
            leaf: None,
            scroll: 0.0,
            room: Size::default(),
            tall: 0.0,
            scale: 1.0,
            now,
            zone,
        };
        history.follow(steps, current);
        history
    }

    /// Lists the versions again after the document changed under the history, as when another writer's
    /// edits came in. The version that was picked stays picked while it is still listed.
    pub fn follow(&mut self, steps: &[Step], current: Revision) {
        let picked = self.versions.get(self.at).map(|version| version.at.clone());
        self.versions = versions(steps, self.zone);
        // The document as it reads now is always the last version, marked or not.
        let newest = steps.last().map_or(&current, |step| &step.after);
        if self.versions.last().is_none_or(|last| last.at != *newest) {
            let time = steps.last().map_or(0, |step| step.time);
            self.versions.push(Version { at: newest.clone(), time, authors: Vec::new(), size: 0, tag: None });
        }
        let kept = picked.and_then(|picked| self.versions.iter().position(|version| version.at == picked));
        self.at = kept.unwrap_or(self.versions.len() - 1);
    }

    pub fn picked(&self) -> &Version {
        &self.versions[self.at]
    }

    /// Whether the version picked is the document as it reads now, which there is nothing to restore of.
    pub fn newest(&self) -> bool {
        self.at + 1 == self.versions.len()
    }

    /// The revision to read, when the version picked is not the one that shows.
    pub fn wanted(&self) -> Option<&Revision> {
        let picked = &self.picked().at;
        self.leaf.as_ref().is_none_or(|leaf| leaf.at != *picked).then_some(picked)
    }

    /// Shows the picked version, as the window read it. With `None` the history has no such revision,
    /// and the page area stays empty.
    pub fn show(&mut self, read: Option<(Document, Style)>) {
        self.leaf = read.map(|(doc, style)| {
            let mut layout = DocLayout::new(Fonts { faces: letters_fonts::bundled() });
            // Nothing has a caret here, so a formula is typeset and never shows its source.
            layout.set_formula_source(false);
            Leaf { at: self.picked().at.clone(), doc, style, layout, laid: None, estimated: true }
        });
    }

    pub fn pick(&mut self, at: usize) {
        self.at = at.min(self.versions.len() - 1);
    }

    /// Picks the version before the one picked, or the one after it, when there is one.
    pub fn step(&mut self, back: bool) {
        self.pick(if back { self.at.saturating_sub(1) } else { self.at + 1 });
    }

    /// Takes faces that arrived late.
    pub fn add_fonts(&mut self, fonts: &Fonts) {
        if let Some(leaf) = &mut self.leaf
            && leaf.layout.add_fonts(fonts)
        {
            leaf.estimated = true;
        }
    }

    pub fn scroll(&mut self, delta: Point) {
        self.scroll += delta.y;
        self.keep();
    }

    /// Scrolls for a key that moves through the version, and says whether `event` was one.
    pub fn key(&mut self, event: &KeyEvent) -> bool {
        if event.mods.primary || event.mods.alt {
            return false;
        }
        let screen = 0.9 * self.room.height;
        match event.key {
            Key::Down => self.scroll += STEP,
            Key::Up => self.scroll -= STEP,
            Key::PageDown => self.scroll += screen,
            Key::PageUp => self.scroll -= screen,
            Key::Home => self.scroll = 0.0,
            Key::End => self.scroll = f32::MAX,
            _ => return false,
        }
        self.keep();
        true
    }

    /// Keeps the scroll within what there is to show.
    fn keep(&mut self) {
        self.scroll = self.scroll.clamp(0.0, (self.tall - self.room.height).max(0.0));
    }

    /// How far down the version shows: the nearest whole device pixel to where it was scrolled to, so
    /// that hinted text stays sharp.
    fn scrolled(&self) -> f32 {
        (self.scroll * self.scale).round() / self.scale
    }

    /// The part of the version in view, in its layout's coordinates.
    fn viewport(&self, width: f32) -> Rect {
        Rect::new(0.0, self.scrolled() - TOP, width, self.room.height)
    }

    /// Lays out some of what is out of view, in idle time. The line at the top of the view stays where it
    /// is while what is above it changes height. Returns whether there was something to lay out.
    pub fn settle(&mut self) -> bool {
        let Some(width) = self.leaf.as_ref().filter(|leaf| leaf.estimated).and_then(|leaf| leaf.laid) else {
            return false;
        };
        let viewport = self.viewport(width);
        let Some(leaf) = &mut self.leaf else { return false };
        let pin = leaf.layout.hit(Point { x: 0.0, y: viewport.y.max(0.0) });
        let top = |layout: &DocLayout| pin.as_ref().and_then(|pos| layout.caret(pos)).map(|caret| caret.y);
        let before = top(&leaf.layout);
        leaf.estimated = leaf.layout.settle(viewport, SETTLE_BYTES);
        if let (Some(before), Some(after)) = (before, top(&leaf.layout)) {
            self.scroll += after - before;
        }
        true
    }

    /// The picked version, for the area where the page was. Also returns the pictures in it that the
    /// shell has not handed over yet, which keep their place.
    pub fn view(
        &mut self,
        room: Size,
        images: &Images,
        palette: Palette,
        scheme: Scheme,
        scale: f32,
    ) -> (Element<Message>, Vec<String>) {
        (self.room, self.scale) = (room, scale);
        let (mut list, mut awaited, mut left) = (DisplayList::default(), Vec::new(), 0.0);
        // The version is set as the page is: as wide as its paper leaves between the margins, or as the
        // window has room for.
        let column = self.leaf.as_ref().map(|leaf| {
            let page = &leaf.style.page;
            let text = (page.width - page.margin_left - page.margin_right) * PT;
            text.min(room.width - 2.0 * GUTTER).max(1.0)
        });
        if let Some((leaf, width)) = self.leaf.as_mut().zip(column) {
            left = ((room.width - width) / 2.0).max(0.0).round();
            if leaf.laid != Some(width) {
                leaf.layout.update(&leaf.doc, &leaf.style, width, None);
                if leaf.laid.is_none() {
                    leaf.dress(images);
                }
                (leaf.laid, leaf.estimated) = (Some(width), true);
            }
            leaf.layout.set_palette(palette);
        }
        if let Some(width) = column {
            // What is about to show is laid out for real. The rest waits for idle time.
            let viewport = self.viewport(width);
            if let Some(leaf) = &mut self.leaf {
                leaf.estimated = leaf.layout.settle(viewport, 0);
                self.tall = leaf.layout.height() + 2.0 * TOP;
            }
            self.keep();
            let viewport = self.viewport(width);
            if let Some(leaf) = &self.leaf {
                leaf.layout.paint(viewport, &mut list);
            }
            awaited = pictures::awaited(images, scheme, &mut list);
        }
        let scrolled = self.scrolled();
        let paint = Page {
            list,
            origin: Point { x: left, y: TOP - scrolled },
            page: palette.page,
            content: self.tall,
            scrolled,
        };
        let canvas = Element::canvas(Rc::new(paint))
            .grow(1.0)
            .fg(tokens::FG)
            .role(Role::Document)
            .label("History")
            .default_focus()
            .on_pointer(|event| match event {
                Pointer::Scroll { delta } => Some(Message::History(Historying::Scroll(delta))),
                _ => None,
            });
        (canvas, awaited)
    }

    /// Fills the bar while the history shows: the way back, what the version is of and when it was
    /// written, and the button that restores it. A narrow window has the bar folded.
    pub fn bar(&self, bar: TopBar<Message>, name: &str) -> TopBar<Message> {
        let back = Message::History(Historying::Back);
        let name = || Element::text(name, TextStyle::control().weight(600)).min_width(tokens::SPACE_3XL).shrink();
        let when = Element::text(self.said(), TextStyle::control().size(tokens::TEXT_SM));
        // The document as it reads now is what a restore would leave: there is nothing to do.
        let restore = |button: Button<Message>| {
            let button = button.primary().small().disabled(self.newest());
            button.on_press(Message::History(Historying::Restore))
        };
        let bar = bar
            .apart()
            .label("History")
            .form(FORMS.0)
            .start(Button::new("Back to editing").icon(icon::ARROW_LEFT).small().on_press(back.clone()))
            .start(name())
            .start(when.fg(tokens::FG_MUTED).shrink())
            .middle(restore(Button::new("Restore this version").icon(icon::ROTATE_CCW)));
        bar.form(FORMS.1)
            .start(Button::icon_only(icon::ARROW_LEFT, "Back to editing").small().on_press(back))
            .start(name())
            .middle(restore(Button::icon_only(icon::ROTATE_CCW, "Restore this version")))
    }

    /// When the picked version was written and by whom, as the bar says it: "Tuesday 16:42, by Alice
    /// Moreau".
    fn said(&self) -> String {
        let version = self.picked();
        let day = version.day(self.today(), self.zone);
        let when = version.clock(self.zone).map_or_else(|| day.clone(), |clock| format!("{day} {clock}"));
        match version.who() {
            Some(who) => format!("{when}, by {who}"),
            None => when,
        }
    }

    fn today(&self) -> Option<i64> {
        self.now.map(|now| day(now, self.zone))
    }

    /// The days the versions were written on, newest first, each with what it is called and its versions.
    fn days(&self) -> Vec<(String, Vec<usize>)> {
        let today = self.today();
        let mut days: Vec<(String, Vec<usize>)> = Vec::new();
        for at in (0..self.versions.len()).rev() {
            let name = self.versions[at].day(today, self.zone);
            match days.last_mut().filter(|(last, _)| *last == name) {
                Some((_, listed)) => listed.push(at),
                None => days.push((name, vec![at])),
            }
        }
        days
    }

    /// The panel beside the version: every version, newest first under the day it was written on. A press
    /// on one picks it.
    pub fn panel(&self) -> Element<Message> {
        let count = match self.versions.len() {
            1 => "1 version".to_owned(),
            count => format!("{count} versions"),
        };
        let mut panel = SidePanel::new("History").aside(count);
        for (name, listed) in self.days() {
            let rows = listed.iter().enumerate().map(|(place, at)| {
                let ends = (place == 0, place + 1 == listed.len());
                self.row(*at, ends)
            });
            let list = Element::column().key(listed[0]).role(Role::ListBox).label(name.as_str()).children(rows);
            panel = panel.child(SidePanel::<Message>::section(&name)).child(list);
        }
        panel.into()
    }

    /// One version in the panel: its marker on the day's rail, what it is listed as, who wrote it and how
    /// much it changed. `ends` says whether it is the first and the last of its day.
    fn row(&self, at: usize, (first, last): (bool, bool)) -> Element<Message> {
        let version = &self.versions[at];
        let (newest, picked) = (at + 1 == self.versions.len(), at == self.at);
        let small = TextStyle::control().size(tokens::TEXT_SM);
        let quiet = TextStyle::control().size(tokens::TEXT_XS);
        let title = version.title((at == 0, newest), self.zone);
        let who = version.who();
        // The time is said once: by the title, or else after who wrote the version.
        let when = version.clock(self.zone).filter(|clock| *clock != title);
        let size = (version.size > 0).then(|| characters(version.size));

        let mut head = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .min_height(HEAD)
            .child(Element::text(title.as_str(), small).shrink())
            .child(Element::spacer());
        if version.name().is_some() {
            let named = Element::row()
                .align(Align::Center)
                .gap(tokens::SPACE_2XS)
                .padding(Edges::xy(tokens::SPACE_XS, 1.0))
                .border(Edges::all(1.0), tokens::LINE)
                .radius(tokens::RADIUS_SM)
                .child(Element::icon(icon::TAG, SIGN))
                .child(Element::text("Named", quiet));
            head = head.child(named.fg(tokens::FG_MUTED));
        } else if let Some(size) = &size {
            head = head.child(Element::text(size.as_str(), quiet).fg(tokens::FG_MUTED));
        }
        let mut body =
            Element::column().grow(1.0).gap(tokens::SPACE_2XS).padding(Edges::xy(0.0, tokens::SPACE_SM)).child(head);
        let by: Vec<&str> = [who.as_deref(), when.as_deref()].into_iter().flatten().collect();
        if !by.is_empty() {
            let face = version.authors.first().map(|(name, _)| threads::face(name, false));
            let by = Element::text(by.join(" \u{b7} "), small).fg(tokens::FG_MUTED).shrink();
            body = body.child(Element::row().align(Align::Center).gap(tokens::SPACE_SM).children(face).child(by));
        }

        // What made it a version of its own shows in its marker: the one that is the document now, and
        // one that came from elsewhere.
        let apart = matches!(version.kind(), Some(TagKind::Import | TagKind::Merge | TagKind::Restore));
        let ink = match (newest, apart) {
            (true, _) => accent().base,
            (false, true) => tokens::INFO,
            (false, false) => tokens::LINE_STRONG,
        };
        let rail =
            Rail { diamond: apart && !newest, ink, at: tokens::SPACE_SM + HEAD / 2.0, above: !first, below: !last };
        let told: Vec<String> = [who, when, size].into_iter().flatten().collect();
        let row = Element::row()
            .key(version.at.to_string())
            .gap(tokens::SPACE_SM)
            .padding(Edges::xy(tokens::SPACE_XS, 0.0))
            .radius(tokens::RADIUS_CONTROL)
            .hover(Look::bg(tokens::NEUTRAL_SOFT))
            .role(Role::ListBoxOption)
            .label(title)
            .description(told.join(", "))
            .selected(picked)
            .in_view(picked)
            // A press picks the version. The keyboard stays with the page, where the arrows step on.
            .on_click(Message::History(Historying::Pick(at)))
            .child(Element::canvas(Rc::new(rail)).width(TRACK).passive().decorative())
            .child(body);
        if picked { row.bg(accent().soft) } else { row }
    }

    /// Tells assistive technology of the version that shows, under the canvas that shows it.
    pub fn describe(&self, told: &mut Told, whole: bool, canvas: &mut Node, nodes: &mut Vec<(NodeId, Node)>) {
        canvas.set_description(self.said());
        if let Some(leaf) = &self.leaf {
            describe::version(&leaf.doc, told, whole, canvas, nodes);
        }
    }
}

impl Leaf {
    /// Gives the layout what the page's has beside the document: the colours of its code, and the
    /// pictures it links to that are there to draw.
    fn dress(&mut self, images: &Images) {
        for block in self.doc.walk() {
            if let BlockKind::Code { language } = &block.kind {
                let tokens = highlight::tokens(&block.plain_text(), language.as_deref().unwrap_or_default());
                self.layout.set_tokens(&block.id, tokens);
            }
            if let Some((src, image)) = pictures::link(block).and_then(|src| Some((src, images.get(src)?))) {
                self.layout.set_loaded(src, Some((image.width(), image.height())));
            }
        }
    }
}

/// What the layout painted of the version in view, placed where the page was.
struct Page {
    list: DisplayList,
    /// Where the layout's origin is inside the page area.
    origin: Point,
    page: Color,
    /// The height of everything that scrolls, and how far it is scrolled.
    content: f32,
    scrolled: f32,
}

impl Paint for Page {
    fn paint(&self, rect: Rect, _visible: Rect, scheme: Scheme, out: &mut DisplayList) {
        out.push(Item::Fill { rect, color: self.page, radius: 0.0 });
        let first = out.items.len();
        out.items.extend(self.list.items.iter().cloned());
        out.translate(first, Point { x: rect.x + self.origin.x, y: rect.y + self.origin.y });
        out.items.extend(scrollbar(rect, self.content, self.scrolled, scheme, 1.0));
    }
}

/// A version's piece of its day's rail, with its marker.
struct Rail {
    /// A version that came from elsewhere is a diamond, and any other a ring.
    diamond: bool,
    ink: Ink,
    /// How far down the piece the middle of the marker is.
    at: f32,
    /// Whether the rail comes in from the version above, and goes on to the one below.
    above: bool,
    below: bool,
}

impl Paint for Rail {
    fn paint(&self, rect: Rect, _visible: Rect, scheme: Scheme, out: &mut DisplayList) {
        let rail = resolve(tokens::LINE_STRONG, scheme, 1.0);
        let x = rect.x + ((rect.width - LINE) / 2.0).round();
        let (top, bottom) = (rect.y + self.at - DOT / 2.0 - CLEAR, rect.y + self.at + DOT / 2.0 + CLEAR);
        if self.above {
            out.push(Item::Fill { rect: Rect::new(x, rect.y, LINE, top - rect.y), color: rail, radius: 0.0 });
        }
        if self.below {
            let rest = Rect::new(x, bottom, LINE, rect.y + rect.height - bottom);
            out.push(Item::Fill { rect: rest, color: rail, radius: 0.0 });
        }
        let color = resolve(self.ink, scheme, 1.0);
        let middle = Point { x: x + LINE / 2.0, y: rect.y + self.at };
        if self.diamond {
            let reach = DOT / 2.0 - LINE / 2.0;
            let corners = [(0.0, -reach), (reach, 0.0), (0.0, reach), (-reach, 0.0)].map(|(x, y)| Point { x, y });
            let mut path = vec![PathEl::MoveTo(corners[0])];
            path.extend(corners[1..].iter().map(|corner| PathEl::LineTo(*corner)));
            path.push(PathEl::Close);
            out.push(Item::Path { origin: middle, scale: 1.0, path: path.into(), color, stroke: Some(LINE) });
        } else {
            let ring = Rect::new(middle.x - DOT / 2.0, middle.y - DOT / 2.0, DOT, DOT);
            out.push(Item::Stroke { rect: ring, color, width: LINE, radius: DOT / 2.0 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Noon on Tuesday 10 November 2026, in UTC.
    const NOON: u64 = 1_794_312_000;
    const HOUR: u64 = 3_600;
    const DAY: u64 = 86_400;

    fn revision(counter: usize) -> Revision {
        format!("{counter}@1").parse().expect("a revision")
    }

    /// When each step of a history was written and by whom, and the steps its versions are expected to
    /// end with.
    type Sittings<'a> = (&'a [(u64, &'a str)], &'a [usize]);

    /// A step of the timeline: its place in it, when it was written and by whom.
    fn step(place: usize, time: u64, author: &str) -> Step {
        Step {
            before: revision(place),
            after: revision(place + 1),
            authors: (!author.is_empty()).then(|| (author.to_owned(), 1)).into_iter().collect(),
            time,
            edits: 1,
            size: 10,
            tags: Vec::new(),
        }
    }

    fn tag(at: usize, kind: TagKind, name: Option<&str>) -> Tag {
        Tag { at: revision(at), kind, name: name.map(str::to_owned), author: None, time: 0, restored: None }
    }

    #[test]
    fn steps_written_at_one_sitting_are_one_version() {
        let cases: [Sittings; 5] = [
            // A minute apart, by one writer and by two.
            (&[(NOON, "Sam"), (NOON + 60, "Sam"), (NOON + 120, "Sam")], &[3]),
            (&[(NOON, "Sam"), (NOON + 60, "Alice"), (NOON + 120, "Sam")], &[3]),
            // A rest longer than ten minutes.
            (&[(NOON, "Sam"), (NOON + 60, "Sam"), (NOON + HOUR, "Sam")], &[2, 3]),
            // A night between two steps a minute apart.
            (&[(NOON + 12 * HOUR - 30, "Sam"), (NOON + 12 * HOUR + 30, "Sam")], &[1, 2]),
            // Steps nobody dated are told apart from those that were.
            (&[(0, ""), (NOON, "Sam"), (NOON + 60, "Sam")], &[1, 3]),
        ];
        for (written, expected) in cases {
            let steps: Vec<Step> =
                written.iter().enumerate().map(|(place, (time, author))| step(place, *time, author)).collect();
            let ends: Vec<Revision> = versions(&steps, 0).into_iter().map(|version| version.at).collect();
            let expected: Vec<Revision> = expected.iter().map(|end| revision(*end)).collect();
            assert_eq!(ends, expected, "{written:?}");
        }
    }

    #[test]
    fn a_version_says_who_wrote_most_of_it_first_and_how_much_it_changed() {
        let steps = [step(0, NOON, "Sam"), step(1, NOON + 60, "Alice"), step(2, NOON + 120, "Alice")];
        let listed = versions(&steps, 0);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].who().as_deref(), Some("Alice and Sam"));
        assert_eq!(listed[0].size, 30);
    }

    #[test]
    fn a_marked_step_ends_its_version_at_the_revision_that_was_marked() {
        let mut steps = vec![step(0, NOON, "Sam"), step(1, NOON + 60, "Sam"), step(2, NOON + 120, "Sam")];
        steps[1].tags = vec![tag(2, TagKind::Named, Some("Sent to builder")), tag(1, TagKind::Merge, None)];
        let listed = versions(&steps, 0);
        let read: Vec<(Revision, String)> = listed
            .iter()
            .enumerate()
            .map(|(at, version)| (version.at.clone(), version.title((at == 0, at + 1 == listed.len()), 0)))
            .collect();
        let expected = [(2, "Sent to builder"), (1, "Merged edits"), (3, "Current version")];
        assert_eq!(read, expected.map(|(at, title)| (revision(at), title.to_owned())));
        // What the run wrote is counted once.
        assert_eq!(listed.iter().map(|version| version.size).collect::<Vec<_>>(), [20, 0, 10]);
    }

    #[test]
    fn a_version_is_listed_under_its_day_on_the_persons_clock() {
        let today = Some(day(NOON, 0));
        // When it was written, how far the clock is ahead of UTC, and the day and time it is listed at.
        let cases = [
            (NOON, 0, "Today", Some("12:00")),
            (NOON - DAY, 0, "Yesterday", Some("12:00")),
            (NOON - 2 * DAY, 0, "Sunday", Some("12:00")),
            (NOON - 6 * DAY, 0, "Wednesday", Some("12:00")),
            (NOON - 7 * DAY, 0, "3 Nov", Some("12:00")),
            (NOON - 400 * DAY, 0, "6 Oct 2025", Some("12:00")),
            // Half past eleven at night in UTC is the next morning two hours east of it.
            (NOON - 12 * HOUR - 1_800, 120, "Today", Some("01:30")),
            (NOON - 12 * HOUR - 1_800, 0, "Yesterday", Some("23:30")),
            (NOON - 12 * HOUR + 1_800, -300, "Yesterday", Some("19:30")),
            (0, 0, "Undated", None),
        ];
        for (time, zone, name, clock) in cases {
            let version = Version { time, ..versions(&[step(0, time, "Sam")], zone).remove(0) };
            let today = today.map(|_| day(NOON, zone));
            assert_eq!(version.day(today, zone), name, "{time} at {zone}");
            assert_eq!(version.clock(zone).as_deref(), clock, "{time} at {zone}");
        }
    }

    #[test]
    fn the_history_always_ends_with_the_document_as_it_reads_now() {
        // A new document has no steps yet, and a mark can stand at an older revision than the last step.
        let mut marked = vec![step(0, NOON, "Sam"), step(1, NOON + 60, "Sam")];
        marked[1].tags = vec![tag(1, TagKind::Named, Some("Draft"))];
        for (steps, expected) in [(Vec::new(), 1), (marked, 2)] {
            let history = History::new(&steps, revision(2), Some(NOON), 0);
            assert_eq!(history.versions.len(), expected);
            assert!(history.newest());
            assert_eq!(history.picked().at, revision(2));
        }
    }
}
