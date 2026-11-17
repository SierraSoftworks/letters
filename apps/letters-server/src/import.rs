//! Bringing a Markdown or a Word file in as a document, as the desktop app does.
//!
//! The file is read here and made a document in the folder it was handed to, under its own name where
//! that is free. What the document cannot hold is flattened and marked with an import note on its passage,
//! and the import is the first version of the document's history.
//!
//! A Markdown file names its pictures by path. No file comes with one that is handed over, so those stay
//! links. One that a space keeps has its pictures read from its folder and the folders in it.

use letters_api::Entry;
use letters_convert::{Assets, Dates, DocxError, ImportNote, from_docx, import_markdown};
use letters_format::WorkingCopy;
use letters_model::{Anchor, Block, BlockId, BlockKind, Note, NoteKind, ReplicaId, Run, TagKind};

use crate::error::{Answer, Refusal};
use crate::session::Signed;
use crate::spaces::{Place, Root, name_ok, seconds};
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
            crate::logs::warn(&format!("a Word file could not be read: {error}"));
            " because it is damaged or is not a Word document"
        }
    }
}

impl App {
    /// Makes a document of `file`, a Markdown or a Word file that was called `name`, in a folder of a
    /// space `who` writes in. Which of the two it is, is told from what it holds, not from its name.
    ///
    /// `modified` is when the file was last changed where it came from, in seconds since 1970, if whoever
    /// sends it knows: a browser does. The document is dated by what the file says of itself, and by that
    /// where it says nothing.
    pub fn import(
        &self,
        who: &Signed,
        space: &str,
        folder: &str,
        name: &str,
        file: &[u8],
        modified: Option<u64>,
    ) -> Answer<Entry> {
        let (space, role) = self.space(who, space)?;
        if !role.edits() {
            return Err(Refusal::forbidden("You can read this space, but not add to it."));
        }
        let (root, folder) = (self.root(who, space)?, Place::parse(folder)?);
        let dates = Dates { created: None, modified };
        self.made(who, &space.told(role), &root, &folder, name, (file, dates), &mut |_| None)
    }

    /// Makes a document of the Markdown or Word file a space keeps at `path`, beside it. The file is left as
    /// it is. Pictures a Markdown file names are read from its folder and the folders in it, and from
    /// nowhere else: whoever wrote the file may not be who brings it in. Where the file does not say when
    /// it was made and last changed, the document is dated by when the file was last written.
    pub fn import_file(&self, who: &Signed, space: &str, path: &str) -> Answer<Entry> {
        let (space, role) = self.space(who, space)?;
        if !role.edits() {
            return Err(Refusal::forbidden("You can read this space, but not add to it."));
        }
        let (root, place) = (self.root(who, space)?, Place::parse(path)?);
        let (folder, name) = place.split();
        let limits = letters_api::Limits::default();
        let kept = root.folder(&folder)?;
        // Only what the folder lists is read: no symlink, and nothing hidden.
        let listed = kept.loose()?.into_iter().find(|file| file.name == name);
        let dates = Dates { created: None, modified: listed.as_ref().map(|file| seconds(file.stamp)) };
        let file = if listed.is_some() { kept.start(name, limits.document + 1)? } else { None };
        let file = file.ok_or_else(Refusal::not_found)?;
        if file.len() as u64 > limits.document {
            return Err(Refusal::new(letters_api::Code::TooLarge, "The file is larger than the server takes."));
        }
        let mut pictures = |path: &str| {
            let path = path.strip_prefix("./").unwrap_or(path);
            let inside = !path.is_empty() && path.split('/').all(name_ok);
            let picture = inside.then(|| kept.start(path, limits.asset + 1).ok().flatten()).flatten();
            picture.filter(|picture| picture.len() as u64 <= limits.asset)
        };
        self.made(who, &space.told(role), &root, &folder, name, (&file, dates), &mut pictures)
    }

    /// The document of `file`, which was called `name`, kept in `folder`. The file comes with what its
    /// place says of when it was made and last changed, for what it does not say itself.
    #[allow(clippy::too_many_arguments)]
    fn made(
        &self,
        who: &Signed,
        space: &letters_api::Space,
        root: &Root,
        folder: &Place,
        name: &str,
        (file, placed): (&[u8], Dates),
        pictures: Assets,
    ) -> Answer<Entry> {
        // The name as the file had it on its device: its last part, without its ending.
        let named = name.rsplit(['/', '\\']).next().unwrap_or_default();
        let stem = named.rsplit_once('.').map_or(named, |(stem, _)| stem);
        let refused = |why: &str| Refusal::invalid(format!("{named} could not be imported{why}."));
        let (style, author, now) = (self.new_style(who)?, self.signs_as(who)?, self.now());
        let noted = ids::rfc3339(now);

        let (document, style, simplified, comments, assets, said) =
            if WORD_STARTS.iter().any(|start| file.starts_with(start)) {
                let word =
                    from_docx(file, &style, &noted, &mut ids::block_id).map_err(|error| refused(unread(&error)))?;
                (word.document, word.style, word.notes, word.comments, word.assets, word.dates)
            } else {
                let markdown = std::str::from_utf8(file)
                    .map_err(|_| refused(" because it is neither Markdown nor a Word document"))?;
                let read = import_markdown(markdown, &mut ids::block_id, pictures);
                (read.document, style, read.notes, Vec::new(), read.assets, read.dates)
            };
        // The document is as old as the file it is made of, and was last changed when that was. The
        // import's own notes are written now.
        let (created, modified) = said.or(placed).settle(now);

        let mut copy = WorkingCopy::new(
            ids::document_id(),
            ids::rfc3339(created),
            document,
            style,
            ReplicaId::new(&ids::block_id()),
        );
        // What the import writes is dated as the file was: so is its mark, which the history lists.
        copy.sign(&author, modified);
        if copy.model().doc().blocks.is_empty() {
            let line = Block::text(BlockId::new(ids::block_id()), BlockKind::Paragraph, "");
            copy.apply([letters_model::Op::InsertBlock { parent: None, index: 0, block: line }])
                .map_err(|error| refused(&format!(": {error}")))?;
        }
        copy.entries.extend(assets);
        // People's comments come along, beside what the import says of itself.
        for note in comments.into_iter().chain(simplified.iter().map(|note| import_note(note, &noted))) {
            copy.add_note(note);
        }
        // The history then lists where the document came from.
        copy.mark(TagKind::Import, Some(named))?;
        let place = self.library.bring(root, folder, stem, &copy)?;
        self.entry(who, space, root, &place)
    }
}
