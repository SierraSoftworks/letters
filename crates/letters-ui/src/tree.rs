//! The window's elements, flattened for layout, painting and input.
//!
//! Nodes are stored parents first, so storage order is paint order and tab order, and a subtree is a
//! contiguous range. An open popover's content is a layer of its own after the window's content, which puts
//! it on top for painting and first for hit testing.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use letters_render::{Rect, Size};

use crate::element::{Content, Element, Focus};
use crate::text::Shaped;
use crate::widgets::{Placement, Popover};

/// An element's identity from one build of the tree to the next. Focus, text selections, scroll positions
/// and accessibility nodes are all kept by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Id(pub u64);

impl Id {
    pub const WINDOW: Id = Id(1);

    pub fn child(self, key: u64) -> Id {
        let mut h = (self.0.rotate_left(23) ^ key).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        Id(h ^ (h >> 32))
    }
}

pub(crate) struct Node<M> {
    pub id: Id,
    /// The parent within the layer. A layer's root has none.
    pub parent: Option<usize>,
    /// One past the last descendant.
    pub end: usize,
    pub layer: usize,
    /// The element, with its children and popover content moved out into nodes of their own.
    pub el: Element<M>,
    /// The popover this element anchors, open or not.
    pub trigger: Option<Trigger>,
    pub size: Size,
    pub rect: Rect,
    /// The height of the children, which a scrolling element may exceed.
    pub content_height: f32,
    pub shaped: Option<Rc<Shaped>>,
    /// An empty input's placeholder.
    pub hint: Option<Rc<Shaped>>,
    /// Left out by the layout, as one of several ways to show something that did not fit or was not needed.
    pub gone: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Trigger {
    /// Whether the app decides when the popover is open.
    pub controlled: bool,
    pub open: bool,
    pub popup: accesskit::HasPopup,
}

pub(crate) struct Layer<M> {
    pub root: usize,
    pub end: usize,
    /// The element the layer hangs from. The window's own content has none.
    pub anchor: Option<usize>,
    pub placement: Placement,
    pub modal: bool,
    /// Whether focus shows on what takes it when the layer opens.
    pub shows_focus: bool,
    /// Whether focus stays where it is when the layer opens, and Tab goes round it and its anchor as one.
    pub beside: bool,
    /// Whether a press outside leaves the layer open.
    pub stays: bool,
    pub controlled: bool,
    pub on_close: Option<M>,
}

/// What the window keeps of its elements from one build of the tree to the next, that a build asks for.
pub(crate) struct Kept<'a> {
    /// The popovers that open and close themselves and are open.
    pub open: &'a HashSet<Id>,
    /// The elements built only near what shows that were near it at the last layout.
    pub near: &'a HashSet<Id>,
}

pub(crate) struct Tree<M> {
    pub nodes: Vec<Node<M>>,
    pub layers: Vec<Layer<M>>,
    index: HashMap<Id, usize>,
}

impl<M> Default for Tree<M> {
    fn default() -> Self {
        Self { nodes: Vec::new(), layers: Vec::new(), index: HashMap::new() }
    }
}

impl<M> Tree<M> {
    /// Flattens `root`.
    pub fn build(root: Element<M>, kept: &Kept<'_>) -> Self {
        let mut tree = Tree::default();
        let mut pending = VecDeque::new();
        tree.add_layer(root, None, Id::WINDOW, kept, &mut pending);
        // Popovers found while flattening one layer become later layers, so a nested one lands above its
        // parent.
        while let Some((anchor, popover)) = pending.pop_front() {
            let popover: Popover<M> = *popover;
            let id = tree.nodes[anchor].id.child(POPOVER);
            let layer = tree.add_layer(popover.content, Some(anchor), id, kept, &mut pending);
            let layer = &mut tree.layers[layer];
            layer.placement = popover.placement;
            layer.modal = popover.modal;
            layer.shows_focus = popover.shows_focus;
            layer.beside = popover.beside;
            layer.stays = popover.stays;
            layer.controlled = popover.open.is_some();
            layer.on_close = popover.on_close;
        }
        tree.index = tree.nodes.iter().enumerate().map(|(i, node)| (node.id, i)).collect();
        tree
    }

    fn add_layer(
        &mut self,
        root: Element<M>,
        anchor: Option<usize>,
        parent_id: Id,
        kept: &Kept<'_>,
        pending: &mut VecDeque<(usize, Box<Popover<M>>)>,
    ) -> usize {
        let layer = self.layers.len();
        let start = self.nodes.len();
        self.layers.push(Layer {
            root: start,
            end: start,
            anchor,
            placement: Placement::default(),
            modal: false,
            shows_focus: false,
            beside: false,
            stays: false,
            controlled: false,
            on_close: None,
        });
        self.add(root, None, parent_id, &mut Vec::new(), layer, kept, pending);
        self.layers[layer].end = self.nodes.len();
        layer
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        mut el: Element<M>,
        parent: Option<usize>,
        parent_id: Id,
        siblings: &mut Vec<u64>,
        layer: usize,
        kept: &Kept<'_>,
        pending: &mut VecDeque<(usize, Box<Popover<M>>)>,
    ) {
        // Siblings with the same key are told apart by their order among themselves, so an element keeps
        // its identity when unrelated siblings come and go.
        let key = el.key.unwrap_or_else(|| default_key(&el));
        let nth = siblings.iter().filter(|&&seen| seen == key).count() as u64;
        siblings.push(key);
        let id = parent_id.child(key).child(nth);

        let at = self.nodes.len();
        let mut children = std::mem::take(&mut el.children);
        if let Some(build) = el.later.as_ref().filter(|_| kept.near.contains(&id)) {
            children.push(build());
        }
        let trigger = el.popover.take().map(|popover| {
            let open = popover.open.unwrap_or_else(|| kept.open.contains(&id));
            let trigger = Trigger { controlled: popover.open.is_some(), open, popup: popover.popup };
            if open {
                pending.push_back((at, popover));
            }
            trigger
        });
        self.nodes.push(Node {
            id,
            parent,
            end: at + 1,
            layer,
            el,
            trigger,
            size: Size::default(),
            rect: Rect::default(),
            content_height: 0.0,
            shaped: None,
            hint: None,
            gone: false,
        });
        let mut seen = Vec::new();
        for child in children {
            self.add(child, Some(at), id, &mut seen, layer, kept, pending);
        }
        self.nodes[at].end = self.nodes.len();
    }

    pub fn find(&self, id: Id) -> Option<usize> {
        self.index.get(&id).copied()
    }

    /// The direct children of a node that are showing, in order.
    pub fn children(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        self.all_children(i).filter(|&child| !self.nodes[child].gone)
    }

    /// The direct children of a node, with those the layout left out.
    pub fn all_children(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let end = self.nodes[i].end;
        let mut next = i + 1;
        std::iter::from_fn(move || {
            (next < end).then(|| {
                let child = next;
                next = self.nodes[child].end;
                child
            })
        })
    }

    /// The node and then each element it sits in, up to the root of its layer.
    pub fn ancestors(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(Some(i), move |&at| self.nodes[at].parent)
    }

    /// The node's parent, stepping from a popover's root to the element it hangs from.
    pub fn owner(&self, i: usize) -> Option<usize> {
        self.nodes[i].parent.or(self.layers[self.nodes[i].layer].anchor)
    }

    /// The node and then each of its owners, up to the window's root.
    pub fn chain(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(Some(i), move |&at| self.owner(at))
    }

    /// Whether a node is in a layer, or in a popover opened from inside it.
    pub fn in_layer(&self, i: usize, layer: usize) -> bool {
        self.chain(i).any(|at| self.nodes[at].layer == layer)
    }

    /// The topmost popover, if any is open.
    pub fn top_layer(&self) -> Option<usize> {
        (self.layers.len() > 1).then(|| self.layers.len() - 1)
    }

    /// The layer the pointer and Tab are held in: the topmost modal one, or none.
    pub fn modal_layer(&self) -> Option<usize> {
        self.layers.iter().rposition(|layer| layer.modal)
    }

    pub fn focusable(&self, i: usize) -> bool {
        let node = &self.nodes[i];
        node.el.focus != Focus::No && !self.disabled(i) && !self.gone(i)
    }

    /// Whether the layout left out the node or anything it sits in.
    pub fn gone(&self, i: usize) -> bool {
        self.ancestors(i).any(|at| self.nodes[at].gone)
    }

    /// Whether the node or anything it sits in is disabled.
    pub fn disabled(&self, i: usize) -> bool {
        self.ancestors(i).any(|at| self.nodes[at].el.disabled)
    }

    /// The focusable nodes in `range`, in tab order.
    pub fn focusables(&self, range: std::ops::Range<usize>) -> Vec<usize> {
        range.filter(|&i| self.focusable(i)).collect()
    }
}

const POPOVER: u64 = 0x0070_6F70_6F76_6572;

fn default_key<M>(el: &Element<M>) -> u64 {
    let mut hasher = DefaultHasher::new();
    let kind = match &el.content {
        Content::None => 0u8,
        Content::Text { .. } => 1,
        Content::Icon { .. } => 2,
        Content::Input(_) => 3,
        Content::Canvas(_) => 4,
    };
    kind.hash(&mut hasher);
    el.access.role.map(|role| role as u8).hash(&mut hasher);
    el.access.label.hash(&mut hasher);
    hasher.finish()
}
