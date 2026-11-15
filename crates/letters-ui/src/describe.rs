//! The document as accessibility nodes under the page, and assistive technology's requests as edits.
//!
//! Each block is a node of its role, and a text block holds one text run whose characters are Unicode scalar
//! values. List items that follow one another stand under a list, a task's tick is a checkbox, a figure's
//! picture an image named by its alt text with a button that turns to that text, and a code block's copy
//! button a button. The page's own node carries the selection, and the markers of the notes in its margin
//! are buttons under it.
//!
//! A long document is not told again for every key: [`Told`] remembers how each block read, and a block that
//! still reads the same is left out when only the changes are asked for.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem;

use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Role, TextPosition, TextSelection, Toggled};
use letters_model::{Block, BlockId, BlockKind, Body, Document, ListKind, Pos, Selection};
use letters_render::Rect;

/// The parts of a block that have nodes of their own, beside its text.
const LIST: u8 = 1;
const TICK: u8 = 2;
const PICTURE: u8 = 3;
const COPY: u8 = 4;
const ALT: u8 = 5;
const NOTE: u8 = 6;
const BREAK: u8 = 7;
const KEEP: u8 = 8;
const USE: u8 = 9;
const HELD: u8 = 10;
/// The handle of a cell's column and of its row, and the pluses that add a column and a row after it and
/// before it.
const TABLE: [u8; 6] = [11, 12, 13, 14, 16, 17];
const CALLOUT: u8 = 15;
const LANGUAGE: u8 = 18;

/// FNV-1a: stable between runs, so a screen reader keeps its place across a restart.
fn hashed(bytes: impl Iterator<Item = u8>) -> u64 {
    bytes.fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3))
}

/// A block's node. Its text run, if it has one, is the next id up.
fn block_node(id: &BlockId) -> NodeId {
    NodeId(hashed(id.as_str().bytes()) & !1)
}

fn run_node(id: &BlockId) -> NodeId {
    NodeId(block_node(id).0 | 1)
}

/// The node of a part of a block: the list it begins, its tick, its picture or its copy button.
fn part_node(id: &BlockId, part: u8) -> NodeId {
    // No id holds a byte of 0xff, so no block's own node is this one.
    NodeId(hashed(id.as_str().bytes().chain([0xff, part])) & !1)
}

/// How each block read when assistive technology was last told of it.
#[derive(Default)]
pub(crate) struct Told(HashMap<NodeId, u64>);

impl Told {
    /// Has every block told again the next time: the nodes that stood for them are gone.
    pub fn forget(&mut self) {
        self.0.clear();
    }
}

/// One telling of the document: what is told, and what it is told against.
struct Telling<'a> {
    /// How the blocks read before, when only those that read differently are wanted.
    before: Option<&'a HashMap<NodeId, u64>>,
    now: HashMap<NodeId, u64>,
    nodes: &'a mut Vec<(NodeId, Node)>,
    /// Whether the document is written on. One that is only read has nothing to press.
    written: bool,
    /// What is said of a block beside what it reads, as of one that a version of the document added.
    said: Option<&'a HashMap<BlockId, String>>,
}

impl Telling<'_> {
    /// Notes how a node reads now. Returns whether it has to be told: it is new, or read otherwise before.
    fn differs(&mut self, id: NodeId, reads: u64) -> bool {
        self.now.insert(id, reads);
        self.before.is_none_or(|before| before.get(&id) != Some(&reads))
    }
}

/// Fills in the page's node and adds the blocks under it: all of them when `whole` is set, and otherwise
/// those that read differently than `told` has them.
pub(crate) fn page(
    doc: &Document,
    selection: Option<&Selection>,
    told: &mut Told,
    whole: bool,
    page: &mut Node,
    nodes: &mut Vec<(NodeId, Node)>,
) {
    page.set_language(doc.language.as_str());
    for action in [Action::SetTextSelection, Action::ReplaceSelectedText] {
        page.add_action(action);
    }
    let position = |pos: &Pos| {
        let text = doc.block(&pos.block)?.plain_text();
        Some(TextPosition { node: run_node(&pos.block), character_index: text.get(..pos.offset)?.chars().count() })
    };
    if let Some(selection) = selection
        && let (Some(anchor), Some(focus)) = (position(&selection.anchor), position(&selection.head))
    {
        page.set_text_selection(TextSelection { anchor, focus });
    }
    tell(doc, None, told, whole, page, nodes);
}

/// Adds the blocks of a version of the document under the canvas that shows it, as [`page`] does for
/// the document itself. A version is read and not written on: nothing in it takes a selection or a press.
/// `said` holds what is told of a block beside what it reads: how it differs from another version.
pub(crate) fn version(
    doc: &Document,
    said: &HashMap<BlockId, String>,
    told: &mut Told,
    whole: bool,
    canvas: &mut Node,
    nodes: &mut Vec<(NodeId, Node)>,
) {
    canvas.set_language(doc.language.as_str());
    tell(doc, Some(said), told, whole, canvas, nodes);
}

/// Adds the blocks of `doc` under `page`. With something `said` of its blocks the document is a version
/// that is only read, and otherwise the one that is written on.
fn tell(
    doc: &Document,
    said: Option<&HashMap<BlockId, String>>,
    told: &mut Told,
    whole: bool,
    page: &mut Node,
    nodes: &mut Vec<(NodeId, Node)>,
) {
    let before = mem::take(&mut told.0);
    let now = HashMap::with_capacity(before.len());
    let mut telling = Telling { before: (!whole).then_some(&before), now, nodes, written: said.is_none(), said };
    page.set_children(push_blocks(&doc.blocks, &mut telling));
    told.0 = telling.now;
}

/// Adds the preview of a figure under the page: a tooltip that reads the figure's caption. `rect` is where
/// it shows in the page area.
pub(crate) fn preview(caption: &str, rect: Rect, page: &mut Node, nodes: &mut Vec<(NodeId, Node)>) {
    let mut tip = Node::new(Role::Tooltip);
    tip.set_label(caption);
    tip.set_bounds(placed(page, rect));
    // No block's id holds a byte of 0xff, so no node of the document is this one.
    let id = NodeId(hashed([0xff, 0xff].into_iter()) & !1);
    page.push_child(id);
    nodes.push((id, tip));
}

/// Adds the marker beside a block under the page: a button named by the notes it opens, which a click
/// presses. `rect` is where it stands in the page area.
pub(crate) fn marker(block: &BlockId, label: &str, rect: Rect, page: &mut Node, nodes: &mut Vec<(NodeId, Node)>) {
    let mut marker = Node::new(Role::Button);
    marker.set_label(label);
    marker.add_action(Action::Click);
    marker.set_bounds(placed(page, rect));
    let id = part_node(block, NOTE);
    page.push_child(id);
    nodes.push((id, marker));
}

/// Adds a handle or a bar of the table that shows them under the page: a button, which a click presses.
/// It is told by `cell`, a cell of the column or row it acts on. `row` is whether it acts on a row, and
/// `does` what a press does: 0 opens its menu, 1 adds one at the end and 2 adds one before it. `rect` is
/// where it stands in the page area.
pub(crate) fn table_part(
    cell: &BlockId,
    (row, does): (bool, u8),
    label: &str,
    rect: Rect,
    page: &mut Node,
    nodes: &mut Vec<(NodeId, Node)>,
) {
    let mut button = Node::new(Role::Button);
    button.set_label(label);
    button.add_action(Action::Click);
    button.set_bounds(placed(page, rect));
    let id = part_node(cell, TABLE[usize::from(row) + 2 * usize::from(does)]);
    page.push_child(id);
    nodes.push((id, button));
}

/// Adds the button that chooses one version of a block two writers changed apart: the one on the page
/// with `keep`, and else the one held back. `place` says whose it is and where it stands.
pub(crate) fn choice(
    block: &BlockId,
    keep: bool,
    label: &str,
    place: &str,
    rect: Rect,
    page: &mut Node,
    nodes: &mut Vec<(NodeId, Node)>,
) {
    let mut button = Node::new(Role::Button);
    button.set_label(label);
    button.set_description(place);
    button.add_action(Action::Click);
    button.set_bounds(placed(page, rect));
    let id = part_node(block, if keep { KEEP } else { USE });
    page.push_child(id);
    nodes.push((id, button));
}

/// Adds the version of a block that a conflict holds back, which shows beside the page's own: what it
/// reads, under a name that says whose it is.
pub(crate) fn held_back(
    block: &BlockId,
    label: &str,
    text: &str,
    rect: Rect,
    page: &mut Node,
    nodes: &mut Vec<(NodeId, Node)>,
) {
    let mut held = Node::new(Role::Note);
    held.set_label(label);
    held.set_value(text);
    held.set_bounds(placed(page, rect));
    let id = part_node(block, HELD);
    page.push_child(id);
    nodes.push((id, held));
}

/// Adds what Print view shows of a page break under its canvas: a button that takes the break before a
/// block away, or one that offers to make it. `place` says which sheet it is on, and `rect` where it shows
/// in the canvas.
pub(crate) fn page_break(
    block: &BlockId,
    label: &str,
    place: &str,
    rect: Rect,
    canvas: &mut Node,
    nodes: &mut Vec<(NodeId, Node)>,
) {
    let mut button = Node::new(Role::Button);
    button.set_label(label);
    button.set_description(place);
    button.add_action(Action::Click);
    button.set_bounds(placed(canvas, rect));
    let id = part_node(block, BREAK);
    canvas.push_child(id);
    nodes.push((id, button));
}

/// Adds a sheet of Print view under its canvas: a picture of the page, named by its number. `rect` is where
/// it shows in the canvas.
pub(crate) fn sheet(index: usize, label: &str, rect: Rect, canvas: &mut Node, nodes: &mut Vec<(NodeId, Node)>) {
    let mut sheet = Node::new(Role::Image);
    sheet.set_label(label);
    sheet.set_bounds(placed(canvas, rect));
    // No block's id holds a byte of 0xff, so no node of the document is this one.
    let id = NodeId(hashed([0xff, 0xfe].into_iter().chain(index.to_le_bytes())) & !1);
    canvas.push_child(id);
    nodes.push((id, sheet));
}

/// Where something at `rect` in the page area is in the window.
fn placed(page: &Node, rect: Rect) -> accesskit::Rect {
    let (x, y) = page.bounds().map_or((0.0, 0.0), |page| (page.x0, page.y0));
    let (x, y) = (x + f64::from(rect.x), y + f64::from(rect.y));
    accesskit::Rect { x0: x, y0: y, x1: x + f64::from(rect.width), y1: y + f64::from(rect.height) }
}

/// A list item's depth and which kind of list it is in. A task belongs with its list ticked or not.
fn item(block: &Block) -> Option<(u8, mem::Discriminant<ListKind>)> {
    match &block.kind {
        BlockKind::ListItem { list, depth } => Some((*depth, mem::discriminant(list))),
        _ => None,
    }
}

/// Adds blocks that stand side by side, and returns the nodes that stand for them under their parent: each
/// block's own, but for list items, which stand under a list.
fn push_blocks(blocks: &[Block], telling: &mut Telling) -> Vec<NodeId> {
    let mut children = Vec::new();
    let mut at = 0;
    while let Some(block) = blocks.get(at) {
        if item(block).is_some() {
            children.push(push_list(blocks, &mut at, telling));
        } else {
            children.push(push_block(block, Vec::new(), telling));
            at += 1;
        }
    }
    children
}

/// Adds the list that the item at `at` begins, and moves `at` past it. The list holds the items of the
/// first one's depth and kind that follow it, and each item holds the deeper lists that follow it.
fn push_list(blocks: &[Block], at: &mut usize, telling: &mut Telling) -> NodeId {
    let first = &blocks[*at];
    let (kind, depth) = (item(first), item(first).map_or(0, |item| item.0));
    let mut items = Vec::new();
    while let Some(block) = blocks.get(*at).filter(|block| item(block) == kind) {
        *at += 1;
        let mut inner = Vec::new();
        while blocks.get(*at).and_then(item).is_some_and(|item| item.0 > depth) {
            inner.push(push_list(blocks, at, telling));
        }
        items.push(push_block(block, inner, telling));
    }
    let id = part_node(&first.id, LIST);
    let mut reads = DefaultHasher::new();
    items.hash(&mut reads);
    if telling.differs(id, reads.finish()) {
        let mut list = Node::new(Role::List);
        list.set_children(items);
        telling.nodes.push((id, list));
    }
    id
}

/// Adds a block and what is under it, with the lists `inner` inside it, and returns its node.
fn push_block(block: &Block, inner: Vec<NodeId>, telling: &mut Telling) -> NodeId {
    let id = block_node(&block.id);
    // What is under the block is gone through whether or not the block itself reads as before.
    let under = push_blocks(block.body.blocks(), telling);
    let mut reads = DefaultHasher::new();
    mem::discriminant(&block.kind).hash(&mut reads);
    match &block.kind {
        BlockKind::Heading { level } => level.hash(&mut reads),
        BlockKind::ListItem { list, depth } => {
            (depth, matches!(list, ListKind::Task { checked: true })).hash(&mut reads)
        }
        BlockKind::Figure { alt, .. } => alt.hash(&mut reads),
        BlockKind::Callout { .. } => block.kind.callout_look().map(|look| look.title.to_owned()).hash(&mut reads),
        BlockKind::Code { language } => language.hash(&mut reads),
        _ => {}
    }
    block.body.runs().iter().for_each(|run| run.text.hash(&mut reads));
    (&under, &inner).hash(&mut reads);
    let said = telling.said.and_then(|said| said.get(&block.id));
    said.hash(&mut reads);
    if !telling.differs(id, reads.finish()) {
        return id;
    }

    let mut node = Node::new(match &block.kind {
        BlockKind::Paragraph => Role::Paragraph,
        BlockKind::Heading { .. } => Role::Heading,
        BlockKind::ListItem { .. } => Role::ListItem,
        BlockKind::Code { .. } => Role::Code,
        BlockKind::Formula => Role::Math,
        BlockKind::Quote => Role::Blockquote,
        BlockKind::Callout { .. } => Role::Note,
        BlockKind::Divider => Role::Splitter,
        BlockKind::Caption => Role::Caption,
        BlockKind::Figure { .. } => Role::Figure,
        BlockKind::Gallery { .. } => Role::Group,
        BlockKind::Table { .. } => Role::Table,
        BlockKind::TableRow => Role::Row,
        BlockKind::TableCell => Role::Cell,
        BlockKind::TableOfContents { .. } => Role::DocToc,
        BlockKind::LinkPreview { .. } => Role::Link,
        BlockKind::Unknown { .. } => Role::Unknown,
    });
    if let Some(said) = said {
        node.set_description(said.as_str());
    }
    let mut children = Vec::new();
    let written = telling.written;
    let mut part = |part: u8, node: Node, children: &mut Vec<NodeId>| {
        telling.nodes.push((part_node(&block.id, part), node));
        children.push(part_node(&block.id, part));
    };
    match &block.kind {
        BlockKind::Heading { level } => node.set_level(usize::from(*level)),
        BlockKind::ListItem { list, depth } => {
            node.set_level(usize::from(*depth) + 1);
            if let ListKind::Task { checked } = list {
                // The tick is a checkbox of its own, which a click ticks.
                let mut tick = Node::new(Role::CheckBox);
                tick.set_toggled(if *checked { Toggled::True } else { Toggled::False });
                if written {
                    tick.add_action(Action::Click);
                }
                part(TICK, tick, &mut children);
            }
        }
        BlockKind::Callout { .. } => {
            // A callout is named by its title, and its icon and title are a button that opens its card.
            let title = block.kind.callout_look().map(|look| look.title).unwrap_or_default();
            if !title.is_empty() {
                node.set_label(title);
            }
            if written {
                let mut edit = Node::new(Role::Button);
                edit.set_label(crate::callouts::LABEL);
                edit.add_action(Action::Click);
                part(CALLOUT, edit, &mut children);
            }
        }
        BlockKind::Code { language } if written => {
            // The language named in the corner of the box, which a click lists the others for, and the
            // button beside it, which a click presses.
            let mut named = Node::new(Role::Button);
            named.set_label(crate::languages::label(language.as_deref()));
            named.add_action(Action::Click);
            part(LANGUAGE, named, &mut children);
            let mut copy = Node::new(Role::Button);
            copy.set_label(crate::code_copy::LABEL);
            copy.add_action(Action::Click);
            part(COPY, copy, &mut children);
        }
        BlockKind::Figure { alt, .. } => {
            // The picture comes before its caption, named by what it shows.
            let mut picture = Node::new(Role::Image);
            picture.set_label(alt.as_str());
            part(PICTURE, picture, &mut children);
            if written {
                // The switch under it, which a click turns to the alt text.
                let mut switch = Node::new(Role::Button);
                switch.set_label(crate::captions::LABEL);
                switch.add_action(Action::Click);
                part(ALT, switch, &mut children);
            }
        }
        _ => {}
    }
    if let Body::Text(_) = &block.body {
        // The line break that ends the block counts as its last character.
        let mut text = block.plain_text();
        text.push('\n');
        let mut run = Node::new(Role::TextRun);
        run.set_character_lengths(text.chars().map(|c| c.len_utf8() as u8).collect::<Vec<_>>());
        run.set_word_starts(word_starts(&text));
        run.set_value(text);
        children.push(run_node(&block.id));
        telling.nodes.push((run_node(&block.id), run));
    }
    children.extend(under);
    children.extend(inner);
    node.set_children(children);
    telling.nodes.push((id, node));
    id
}

/// The character index at which each word starts. Whitespace belongs to the word before it.
fn word_starts(text: &str) -> Vec<u8> {
    let mut starts = Vec::new();
    let mut after_space = true;
    for (index, c) in text.chars().enumerate() {
        // A run can only describe its first 256 characters this way. Longer blocks lose word steps past that.
        let Ok(index) = u8::try_from(index) else { break };
        if after_space && !c.is_whitespace() {
            starts.push(index);
        }
        after_space = c.is_whitespace();
    }
    starts
}

/// What a request from assistive technology asks of the editor.
pub(crate) enum Request {
    Select(Selection),
    /// Replaces the selection with text.
    Replace(String),
    /// Ticks or unticks a task.
    Tick(BlockId),
    /// Copies a code block.
    Copy(BlockId),
    /// Presses the language of a code block, which lists the languages it can be in.
    Language(BlockId),
    /// Presses a callout's icon and title, which opens its card.
    Callout(BlockId),
    /// Turns the line under a figure's picture to its alt text.
    Alt(BlockId),
    /// Presses the marker of the notes beside a block.
    Note(BlockId),
    /// Presses the handle of a cell's column or of its row, the plus that adds one at the end or the plus
    /// that adds one before it: `does` is 0, 1 or 2 in that order.
    Table {
        cell: BlockId,
        row: bool,
        does: u8,
    },
    /// Presses what Print view shows of the page break before a block.
    Break(BlockId),
    /// Chooses a version of a block two writers changed apart: the one on the page, or the one held back.
    Choose {
        block: BlockId,
        keep: bool,
    },
}

pub(crate) fn request(doc: &Document, request: &ActionRequest) -> Option<Request> {
    let pos = |position: &TextPosition| {
        let block = doc.walk().find(|block| run_node(&block.id) == position.node)?;
        let text = block.plain_text();
        let offset = text.char_indices().nth(position.character_index).map_or(text.len(), |(offset, _)| offset);
        Some(Pos { block: block.id.clone(), offset })
    };
    match (&request.action, &request.data) {
        (Action::SetTextSelection, Some(ActionData::SetTextSelection(selection))) => {
            Some(Request::Select(Selection { anchor: pos(&selection.anchor)?, head: pos(&selection.focus)? }))
        }
        (Action::ReplaceSelectedText, Some(ActionData::Value(text))) => Some(Request::Replace(text.to_string())),
        (Action::Click, _) => {
            let part = |part| doc.walk().find(|block| part_node(&block.id, part) == request.target_node);
            let ticked = part(TICK).map(|block| Request::Tick(block.id.clone()));
            let copied = || part(COPY).map(|block| Request::Copy(block.id.clone()));
            let languaged = || part(LANGUAGE).map(|block| Request::Language(block.id.clone()));
            let switched = || part(ALT).map(|block| Request::Alt(block.id.clone()));
            let called = || part(CALLOUT).map(|block| Request::Callout(block.id.clone()));
            let noted = || part(NOTE).map(|block| Request::Note(block.id.clone()));
            let broken = || part(BREAK).map(|block| Request::Break(block.id.clone()));
            let chosen = |keep| part(if keep { KEEP } else { USE }).map(|block| (block.id.clone(), keep));
            let chosen = || chosen(true).or_else(|| chosen(false)).map(|(block, keep)| Request::Choose { block, keep });
            let tabled = || {
                let found = |at: usize| Some((part(TABLE[at])?.id.clone(), at));
                (0..TABLE.len()).find_map(found).map(|(cell, at)| Request::Table {
                    cell,
                    row: at % 2 == 1,
                    does: (at / 2) as u8,
                })
            };
            let found = ticked.or_else(copied).or_else(switched).or_else(called).or_else(noted).or_else(broken);
            found.or_else(chosen).or_else(tabled).or_else(languaged)
        }
        _ => None,
    }
}
