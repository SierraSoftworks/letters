//! What an export will simplify, told before anything is written.

use std::fmt;

use letters_model::{Block, BlockKind, Document, Mark, Note, NoteKind, Notes};

/// A format a document is exported to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Markdown,
    Html,
    Docx,
    Pdf,
    /// A document of its own with nothing of how it came to be: no history and no notes.
    Clean,
}

/// One way an export gives a document less than it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LossKind {
    /// Blocks of a kind this build does not know. A page draws each as a placeholder that names its kind,
    /// as the window does, and the other formats write its fallback text.
    UnknownBlocks,
    /// Passages with formatting this build does not know, which is left out.
    UnknownMarks,
    /// Comments, which only a Word file carries.
    Comments,
    /// Pictures that are still links. An export fetches nothing, so a web page or a Word file gets the
    /// link, and a page the frame the window shows before the picture loads. Markdown names a picture by
    /// its address anyway.
    LinkedPictures,
    /// Blocks inside a quote or a callout that Word writes outside it, since it cannot nest them.
    NestedBlocks,
    /// Callouts with an icon or a colour of their own, where the format has a place for neither. Word has
    /// no place for an icon at all, so there it is every callout that shows one.
    CalloutLooks,
    /// Galleries, where the format has pictures but no way to set them side by side.
    Galleries,
    /// Page breaks, which Markdown has no word for.
    PageBreaks,
    /// Passages said to be in a language other than the document's, which Markdown cannot say.
    Languages,
    /// Code blocks whose language the format does not keep.
    CodeLanguages,
    /// The history: every earlier version, and who wrote what. A clean copy is the document as it stands.
    /// Counted once.
    History,
    /// Conflicts nobody has settled, which go with the notes. The version the page shows is the one kept.
    Conflicts,
}

/// One kind of loss, and how many places in the document it happens at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loss {
    pub kind: LossKind,
    /// How many blocks, passages or comments, by the kind. Never zero.
    pub count: usize,
    /// The format the loss is in, which the wording depends on.
    pub to: Target,
}

/// What an export of `doc` to a format will simplify, a [`Loss`] for each kind there is, without writing
/// anything. `notes` are the document's, for the comments and conflicts a format leaves behind.
///
/// The list holds what a reader of the export would miss. It leaves out what only shows when the export
/// is read back, which each writer documents: the sizes of pictures in Markdown, a justified column, and
/// the like. An empty list means the export carries the document as the reader sees it.
pub fn losses(doc: &Document, notes: &Notes, to: Target) -> Vec<Loss> {
    losses_with(doc, notes, to, &|_| false)
}

/// As [`losses`], for a shell that hands a PDF the linked pictures it has loaded, as
/// `PdfSources::linked` takes them. `loaded` says whether it has the picture at an address: a PDF shows
/// that one, so it is no loss there.
pub fn losses_with(doc: &Document, notes: &Notes, to: Target, loaded: &dyn Fn(&str) -> bool) -> Vec<Loss> {
    use LossKind::*;
    use Target::*;
    let blocks = |is: &dyn Fn(&Block) -> bool| doc.walk().filter(|block| is(block)).count();
    let runs = || doc.walk().flat_map(|block| block.body.runs());

    let nested = || {
        // Only what is one paragraph in Word can take a quote's style.
        let kept =
            |kind: &BlockKind| matches!(kind, BlockKind::Paragraph | BlockKind::ListItem { .. } | BlockKind::Formula);
        let boxes = doc.walk().filter(|block| matches!(block.kind, BlockKind::Quote | BlockKind::Callout { .. }));
        boxes.flat_map(|outer| outer.body.blocks()).filter(|inner| !kept(&inner.kind)).count()
    };
    let linked = |block: &Block| match &block.kind {
        BlockKind::Figure { asset: None, src: Some(src), .. } => to != Pdf || !loaded(src),
        _ => false,
    };
    let looks = |block: &Block| match (&block.kind, block.kind.callout_look()) {
        // Word draws a callout's tone and the line that heads it, and no icon.
        (_, Some(look)) if to == Docx => look.icon.is_some() || look.color.is_some(),
        (BlockKind::Callout { icon, color, .. }, _) => icon.is_some() || color.is_some(),
        _ => false,
    };
    let coded =
        |block: &Block| matches!(&block.kind, BlockKind::Code { language: Some(language) } if !language.is_empty());

    let open_conflict = |note: &Note| matches!(note.kind, NoteKind::Conflict { .. }) && !note.resolved;

    let counted = [
        // A clean copy holds the document as it is, so it loses only what is kept beside the document.
        (UnknownBlocks, to != Clean, blocks(&|block| matches!(block.kind, BlockKind::Unknown { .. }))),
        (
            UnknownMarks,
            to != Clean,
            runs().filter(|run| run.marks.iter().any(|mark| matches!(mark, Mark::Other(_)))).count(),
        ),
        (Comments, to != Docx, notes.notes.iter().filter(|note| note.kind == NoteKind::Comment).count()),
        (LinkedPictures, matches!(to, Html | Docx | Pdf), blocks(&linked)),
        (NestedBlocks, to == Docx, nested()),
        (CalloutLooks, matches!(to, Markdown | Docx), blocks(&looks)),
        (Galleries, to == Markdown, blocks(&|block| matches!(block.kind, BlockKind::Gallery { .. }))),
        (PageBreaks, to == Markdown, blocks(&|block| block.page_break)),
        (
            Languages,
            to == Markdown,
            blocks(&|block| block.lang.is_some()) + runs().filter(|run| run.lang.is_some()).count(),
        ),
        (CodeLanguages, to == Docx, blocks(&coded)),
        (History, to == Clean, 1),
        (Conflicts, to == Clean, notes.notes.iter().filter(|note| open_conflict(note)).count()),
    ];
    let lost = counted.into_iter().filter(|(_, applies, count)| *applies && *count > 0);
    lost.map(|(kind, _, count)| Loss { kind, count, to }).collect()
}

/// The loss as a line for a person, such as "3 galleries become pictures one after another". It says how
/// many, but for the history, of which a document has one.
impl fmt::Display for Loss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.count;
        let one = n == 1;
        let count = |single: &str, many: &str| format!("{n} {}", if one { single } else { many });
        match self.kind {
            LossKind::UnknownBlocks => {
                let blocks = count("block is", "blocks are");
                let shown = if self.to == Target::Pdf { "shown as a placeholder" } else { "written as fallback text" };
                write!(f, "{blocks} of a kind this version does not know, and {shown}")
            }
            LossKind::UnknownMarks => {
                write!(f, "formatting this version does not know is left out of {}", count("passage", "passages"))
            }
            LossKind::Comments => write!(f, "{} left out", count("comment is", "comments are")),
            LossKind::LinkedPictures => {
                let pictures = count("linked picture is", "linked pictures are");
                let shown = match (self.to, one) {
                    (Target::Pdf, true) => "shown as an empty frame",
                    (Target::Pdf, false) => "shown as empty frames",
                    (_, true) => "written as a link",
                    (_, false) => "written as links",
                };
                write!(f, "{pictures} not fetched, and {shown}")
            }
            LossKind::NestedBlocks => {
                let blocks = count("block inside a quote or callout is", "blocks inside quotes or callouts are");
                write!(f, "{blocks} written outside it, since Word cannot nest them")
            }
            LossKind::CalloutLooks if self.to == Target::Docx => {
                let callouts = count("callout loses its icon", "callouts lose their icons");
                write!(f, "{callouts} and any colour of {} own", if one { "its" } else { "their" })
            }
            LossKind::CalloutLooks => {
                write!(f, "{} own icon and colour", count("callout loses its", "callouts lose their"))
            }
            LossKind::Galleries => {
                write!(f, "{} pictures one after another", count("gallery becomes", "galleries become"))
            }
            LossKind::PageBreaks => write!(f, "{} left out", count("page break is", "page breaks are")),
            LossKind::Languages => {
                let passages = count("passage in another language is", "passages in another language are");
                write!(f, "{passages} no longer marked as such")
            }
            LossKind::CodeLanguages => {
                write!(f, "{} language", count("code block loses its", "code blocks lose their"))
            }
            LossKind::History => write!(f, "the history is left out"),
            LossKind::Conflicts => write!(f, "{} dropped", count("open conflict is", "open conflicts are")),
        }
    }
}
