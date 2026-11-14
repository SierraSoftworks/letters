//! Word documents in both directions.
//!
//! Reading maps Word's paragraph styles, lists, links, footnotes and comments onto the document, and seeds the
//! document's style from Word's own, so the result looks familiar. What the document cannot hold is flattened
//! and named in a note on the passage it happened to.
//!
//! Writing generates real Word styles from the document's style and points each paragraph at one, so the file
//! restyles in Word as the document does here. Only marks are written as formatting on runs.

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

/// The paragraph style of a divider, by id and name.
const DIVIDER: (&str, &str) = ("Divider", "Divider");
/// The paragraph style list items take, which sets them closer together than paragraphs.
const LIST: (&str, &str) = ("ListParagraph", "List Paragraph");
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
