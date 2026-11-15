//! Bringing a Markdown or a Word file in as a document, as the desktop app does.
//!
//! The file is read here and made a document in the folder it was handed to, under its own name where
//! that is free. What the document cannot hold is flattened and marked with an import note on its passage,
//! and the import is the first version of the document's history.
//!
//! A Markdown file names its pictures by path. No file comes with it here, so those stay links.

use letters_api::Entry;
use letters_convert::{DocxError, ImportNote, from_docx, import_markdown};
use letters_format::WorkingCopy;
use letters_model::{Anchor, Block, BlockId, BlockKind, Note, NoteKind, ReplicaId, Run, TagKind};

use crate::error::{Answer, Refusal};
use crate::session::Signed;
use crate::spaces::Place;
use crate::{App, ids};

/// How a zip starts, and how the older kind of Word file does. Either is read as Word, whatever its name.
const WORD_STARTS: [&[u8]; 2] = [b"PK\x03\x04", &[0xd0, 0xcf, 0x11, 0xe0]];
/// The name import notes are written under.
const IMPORTER: &str = "Letters import";

/// The note that marks, on its block, something the import simplified.
fn import_note(note: &ImportNote, created: &str) -> Note {
    Note {
        id: ids::block_id(),
        kind: NoteKind::Import,
        anchor: Anchor::block(note.block.as_str()),
        author: IMPORTER.into(),
        created: created.into(),
        resolved: false,
        content: vec![Run::plain(note.message.as_str())],
        replies: Vec::new(),
        extra: Default::default(),
    }
}

/// Why a Word file made no document, as the end of a sentence. What the reader said of it goes to the log.
fn unread(error: &DocxError) -> &'static str {
    match error {
        DocxError::Legacy => {
            " because it is an older .doc or is locked with a password. Save it from Word as an unlocked .docx"
        }
        DocxError::TooLarge(_) => " because it is too large to bring in",
        error => {
            crate::log(&format!("a Word file could not be read: {error}"));
            " because it is damaged or is not a Word document"
        }
    }
}

impl App {
    /// Makes a document of `file`, a Markdown or a Word file that was called `name`, in a folder of a
    /// space `who` writes in. Which of the two it is, is told from what it holds, not from its name.
    pub fn import(&self, who: &Signed, space: &str, folder: &str, name: &str, file: &[u8]) -> Answer<Entry> {
        let (space, role) = self.space(who, space)?;
        if !role.edits() {
            return Err(Refusal::forbidden("You can read this space, but not add to it."));
        }
        let (root, folder) = (self.root(who, space)?, Place::parse(folder)?);
        // The name as the file had it on its device: its last part, without its ending.
        let named = name.rsplit(['/', '\\']).next().unwrap_or_default();
        let stem = named.rsplit_once('.').map_or(named, |(stem, _)| stem);
        let refused = |why: &str| Refusal::invalid(format!("{named} could not be imported{why}."));
        let (style, author, now) = (self.new_style(who)?, self.signs_as(who)?, self.now());
        let created = ids::rfc3339(now);

        let (document, style, simplified, comments, assets) = if WORD_STARTS.iter().any(|start| file.starts_with(start))
        {
            let word =
                from_docx(file, &style, &created, &mut ids::block_id).map_err(|error| refused(unread(&error)))?;
            (word.document, word.style, word.notes, word.comments, word.assets)
        } else {
            let markdown = std::str::from_utf8(file)
                .map_err(|_| refused(" because it is neither Markdown nor a Word document"))?;
            let read = import_markdown(markdown, &mut ids::block_id, &mut |_| None);
            (read.document, style, read.notes, Vec::new(), read.assets)
        };

        let mut copy =
            WorkingCopy::new(ids::document_id(), created.clone(), document, style, ReplicaId::new(&ids::block_id()));
        copy.sign(&author, now);
        if copy.model().doc().blocks.is_empty() {
            let line = Block::text(BlockId::new(ids::block_id()), BlockKind::Paragraph, "");
            copy.apply([letters_model::Op::InsertBlock { parent: None, index: 0, block: line }])
                .map_err(|error| refused(&format!(": {error}")))?;
        }
        copy.entries.extend(assets);
        // People's comments come along, beside what the import says of itself.
        for note in comments.into_iter().chain(simplified.iter().map(|note| import_note(note, &created))) {
            copy.add_note(note);
        }
        // The history then lists where the document came from.
        copy.mark(TagKind::Import, Some(named))?;
        let place = self.library.bring(&root, &folder, stem, &copy)?;
        self.entry(who, &space.told(role), &root, &place)
    }
}
