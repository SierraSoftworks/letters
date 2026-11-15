//! Builds the accessibility tree from the laid-out elements.
//!
//! The tree a screen reader walks is the tree tests find controls in, so a control that is missing here is
//! missing for both. Every element with a role becomes a node, and plain text becomes a label unless it
//! already names its button or menu item.

use std::collections::HashMap;

use accesskit::{Action, Affine, Node as AccessNode, NodeId, Role, TextDirection, TextPosition, TextSelection};
use accesskit::{Toggled, TreeId, TreeInfo, TreeUpdate};
use letters_render::Rect;

use crate::element::{Content, Focus};
use crate::input::InputState;
use crate::text::Shaped;
use crate::tree::{Id, Tree};

/// AccessKit indexes a run's words with a byte, so long values are split into runs of this many characters.
const RUN: usize = 200;

/// Nodes by id, in the order AccessKit takes them.
pub(crate) type Nodes = Vec<(NodeId, AccessNode)>;

pub(crate) struct Window<'a> {
    pub title: &'a str,
    pub rect: Rect,
    pub scale: f32,
    pub focus: Option<Id>,
    pub inputs: &'a HashMap<Id, InputState>,
    /// The tooltip that is showing, and where.
    pub tip: Option<(&'a str, Rect)>,
    /// Lets the app fill in a canvas's node and add the nodes under it.
    pub describe: &'a dyn Fn(&mut AccessNode, &mut Nodes),
}

/// The key of the tooltip's node under the window.
const TIP: u64 = 0x0074_6970;

pub(crate) fn node_id(id: Id) -> NodeId {
    NodeId(id.0)
}

/// The id of the `n`th text run of an input.
fn run_id(input: Id, n: usize) -> NodeId {
    node_id(input.child(0x0072_756E).child(n as u64))
}

pub(crate) fn build<M>(tree: &Tree<M>, window: &Window<'_>) -> TreeUpdate {
    let mut nodes = Vec::with_capacity(tree.nodes.len());
    let mut children = Vec::new();
    for layer in &tree.layers {
        collect(tree, window, layer.root, false, &mut children, &mut nodes);
    }

    if let Some((text, rect)) = window.tip {
        let mut tip = AccessNode::new(Role::Tooltip);
        tip.set_label(text);
        tip.set_bounds(bounds(rect));
        let id = node_id(Id::WINDOW.child(TIP));
        nodes.push((id, tip));
        children.push(id);
    }

    let mut root = AccessNode::new(Role::Window);
    root.set_label(window.title);
    root.set_bounds(bounds(window.rect));
    // Elements are measured in logical pixels, and platforms expect physical ones.
    if window.scale != 1.0 {
        root.set_transform(Affine::scale(f64::from(window.scale)));
    }
    root.set_children(children);
    let root_id = node_id(Id::WINDOW);
    nodes.push((root_id, root));

    // Focus is reported on the nearest element that is in this tree.
    let exposed = |i: &usize| role_of(tree, *i, false).is_some() && !hidden(tree, *i);
    let focus = window.focus.and_then(|id| tree.find(id)).and_then(|i| tree.chain(i).find(exposed));
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(root_id)),
        tree_id: TreeId::ROOT,
        focus: focus.map_or(root_id, |i| node_id(tree.nodes[i].id)),
    }
}

fn hidden<M>(tree: &Tree<M>, i: usize) -> bool {
    tree.ancestors(i).any(|at| tree.nodes[at].el.access.hidden)
}

/// The role a node is exposed with. Text is a label, unless an ancestor already takes its name from it.
fn role_of<M>(tree: &Tree<M>, i: usize, named: bool) -> Option<Role> {
    let el = &tree.nodes[i].el;
    el.access.role.or(match &el.content {
        Content::Text { .. } if !named => Some(Role::Label),
        Content::Input(_) => Some(Role::TextInput),
        _ => None,
    })
}

/// Whether a role's name is the text inside it, as for a button.
fn named_by_content(role: Role) -> bool {
    use Role::*;
    matches!(
        role,
        Button | DefaultButton | Link | MenuItem | MenuItemCheckBox | MenuItemRadio | Tab | ListBoxOption | TreeItem
    )
}

fn collect<M>(tree: &Tree<M>, window: &Window<'_>, i: usize, named: bool, siblings: &mut Vec<NodeId>, out: &mut Nodes) {
    let n = &tree.nodes[i];
    if n.el.access.hidden {
        return;
    }
    let Some(role) = role_of(tree, i, named) else {
        for child in tree.children(i) {
            collect(tree, window, child, named, siblings, out);
        }
        return;
    };

    let from_content = named_by_content(role);
    let mut children = Vec::new();
    for child in tree.children(i) {
        collect(tree, window, child, named || from_content, &mut children, out);
    }

    let access = &n.el.access;
    let mut node = AccessNode::new(role);
    node.set_bounds(bounds(n.rect));
    match (&access.label, &n.el.content) {
        // A label's text is its value, also when it shows as an icon and its text was given in words.
        (Some(label), _) if role == Role::Label => node.set_value(label.as_str()),
        (Some(label), _) => node.set_label(label.as_str()),
        (None, Content::Text { text, .. }) => node.set_value(text.as_str()),
        (None, _) if from_content => node.set_label(text_of(tree, i)),
        _ => {}
    }
    if let Some(description) = &access.description {
        node.set_description(description.as_str());
    }
    if tree.disabled(i) {
        node.set_disabled();
    }
    if let Some(on) = access.pressed.or(access.checked) {
        node.set_toggled(if on { Toggled::True } else { Toggled::False });
    }
    if let Some(selected) = access.selected {
        node.set_selected(selected);
    }
    if let Some(expanded) = access.expanded {
        node.set_expanded(expanded);
    }
    if access.modal {
        node.set_modal();
    }
    if let Some(trigger) = n.trigger {
        node.set_has_popup(trigger.popup);
        node.set_expanded(trigger.open);
    }
    if n.el.on_press.is_some() || n.trigger.is_some() {
        node.add_action(Action::Click);
    }
    if n.el.focus != Focus::No {
        node.add_action(Action::Focus);
    }
    if n.el.style.scroll {
        node.set_clips_children();
    }
    if let (Content::Input(input), Some(shaped)) = (&n.el.content, &n.shaped) {
        let state = window.inputs.get(&n.id).cloned().unwrap_or_default();
        let shown = state.shown(&input.value);
        node.set_value(shown.as_str());
        if !input.placeholder.is_empty() {
            node.set_placeholder(input.placeholder.as_str());
        }
        if input.on_input.is_some() {
            node.add_action(Action::SetValue);
            node.add_action(Action::SetTextSelection);
            node.add_action(Action::ReplaceSelectedText);
        } else {
            node.set_read_only();
        }
        let runs = text_runs(n.id, n.rect, &shown, shaped, out);
        let position = |offset: usize| {
            let character = shaped.stops.iter().filter(|stop| stop.offset < offset).count();
            let run = (character / RUN).min(runs.len() - 1);
            TextPosition { node: runs[run], character_index: character - run * RUN }
        };
        let (anchor, head) = if state.composing.is_some() {
            (state.caret(&input.value), state.caret(&input.value))
        } else {
            (state.anchor.min(shown.len()), state.head.min(shown.len()))
        };
        node.set_text_selection(TextSelection { anchor: position(anchor), focus: position(head) });
        children.extend(runs);
    }
    node.set_children(children);
    if let Content::Canvas(canvas) = &n.el.content {
        match &canvas.field {
            // A field says what is written in it.
            Some(value) => node.set_value(value.as_str()),
            // The app describes the canvas that takes its input. A passive one has only its own name.
            None if n.el.focus != Focus::No => (window.describe)(&mut node, out),
            None => {}
        }
    }

    let id = node_id(n.id);
    out.push((id, node));
    siblings.push(id);
}

/// The text runs of an input: what a screen reader needs to read by character and word, and to follow the
/// caret.
fn text_runs(input: Id, rect: Rect, text: &str, shaped: &Shaped, out: &mut Nodes) -> Vec<NodeId> {
    // Each caret stop starts one character, as AccessKit means it.
    let stops = &shaped.stops;
    let characters = stops.len().saturating_sub(1);
    let mut ids = Vec::new();
    let mut first = 0;
    loop {
        let last = (first + RUN).min(characters);
        let range = stops[first].offset..stops.get(last).map_or(text.len(), |stop| stop.offset);
        let chunk = &stops[first..=last.min(stops.len() - 1)];
        let mut node = AccessNode::new(Role::TextRun);
        node.set_bounds(bounds(rect));
        node.set_value(&text[range.clone()]);
        node.set_text_direction(TextDirection::LeftToRight);
        node.set_character_lengths(
            chunk.windows(2).map(|pair| (pair[1].offset - pair[0].offset) as u8).collect::<Vec<u8>>(),
        );
        node.set_character_positions(chunk.windows(2).map(|pair| pair[0].x - chunk[0].x).collect::<Vec<f32>>());
        node.set_character_widths(chunk.windows(2).map(|pair| pair[1].x - pair[0].x).collect::<Vec<f32>>());
        // A word starts where a non-space follows a space.
        let chars: Vec<char> =
            chunk.windows(2).map(|pair| text[pair[0].offset..].chars().next().unwrap_or(' ')).collect();
        let starts =
            (0..chars.len()).filter(|&c| !chars[c].is_whitespace() && (c == 0 || chars[c - 1].is_whitespace()));
        node.set_word_starts(starts.map(|c| c as u8).collect::<Vec<u8>>());

        let id = run_id(input, ids.len());
        out.push((id, node));
        ids.push(id);
        first = last;
        if first >= characters {
            return ids;
        }
    }
}

/// The text inside a node, as its name.
pub(crate) fn text_of<M>(tree: &Tree<M>, i: usize) -> String {
    let mut name = String::new();
    for at in i..tree.nodes[i].end {
        if let Content::Text { text, .. } = &tree.nodes[at].el.content
            && !hidden(tree, at)
        {
            if !name.is_empty() {
                name.push(' ');
            }
            name.push_str(text);
        }
    }
    name
}

fn bounds(rect: Rect) -> accesskit::Rect {
    let (x, y) = (f64::from(rect.x), f64::from(rect.y));
    accesskit::Rect { x0: x, y0: y, x1: x + f64::from(rect.width), y1: y + f64::from(rect.height) }
}

/// The byte offset an AccessKit text position names, for selections set by assistive technology.
pub(crate) fn offset_of(input: Id, shaped: &Shaped, position: &TextPosition) -> Option<usize> {
    let run = (0..=shaped.stops.len() / RUN).find(|&n| run_id(input, n) == position.node)?;
    shaped.stops.get(run * RUN + position.character_index).map(|stop| stop.offset)
}
