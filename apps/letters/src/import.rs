//! Bringing a Markdown or Word file in as a document: for the `import` subcommand, and for a window that is
//! asked to open one.
//!
//! - [`Plan::of`] reads the file and says what it would bring, for a person to choose from. It reads the
//!   whole file, and converts a Word file there and then, so a window calls it off its own thread.
//! - [`Plan::import`] makes the document those [`Choices`] give. It writes nothing: the package is the
//!   caller's to save.
//!
//! A Markdown file names its pictures by path, so which folders they are read from is a choice. A file from
//! elsewhere could otherwise name a picture anywhere on this machine and carry it off in the document.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use letters_convert::{DocxImport, ImportNote, from_docx, import_markdown};
use letters_format::Package;
use letters_model::{Anchor, Document, Note, NoteKind, Notes, Run, Style};

use crate::library::free_path;
use crate::{convert, files, platform};

/// How a zip starts, and how the older kind of Word file does. Either is read as Word, whatever its name.
const WORD_STARTS: [&[u8]; 2] = [b"PK\x03\x04", &[0xd0, 0xcf, 0x11, 0xe0]];
/// The largest picture an import reads, in bytes. The reader keeps none larger.
const LARGEST_PICTURE: u64 = 64 << 20;
/// The name import notes are written under.
const IMPORTER: &str = "Letters import";

/// The endings of the files a window brings in when it is asked to open them.
pub const ENDINGS: [&str; 3] = [convert::MARKDOWN[0], convert::MARKDOWN[1], convert::WORD];

/// Whether `path` is named as a file to bring in and not as a document: a window opens the one and offers
/// to import the other.
pub fn offered(path: &Path) -> bool {
    convert::is_markdown(path) || convert::is_word(path)
}

/// What kind of file is being brought in. It is told from what the file holds, not from its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Markdown,
    Word,
}

/// A folder that holds pictures a Markdown file names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PictureFolder {
    pub path: PathBuf,
    /// How many of the file's pictures are in it.
    pub pictures: usize,
    /// Whether it is the file's own folder or one inside it. Such a folder came with the file, so its
    /// pictures are brought in unless the person says otherwise. Any other is theirs to add.
    pub beside: bool,
}

/// What a person chose to bring in with a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choices {
    /// The folders a Markdown file's pictures are read from, each with the folders inside it. A picture
    /// anywhere else stays a link.
    pub pictures: Vec<PathBuf>,
    /// Whether a Word file's comments come along.
    pub comments: bool,
}

/// A file made into a document.
#[derive(Clone, Debug)]
pub struct Imported {
    pub package: Package,
    /// How many passages the import simplified. Each carries a note that says how.
    pub notes: usize,
}

/// How the card that offers a file names a folder its pictures are in, so that the folder's own name is
/// there to read: from the file's folder on when it is that folder or inside it (`notes / img`), by the
/// short climb from it when it is near (`../shared`), and from the home folder or the root otherwise.
pub fn shown(folder: &Path, beside: &Path, home: Option<&Path>) -> String {
    let parts =
        |path: &Path, between: &str| path.iter().map(|part| part.to_string_lossy()).collect::<Vec<_>>().join(between);
    let outer = beside.parent().unwrap_or(Path::new(""));
    if folder.starts_with(beside) {
        return parts(folder.strip_prefix(outer).unwrap_or(folder), " / ");
    }
    // One or two folders up, and down again from there. A folder the file's own lies in is not near in
    // that sense: a climb alone would not name it.
    let near = beside.ancestors().enumerate().skip(1).take(2).filter(|_| !beside.starts_with(folder));
    let climbed = near.filter(|(_, above)| above.parent().is_some()).find_map(|(up, above)| {
        let down = folder.strip_prefix(above).ok().filter(|down| !down.as_os_str().is_empty())?;
        Some(format!("{}{}", "../".repeat(up), parts(down, "/")))
    });
    let from_home = || {
        let inside = folder.strip_prefix(home?).ok()?;
        Some(if inside.as_os_str().is_empty() { "~".to_owned() } else { format!("~/{}", parts(inside, "/")) })
    };
    climbed.or_else(from_home).unwrap_or_else(|| folder.display().to_string())
}

impl Imported {
    /// Keeps the document as a new file in `folder`, called `name` or by the next name after it that is
    /// free, and returns where. No file is replaced.
    pub fn keep(&self, folder: &Path, name: &str) -> Result<PathBuf, String> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        letters_format::write(&mut bytes, &self.package).map_err(|error| error.to_string())?;
        fs::create_dir_all(folder).map_err(|error| error.to_string())?;
        // A name that was free may be taken by the time it is written: the next one is tried then.
        for _ in 0..8 {
            let path = free_path(folder, name);
            match convert::write(&path, bytes.get_ref(), false) {
                Ok(()) => return Ok(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Err(format!("no name like {name} is free in {}", folder.display()))
    }
}

/// A file that can be brought in, and what there is to choose about it.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The id the document will have. It is known before the document is made so that the import can be
    /// recorded first, with `trust::import`: only the folder the file lies in vouches for what it holds.
    pub id: String,
    /// What to call the document: the file's name without its extension.
    pub name: String,
    pub kind: Kind,
    /// Where the pictures a Markdown file names are, in order of path. A Word file holds its own.
    pub folders: Vec<PictureFolder>,
    /// How many pictures the file names that are not there. They stay links.
    pub missing: usize,
    /// How many comments a Word file carries.
    pub comments: usize,
    /// The file's folder, which the paths in it start from.
    source: PathBuf,
    folder: PathBuf,
    created: String,
    content: Content,
}

#[derive(Clone, Debug)]
enum Content {
    /// The source, which is read again once its pictures are chosen, and the style its document takes.
    Markdown(String, Box<Style>),
    /// A Word file as it was converted, which no choice changes but for its comments.
    Word(Box<DocxImport>),
}

impl Plan {
    /// Reads `source`. Fails, in words for the person, on a file that is neither Markdown nor a Word document.
    ///
    /// `style` is the person's own, which the document starts with. A Word file's styles are laid over it,
    /// so that the document looks as it did.
    pub fn of(source: &Path, style: &Style) -> Result<Self, String> {
        let bytes = fs::read(source).map_err(|error| error.to_string())?;
        let name = source.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let folder = normal(source.parent().unwrap_or(Path::new("")));
        let (id, created) = (platform::document_id(), platform::now_rfc3339());
        if WORD_STARTS.iter().any(|start| bytes.starts_with(start)) {
            let word =
                from_docx(&bytes, style, &created, &mut platform::block_id).map_err(|error| error.to_string())?;
            let (comments, content) = (word.comments.len(), Content::Word(Box::new(word)));
            return Ok(Self {
                id,
                name,
                kind: Kind::Word,
                folders: Vec::new(),
                missing: 0,
                comments,
                source: source.to_owned(),
                folder,
                created,
                content,
            });
        }
        let Ok(markdown) = String::from_utf8(bytes) else {
            return Err("the file is neither Markdown in UTF-8 nor a Word document".into());
        };
        // A read that is handed no picture finds out which ones the file names.
        let mut named = Vec::new();
        import_markdown(&markdown, &mut String::new, &mut |path| {
            named.push(normal(&folder.join(path)));
            None
        });
        named.sort();
        named.dedup();
        let (mut found, mut missing) = (BTreeMap::<PathBuf, usize>::new(), 0);
        for file in named {
            match file.parent().filter(|_| size(&file).is_some()) {
                Some(folder) => *found.entry(folder.into()).or_default() += 1,
                None => missing += 1,
            }
        }
        let folders = found.into_iter().map(|(path, pictures)| {
            let beside = path.starts_with(&folder);
            PictureFolder { path, pictures, beside }
        });
        let content = Content::Markdown(markdown, Box::new(style.clone()));
        Ok(Self {
            id,
            name,
            kind: Kind::Markdown,
            folders: folders.collect(),
            missing,
            comments: 0,
            source: source.to_owned(),
            folder,
            created,
            content,
        })
    }

    /// What is brought in when nobody says otherwise: the pictures that came with the file, and every comment.
    pub fn choices(&self) -> Choices {
        let beside = self.folders.iter().filter(|folder| folder.beside);
        Choices { pictures: beside.map(|folder| folder.path.clone()).collect(), comments: true }
    }

    /// Makes the document.
    pub fn import(self, choices: &Choices) -> Imported {
        let Self { id, folder, created, content, .. } = self;
        let mut package =
            Package { id, created: created.clone(), document: Document::default(), entries: Default::default() };
        let mut notes = Vec::new();
        let simplified = match content {
            Content::Word(word) => {
                package.document = word.document;
                package.set_style(&word.style);
                package.entries.extend(word.assets);
                // People's comments are not what the import counts as its notes.
                notes.extend(word.comments.into_iter().filter(|_| choices.comments));
                word.notes
            }
            Content::Markdown(markdown, style) => {
                let roots: Vec<PathBuf> = choices.pictures.iter().map(|folder| normal(folder)).collect();
                let mut picture = |path: &str| picture(&normal(&folder.join(path)), &roots);
                let read = import_markdown(&markdown, &mut platform::block_id, &mut picture);
                package.document = read.document;
                package.entries.extend(read.assets);
                package.set_style(&style);
                read.notes
            }
        };
        if package.document.blocks.is_empty() {
            package.document = files::blank();
        }
        notes.extend(simplified.iter().map(|note| import_note(note, &created)));
        if !notes.is_empty() {
            package.set_notes(&Notes { notes, ..Notes::default() });
        }
        Imported { package, notes: simplified.len() }
    }

    /// The file, as it was named.
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// The file's folder, with its `.` and `..` worked out.
    pub fn folder(&self) -> &Path {
        &self.folder
    }
}

/// The size of a file that could be a picture: one that is there and no larger than an import reads.
fn size(file: &Path) -> Option<u64> {
    let size = fs::metadata(file).ok().filter(|about| about.is_file())?.len();
    (size <= LARGEST_PICTURE).then_some(size)
}

/// Reads a picture, if it is in one of the folders `roots` or below one.
fn picture(file: &Path, roots: &[PathBuf]) -> Option<Vec<u8>> {
    size(file).filter(|_| roots.iter().any(|root| file.starts_with(root)))?;
    fs::read(file).ok()
}

/// The note that marks, on its block, something the import simplified.
fn import_note(note: &ImportNote, created: &str) -> Note {
    Note {
        id: platform::block_id(),
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

/// A path from the root with its `.` and `..` worked out, so that two spellings of one file compare equal.
pub fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in std::path::absolute(path).unwrap_or_else(|_| path.into()).components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => _ = out.pop(),
            part => out.push(part),
        }
    }
    out
}
