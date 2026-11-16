//! A document in another format, written here: a PDF, a Word file, Markdown or one web page, for whoever
//! reads in a browser and has no app to write one with.
//!
//! The formats are `letters_convert`'s, as the desktop app writes them, and a PDF is laid out by the
//! layout the window draws with. What a format simplifies is told before it is asked for, on the pages
//! that offer it ([`Source::offers`]).

use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::path::Path as FilePath;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS,
};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use letters_api::{Action, Code, Export, paths};
use letters_convert::{
    Loss, PdfInfo, PdfSources, Target, losses_with, markdown_assets, to_docx, to_html, to_markdown, to_pdf_with,
};
use letters_format::ASSETS;
use letters_layout::{Fonts, Uncovered};
use letters_model::{BlockKind, Document, Notes, Style};
use letters_render::Font;
use letters_web::faces::{self, Face};
use tokio::sync::Semaphore;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::api::{Found, attached, blocking};
use crate::error::{Answer, Refusal};
use crate::fetch::Fetched;
use crate::pages::editor::FALLBACK;
use crate::session::{Reader, Signed};
use crate::spaces::shown;
use crate::telemetry::Event;
use crate::{App, Shared};

/// How many documents are written at once. A long one takes a processor for seconds and memory by its
/// size, and whoever asks next waits their turn.
const AT_ONCE: usize = 2;
static TURNS: Semaphore = Semaphore::const_new(AT_ONCE);

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/v1/documents/{id}/export/{format}", get(export))
}

/// A document's file, open to be read.
type File = letters_format::Archive<std::io::BufReader<std::fs::File>>;

/// A document as it is read to be written in another format, for one reader.
pub struct Source {
    pub(crate) file: File,
    pub document: Document,
    pub(crate) style: Style,
    /// Its comments, which a Word file carries.
    notes: Notes,
    /// Its name.
    pub(crate) title: String,
    /// Who wrote it, which a PDF says.
    authors: Vec<String>,
}

/// A format a page offers a document in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub to: Export,
    /// What a person calls it.
    pub label: &'static str,
    pub href: String,
    /// What it will simplify, a sentence each. None where it carries the document as it reads.
    pub simplified: Vec<String>,
}

/// What a document was written as: the name of the file to keep it in, what kind of file that is, and
/// the file.
struct Written {
    name: String,
    kind: &'static str,
    bytes: Vec<u8>,
}

/// The addresses of the pictures the document links to and does not hold, each once.
pub fn linked(document: &Document) -> Vec<String> {
    let mut urls = Vec::new();
    for block in document.walk() {
        if let BlockKind::Figure { asset: None, src: Some(src), .. } = &block.kind
            && !urls.contains(src)
        {
            urls.push(src.clone());
        }
    }
    urls
}

/// A loss as a sentence of its own.
fn sentence(loss: &Loss) -> String {
    let said = loss.to_string();
    let mut rest = said.chars();
    let first: String = rest.next().into_iter().flat_map(char::to_uppercase).collect();
    format!("{first}{}.", rest.as_str())
}

impl Source {
    /// Each format the document `id` is offered in, with what it will simplify. `fetched` says of a
    /// picture the document links to whether a PDF is handed it.
    pub fn offers(&self, id: &str, fetched: &dyn Fn(&str) -> bool) -> Vec<Offer> {
        let pictures = !markdown_assets(&self.document).is_empty();
        let offer = |to: Export| {
            let (label, target) = match to {
                Export::Pdf => ("PDF", Target::Pdf),
                Export::Docx => ("Word", Target::Docx),
                Export::Markdown if pictures => ("Markdown, zipped with its pictures", Target::Markdown),
                Export::Markdown => ("Markdown", Target::Markdown),
                Export::Html => ("Web page", Target::Html),
            };
            let lost = losses_with(&self.document, &self.notes, target, fetched);
            Offer { to, label, href: paths::export(id, to), simplified: lost.iter().map(sentence).collect() }
        };
        Export::ALL.map(offer).into()
    }

    /// Writes the document in a format. `linked` are the pictures that were fetched for it, each by its
    /// address, `now` is when, and `fallback` the folder of the faces that back the fonts up.
    fn written(
        mut self,
        to: Export,
        linked: &HashMap<String, Fetched>,
        now: u64,
        fallback: &FilePath,
    ) -> Answer<Written> {
        // A picture that cannot be read is treated as missing, and its description stands in for it.
        let mut assets = |name: &str| self.file.read(name).ok().flatten();
        let (ending, kind, bytes) = match to {
            Export::Html => {
                let page = to_html(&self.document, &self.style, &self.title, &mut assets);
                ("html", "text/html; charset=utf-8", page.into_bytes())
            }
            Export::Docx => {
                let file = to_docx(&self.document, &self.style, &self.notes, &self.title, &mut assets);
                ("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document", file)
            }
            Export::Pdf => {
                let fonts = Fonts { faces: letters_fonts::bundled() };
                let info = PdfInfo { title: self.title.clone(), authors: self.authors.clone(), created: Some(now) };
                let mut backed = |lacking: &[Uncovered]| backups(fallback, lacking);
                let mut fetched = |address: &str| linked.get(address).map(|picture| picture.bytes.clone());
                let sources = PdfSources { backups: Some(&mut backed), words: None, linked: Some(&mut fetched) };
                let written = to_pdf_with(&self.document, &self.style, fonts, &mut assets, &info, sources);
                let file = written.map_err(|error| {
                    crate::logs::warn(&format!("a document could not be written as a PDF: {error}"));
                    Refusal::new(Code::Internal, "The server could not write this document as a PDF.")
                })?;
                ("pdf", "application/pdf", file)
            }
            Export::Markdown => {
                let text = to_markdown(&self.document);
                // The text names each picture by its entry in the file, as a path from its own folder.
                // Only an entry among the file's pictures is written there, whatever the document names.
                let plain = |entry: &String| {
                    let name = entry.strip_prefix(ASSETS).unwrap_or_default();
                    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
                };
                let entries = markdown_assets(&self.document).into_iter().filter(plain);
                let pictures: Vec<_> = entries.filter_map(|entry| Some((assets(&entry)?, entry))).collect();
                match pictures.is_empty() {
                    true => ("md", "text/markdown; charset=utf-8", text.into_bytes()),
                    false => ("zip", "application/zip", zipped(&self.title, &text, &pictures)?),
                }
            }
        };
        Ok(Written { name: format!("{}.{ending}", self.title), kind, bytes })
    }
}

/// A document's Markdown and the pictures it names in one zip: the text under the document's name, and
/// each picture where the text looks for it.
fn zipped(title: &str, text: &str, pictures: &[(Vec<u8>, String)]) -> Answer<Vec<u8>> {
    let pack = || -> zip::result::ZipResult<Vec<u8>> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(format!("{title}.md"), SimpleFileOptions::default())?;
        zip.write_all(text.as_bytes())?;
        // A picture is pressed already.
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (bytes, entry) in pictures {
            zip.start_file(entry.as_str(), stored)?;
            zip.write_all(bytes)?;
        }
        Ok(zip.finish()?.into_inner())
    };
    pack().map_err(|_| Refusal::new(Code::Internal, "The server could not pack this document's Markdown."))
}

/// The faces in `folder` for the characters none of the fonts has, as the editor in a browser chooses
/// among the same files. A face is read when a character first asks for it, and one that is not there
/// leaves its characters to the next that has them, or as boxes.
fn backups(folder: &FilePath, uncovered: &[Uncovered]) -> Fonts {
    let mut read: Vec<(Face, Option<Arc<[u8]>>)> = Vec::new();
    loop {
        let file = |face: Face| read.iter().find(|(read, _)| *read == face).map(|(_, file)| file);
        let lacks = |face: Face, ch: char| match file(face) {
            Some(Some(file)) => !faces::has(file, ch),
            Some(None) => true,
            None => false,
        };
        let asked: Vec<Face> = read.iter().map(|(face, _)| *face).collect();
        let told = uncovered.iter().map(|one| (one.ch, one.emoji, one.lang.as_deref()));
        let chosen = faces::chosen(told, &asked, lacks);
        let unread = faces::lacking(&chosen, &asked);
        if unread.is_empty() {
            // Every face chosen was read, in the order the characters first need them.
            let used = faces::lacking(&chosen, &[]).into_iter().filter_map(|face| file(face).cloned().flatten());
            return Fonts { faces: used.map(|file| Font::new(file, 0)).collect() };
        }
        // Each round reads a face that was not read before, and there are only so many.
        read.extend(unread.into_iter().map(|face| (face, std::fs::read(folder.join(face.file())).ok().map(Arc::from))));
    }
}

impl App {
    /// Reads the document that was found, as whoever found it takes it away. The notes and the names
    /// are of how the document came to be: whoever has no more than a link gets neither, as with the
    /// file itself.
    pub fn source(&self, found: &Found) -> Answer<Source> {
        // What is being written now is read too.
        self.rooms.save(&found.root, &found.place);
        let mut file = self.library.open(&found.root, &found.place)?;
        let document = file.document()?;
        // A style this build cannot read gives way to the built-in one. Notes that cannot be read are
        // left behind: the text matters more than its comments.
        let style = file.style().unwrap_or_default();
        let (notes, authors) = match found.by_link() {
            true => Default::default(),
            false => {
                let known = self.row(&found.root, &found.place).ok().and_then(|row| row.known);
                (file.notes().unwrap_or_default(), known.map(|known| known.contributors).unwrap_or_default())
            }
        };
        Ok(Source { file, document, style, notes, title: shown(found.place.name()).to_owned(), authors })
    }

    /// Each format the document that was found is offered in to `who`, with what it will simplify.
    pub fn offers(&self, who: &Signed, found: &Found, id: &str) -> Answer<Vec<Offer>> {
        let source = self.source(found)?;
        let fetched = self.fetched_unasked(who, id, linked(&source.document));
        Ok(source.offers(id, &|address| fetched.iter().any(|fetched| fetched == address)))
    }

    /// Reads the document `id` for `who` to take away in another format, and notes that they asked.
    fn exported(&self, who: &Signed, id: &str) -> Answer<Source> {
        let found = self.locate(who, id)?;
        self.note(who, &found, id, Action::Download, !found.allows.download);
        if !found.allows.download {
            return Err(Refusal::forbidden("This link does not let its reader take the document away."));
        }
        self.source(&found)
    }
}

/// `GET`: the document in another format, as a file to keep.
async fn export(
    State(app): State<Shared>,
    Reader(who): Reader,
    Path((id, format)): Path<(String, String)>,
) -> Answer<Response> {
    let to = Export::named(&format).ok_or_else(Refusal::not_found)?;
    app.exporting(&who)?;
    // The turns are never closed, so there is always one to wait for.
    let turn = TURNS.acquire().await.ok();
    let source = blocking({
        let (app, who, id) = (app.clone(), who.clone(), id.clone());
        move || app.exported(&who, &id)
    })
    .await?;
    // A PDF shows a picture that is still a link where the page to read shows it: fetched here from a
    // source that is trusted. Every other format writes the link.
    let linked = match to {
        Export::Pdf => app.linked_pictures(&who, &id, linked(&source.document)).await,
        _ => HashMap::new(),
    };
    let (now, fallback) = (app.now(), app.config.server.state.join(FALLBACK));
    let written = blocking(move || source.written(to, &linked, now, &fallback)).await?;
    drop(turn);
    app.telemetry.record(Event::Exported(to));
    let headers = [
        (CONTENT_TYPE, written.kind.to_owned()),
        (CONTENT_DISPOSITION, attached(&written.name)),
        // The document may be another tomorrow, and what one reader may take is not another's.
        (CACHE_CONTROL, "no-store".to_owned()),
        (X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        // A web page of the document is a file to keep. A browser that shows it here all the same runs
        // nothing of it and sends nothing from it.
        (CONTENT_SECURITY_POLICY, "sandbox; default-src 'none'; style-src 'unsafe-inline'; img-src data:".to_owned()),
    ];
    Ok((headers, written.bytes).into_response())
}
