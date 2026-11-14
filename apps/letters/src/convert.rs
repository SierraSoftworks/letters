//! The `export` and `import` subcommands: documents to and from other formats, with no window.
//!
//! - [`export`] writes one document as Markdown or as a single HTML file.
//! - [`import`] makes a document beside each Markdown file and prints a line for each.
//!
//! Neither changes a file that is already there: an export asks for `--force` first, and an import skips.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use letters_convert::{ImportNote, from_markdown, to_html, to_markdown};
use letters_format::{Archive, Package};
use letters_model::{Anchor, Block, BlockKind, Body, Document, Mark, Note, NoteKind, Notes, Run, Style};
use serde_json::json;

use crate::cli::{Export, Format, Import, Output};
use crate::{files, platform};

/// The extensions of the files an import looks for in a folder.
const MARKDOWN: [&str; 2] = ["md", "markdown"];
/// The name import notes are written under.
const IMPORTER: &str = "Letters import";

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
    let mut archive = Archive::open(Cursor::new(bytes)).map_err(|error| failed(&error))?;
    let document = archive.document().map_err(|error| failed(&error))?;
    // An export says what it will simplify before it writes.
    for line in simplified(&document, export.to) {
        eprintln!("letters: {name}: {line}");
    }
    let text = match export.to {
        Format::Markdown => to_markdown(&document),
        Format::Html => {
            // A style this build cannot read gives way to the built-in one, as it does on screen.
            let style = archive.style().unwrap_or_default();
            let title = export.file.file_stem().unwrap_or_default().to_string_lossy();
            // An image that cannot be read is treated as missing, and its description stands in for it.
            to_html(&document, &style, &title, &mut |name| archive.read(name).ok().flatten())
        }
    };

    let output = match &export.output {
        Some(Output::Stdout) => return io::stdout().write_all(text.as_bytes()).map_err(|error| error.to_string()),
        Some(Output::File(path)) => path.clone(),
        None => export.file.with_extension(export.to.extension()),
    };
    write(&output, text.as_bytes(), export.force).map_err(|error| match error.kind() {
        io::ErrorKind::AlreadyExists => {
            format!("{} is already there: pass --force to replace it, or --output to write elsewhere", output.display())
        }
        _ => format!("{}: {error}", output.display()),
    })?;
    println!("{name} -> {}", output.display());
    Ok(())
}

/// What the format cannot carry over, a line for each kind of loss.
fn simplified(document: &Document, to: Format) -> Vec<String> {
    let count = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    // HTML already writes figures, which the model has yet to learn.
    let known = |name: &str| to == Format::Html && name == "figure";
    let lost = |block: &&Block| matches!(&block.kind, BlockKind::Unknown { type_name } if !known(type_name));
    let unknown = document.walk().filter(lost).count();
    let other = |run: &&Run| run.marks.iter().any(|mark| matches!(mark, Mark::Other(_)));
    let marks = document.walk().flat_map(|block| block.body.runs()).filter(other).count();
    let mut lines = Vec::new();
    if unknown > 0 {
        let blocks = count(unknown, "block is", "blocks are");
        lines.push(format!("{blocks} of a kind this version does not know, and written as fallback text"));
    }
    if marks > 0 {
        lines.push(format!(
            "formatting this version does not know is left out of {}",
            count(marks, "passage", "passages")
        ));
    }
    lines
}

/// Writes a file. Unless `replace` is set, one that is already there is left alone and reported.
fn write(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
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
    let mut sources = BTreeSet::new();
    found.retain(|(path, error)| error.is_some() || sources.insert(normal(path)));
    if found.is_empty() {
        eprintln!("letters: no Markdown files to import");
    }

    let style = files::new_style(std::env::home_dir().as_deref());
    let mut stdout = io::stdout().lock();
    let mut failures = false;
    for (source, error) in found {
        let outcome = match error {
            Some(error) => Outcome::Failed { error },
            None => import_file(&source, &style, &sources),
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

/// Lists what a path names: itself, or for a folder the Markdown files in it, by name, before those of the
/// folders inside it. Hidden files and folders are passed over, and a linked folder is not followed.
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
    for file in files.iter().map(|entry| entry.path()).filter(|file| is_markdown(file) && file.is_file()) {
        found.push((file, None));
    }
    for folder in folders.iter().filter(|_| recursive) {
        find(&folder.path(), recursive, found);
    }
}

fn is_markdown(path: &Path) -> bool {
    let extension = path.extension().unwrap_or_default();
    MARKDOWN.iter().any(|known| extension.eq_ignore_ascii_case(known))
}

/// A path from the root with its `.` and `..` worked out, so that two spellings of one file compare equal.
fn normal(path: &Path) -> PathBuf {
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

/// Makes a document beside one Markdown file. `sources` holds every file of this import, by [`normal`] path.
fn import_file(source: &Path, style: &Style, sources: &BTreeSet<PathBuf>) -> Outcome {
    let failed = |error: &dyn std::fmt::Display| Outcome::Failed { error: error.to_string() };
    let output = source.with_extension(files::EXTENSION);
    if output.exists() {
        return Outcome::Skipped { output };
    }
    let markdown = match fs::read_to_string(source) {
        Ok(markdown) => markdown,
        Err(error) => return failed(&error),
    };
    let (mut document, simplified) = from_markdown(&markdown, &mut platform::block_id);
    if document.blocks.is_empty() {
        document = files::blank();
    }
    relink(&mut document.blocks, source.parent().unwrap_or(Path::new("")), sources);

    let created = platform::now_rfc3339();
    let mut package =
        Package { id: platform::document_id(), created: created.clone(), document, entries: Default::default() };
    package.set_style(style);
    if !simplified.is_empty() {
        let notes = simplified.iter().map(|note| import_note(note, &created)).collect();
        package.set_notes(&Notes { notes, ..Notes::default() });
    }
    let mut bytes = Cursor::new(Vec::new());
    if let Err(error) = letters_format::write(&mut bytes, &package) {
        return failed(&error);
    }
    match write(&output, bytes.get_ref(), false) {
        Ok(()) => Outcome::Imported { output, notes: simplified.len() },
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Outcome::Skipped { output },
        Err(error) => failed(&error),
    }
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
