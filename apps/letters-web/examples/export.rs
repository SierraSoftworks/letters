//! The exports as a module of their own, which the page fetches when the person first prints or saves a
//! copy.
//!
//! They bring a PDF writer, the Word and web page writers, the picture decoders and font subsetting: more
//! than two megabytes that the editor's first frame would otherwise wait for, and that most visits never
//! use.

use std::io::{Cursor, Write};

use js_sys::{Array, Date, Map, Uint8Array};
use letters_convert::{PdfInfo, markdown_assets, to_docx, to_html, to_markdown, to_pdf};
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
