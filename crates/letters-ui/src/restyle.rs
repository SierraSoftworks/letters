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
    /// As `#rrggbb`.
    Color(String),
    Align(Align),
    /// A multiple of the font size.
    LineHeight(f32),
    /// In points.
    SpaceBefore(f32),
    /// In points.
    SpaceAfter(f32),
}

impl StyleEdit {
    pub(crate) fn apply(&self, style: &mut TextStyle) {
        match self {
            Self::Family(family) => style.family.clone_from(family),
            Self::Weight(weight) => style.weight = *weight,
            Self::Size(size) => style.size = *size,
            Self::Color(color) => style.color.clone_from(color),
            Self::Align(align) => style.align = *align,
            Self::LineHeight(height) => style.line_height = *height,
            Self::SpaceBefore(space) => style.space_before = *space,
            Self::SpaceAfter(space) => style.space_after = *space,
        }
    }
}

/// A part of a style that is typed into a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Size,
    Color,
    LineHeight,
    SpaceBefore,
    SpaceAfter,
}

impl Field {
    /// The field's name, as assistive technology reads it.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Size => "Size",
            Self::Color => "Hex",
            Self::LineHeight => "Line height",
            Self::SpaceBefore => "Space before",
            Self::SpaceAfter => "Space after",
        }
    }

    /// What the field shows for a style.
    pub(crate) fn show(self, style: &TextStyle) -> String {
        match self {
            Self::Size => style.size.to_string(),
            Self::Color => style.color.clone(),
            Self::LineHeight => style.line_height.to_string(),
            Self::SpaceBefore => style.space_before.to_string(),
            Self::SpaceAfter => style.space_after.to_string(),
        }
    }

    /// The change that typed text asks for. Text that makes no sense for the field asks for none.
    pub(crate) fn read(self, text: &str) -> Option<StyleEdit> {
        let number = |range: std::ops::RangeInclusive<f32>| text.trim().parse().ok().filter(|n| range.contains(n));
        match self {
            Self::Size => number(1.0..=400.0).map(StyleEdit::Size),
            Self::Color => color(text).map(StyleEdit::Color),
            Self::LineHeight => number(0.5..=4.0).map(StyleEdit::LineHeight),
            Self::SpaceBefore => number(0.0..=200.0).map(StyleEdit::SpaceBefore),
            Self::SpaceAfter => number(0.0..=200.0).map(StyleEdit::SpaceAfter),
        }
    }
}

/// Reads a colour typed as `#rrggbb`, `rrggbb` or the short `#rgb`, as the style stores it.
fn color(text: &str) -> Option<String> {
    let digits = text.trim().trim_start_matches('#').to_ascii_lowercase();
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match digits.len() {
        6 => Some(format!("#{digits}")),
        3 => Some(digits.chars().fold(String::from("#"), |hex, c| format!("{hex}{c}{c}"))),
        _ => None,
    }
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
    fn a_field_takes_only_what_makes_sense_for_it() {
        let cases = [
            (Field::Size, "12", Some(StyleEdit::Size(12.0))),
            (Field::Size, " 10.5 ", Some(StyleEdit::Size(10.5))),
            (Field::Size, "0", None),
            (Field::Size, "-4", None),
            (Field::Size, "big", None),
            (Field::Size, "", None),
            (Field::Size, "inf", None),
            (Field::Color, "#B82C24", Some(StyleEdit::Color("#b82c24".into()))),
            (Field::Color, "146f43", Some(StyleEdit::Color("#146f43".into()))),
            (Field::Color, "#fa0", Some(StyleEdit::Color("#ffaa00".into()))),
            (Field::Color, "#12345", None),
            (Field::Color, "red", None),
            (Field::LineHeight, "1.6", Some(StyleEdit::LineHeight(1.6))),
            (Field::LineHeight, "0", None),
            (Field::LineHeight, "12", None),
            (Field::SpaceBefore, "0", Some(StyleEdit::SpaceBefore(0.0))),
            (Field::SpaceAfter, "8", Some(StyleEdit::SpaceAfter(8.0))),
            (Field::SpaceAfter, "-1", None),
        ];
        for (field, typed, expected) in cases {
            assert_eq!(field.read(typed), expected, "{field:?} {typed:?}");
        }
    }

    #[test]
    fn a_field_reads_back_what_it_shows() {
        let style = Style::default().types["heading1"].clone();
        for field in [Field::Size, Field::Color, Field::LineHeight, Field::SpaceBefore, Field::SpaceAfter] {
            let mut again = style.clone();
            field.read(&field.show(&style)).expect("a shown value is a sensible one").apply(&mut again);
            assert_eq!(again, style, "{field:?}");
        }
    }

    #[test]
    fn types_are_named_for_people() {
        for (element, expected) in [("body", "Body"), ("heading2", "Heading 2"), ("quote", "Quote")] {
            assert_eq!(name(element), expected);
        }
    }
}
