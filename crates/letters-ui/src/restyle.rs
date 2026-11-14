//! Style changes made from the top bar.
//!
//! The bar edits an element type, never a selection, so two headings of one level cannot drift apart.

use letters_model::{Align, Block, BlockId, BlockKind, Document, Style, TextStyle};

/// Where the bar's style changes go.
///
/// The document model does not take a style change as an operation yet. Until it does, the window applies
/// them to its own copy of the style: they show at once, but undo does not take them back.
pub trait Styles {
    fn style(&self) -> &Style;

    /// Restyles every block of one element type.
    fn restyle(&mut self, element: &str, style: TextStyle);
}

impl Styles for Style {
    fn style(&self) -> &Style {
        self
    }

    fn restyle(&mut self, element: &str, style: TextStyle) {
        self.types.insert(element.to_owned(), style);
    }
}

/// One change to an element type's style.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleEdit {
    Family(String),
    Weight(u16),
    /// In points.
    Size(f32),
    Align(Align),
}

impl StyleEdit {
    pub(crate) fn apply(&self, style: &mut TextStyle) {
        match self {
            Self::Family(family) => style.family.clone_from(family),
            Self::Weight(weight) => style.weight = *weight,
            Self::Size(size) => style.size = *size,
            Self::Align(align) => style.align = *align,
        }
    }
}

/// The sizes a style may take, in points. Anything else typed into the bar is dropped.
pub(crate) fn size(text: &str) -> Option<f32> {
    text.trim().parse::<f32>().ok().filter(|size| size.is_finite() && (1.0..=400.0).contains(size))
}

/// The element type whose style a block is drawn with, as the layout picks it: text inside a quote is
/// `quote` at any depth, and lists and callouts hold body text.
pub(crate) fn element(doc: &Document, block: &BlockId) -> Option<&'static str> {
    fn find(blocks: &[Block], id: &BlockId, quoted: bool) -> Option<&'static str> {
        blocks.iter().find_map(|block| {
            if block.id != *id {
                let quoted = quoted || block.kind == BlockKind::Quote;
                return find(block.body.blocks(), id, quoted);
            }
            Some(match &block.kind {
                BlockKind::Heading { level: 1 } => "heading1",
                BlockKind::Heading { level: 2 } => "heading2",
                BlockKind::Heading { level: 3 } => "heading3",
                BlockKind::Heading { .. } => "heading4",
                BlockKind::Code { .. } => "code",
                BlockKind::Unknown { .. } => "caption",
                _ if quoted => "quote",
                _ => "body",
            })
        })
    }
    find(&doc.blocks, block, false)
}

/// An element type's name, as the bar and the Styles panel show it.
pub(crate) fn name(element: &str) -> String {
    match element.strip_prefix("heading") {
        Some(level) => format!("Heading {level}"),
        None => {
            let mut letters = element.chars();
            letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use letters_model::{Body, ListKind, Tone};

    use super::*;

    fn text(id: &str, kind: BlockKind) -> Block {
        Block::text(id, kind, "Text")
    }

    fn holding(id: &str, kind: BlockKind, blocks: Vec<Block>) -> Block {
        Block { body: Body::Blocks(blocks), ..Block::new(id, kind) }
    }

    #[test]
    fn a_block_takes_the_style_the_layout_draws_it_with() {
        let doc = Document {
            blocks: vec![
                text("p", BlockKind::Paragraph),
                text("h2", BlockKind::Heading { level: 2 }),
                text("h4", BlockKind::Heading { level: 4 }),
                text("item", BlockKind::ListItem { list: ListKind::Bullet, depth: 0 }),
                text("code", BlockKind::Code { language: None }),
                holding("quote", BlockKind::Quote, vec![text("said", BlockKind::Paragraph)]),
                holding(
                    "note",
                    BlockKind::Callout { tone: Tone::Info },
                    vec![
                        text("noted", BlockKind::Paragraph),
                        holding("within", BlockKind::Quote, vec![text("deep", BlockKind::Paragraph)]),
                    ],
                ),
            ],
            ..Document::default()
        };
        let cases = [
            ("p", Some("body")),
            ("h2", Some("heading2")),
            ("h4", Some("heading4")),
            ("item", Some("body")),
            ("code", Some("code")),
            ("said", Some("quote")),
            ("noted", Some("body")),
            ("deep", Some("quote")),
            ("missing", None),
        ];
        for (block, expected) in cases {
            assert_eq!(element(&doc, &block.into()), expected, "{block}");
        }
    }

    #[test]
    fn an_edit_changes_one_field_of_one_type() {
        let mut style = Style::default();
        let before = style.clone();
        let mut heading = style.types["heading1"].clone();
        StyleEdit::Size(30.0).apply(&mut heading);
        StyleEdit::Align(Align::Center).apply(&mut heading);
        style.restyle("heading1", heading);

        let after = &style.types["heading1"];
        assert_eq!((after.size, after.align), (30.0, Align::Center));
        assert_eq!(after.family, before.types["heading1"].family);
        assert_eq!(style.types["body"], before.types["body"]);
    }

    #[test]
    fn only_a_sensible_size_is_taken() {
        let cases = [("12", Some(12.0)), (" 10.5 ", Some(10.5)), ("0", None), ("-4", None), ("big", None), ("", None)];
        for (typed, expected) in cases {
            assert_eq!(size(typed), expected, "{typed:?}");
        }
    }

    #[test]
    fn types_are_named_for_people() {
        for (element, expected) in [("body", "Body"), ("heading2", "Heading 2"), ("quote", "Quote")] {
            assert_eq!(name(element), expected);
        }
    }
}
