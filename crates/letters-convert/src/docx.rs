//! Word documents in both directions.
//!
//! Reading maps Word's paragraph styles, lists, tables, pictures, links, footnotes and comments onto the
//! document, and seeds the document's style from Word's own, so the result looks familiar. What the document
//! cannot hold is flattened and named in a note on the passage it happened to.
//!
//! Writing generates real Word styles from the document's style and points each paragraph at one, so the file
//! restyles in Word as the document does here. Only marks are written as formatting on runs.
//!
//! Maths is TeX here and an equation in Word, and each is made from the other.

mod math;
mod numbering;
mod package;
mod read;
mod styles;
mod write;

use std::fmt;

use letters_model::Tone;

pub use read::{DocxImport, from_docx};
pub use write::to_docx;

/// Why a file could not be read as a Word document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocxError {
    /// An older `.doc`, or a `.docx` locked with a password. From outside the two are the same kind of file.
    Legacy,
    /// Not a zip, or a damaged one.
    Damaged(String),
    /// A zip that holds no Word document.
    NotWord,
    /// A part of the file is not the XML it should be.
    Xml { part: String, message: String },
    /// A part is larger than an import will read.
    TooLarge(String),
}

impl fmt::Display for DocxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy => f.write_str(
                "the file is an older .doc or is locked with a password: save it from Word as an unlocked .docx",
            ),
            Self::Damaged(message) => write!(f, "the file is damaged or is not a .docx: {message}"),
            Self::NotWord => f.write_str("the file is not a Word document"),
            Self::Xml { part, message } => write!(f, "{part} cannot be read: {message}"),
            Self::TooLarge(part) => write!(f, "{part} is too large to import"),
        }
    }
}

impl std::error::Error for DocxError {}

/// The Word style that stands for each element type: the type, the id the style is written under, and its
/// name. Word keeps the names of its built-in styles in English whatever language it runs in, so a style is
/// recognised by name and never by id.
const TYPES: [(&str, &str, &str); 9] = [
    ("title", "Title", "Title"),
    ("heading1", "Heading1", "heading 1"),
    ("heading2", "Heading2", "heading 2"),
    ("heading3", "Heading3", "heading 3"),
    ("heading4", "Heading4", "heading 4"),
    ("body", "Normal", "Normal"),
    ("quote", "Quote", "Quote"),
    ("code", "Code", "Code"),
    ("caption", "Caption", "caption"),
];

/// The paragraph style of each callout tone, by id and name. Word has none of its own.
const CALLOUTS: [(Tone, &str, &str); 4] = [
    (Tone::Tip, "CalloutTip", "Callout Tip"),
    (Tone::Info, "CalloutInfo", "Callout Info"),
    (Tone::Warn, "CalloutWarn", "Callout Warn"),
    (Tone::Danger, "CalloutDanger", "Callout Danger"),
];

/// What the style of the line that heads a callout adds to its tone's style, in id and in name.
const CALLOUT_TITLE: (&str, &str) = ("Title", " Title");

/// The paragraph style of a divider, by id and name.
const DIVIDER: (&str, &str) = ("Divider", "Divider");
/// The paragraph style list items take, which sets them closer together than paragraphs.
const LIST: (&str, &str) = ("ListParagraph", "List Paragraph");
/// The style of a table, and the paragraph styles of the text in its cells and in the cells of a row that
/// heads it. A cell's padding is its space, so its style leaves none of a paragraph's. LibreOffice has
/// styles of these names for the same.
const TABLE: (&str, &str) = ("Table", "Table");
const CELL: (&str, &str) = ("TableText", "Table Text");
const HEAD_CELL: (&str, &str) = ("TableHeading", "Table Heading");
/// The paragraph style of the empty paragraph after a table. Word joins two tables that touch, and sets a
/// table against the text under it.
const TABLE_SPACE: (&str, &str) = ("TableSpace", "Table Space");
/// The paragraph style of the empty paragraph between two boxes of one kind, as two quotes are. Word draws
/// neighbours that are framed alike as one.
const BOX_SPACE: (&str, &str) = ("BoxSpace", "Box Space");
/// What Word counts figures under, and what opens the name of the bookmark around a figure's label.
const SEQUENCE: &str = "Figure";
const BOOKMARK: &str = "_RefFigure";
/// The paragraph styles of the lines of a table of contents, one for each level it lists. They are Word's
/// own, which it sets the lines it makes in.
const CONTENTS: [(&str, &str); 4] = [("TOC1", "toc 1"), ("TOC2", "toc 2"), ("TOC3", "toc 3"), ("TOC4", "toc 4")];
/// What opens the name of the bookmark at a heading that a table of contents lists.
const HEADING_MARK: &str = "_Toc";
/// The paragraph style of what a link's preview shows.
const PREVIEW: (&str, &str) = ("LinkPreview", "Link Preview");
/// The character style of code inside a line.
const INLINE_CODE: (&str, &str) = ("InlineCode", "Inline Code");
/// The character style of a link.
const LINK: (&str, &str) = ("Hyperlink", "Hyperlink");

/// The bullets of an open and a ticked task. A task is a list item whose bullet is one of these.
const TASK_OPEN: char = '\u{2610}';
const TASK_DONE: char = '\u{2611}';

/// Twentieths of a point, Word's unit for spacing and page measures.
const TWIPS: f32 = 20.0;
/// What Word's single line spacing comes to as a multiple of the font size, near enough for the usual faces.
const SINGLE: f32 = 1.2;

/// Whether a family is one whose letters all share a width, going by its name.
fn monospace(family: &str) -> bool {
    let name = family.to_ascii_lowercase();
    ["mono", "courier", "consolas", "code", "menlo", "monaco", "lucida console", "typewriter"]
        .iter()
        .any(|part| name.contains(part))
}
