//! The `export` and `import` subcommands: documents to and from other formats, with no window.
//!
//! - [`export`] writes one document as Markdown, as a single HTML file, as a Word document or as a PDF.
//!   Markdown cannot hold a picture, so the document's pictures are written beside it.
//! - [`import`] makes a document beside each Markdown or Word file and prints a line for each. The pictures
//!   a Markdown file shows come along.
//!
//! Neither changes a file that is already there: an export asks for `--force` first, and an import skips.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use letters_convert::{
    Linked, Loss, PdfInfo, PdfSources, Target, losses_with, markdown_assets, to_docx, to_html, to_markdown, to_pdf_with,
};
use letters_format::{Archive, OPLOG};
use letters_layout::{Fonts, Uncovered};
use letters_model::{Block, Body, Model, ReplicaId, Style};
use serde_json::json;

use crate::cli::{Export, Format, Import, Output};
use crate::fallback;
use crate::files;
use crate::import::{Choices, Plan, normal};
use crate::trust::{self, Trusted};

/// The extensions of the Markdown files an import looks for in a folder.
pub(crate) const MARKDOWN: [&str; 2] = ["md", "markdown"];
/// The extension of the Word files it looks for.
pub(crate) const WORD: &str = "docx";
/// Runs `letters export`.
pub fn export(export: &Export) -> ExitCode {
    match try_export(export) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("letters: {message}");
            ExitCode::FAILURE
        }
    }
}

fn try_export(export: &Export) -> Result<(), String> {
    let name = export.file.display();
    let failed = |error: &dyn std::fmt::Display| format!("{name}: {error}");
    let bytes = fs::read(&export.file).map_err(|error| failed(&error))?;
    let title = export.file.file_stem().unwrap_or_default().to_string_lossy();
    let exported = exported(bytes, &title, export.to).map_err(|error| failed(&error))?;
    // An export says what it simplified before it writes.
    for line in &exported.simplified {
        eprintln!("letters: {name}: {line}");
    }
    let output = match &export.output {
        Some(Output::Stdout) => return io::stdout().write_all(&exported.bytes).map_err(|error| error.to_string()),
        Some(Output::File(path)) => path.clone(),
        None => export.file.with_extension(export.to.extension()),
    };
    exported.write(&output, export.force).map_err(|(path, error)| match error.kind() {
        io::ErrorKind::AlreadyExists => {
            format!("{} is already there: pass --force to replace it, or --output to write elsewhere", path.display())
        }
        _ => format!("{}: {error}", path.display()),
    })?;
    println!("{name} -> {}", output.display());
    Ok(())
}

/// A document as another format holds it.
pub struct Exported {
    /// The one file the format writes.
    pub bytes: Vec<u8>,
    /// What goes beside that file: the pictures a Markdown file shows, each by its path from the file's
    /// folder. The other formats hold their own.
    pub pictures: Vec<(String, Vec<u8>)>,
    /// What the format could not carry over, a line for each kind of loss.
    pub simplified: Vec<String>,
}

/// Writes the document in `file`, the bytes of a `.let` file, in another format. `title` is what the
/// document is called where the format has a place for a name.
pub fn exported(file: Vec<u8>, title: &str, to: Format) -> Result<Exported, String> {
    exported_with(file, title, to, &mut |_| None)
}

/// As [`exported`], for a window that has loaded pictures its document only links to. `linked` hands over
/// the file an address led to, as it was fetched, and a PDF then shows that picture as the window does.
/// An export fetches nothing itself, so any other linked picture is an empty frame there.
pub fn exported_with(file: Vec<u8>, title: &str, to: Format, linked: Linked) -> Result<Exported, String> {
    let failed = |error: letters_format::Error| error.to_string();
    let mut archive = Archive::open(Cursor::new(file)).map_err(failed)?;
    let document = archive.document().map_err(failed)?;
    // A style this build cannot read gives way to the built-in one, as it does on screen.
    let style = archive.style().unwrap_or_default();
    // Notes that cannot be read are left behind: the text matters more than its comments.
    let notes = archive.notes().unwrap_or_default();
    let target = match to {
        Format::Markdown => Target::Markdown,
        Format::Html => Target::Html,
        Format::Docx => Target::Docx,
        Format::Pdf => Target::Pdf,
    };
    // The addresses of the linked pictures a PDF was handed.
    let mut shown = BTreeSet::new();
    let bytes = match to {
        Format::Markdown => to_markdown(&document).into_bytes(),
        // An image that cannot be read is treated as missing, and its description stands in for it.
        Format::Html => to_html(&document, &style, title, &mut |name| archive.read(name).ok().flatten()).into_bytes(),
        Format::Docx => to_docx(&document, &style, &notes, title, &mut |name| archive.read(name).ok().flatten()),
        Format::Pdf => {
            let fonts = Fonts { faces: letters_fonts::bundled() };
            let created = SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|since| since.as_secs());
            // Who wrote a document is in its history. A file with none, or with one this build cannot read,
            // names nobody.
            let history = archive.read(OPLOG).ok().flatten();
            let model = history.and_then(|history| Model::load(&history, ReplicaId::new("export")).ok());
            let authors = model.map(|model| model.authors().into_iter().collect()).unwrap_or_default();
            let info = PdfInfo { title: title.to_owned(), authors, created };
            // What the bundled fonts cannot draw is set in the system's faces, as the window sets it. The
            // system's fonts are read only when a page has such a character.
            let mut assets = |name: &str| archive.read(name).ok().flatten();
            let mut backups = |lacking: &[Uncovered]| fallback::system_fonts(lacking);
            let mut linked = |address: &str| {
                let picture = linked(address);
                if picture.is_some() {
                    shown.insert(address.to_owned());
                }
                picture
            };
            let sources = PdfSources { backups: Some(&mut backups), words: None, linked: Some(&mut linked) };
            to_pdf_with(&document, &style, fonts, &mut assets, &info, sources).map_err(|error| error.to_string())?
        }
    };
    // The Markdown names each picture by its entry in the file, as a path from its own folder. A name
    // that climbs out of that folder is not written, whatever the file says it holds.
    let inside = |entry: &String| Path::new(entry).components().all(|part| matches!(part, Component::Normal(_)));
    let entries = markdown_assets(&document).into_iter().filter(|_| to == Format::Markdown).filter(inside);
    let pictures = entries.filter_map(|entry| Some((archive.read(&entry).ok().flatten()?, entry)));
    let pictures = pictures.map(|(bytes, entry)| (entry, bytes)).collect();
    let lost = losses_with(&document, &notes, target, &|address| shown.contains(address));
    Ok(Exported { bytes, pictures, simplified: lost.iter().map(Loss::to_string).collect() })
}

impl Exported {
    /// Writes the file at `output`, and what goes with it beside it. Unless `replace` is set, a file
    /// already at `output` is left alone and reported as `AlreadyExists`. A failure names the path it
    /// was at.
    pub fn write(&self, output: &Path, replace: bool) -> Result<(), (PathBuf, io::Error)> {
        write(output, &self.bytes, replace).map_err(|error| (output.to_owned(), error))?;
        let folder = output.parent().unwrap_or(Path::new(""));
        for (entry, bytes) in &self.pictures {
            let picture = folder.join(entry);
            let made = fs::create_dir_all(picture.parent().unwrap_or(folder));
            match made.and_then(|()| write(&picture, bytes, replace)) {
                // A picture is named after what it holds, so one that is there already is this one.
                Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err((picture, error)),
                _ => {}
            }
        }
        Ok(())
    }
}

/// Writes a file. Unless `replace` is set, one that is already there is left alone and reported.
pub(crate) fn write(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true);
    if replace {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let written = options.open(path)?.write_all(bytes);
    if written.is_err() {
        // Half a file is worse than none.
        let _ = fs::remove_file(path);
    }
    written
}

/// What became of one file of an import.
enum Outcome {
    Imported {
        output: PathBuf,
        notes: usize,
    },
    /// The document was already there.
    Skipped {
        output: PathBuf,
    },
    Failed {
        error: String,
    },
}

/// Runs `letters import`. Fails if any file could not be imported.
pub fn import(import: &Import) -> ExitCode {
    // Each file found, or the reason a folder could not be searched.
    let mut found: Vec<(PathBuf, Option<String>)> = Vec::new();
    for path in &import.paths {
        find(path, import.recursive, &mut found);
    }
    // Pictures come from what was asked for: each folder named, the folder of each file named, and any
    // folder added for them.
    let folder = |path: &PathBuf| if path.is_dir() { normal(path) } else { normal(path.parent().unwrap_or(path)) };
    let roots: Vec<PathBuf> =
        import.paths.iter().map(folder).chain(import.pictures.iter().map(|path| normal(path))).collect();
    let mut sources = BTreeSet::new();
    found.retain(|(path, error)| error.is_some() || sources.insert(normal(path)));
    if found.is_empty() {
        eprintln!("letters: no Markdown or Word files to import");
    }

    let home = std::env::home_dir();
    let style = files::new_style(home.as_deref());
    // Each import is recorded where the person's trust is kept. With no home there is no such place, and
    // then nothing is trusted at all.
    let mut trust = home.as_deref().map(trust::store);
    // Settings that cannot be read trust nothing, and each file then says why it could not be recorded.
    let _ = trust.as_mut().map(Trusted::load);
    let mut stdout = io::stdout().lock();
    let mut failures = false;
    for (source, error) in found {
        let outcome = match error {
            Some(error) => Outcome::Failed { error },
            None => import_file(&source, &style, &sources, &roots, trust.as_mut()),
        };
        failures |= matches!(outcome, Outcome::Failed { .. });
        let line = if import.json { json_line(&source, &outcome) } else { text_line(&source, &outcome) };
        // A closed pipe ends the report, not the import.
        let _ = writeln!(stdout, "{line}");
    }
    if failures { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn text_line(source: &Path, outcome: &Outcome) -> String {
    let source = source.display();
    match outcome {
        Outcome::Imported { output, notes: 0 } => format!("{source} -> {}", output.display()),
        Outcome::Imported { output, notes: 1 } => format!("{source} -> {} (1 note)", output.display()),
        Outcome::Imported { output, notes } => format!("{source} -> {} ({notes} notes)", output.display()),
        Outcome::Skipped { output } => format!("{source}: skipped, {} is already there", output.display()),
        Outcome::Failed { error } => format!("{source}: failed, {error}"),
    }
}

fn json_line(source: &Path, outcome: &Outcome) -> String {
    let source = source.to_string_lossy();
    let line = match outcome {
        Outcome::Imported { output, notes } => {
            json!({ "source": source, "status": "imported", "output": output.to_string_lossy(), "notes": notes })
        }
        Outcome::Skipped { output } => {
            json!({ "source": source, "status": "skipped", "output": output.to_string_lossy() })
        }
        Outcome::Failed { error } => json!({ "source": source, "status": "failed", "error": error }),
    };
    line.to_string()
}

/// Lists what a path names: itself, or for a folder the Markdown and Word files in it, by name, before those
/// of the folders inside it. Hidden files and folders are passed over, and a linked folder is not followed.
fn find(path: &Path, recursive: bool, found: &mut Vec<(PathBuf, Option<String>)>) {
    if !path.is_dir() {
        // A file named outright is imported whatever it is called. If it is missing, reading it says so.
        found.push((path.into(), None));
        return;
    }
    let entries = fs::read_dir(path).and_then(|entries| entries.collect::<io::Result<Vec<_>>>());
    let mut entries = match entries {
        Ok(entries) => entries,
        Err(error) => return found.push((path.into(), Some(error.to_string()))),
    };
    entries.retain(|entry| !entry.file_name().to_string_lossy().starts_with('.'));
    entries.sort_by_key(|entry| entry.file_name());
    let (folders, files): (Vec<_>, Vec<_>) =
        entries.into_iter().partition(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()));
    let wanted = |file: &PathBuf| (is_markdown(file) || is_word(file)) && file.is_file();
    for file in files.iter().map(|entry| entry.path()).filter(wanted) {
        found.push((file, None));
    }
    for folder in folders.iter().filter(|_| recursive) {
        find(&folder.path(), recursive, found);
    }
}

pub(crate) fn is_markdown(path: &Path) -> bool {
    let extension = path.extension().unwrap_or_default();
    MARKDOWN.iter().any(|known| extension.eq_ignore_ascii_case(known))
}

pub(crate) fn is_word(path: &Path) -> bool {
    path.extension().unwrap_or_default().eq_ignore_ascii_case(WORD)
}

/// Makes a document beside one Markdown or Word file. `sources` holds every file of this import, by
/// [`normal`] path, and `roots` the folders its pictures may be read from.
///
/// The import is recorded in `trust` before the document is written: unless the file lies in a folder the
/// person trusts, the document opens in safe mode until they trust it.
fn import_file(
    source: &Path,
    style: &Style,
    sources: &BTreeSet<PathBuf>,
    roots: &[PathBuf],
    trust: Option<&mut Trusted>,
) -> Outcome {
    let failed = |error: &dyn std::fmt::Display| Outcome::Failed { error: error.to_string() };
    let output = source.with_extension(files::EXTENSION);
    // A document is a zip too, so this comes before the file is looked into.
    if output.exists() {
        return Outcome::Skipped { output };
    }
    let choices = Choices { pictures: roots.to_vec(), comments: true };
    let plan = match Plan::of(source, style) {
        Ok(plan) => plan,
        Err(error) => return failed(&error),
    };
    if let Some(Err(error)) = trust.map(|trust| trust::import(trust, &plan.id, source)) {
        return failed(&format!("the import could not be recorded, so nothing was written: {error}"));
    }
    let mut imported = plan.import(&choices);
    relink(&mut imported.package.document.blocks, source.parent().unwrap_or(Path::new("")), sources);
    let mut bytes = Cursor::new(Vec::new());
    if let Err(error) = letters_format::write(&mut bytes, &imported.package) {
        return failed(&error);
    }
    match write(&output, bytes.get_ref(), false) {
        Ok(()) => Outcome::Imported { output, notes: imported.notes },
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Outcome::Skipped { output },
        Err(error) => failed(&error),
    }
}

/// Points the links between imported files at the documents made from them.
fn relink(blocks: &mut [Block], folder: &Path, sources: &BTreeSet<PathBuf>) {
    for block in blocks {
        match &mut block.body {
            Body::Text(runs) => {
                for run in runs {
                    if let Some(document) = run.link.as_deref().and_then(|url| retarget(url, folder, sources)) {
                        run.link = Some(document);
                    }
                }
            }
            Body::Blocks(children) => relink(children, folder, sources),
            Body::None => {}
        }
    }
}

/// Where a link should point once the Markdown file it names is a document, or `None` to leave it alone.
///
/// `folder` is where the link was written. The file must be one of this import or have its document already.
/// A wiki link names a file without its extension, so one is not asked for.
fn retarget(url: &str, folder: &Path, sources: &BTreeSet<PathBuf>) -> Option<String> {
    let (path, rest) = url.split_at(url.find(['#', '?']).unwrap_or(url.len()));
    // A scheme, a drive or a rooted path points outside the notes.
    if path.is_empty() || path.starts_with(['/', '\\']) || path.contains(':') {
        return None;
    }
    let dotted = MARKDOWN.iter().map(|extension| format!(".{extension}"));
    let cut = dotted.filter(|ending| path.len() > ending.len()).find_map(|ending| {
        let at = path.len() - ending.len();
        path.get(at..).filter(|end| end.eq_ignore_ascii_case(&ending)).map(|_| at)
    });
    let stem = &path[..cut.unwrap_or(path.len())];
    let named = |ending: &str| normal(&folder.join(format!("{}{ending}", decode(stem))));
    let known = MARKDOWN.iter().any(|extension| sources.contains(&named(&format!(".{extension}"))))
        || named(&format!(".{}", files::EXTENSION)).is_file();
    known.then(|| format!("{stem}.{}{rest}", files::EXTENSION))
}

/// Undoes the percent-encoding of an address, which is how a link spells a space.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = bytes.get(at + 1..at + 3).and_then(|hex| std::str::from_utf8(hex).ok());
        match hex.filter(|_| bytes[at] == b'%').and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
            Some(byte) => {
                out.push(byte);
                at += 3;
            }
            None => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
