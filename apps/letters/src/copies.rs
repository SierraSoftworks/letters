//! Copies of a document: another document of its own, or the same text in another format.
//!
//! A copy is made from the document's file, which its window saves first. Nothing here replaces a document,
//! and a file in another format is replaced only when its writer has said so.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};

use letters_format::{Archive, HISTORY, NOTES};
use letters_model::{Document, Style};
use letters_ui::CopyFormat;

use crate::cli::Format;
use crate::{convert, files, platform};

/// Why a copy was not written. Shown to its writer as it reads.
#[derive(Debug)]
pub enum Refused {
    /// Nothing of what was typed can be a file name.
    NoName,
    /// The document has no file to copy, since nothing was written in it.
    Nothing,
    /// The version asked for is not in the document's history.
    NoVersion,
    /// A file of that name is in the folder already. A document is never replaced.
    Taken { name: String, format: CopyFormat },
    /// The document could not be saved first, for this reason, which is a sentence of its own.
    Unsaved(String),
    /// The system would not write it, or the document could not be read to make it.
    Failed(io::Error),
}

impl Refused {
    /// Whether the copy can be asked for again with the file that is in the way replaced.
    pub fn replaceable(&self) -> bool {
        matches!(self, Self::Taken { format, .. } if !document(*format))
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoName => f.write_str("A copy needs a name."),
            Self::Nothing => f.write_str("There is nothing to copy yet."),
            Self::NoVersion => f.write_str("That version could not be read."),
            Self::Taken { name, format } if document(*format) => {
                write!(f, "There is already a document called \u{201c}{name}\u{201d} in that folder.")
            }
            Self::Taken { name, .. } => write!(f, "\u{201c}{name}\u{201d} is already in that folder."),
            Self::Unsaved(why) => f.write_str(why),
            Self::Failed(error) => f.write_str(&files::unwritten("The copy", error)),
        }
    }
}

impl std::error::Error for Refused {}

/// A copy that could not be made from the document, whatever the folder it was for would have said: the
/// system's reason is none to give the writer, whose folder is not at fault.
fn unmade(error: &dyn fmt::Display) -> Refused {
    Refused::Failed(io::Error::other(error.to_string()))
}

/// Whether a copy in a format is a Letters document, which never takes the place of another.
fn document(format: CopyFormat) -> bool {
    matches!(format, CopyFormat::Letters | CopyFormat::Clean)
}

/// What a file in a format ends in.
pub fn extension(format: CopyFormat) -> &'static str {
    match exported(format) {
        Some(format) => format.extension(),
        None => files::EXTENSION,
    }
}

/// The format's name for a file dialog.
pub fn kind(format: CopyFormat) -> &'static str {
    match format {
        CopyFormat::Letters | CopyFormat::Clean => "Letters document",
        CopyFormat::Word => "Word document",
        CopyFormat::Pdf => "PDF",
        CopyFormat::WebPage => "Web page",
        CopyFormat::Markdown => "Markdown",
    }
}

/// The exporter that writes a format, which a document of its own needs none of.
fn exported(format: CopyFormat) -> Option<Format> {
    match format {
        CopyFormat::Letters => None,
        CopyFormat::Clean => Some(Format::Clean),
        CopyFormat::Word => Some(Format::Docx),
        CopyFormat::Pdf => Some(Format::Pdf),
        CopyFormat::WebPage => Some(Format::Html),
        CopyFormat::Markdown => Some(Format::Markdown),
    }
}

/// The file of a copy called `typed`, in `folder`.
pub fn path(folder: &Path, typed: &str, format: CopyFormat) -> Result<PathBuf, Refused> {
    let name = letters_format::file_name(typed).ok_or(Refused::NoName)?;
    Ok(folder.join(format!("{name}.{}", extension(format))))
}

/// A path the person chose in a file dialog, with the format's ending if they typed none.
pub fn ending(chosen: PathBuf, format: CopyFormat) -> PathBuf {
    let ending = extension(format);
    if chosen.extension().is_some_and(|typed| typed.eq_ignore_ascii_case(ending)) {
        return chosen;
    }
    let mut path = chosen.into_os_string();
    path.push(format!(".{ending}"));
    path.into()
}

/// The files of the linked pictures a window shows, by their addresses. A PDF shows them too. A copy
/// fetches nothing itself, so a linked picture the window does not show is an empty frame there.
pub type Linked = BTreeMap<String, Vec<u8>>;

/// Writes a copy of the document in the file at `source` to `output`, and returns the id of the document
/// it made, if it made one. `replace` lets a file in another format take the place of one at `output`.
pub fn write(
    source: &Path,
    output: &Path,
    format: CopyFormat,
    replace: bool,
    linked: &Linked,
) -> Result<Option<String>, Refused> {
    let file = fs::read(source).map_err(|error| unmade(&error))?;
    put(file, output, format, replace, linked)
}

/// Writes a copy of the document as an older version of it read: `version`, with the pictures the file
/// at `source` still holds. A document of its own starts with the version's text and has no history.
/// The document's comments stay behind, since they were made on what it reads now.
pub fn write_version(
    source: &Path,
    version: (Document, Style),
    output: &Path,
    format: CopyFormat,
    replace: bool,
    linked: &Linked,
) -> Result<Option<String>, Refused> {
    let failed = unmade;
    let file = fs::read(source).map_err(|error| failed(&error))?;
    let mut package =
        Archive::open(Cursor::new(file)).and_then(|mut file| file.package()).map_err(|error| failed(&error))?;
    let (document, style) = version;
    package.document = document;
    package.set_style(&style);
    package.entries.remove(NOTES);
    // An export names who wrote the document from its history, which a document of its own leaves behind.
    if format == CopyFormat::Letters {
        package.entries.retain(|name, _| !name.starts_with(HISTORY));
    }
    let mut bytes = Cursor::new(Vec::new());
    letters_format::write(&mut bytes, &package).map_err(|error| failed(&error))?;
    put(bytes.into_inner(), output, format, replace, linked)
}

/// Writes a copy of the document in `file`, the bytes of a `.let` file.
fn put(
    file: Vec<u8>,
    output: &Path,
    format: CopyFormat,
    replace: bool,
    linked: &Linked,
) -> Result<Option<String>, Refused> {
    let failed = unmade;
    let name = |path: &Path| path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let title = output.file_stem().unwrap_or_default().to_string_lossy();
    let taken = |path: &Path| match document(format) {
        true => Refused::Taken { name: title.to_string(), format },
        false => Refused::Taken { name: name(path), format },
    };
    let refused = |path: &Path, error: io::Error| match error.kind() {
        io::ErrorKind::AlreadyExists => taken(path),
        _ => Refused::Failed(error),
    };
    let Some(to) = exported(format) else {
        // A document of its own has its own id: with this one's, the two would pass for one document
        // that was written in two places.
        let mut package =
            Archive::open(Cursor::new(file)).and_then(|mut file| file.package()).map_err(|error| failed(&error))?;
        (package.id, package.created) = (platform::document_id(), platform::now_rfc3339());
        let mut bytes = Cursor::new(Vec::new());
        letters_format::write(&mut bytes, &package).map_err(|error| failed(&error))?;
        convert::write(output, bytes.get_ref(), false).map_err(|error| refused(output, error))?;
        return Ok(Some(package.id));
    };
    let mut linked = |address: &str| linked.get(address).cloned();
    let exported = convert::exported_with(file, &title, to, &mut linked).map_err(|error| failed(&error))?;
    for line in &exported.simplified {
        eprintln!("letters: {}: {line}", output.display());
    }
    exported.write(output, replace && !document(format)).map_err(|(path, error)| refused(&path, error))?;
    // A clean copy is a document of its own, which the shell is told of as it is of any copy.
    Ok(exported.id)
}
