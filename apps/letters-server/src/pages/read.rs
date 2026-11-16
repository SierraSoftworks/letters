//! A document as plain HTML: its whole text, pictures and maths in one page that runs nothing, for a
//! browser with no editor and for reading on a phone.
//!
//! It is also all that a link for a reader shows: the document as it stands, with nothing of its past
//! and no line to its room. What such a link does not let its reader take away, the page withholds as
//! far as a page can: the editor is not there to do it.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use letters_api::{Action, Allows, paths};
use letters_convert::{HtmlPage, WebFont, to_html_with};
use letters_fonts::{DISPLAY, MONO, SANS};
use letters_model::{Block, BlockKind, Body, Document};
use letters_ui::{Scheme, palette};

use super::statics::{FACES, statics};
use super::{Opener, escape, guarded, refused};
use crate::Shared;
use crate::api::blocking;
use crate::fetch::Fetched;
use crate::spaces::shown;

/// The families a document is set in, each with its file among [`letters_fonts::FILES`] and whether that
/// file is the family's italic.
const FAMILIES: [(&str, &str, bool); 5] = [
    (SANS, "Inter.ttf", false),
    (SANS, "Inter-Italic.ttf", true),
    (MONO, "FiraCode.ttf", false),
    (DISPLAY, "Montserrat.ttf", false),
    (DISPLAY, "Montserrat-Italic.ttf", true),
];

/// What a fetched picture goes by in the page's making, before its place among those fetched. No
/// entry of a file is named so: a file's pictures are under `assets/`.
const FETCHED: &str = "fetched/";

/// What the page may load: its own styles, the pictures written into it, and the server's fonts.
const POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src 'self'; \
                      form-action 'none'; base-uri 'none'; frame-ancestors 'none'";

/// The rules of a page whose text is not to be copied: nothing of the document is selected.
const NO_COPY: &str = "article { -webkit-user-select: none; user-select: none; }\n";
/// The rules of a page that is not to be printed: paper gets a line that says so, and no document.
const NO_PRINT: &str = "@media print { article { display: none; } body::before { display: block; padding: 2em; \
                        font: 12pt sans-serif; content: \"This link does not let its reader print the document.\"; } }\n";
/// The rules of the bar a link's visitor is offered the file in. It takes the page's own colours, and
/// paper gets none of it.
const BAR: &str = ".taken { display: flex; justify-content: flex-end; padding: 12px 16px 0; \
                   font: 500 13px/1.4 Inter, system-ui, sans-serif; }\n\
                   .taken a { padding: 5px 12px; border: 1px solid color-mix(in srgb, currentColor 22%, transparent); \
                   border-radius: 6px; color: inherit; text-decoration: none; opacity: 0.72; }\n\
                   .taken a:hover, .taken a:focus-visible { opacity: 1; }\n\
                   @media print { .taken { display: none; } }\n";

/// What the page of the document `id` adds to its text for whoever a link alone leads to it, since
/// they have no other page to take the file from: the elements above the document, and the rules that
/// come with them and that withhold what the link does not allow.
fn offered(id: &str, allows: Allows) -> (String, String) {
    let mut css = String::new();
    let above = match allows.download {
        true => {
            css.push_str(BAR);
            format!("<nav class=\"taken\"><a href=\"{}\" download>Download</a></nav>\n", escape(&paths::file(id)))
        }
        false => String::new(),
    };
    css.push_str(if allows.copy { "" } else { NO_COPY });
    css.push_str(if allows.print { "" } else { NO_PRINT });
    (above, css)
}

/// The addresses of the pictures the document links to and does not hold, each once.
fn linked(document: &Document) -> Vec<String> {
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

/// Makes each figure whose linked picture was fetched show it as a picture of its own, under the name
/// `named` gives the address.
fn show(blocks: &mut [Block], named: &dyn Fn(&str) -> Option<String>) {
    for block in blocks {
        if let BlockKind::Figure { asset: asset @ None, src: Some(src), .. } = &mut block.kind {
            *asset = named(src);
        }
        if let Body::Blocks(inner) = &mut block.body {
            show(inner, named);
        }
    }
}

/// `GET /d/{id}/read`.
pub async fn page(State(app): State<Shared>, Opener(who): Opener, Path(id): Path<String>) -> Response {
    let opened = blocking({
        let (app, who, id) = (app.clone(), who.clone(), id.clone());
        move || {
            let found = app.locate(&who, &id)?;
            // What is being written now is read too.
            app.rooms.save(&found.root, &found.place);
            let mut archive = app.library.open(&found.root, &found.place)?;
            let (document, style) = (archive.document()?, archive.style()?);
            app.note_opened(&who, &id)?;
            app.note(&who, &found, &id, Action::Open, false);
            let offered = if found.by_link() { offered(&id, found.allows) } else { Default::default() };
            Ok((archive, document, style, shown(found.place.name()).to_owned(), offered))
        }
    });
    let (mut archive, mut document, style, title, (above, css)) = match opened.await {
        Ok(opened) => opened,
        Err(refusal) => return refused(&app, Some(&who), &refusal).await,
    };
    // A picture that is still a link is fetched here where its source is trusted, so that its site
    // learns nothing of who reads. The page carries it as it carries the file's own.
    let fetched = app.linked_pictures(&who, &id, linked(&document)).await;
    let fetched: Vec<(String, Fetched)> = fetched.into_iter().collect();
    let written = blocking(move || {
        let name = |at: usize| format!("{FETCHED}{at}");
        show(&mut document.blocks, &|src| fetched.iter().position(|(url, _)| url == src).map(name));
        let urls = FAMILIES.map(|(_, file, _)| format!("{}/{FACES}/{file}", statics().base));
        let fonts: Vec<WebFont> = FAMILIES
            .iter()
            .zip(&urls)
            .map(|((family, _, italic), url)| WebFont { family, url, italic: *italic })
            .collect();
        // Dark where the reader asks for it, in the colours the editor's own dark page has.
        let dark = palette(Scheme::Dark);
        let page = HtmlPage { dark: Some(&dark), fonts: &fonts, css: &css, above: &above, ..HtmlPage::default() };
        let mut assets = |name: &str| match name.strip_prefix(FETCHED).and_then(|at| at.parse::<usize>().ok()) {
            Some(at) => fetched.get(at).map(|(_, picture)| picture.bytes.clone()),
            None => archive.read(name).ok().flatten(),
        };
        Ok(to_html_with(&document, &style, &title, &mut assets, &page))
    });
    match written.await {
        Ok(html) => (StatusCode::OK, guarded(POLICY), html).into_response(),
        Err(refusal) => refused(&app, Some(&who), &refusal).await,
    }
}
