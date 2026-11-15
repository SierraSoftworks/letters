//! The exports and the reader of a document's file as a module of their own, which the page fetches when
//! the person first prints, saves a copy or opens a file.
//!
//! They bring a PDF writer, the Word and web page writers, the picture decoders, font subsetting and the
//! document's own file format: more than two megabytes that the editor's first frame would otherwise wait
//! for, and that most visits never use.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use js_sys::{Array, Date, Map, Uint8Array};
use letters_convert::{PdfInfo, markdown_assets, to_docx, to_html, to_markdown, to_pdf};
use letters_format::{ASSETS, Archive, Error, MIMETYPE, OPLOG, Package};
use letters_layout::Fonts;
use letters_model::{Document, Notes, Style};
use wasm_bindgen::prelude::*;
use zip::result::ZipResult;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

fn files(files: &Array) -> Vec<Vec<u8>> {
    files.iter().map(|file| Uint8Array::new(&file).to_vec()).collect()
}

/// Hands over the fonts and the maths faces, each in the order its crate names its files. Called once,
/// before the first [`exported`].
#[wasm_bindgen]
pub fn fonts(fonts: &Array, maths: &Array) {
    letters_fonts::provide(files(fonts));
    letters_math::provide_fonts(files(maths));
}

/// Writes a document in another format, from its `document.json`, `style.json` and `notes.json`. `format`
/// is what such a file's name ends in: `pdf`, `docx`, `html` or `md`. `assets` holds the pictures the
/// document shows, by their names in it. `created` is in seconds since 1970 began.
///
/// The format `let` is the document's own file: it takes its `history`, which is what `Model::save`
/// writes, and its `identity`, which is its id and when it was created as RFC 3339. A copy is given an
/// identity of its own, so that the two never pass for one document written in two places.
///
/// Returns the file's name, its media type and its bytes. The name is `title` as a file can be called,
/// and `undefined` when nothing of the title can name a file.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments, reason = "the arguments cross from the page one by one")]
pub fn exported(
    format: &str,
    title: &str,
    document: &str,
    style: &str,
    notes: &str,
    authors: Vec<String>,
    created: Option<f64>,
    assets: &Map,
    history: Option<Vec<u8>>,
    identity: Option<Vec<String>>,
) -> Result<Array, JsError> {
    let (document, style) = (Document::from_json(document)?, Style::from_json(style)?);
    let mut asset =
        |name: &str| Some(Uint8Array::new(&assets.get(&name.into())).to_vec()).filter(|file| !file.is_empty());
    let name = letters_format::file_name(title);
    let (file, ending, kind) = match format {
        "pdf" => {
            let info = PdfInfo { title: title.to_owned(), authors, created: created.map(|seconds| seconds as u64) };
            let fonts = Fonts { faces: letters_fonts::bundled() };
            (to_pdf(&document, &style, fonts, &mut asset, &info)?, "pdf", "application/pdf")
        }
        "docx" => (
            to_docx(&document, &style, &Notes::from_json(notes)?, title, &mut asset),
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        "html" => (to_html(&document, &style, title, &mut asset).into_bytes(), "html", "text/html"),
        "md" => {
            let text = to_markdown(&document).into_bytes();
            // The Markdown names each picture by a path from its own folder. A name that climbs out of
            // that folder is not written, whatever the document says it holds.
            let inside = |entry: &String| entry.split(['/', '\\']).all(|part| !matches!(part, "" | "." | ".."));
            let pictures = markdown_assets(&document).into_iter().filter(inside);
            let pictures: Vec<_> = pictures.filter_map(|entry| Some((asset(&entry)?, entry))).collect();
            match pictures.is_empty() {
                true => (text, "md", "text/markdown"),
                // A page can hand over one file, so the pictures that belong beside the text come with it.
                false => {
                    let text = (text, format!("{}.md", name.as_deref().unwrap_or_default()));
                    (zipped(std::iter::once(text).chain(pictures))?, "zip", "application/zip")
                }
            }
        }
        "let" => {
            let Some([id, created]) = identity.and_then(|identity| <[String; 2]>::try_from(identity).ok()) else {
                return Err(JsError::new("the document came without its id"));
            };
            let mut package = Package { id, created, document, entries: BTreeMap::new() };
            package.set_style(&style);
            package.set_notes(&Notes::from_json(notes)?);
            let history = history.ok_or_else(|| JsError::new("the document came without its history"))?;
            package.entries.insert(OPLOG.into(), history);
            assets.for_each(&mut |file, name| {
                let file = Uint8Array::new(&file).to_vec();
                package.entries.extend(name.as_string().map(|name| (name, file)));
            });
            let mut file = Cursor::new(Vec::new());
            letters_format::write(&mut file, &package)?;
            (file.into_inner(), "let", MIMETYPE)
        }
        _ => return Err(JsError::new("there is no such format")),
    };
    let name = name.map(|name| format!("{name}.{ending}"));
    Ok(Array::of3(&name.into(), &kind.into(), &Uint8Array::from(&file[..])))
}

/// Files in one archive, each under its name and dated now.
fn zipped(files: impl Iterator<Item = (Vec<u8>, String)>) -> ZipResult<Vec<u8>> {
    let now = Date::new_0();
    let part = |part: u32| part as u8;
    let (day, time) = ((part(now.get_month() + 1), part(now.get_date())), (now.get_hours(), now.get_minutes()));
    // An archive keeps its files' dates as the person's own clock shows them.
    let dated = DateTime::from_date_and_time(now.get_full_year() as u16, day.0, day.1, part(time.0), part(time.1), 0);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(dated.unwrap_or_default());
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (file, name) in files {
        zip.start_file(name, options)?;
        zip.write_all(&file)?;
    }
    Ok(zip.finish()?.into_inner())
}

/// Reads a `.let` file for the editor to open. Returns its `document.json`, `style.json` and `notes.json`
/// as this build writes them, its history as `Model::prepare` takes it, its pictures by their names, and
/// its identity as [`exported`] takes it.
///
/// A style or notes that this build cannot read are `undefined`, as a newer build's may be, and so is the
/// history of a file that has none.
#[wasm_bindgen]
pub fn opened(file: Vec<u8>) -> Result<Array, JsError> {
    let mut file = Archive::open(Cursor::new(file))?;
    let document = file.document()?.to_json();
    let style = match file.style() {
        Err(Error::Style(_)) => None,
        style => Some(style?.to_json()),
    };
    let notes = match file.notes() {
        Err(Error::Notes(_)) => None,
        notes => Some(notes?.to_json()),
    };
    let history = file.read(OPLOG)?.map(|history| Uint8Array::from(&history[..]));
    let assets = Map::new();
    let names: Vec<String> = file.names().filter(|name| name.starts_with(ASSETS)).map(str::to_owned).collect();
    for name in names {
        // A picture that cannot be read is treated as missing, and its description stands in for it.
        if let Ok(Some(picture)) = file.read(&name) {
            assets.set(&name.into(), &Uint8Array::from(&picture[..]));
        }
    }
    let identity = Array::of2(&file.manifest().id.as_str().into(), &file.manifest().created.as_str().into());
    let read: [JsValue; 6] =
        [document.into(), style.into(), notes.into(), history.into(), assets.into(), identity.into()];
    Ok(read.into_iter().collect())
}
