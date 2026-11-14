//! The window as HTML beside the canvas, for assistive technology, which finds nothing to read in a canvas.
//!
//! The window describes itself as an AccessKit tree, the same one the desktop shell hands to the system.
//! [`Mirror::of`] turns that tree into elements, which is pure, and [`Page`] keeps the page's elements in
//! step with it, touching only what changed.

use std::collections::HashMap;

use accesskit::{Action, HasPopup, Node, NodeId, Role, Toggled, TreeUpdate};
use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

/// What an element's `id` starts with. The node's own id follows.
const PREFIX: &str = "letters-node-";

/// One element of the mirror.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub tag: &'static str,
    /// In a fixed order, so that two descriptions of one node compare equal.
    pub attributes: Vec<(&'static str, String)>,
    /// The text it holds. An element holds text or children, never both.
    pub text: String,
    pub children: Vec<u64>,
    /// What activating it asks of the window.
    pub act: Option<Act>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Press it, as a click does.
    Click,
    /// Put the caret at the start of its text: `run` is the text, and `page` the node that holds the
    /// selection.
    Caret { page: u64, run: u64 },
}

/// The window's accessibility tree as elements, by node id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mirror {
    pub root: u64,
    /// The control that has the keyboard inside the window. `None` while the document has it, which is
    /// when the hidden input speaks for itself.
    pub focus: Option<u64>,
    pub items: HashMap<u64, Item>,
}

impl Mirror {
    pub fn of(update: &TreeUpdate) -> Self {
        let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, node)| (*id, node)).collect();
        let Some(root) = update.tree.as_ref().map(|tree| tree.root) else { return Self::default() };
        let mut mirror = Self { root: root.0, focus: None, items: HashMap::with_capacity(nodes.len()) };
        mirror.add(&nodes, root, [0.0, 0.0], None);
        let document = |id: &NodeId| nodes.get(id).is_some_and(|node| node.role() == Role::Document);
        mirror.focus = Some(update.focus).filter(|focus| *focus != root && !document(focus)).map(|focus| focus.0);
        mirror
    }

    /// The element's `id` on the page.
    pub fn name(node: u64) -> String {
        format!("{PREFIX}{node}")
    }

    /// The node an element's `id` names, if it is one of the mirror's.
    pub fn node(name: &str) -> Option<u64> {
        name.strip_prefix(PREFIX)?.parse().ok()
    }

    /// Adds a node and what is under it. `origin` is where the nearest placed element above it starts, and
    /// `page` the nearest node above it whose text can be selected.
    fn add(&mut self, nodes: &HashMap<NodeId, &Node>, id: NodeId, origin: [f64; 2], page: Option<NodeId>) {
        let Some(node) = nodes.get(&id) else { return };
        let (tag, role) = element(node);
        let mut item = Item { tag, ..Item::default() };
        let mut set = |name, value: String| item.attributes.push((name, value));
        set("id", Self::name(id.0));
        if let Some(role) = role {
            set("role", role.into());
        }
        if matches!(tag, "button" | "span") && node.supports_action(Action::Click) {
            // Reachable by assistive technology, and never by Tab: the keyboard stays with the hidden input.
            set("tabindex", "-1".into());
        }
        if let Some(label) = node.label() {
            set("aria-label", label.into());
        }
        if let Some(description) = node.description() {
            set("aria-description", description.into());
        }
        if let Some(language) = node.language() {
            set("lang", language.into());
        }
        if let Some(placeholder) = node.placeholder() {
            set("aria-placeholder", placeholder.into());
        }
        if let Some(level) = node.level().filter(|_| !tag.starts_with('h')) {
            set("aria-level", level.to_string());
        }
        if let Some(toggled) = node.toggled() {
            let state = if node.role() == Role::Button { "aria-pressed" } else { "aria-checked" };
            set(state, matches!(toggled, Toggled::True).to_string());
        }
        if let Some(selected) = node.is_selected() {
            set("aria-selected", selected.to_string());
        }
        if let Some(expanded) = node.is_expanded() {
            set("aria-expanded", expanded.to_string());
        }
        if let Some(popup) = node.has_popup() {
            let kind = match popup {
                HasPopup::Menu => "menu",
                HasPopup::Listbox => "listbox",
                HasPopup::Tree => "tree",
                HasPopup::Grid => "grid",
                HasPopup::Dialog => "dialog",
            };
            set("aria-haspopup", kind.into());
        }
        for (flag, name) in [
            (node.is_disabled(), "aria-disabled"),
            (node.is_modal(), "aria-modal"),
            (node.is_read_only() && node.role() == Role::TextInput, "aria-readonly"),
        ] {
            if flag {
                set(name, "true".into());
            }
        }
        // A node that says where it is lies there, over what the canvas shows. The rest follow one
        // another inside their parent, as the blocks of a document do.
        let mut inner = origin;
        if let Some(bounds) = node.bounds() {
            let (left, top) = (bounds.x0 - origin[0], bounds.y0 - origin[1]);
            set(
                "style",
                format!(
                    "position:absolute;overflow:hidden;left:{left:.0}px;top:{top:.0}px;width:{:.0}px;height:{:.0}px",
                    bounds.width(),
                    bounds.height()
                ),
            );
            inner = [bounds.x0, bounds.y0];
        }

        let page = if node.supports_action(Action::SetTextSelection) { Some(id) } else { page };
        let mut run = None;
        for child in node.children() {
            match nodes.get(child) {
                // A run of text is its parent's text, not an element of its own.
                Some(text) if text.role() == Role::TextRun => {
                    run.get_or_insert(*child);
                    item.text.push_str(text.value().unwrap_or_default());
                }
                _ => {
                    item.children.push(child.0);
                    self.add(nodes, *child, inner, page);
                }
            }
        }
        match (run, node.value()) {
            // A block's run ends with the line break that ends the block.
            (Some(_), _) => item.text.truncate(item.text.trim_end_matches('\n').len()),
            (None, Some(value)) if item.children.is_empty() => item.text = value.into(),
            _ => {}
        }
        item.act = match (node.supports_action(Action::Click), page.zip(run)) {
            (true, _) => Some(Act::Click),
            (false, Some((page, run))) if page != id => Some(Act::Caret { page: page.0, run: run.0 }),
            _ => None,
        };
        self.items.insert(id.0, item);
    }
}

/// The element a node is shown as, and the role it is given when the element has none of its own.
fn element(node: &Node) -> (&'static str, Option<&'static str>) {
    let role = |role| ("div", Some(role));
    match node.role() {
        Role::Paragraph => ("p", None),
        Role::Heading => (["h1", "h2", "h3", "h4", "h5", "h6"][node.level().unwrap_or(1).clamp(1, 6) - 1], None),
        Role::Code => ("pre", None),
        Role::Blockquote => ("blockquote", None),
        Role::Splitter => ("hr", None),
        Role::Button | Role::DefaultButton => ("button", None),
        Role::Label => ("span", None),
        Role::Link => ("span", Some("link")),
        Role::Window => role("group"),
        Role::Document => role("document"),
        Role::ListItem => role("listitem"),
        Role::List => role("list"),
        Role::Note => role("note"),
        Role::TextInput => role("textbox"),
        Role::Toolbar => role("toolbar"),
        Role::Navigation => role("navigation"),
        Role::Complementary => role("complementary"),
        Role::Group => role("group"),
        Role::Dialog => role("dialog"),
        Role::Tooltip => role("tooltip"),
        Role::Menu => role("menu"),
        Role::MenuItem => role("menuitem"),
        Role::MenuItemCheckBox => role("menuitemcheckbox"),
        Role::MenuItemRadio => role("menuitemradio"),
        Role::ListBox => role("listbox"),
        Role::ListBoxOption => role("option"),
        Role::CheckBox => role("checkbox"),
        Role::Switch => role("switch"),
        Role::RadioButton => role("radio"),
        Role::RadioGroup => role("radiogroup"),
        Role::Tab => role("tab"),
        Role::TabList => role("tablist"),
        Role::TabPanel => role("tabpanel"),
        Role::Status => role("status"),
        Role::Alert => role("alert"),
        _ => ("div", None),
    }
}

/// The mirror's elements on the page.
pub struct Page {
    document: Document,
    /// The element the mirror's root is kept in.
    holder: Element,
    elements: HashMap<u64, Element>,
    shown: Mirror,
}

impl Page {
    pub fn new(document: &Document, holder: Element) -> Self {
        Self { document: document.clone(), holder, elements: HashMap::new(), shown: Mirror::default() }
    }

    /// What is on the page now.
    pub fn shown(&self) -> &Mirror {
        &self.shown
    }

    /// Brings the page's elements in step with `mirror`, leaving alone every element that reads as before.
    pub fn show(&mut self, mirror: Mirror) -> Result<(), JsValue> {
        // The elements whose children changed, and whether each is the element that was there before.
        let mut moved = Vec::new();
        for (id, item) in &mirror.items {
            let before = self.shown.items.get(id).filter(|before| before.tag == item.tag);
            if before == Some(item) {
                continue;
            }
            let element = match (before, self.elements.get(id)) {
                (Some(_), Some(element)) => element.clone(),
                _ => {
                    // An element cannot change what it is, so a node that did gets a new one in its place.
                    let element = self.document.create_element(item.tag)?;
                    if let Some(old) = self.elements.insert(*id, element.clone()) {
                        old.replace_with_with_node_1(&element)?;
                    }
                    element
                }
            };
            for (name, _) in before.map(|before| &before.attributes).into_iter().flatten() {
                if !item.attributes.iter().any(|(kept, _)| kept == name) {
                    element.remove_attribute(name)?;
                }
            }
            for (name, value) in &item.attributes {
                if !before.is_some_and(|before| before.attributes.contains(&(*name, value.clone()))) {
                    element.set_attribute(name, value)?;
                }
            }
            if before.is_none_or(|before| before.children != item.children) {
                moved.push((*id, before.is_some()));
            } else if before.is_none_or(|before| before.text != item.text) {
                element.set_text_content(Some(&item.text));
            }
        }
        // Children go in once every element exists. Appending one that is elsewhere moves it, so the
        // children that still lead the list stay put and the rest follow them in order.
        for (id, reused) in moved {
            let (item, parent) = (&mirror.items[&id], &self.elements[&id]);
            let before = self.shown.items.get(&id).filter(|before| reused && before.text.is_empty());
            let kept = match before.filter(|_| item.text.is_empty()) {
                Some(before) => before.children.iter().zip(&item.children).take_while(|(old, new)| old == new).count(),
                None => {
                    parent.set_text_content(Some(&item.text));
                    0
                }
            };
            for child in item.children[kept..].iter().filter_map(|child| self.elements.get(child)) {
                parent.append_child(child)?;
            }
        }
        for id in self.shown.items.keys().filter(|id| !mirror.items.contains_key(id)) {
            if let Some(gone) = self.elements.remove(id) {
                gone.remove();
            }
        }
        if let Some(root) = self.elements.get(&mirror.root).filter(|root| root.parent_node().is_none()) {
            self.holder.append_child(root)?;
        }
        self.shown = mirror;
        Ok(())
    }
}
