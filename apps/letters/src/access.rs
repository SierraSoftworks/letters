//! A document as an AccessKit tree, and AccessKit's requests as edits.
//!
//! The document node carries the selection. Each block is a node of its role, and a text block holds one text
//! run whose characters are Unicode scalar values.

use accesskit::{
    Action, ActionData, ActionRequest, Node, NodeId, Role, TextPosition, TextSelection, Toggled, TreeId, TreeInfo,
    TreeUpdate,
};
use letters_model::{Block, BlockId, BlockKind, Body, Document, ListKind, Pos, Selection};

pub const WINDOW: NodeId = NodeId(1);
pub const DOCUMENT: NodeId = NodeId(2);

/// A block's node. Its text run, if it has one, is the next id up.
fn block_node(id: &BlockId) -> NodeId {
    // FNV-1a: stable between runs, so a screen reader keeps its place across a restart.
    let hash = id
        .as_str()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3));
    NodeId(hash & !1)
}

fn run_node(id: &BlockId) -> NodeId {
    NodeId(block_node(id).0 | 1)
}

/// The whole tree: a window holding the document.
pub fn tree(title: &str, doc: &Document, selection: Option<&Selection>) -> TreeUpdate {
    let mut nodes = Vec::new();

    let mut window = Node::new(Role::Window);
    window.set_label(title);
    window.set_children([DOCUMENT]);
    nodes.push((WINDOW, window));

    let mut document = Node::new(Role::Document);
    document.set_label(title);
    document.set_language(doc.language.as_str());
    document.set_children(doc.blocks.iter().map(|block| block_node(&block.id)).collect::<Vec<_>>());
    for action in [Action::Focus, Action::SetTextSelection, Action::ReplaceSelectedText] {
        document.add_action(action);
    }
    let position = |pos: &Pos| {
        let text = doc.block(&pos.block)?.plain_text();
        Some(TextPosition { node: run_node(&pos.block), character_index: text.get(..pos.offset)?.chars().count() })
    };
    if let Some(selection) = selection
        && let (Some(anchor), Some(focus)) = (position(&selection.anchor), position(&selection.head))
    {
        document.set_text_selection(TextSelection { anchor, focus });
    }
    nodes.push((DOCUMENT, document));

    for block in &doc.blocks {
        push_block(block, &mut nodes);
    }
    TreeUpdate { nodes, tree: Some(TreeInfo::new(WINDOW)), tree_id: TreeId::ROOT, focus: DOCUMENT }
}

fn push_block(block: &Block, nodes: &mut Vec<(NodeId, Node)>) {
    let mut node = Node::new(match &block.kind {
        BlockKind::Paragraph => Role::Paragraph,
        BlockKind::Heading { .. } => Role::Heading,
        BlockKind::ListItem { .. } => Role::ListItem,
        BlockKind::Code { .. } => Role::Code,
        BlockKind::Quote => Role::Blockquote,
        BlockKind::Callout { .. } => Role::Note,
        BlockKind::Divider => Role::Splitter,
        BlockKind::Unknown { .. } => Role::Unknown,
    });
    match &block.kind {
        BlockKind::Heading { level } => node.set_level(usize::from(*level)),
        BlockKind::ListItem { list, depth } => {
            node.set_level(usize::from(*depth) + 1);
            if let ListKind::Task { checked } = list {
                node.set_toggled(if *checked { Toggled::True } else { Toggled::False });
            }
        }
        _ => {}
    }
    match &block.body {
        Body::Text(_) => {
            // The line break that ends the block counts as its last character.
            let mut text = block.plain_text();
            text.push('\n');
            let mut run = Node::new(Role::TextRun);
            run.set_character_lengths(text.chars().map(|c| c.len_utf8() as u8).collect::<Vec<_>>());
            run.set_word_starts(word_starts(&text));
            run.set_value(text);
            node.set_children([run_node(&block.id)]);
            nodes.push((run_node(&block.id), run));
        }
        Body::Blocks(children) => {
            node.set_children(children.iter().map(|child| block_node(&child.id)).collect::<Vec<_>>());
            for child in children {
                push_block(child, nodes);
            }
        }
        Body::None => {}
    }
    nodes.push((block_node(&block.id), node));
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Select(Selection),
    /// Replaces the selection with text.
    Replace(String),
}

pub fn request(doc: &Document, request: &ActionRequest) -> Option<Request> {
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
        _ => None,
    }
}
