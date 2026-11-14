//! What a window shows: a document, its caret and its selection.
//!
//! This is a stand-in for `letters_ui::Window` and keeps to its method names, so that taking the real window
//! is a change of type. It drives the editor and the layout directly and has no chrome.

use accesskit::{ActionRequest, TreeUpdate};
use letters_editor::{Editor, InputEvent, Key, KeyEvent, Modifiers, Platform};
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
    /// The horizontal position vertical movement keeps to.
    goal_x: Option<f32>,
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
            goal_x: None,
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

    /// Offers a key to the editor, then to the view. `false` means neither used it.
    pub fn key(&mut self, event: KeyEvent, platform: &mut dyn Platform) -> bool {
        if let Some(change) = self.editor.key(event.clone(), platform) {
            self.goal_x = None;
            self.changed(&change);
            return true;
        }
        match event.key {
            Key::Up | Key::Down if !event.mods.primary && !event.mods.alt => {
                self.step_line(event.key == Key::Down, event.mods.shift)
            }
            Key::PageUp | Key::PageDown => {
                let page = (self.size.height - 2.0 * TOP).max(TOP);
                self.scroll_by(if event.key == Key::PageDown { page } else { -page });
                true
            }
            _ => false,
        }
    }

    pub fn input(&mut self, event: InputEvent, platform: &mut dyn Platform) {
        let change = self.editor.input(event, platform);
        self.goal_x = None;
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

    /// Moves the head of the selection one visual line, which only the layout can do.
    fn step_line(&mut self, down: bool, extend: bool) -> bool {
        let Some(selection) = self.editor.selection().cloned() else { return false };
        let Some(caret) = self.layout.caret(&selection.head) else { return false };
        let x = *self.goal_x.get_or_insert(caret.x);
        if let Some(head) = self.layout.line_step(&selection.head, x, down) {
            let anchor = if extend { selection.anchor } else { head.clone() };
            self.editor.select(Selection { anchor, head });
            self.reveal_caret();
        }
        true
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

    pub fn pointer_down(&mut self, at: Point, mods: Modifiers, _platform: &mut dyn Platform) {
        let Some(head) = self.hit(at) else { return };
        let anchor = match self.editor.selection() {
            Some(selection) if mods.shift => selection.anchor.clone(),
            _ => head.clone(),
        };
        self.editor.select(Selection { anchor, head });
        self.dragging = true;
        self.goal_x = None;
        self.reveal_caret();
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
                self.goal_x = None;
                self.reveal_caret();
            }
            Some(Request::Replace(text)) => self.input(InputEvent::Insert(text), platform),
            None => {}
        }
    }
}
