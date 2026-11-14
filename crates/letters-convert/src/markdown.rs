//! Markdown in both directions.
//!
//! Reading follows CommonMark and GFM, with front matter, footnotes, dollar maths and wiki links. What the
//! document cannot hold yet (tables, images, maths, footnotes, front matter and raw HTML) is kept as text or as
//! a code block, with an import note that says so.

mod read;
mod write;

use letters_model::{Mark, Tone};

pub use read::from_markdown;
pub(crate) use read::{merge, nest, table};
pub(crate) use write::cell;
pub use write::to_markdown;

/// The tags a callout opens with. The first name of each tone is GitHub's and is the one written, so that the
/// file renders as alerts there.
pub(crate) const TONES: [(&str, Tone); 8] = [
    ("NOTE", Tone::Info),
    ("TIP", Tone::Tip),
    ("WARNING", Tone::Warn),
    ("CAUTION", Tone::Danger),
    ("INFO", Tone::Info),
    ("IMPORTANT", Tone::Info),
    ("WARN", Tone::Warn),
    ("DANGER", Tone::Danger),
];

/// The HTML tags read as marks. The first name of each mark is the one written when Markdown has no syntax
/// for it.
pub(crate) const TAGS: [(&str, Mark); 11] = [
    ("strong", Mark::Bold),
    ("em", Mark::Italic),
    ("del", Mark::Strike),
    ("u", Mark::Underline),
    ("mark", Mark::Highlight),
    ("code", Mark::Code),
    ("b", Mark::Bold),
    ("i", Mark::Italic),
    ("s", Mark::Strike),
    ("strike", Mark::Strike),
    ("ins", Mark::Underline),
];
