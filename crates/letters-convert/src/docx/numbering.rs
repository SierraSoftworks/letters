//! Word's lists: what each level of each numbered or bulleted list is.

use std::collections::BTreeMap;

use letters_model::ListKind;
use roxmltree::{Document, Node};

use super::package::Element;
use super::{TASK_DONE, TASK_OPEN};

/// One level of a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Level {
    /// `None` for a level that shows no marker, which Word uses to indent text without listing it.
    pub kind: Option<ListKind>,
    /// The number its first item takes.
    pub start: u32,
}

impl Default for Level {
    fn default() -> Self {
        Self { kind: Some(ListKind::Ordered), start: 1 }
    }
}

impl Level {
    fn read(level: Node) -> Self {
        let text = level.value("lvlText").unwrap_or_default();
        let kind = match level.value("numFmt") {
            Some("none") => None,
            Some("bullet") if text.starts_with(TASK_OPEN) => Some(ListKind::Task { checked: false }),
            // The crossed box is how Word draws a ticked one.
            Some("bullet") if text.starts_with([TASK_DONE, '\u{2612}']) => Some(ListKind::Task { checked: true }),
            Some("bullet") => Some(ListKind::Bullet),
            _ => Some(ListKind::Ordered),
        };
        Self { kind, start: level.number("start").unwrap_or(1) }
    }
}

/// Every list of a document, by the id paragraphs name it with.
#[derive(Clone, Debug, Default)]
pub(super) struct Numbering(BTreeMap<String, BTreeMap<u8, Level>>);

impl Numbering {
    pub fn read(xml: &Document) -> Self {
        let root = xml.root_element();
        let levels = |node: Node| -> BTreeMap<u8, Level> {
            let level = |level: Node| Some((level.attr("ilvl")?.parse().ok()?, Level::read(level)));
            node.all("lvl").filter_map(level).collect()
        };
        let mut patterns: BTreeMap<&str, BTreeMap<u8, Level>> = BTreeMap::new();
        // A pattern can stand in for another that a list style owns.
        let mut owned: BTreeMap<&str, &str> = BTreeMap::new();
        let mut borrowed: Vec<(&str, &str)> = Vec::new();
        for pattern in root.all("abstractNum") {
            let Some(id) = pattern.attr("abstractNumId") else { continue };
            patterns.insert(id, levels(pattern));
            if let Some(style) = pattern.value("styleLink") {
                owned.insert(style, id);
            }
            if let Some(style) = pattern.value("numStyleLink") {
                borrowed.push((id, style));
            }
        }
        for (id, style) in borrowed {
            if let Some(real) = owned.get(style).and_then(|real| patterns.get(real)).cloned() {
                patterns.insert(id, real);
            }
        }

        let mut lists = BTreeMap::new();
        for list in root.all("num") {
            let Some(id) = list.attr("numId") else { continue };
            let mut list_levels =
                list.value("abstractNumId").and_then(|id| patterns.get(id)).cloned().unwrap_or_default();
            for change in list.all("lvlOverride") {
                let Some(depth) = change.attr("ilvl").and_then(|depth| depth.parse::<u8>().ok()) else { continue };
                if let Some(level) = change.child("lvl") {
                    list_levels.insert(depth, Level::read(level));
                }
                if let Some(start) = change.number("startOverride") {
                    list_levels.entry(depth).or_default().start = start;
                }
            }
            lists.insert(id.to_owned(), list_levels);
        }
        Self(lists)
    }

    /// A level of a list. A list or level the file does not define counts as numbered, which is what Word
    /// shows for one.
    pub fn level(&self, list: &str, depth: u8) -> Level {
        self.0.get(list).and_then(|levels| levels.get(&depth)).copied().unwrap_or_default()
    }
}
