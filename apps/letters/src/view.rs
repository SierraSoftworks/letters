//! What a window shows: a document, its caret and its selection.
//!
//! This is a stand-in for `letters_ui::Window` and keeps to its method names, so that taking the real window
//! is a change of type. It drives the editor and the layout directly and has no chrome.

use accesskit::{ActionRequest, TreeUpdate};
use letters_editor::{Clip, Command, Editor, InputEvent, KeyEvent, Lines, Modifiers, Platform};
use letters_layout::{DocLayout, Fonts, Palette};
use letters_model::{Change, Pos, Selection, Style};
use letters_render::{DisplayList, Item, Point, Rect, Size};

use crate::access::{self, Request};

/// Space kept around the text column.
const GUTTER: f32 = 24.0;
const TOP: f32 = 48.0;
const CARET_WIDTH: f32 = 1.5;
const BLINK_MS: u64 = 530;
/// How much text one idle step lays out: about 2 ms of work.
const SETTLE_BYTES: usize = 8192;
/// Clicks this close together, in time and place, count as one double or triple click.
const CLICK_MS: u64 = 500;
const CLICK_SLOP: f32 = 4.0;
/// Points to logical pixels.
const PT: f32 = 96.0 / 72.0;

/// The pointer shape the view asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Default,
    Pointer,
    Text,
}

pub struct View {
    editor: Editor,
    style: Style,
    layout: DocLayout,
    palette: Palette,
    name: String,
    size: Size,
    /// The width the layout was last made for, or zero before the first layout.
    laid_width: f32,
    /// Whether the layout still holds height estimates for blocks it has not laid out.
    estimated: bool,
    scroll: f32,
    focused: bool,
    dragging: bool,
    /// The last press of the pointer: when, where, and which click of a double or triple it was.
    clicked: Option<(u64, Point, u8)>,
    /// When the caret last moved. It shows solid from then, and blinks after.
    caret_moved_ms: u64,
    caret_shown: bool,
    edits: u64,
    needs_paint: bool,
}

impl View {
    pub fn new(editor: Editor, style: Style) -> Self {
        Self {
            editor,
            style,
            layout: DocLayout::new(Fonts { faces: letters_fonts::bundled() }),
            palette: Palette::default(),
            name: String::new(),
            size: Size::default(),
            laid_width: 0.0,
            estimated: false,
            scroll: 0.0,
            focused: true,
            dragging: false,
            clicked: None,
            caret_moved_ms: 0,
            caret_shown: true,
            edits: 0,
            needs_paint: true,
        }
    }

    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    pub fn style(&self) -> &Style {
        &self.style
    }

    /// The colours of the page, and of everything on it that the style does not colour.
    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.layout.set_palette(palette);
        self.palette = palette;
        self.needs_paint = true;
    }

    /// Goes up each time the document changes.
    pub fn edits(&self) -> u64 {
        self.edits
    }

    pub fn set_document_name(&mut self, name: &str) {
        self.name = name.into();
    }

    pub fn needs_paint(&self) -> bool {
        self.needs_paint
    }

    pub fn focus(&mut self, focused: bool) {
        self.focused = focused;
        self.needs_paint = true;
    }

    /// The text column: as wide as the page's text area, or the window when that is narrower.
    fn column(&self) -> Rect {
        let page = &self.style.page;
        let text_width = (page.width - page.margin_left - page.margin_right) * PT;
        let width = text_width.min(self.size.width - 2.0 * GUTTER).max(1.0);
        Rect::new(((self.size.width - width) / 2.0).round(), TOP - self.scroll, width, self.layout_height())
    }

    /// The part of the column the window shows, in the column's own coordinates.
    fn viewport(&self) -> Rect {
        Rect::new(0.0, self.scroll - TOP, self.laid_width, self.size.height)
    }

    fn layout_height(&self) -> f32 {
        if self.laid_width > 0.0 { self.layout.height() } else { 0.0 }
    }

    fn max_scroll(&self) -> f32 {
        (self.layout_height() + 2.0 * TOP - self.size.height).max(0.0)
    }

    /// The scale is the renderer's business: everything here is in logical pixels.
    pub fn resize(&mut self, size: Size, _scale: f32) {
        self.size = size;
        let width = self.column().width;
        if width != self.laid_width {
            self.layout.update(self.editor.doc(), &self.style, width, None);
            self.laid_width = width;
            self.estimated = true;
        }
        self.scroll = self.scroll.min(self.max_scroll());
        self.needs_paint = true;
    }

    pub fn paint(&mut self, out: &mut DisplayList) {
        self.needs_paint = false;
        let window = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        out.push(Item::Fill { rect: window, color: self.palette.page, radius: 0.0 });
        if self.laid_width == 0.0 {
            return;
        }
        // What is about to be shown must be laid out for real. The rest waits for idle time.
        let viewport = self.viewport();
        self.estimated = self.layout.settle(viewport, 0);
        let column = self.column();
        let first = out.items.len();
        if let Some(selection) = self.editor.selection().filter(|selection| !selection.is_caret()) {
            for rect in self.layout.selection_in(selection, viewport) {
                out.push(Item::Fill { rect, color: self.palette.selection, radius: 0.0 });
            }
        }
        self.layout.paint(viewport, out);
        // Text an input method is still composing is underlined.
        if let Some((block, range)) = self.editor.composition() {
            let at = |offset| Pos { block: block.clone(), offset };
            let composing = Selection { anchor: at(range.start), head: at(range.end) };
            for line in self.layout.selection_in(&composing, viewport) {
                let rect = Rect { y: line.y + line.height - 1.0, height: 1.0, ..line };
                out.push(Item::Fill { rect, color: self.palette.text, radius: 0.0 });
            }
        }
        if self.focused
            && self.caret_shown
            && let Some(caret) = self.caret()
        {
            out.push(Item::Fill { rect: Rect { width: CARET_WIDTH, ..caret }, color: self.palette.caret, radius: 0.0 });
        }
        // The layout speaks in column coordinates.
        out.translate(first, Point { x: column.x, y: column.y });
    }

    /// The caret in column coordinates, when the selection is one.
    fn caret(&self) -> Option<Rect> {
        let selection = self.editor.selection().filter(|selection| selection.is_caret())?;
        self.layout.caret(&selection.head)
    }

    /// Where the head of the selection is, in window coordinates, for the input method's candidate window.
    pub fn caret_rect(&self) -> Option<Rect> {
        let head = &self.editor.selection()?.head;
        let rect = self.layout.caret(head)?;
        let column = self.column();
        Some(Rect { x: rect.x + column.x, y: rect.y + column.y, ..rect })
    }

    pub fn cursor(&self) -> Cursor {
        Cursor::Text
    }

    /// Advances what moves on its own: the caret's blink, and laying out the rest of the document while
    /// nothing else is happening. Returns when to call again, or `None` when nothing is left to move.
    pub fn animate(&mut self, now_ms: u64) -> Option<u64> {
        let blink = self.blink(now_ms);
        if !self.estimated {
            return blink;
        }
        // A frame that is owed comes before any work on what is off screen.
        if !self.needs_paint {
            self.settle();
        }
        Some(now_ms)
    }

    /// Lays out a little more of the document, holding still what the window shows.
    fn settle(&mut self) {
        let viewport = self.viewport();
        // Blocks above the viewport change height as they are laid out, so the top line is pinned.
        let pin = self.layout.hit(Point { x: 0.0, y: viewport.y.max(0.0) });
        let top = |layout: &DocLayout| pin.as_ref().and_then(|pos| layout.caret(pos)).map(|caret| caret.y);
        let before = top(&self.layout);
        self.estimated = self.layout.settle(viewport, SETTLE_BYTES);
        if let (Some(before), Some(after)) = (before, top(&self.layout)) {
            self.scroll = (self.scroll + after - before).clamp(0.0, self.max_scroll());
        }
    }

    fn blink(&mut self, now_ms: u64) -> Option<u64> {
        if !self.focused || self.editor.selection().is_none_or(|selection| !selection.is_caret()) {
            return None;
        }
        // Zero marks a caret that has just moved, so that its solid spell starts at the next frame.
        if self.caret_moved_ms == 0 {
            self.caret_moved_ms = now_ms.max(1);
        }
        let phase = now_ms.saturating_sub(self.caret_moved_ms) / BLINK_MS;
        let shown = phase.is_multiple_of(2);
        self.needs_paint |= shown != self.caret_shown;
        self.caret_shown = shown;
        Some(self.caret_moved_ms + (phase + 1) * BLINK_MS)
    }

    /// What the editor is lent: the shell's platform, and the laid-out lines to move between.
    fn lend<'a>(layout: &'a DocLayout, size: Size, platform: &'a mut dyn Platform) -> Lent<'a> {
        Lent { platform, lines: LaidOut { layout, page: (size.height - 2.0 * TOP).max(TOP) } }
    }

    /// Offers a key to the editor. `false` means it is the shell's to use.
    pub fn key(&mut self, event: KeyEvent, platform: &mut dyn Platform) -> bool {
        let Some(change) = self.editor.key(event, &mut Self::lend(&self.layout, self.size, platform)) else {
            return false;
        };
        self.changed(&change);
        true
    }

    pub fn input(&mut self, event: InputEvent, platform: &mut dyn Platform) {
        let change = self.editor.input(event, &mut Self::lend(&self.layout, self.size, platform));
        self.changed(&change);
    }

    pub fn run(&mut self, command: Command, platform: &mut dyn Platform) {
        let change = self.editor.run(command, &mut Self::lend(&self.layout, self.size, platform));
        self.changed(&change);
    }

    /// Brings the layout and the viewport up to date after the editor did something.
    fn changed(&mut self, change: &Change) {
        if !change.blocks.is_empty() || change.structure {
            self.edits += 1;
            self.layout.update(self.editor.doc(), &self.style, self.laid_width, Some(change));
            self.estimated = true;
        }
        self.reveal_caret();
    }

    /// Scrolls just far enough to show the head of the selection, and restarts the caret's blink.
    fn reveal_caret(&mut self) {
        self.caret_moved_ms = 0;
        self.caret_shown = true;
        self.needs_paint = true;
        let Some(head) = self.editor.selection().map(|selection| selection.head.clone()) else { return };
        let Some(caret) = self.layout.caret(&head) else { return };
        let (top, bottom) = (caret.y + TOP, caret.y + caret.height + TOP);
        let margin = GUTTER.min(self.size.height / 4.0);
        let visible_top = self.scroll + margin;
        let visible_bottom = self.scroll + self.size.height - margin;
        if top < visible_top {
            self.scroll = top - margin;
        } else if bottom > visible_bottom {
            self.scroll = bottom + margin - self.size.height;
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    fn scroll_by(&mut self, delta: f32) {
        let scroll = (self.scroll + delta).clamp(0.0, self.max_scroll());
        self.needs_paint |= scroll != self.scroll;
        self.scroll = scroll;
    }

    /// `delta` is in logical pixels. A positive `y` moves the content up.
    pub fn scroll(&mut self, _at: Point, delta: Point) {
        self.scroll_by(delta.y);
    }

    fn hit(&self, at: Point) -> Option<Pos> {
        let column = self.column();
        self.layout.hit(Point { x: at.x - column.x, y: at.y - column.y })
    }

    pub fn pointer_down(&mut self, at: Point, mods: Modifiers, platform: &mut dyn Platform) {
        let Some(head) = self.hit(at) else { return };
        let now_ms = platform.now_ms();
        let near = |was: Point| (was.x - at.x).abs() <= CLICK_SLOP && (was.y - at.y).abs() <= CLICK_SLOP;
        let clicks = match self.clicked {
            Some((then_ms, was, clicks)) if now_ms.saturating_sub(then_ms) <= CLICK_MS && near(was) => clicks % 3 + 1,
            _ => 1,
        };
        self.clicked = Some((now_ms, at, clicks));

        let anchor = match self.editor.selection() {
            Some(selection) if mods.shift && clicks == 1 => selection.anchor.clone(),
            _ => head.clone(),
        };
        self.editor.select(Selection { anchor, head });
        // A second click takes the word and a third the block. Only a single click starts a drag.
        self.dragging = clicks == 1;
        match clicks {
            2 => self.run(Command::SelectWord, platform),
            3 => self.run(Command::SelectBlock, platform),
            _ => self.reveal_caret(),
        }
    }

    pub fn pointer_move(&mut self, at: Point, _platform: &mut dyn Platform) {
        if !self.dragging {
            return;
        }
        let (Some(head), Some(selection)) = (self.hit(at), self.editor.selection()) else { return };
        if head != selection.head {
            self.editor.select(Selection { anchor: selection.anchor.clone(), head });
            self.reveal_caret();
        }
    }

    pub fn pointer_up(&mut self, _at: Point, _platform: &mut dyn Platform) {
        self.dragging = false;
    }

    pub fn pointer_leave(&mut self) {}

    pub fn access(&self) -> TreeUpdate {
        access::tree(&self.name, self.editor.doc(), self.editor.selection())
    }

    pub fn access_action(&mut self, request: ActionRequest, platform: &mut dyn Platform) {
        match access::request(self.editor.doc(), &request) {
            Some(Request::Select(selection)) => {
                self.editor.select(selection);
                self.reveal_caret();
            }
            Some(Request::Replace(text)) => self.input(InputEvent::Insert(text), platform),
            None => {}
        }
    }
}

/// The laid-out document, as the editor asks about its lines.
struct LaidOut<'a> {
    layout: &'a DocLayout,
    /// How far Page Up and Page Down move.
    page: f32,
}

impl Lines for LaidOut<'_> {
    fn caret_x(&self, pos: &Pos) -> Option<f32> {
        Some(self.layout.caret(pos)?.x)
    }

    fn line_step(&self, pos: &Pos, x: f32, down: bool) -> Option<Pos> {
        self.layout.line_step(pos, x, down)
    }

    fn line_edge(&self, pos: &Pos, end: bool) -> Option<Pos> {
        let caret = self.layout.caret(pos)?;
        // Far enough to either side to be past any line.
        let x = if end { 1.0e6 } else { -1.0e6 };
        self.layout.hit(Point { x, y: caret.y + caret.height / 2.0 })
    }

    fn page_lines(&self) -> usize {
        // Every line is taken to be as tall as the first: near enough for a page's worth of movement.
        let line = self.layout.hit(Point::default()).and_then(|pos| self.layout.caret(&pos)).map(|caret| caret.height);
        line.map_or(20, |line| (self.page / line.max(1.0)) as usize).max(1)
    }
}

/// The shell's platform with the document's lines added.
struct Lent<'a> {
    platform: &'a mut dyn Platform,
    lines: LaidOut<'a>,
}

impl Platform for Lent<'_> {
    fn now_ms(&self) -> u64 {
        self.platform.now_ms()
    }

    fn clipboard_read(&mut self) -> Option<Clip> {
        self.platform.clipboard_read()
    }

    fn clipboard_write(&mut self, clip: Clip) {
        self.platform.clipboard_write(clip)
    }

    fn new_id(&mut self) -> String {
        self.platform.new_id()
    }

    fn lines(&self) -> Option<&dyn Lines> {
        Some(&self.lines)
    }
}
