//! A file name for a document that has none yet.

use letters_model::{BlockKind, Document};

/// The longest suggested name, in bytes. With an extension and a number after it, a name of this length
/// still fits the 255 bytes most file systems allow.
const LONGEST: usize = 120;

/// The name to give a document's file when its writer has not chosen one: the text of its first Heading 1,
/// made safe for file systems. `None` when there is no such heading, or nothing of it is left.
///
/// The name has no extension. Characters that some file system refuses are dropped, a leading dot is
/// removed so the file is not hidden, and a long heading is cut at a word.
pub fn suggested_name(document: &Document) -> Option<String> {
    let heading = document.walk().find(|block| block.kind == BlockKind::Heading { level: 1 })?;
    let refused = |c: char| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
    let text = heading.plain_text().replace(refused, " ");
    let mut name = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.len() > LONGEST {
        let mut end = LONGEST;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        // A cut inside a word reads as a mistake, unless the heading is one long word.
        end = name[..end].rfind(' ').filter(|space| *space >= LONGEST / 2).unwrap_or(end);
        name.truncate(end);
    }
    let name = name.trim_start_matches(['.', ' ']).trim_end_matches(['.', ' ']);
    (!name.is_empty()).then(|| name.to_owned())
}
