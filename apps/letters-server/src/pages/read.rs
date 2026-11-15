//! A document as plain HTML: its whole text, pictures and maths in one page that runs nothing, for a
//! browser with no editor and for reading on a phone.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use letters_api::Action;
use letters_convert::{HtmlPage, WebFont, to_html_with};
use letters_fonts::{DISPLAY, MONO, SANS};
use letters_model::{Block, BlockKind, Body, Document};
use letters_ui::{Scheme, palette};

use super::statics::{FACES, statics};
use super::{Opener, guarded, refused};
use crate::Shared;
use crate::api::blocking;
use crate::error::Refusal;
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
            // The page is the document's text, for a browser to copy, print and keep as it likes.
            if !(found.allows.copy && found.allows.print) {
                let editor =
                    "This link shows the document in the editor alone: its text is not to be copied or printed.";
                return Err(Refusal::forbidden(editor));
            }
            // What is being written now is read too.
            app.rooms.save(&found.root, &found.place);
            let mut archive = app.library.open(&found.root, &found.place)?;
            let (document, style) = (archive.document()?, archive.style()?);
            app.note_opened(&who, &id)?;
            app.note(&who, &found, &id, Action::Open, false);
            Ok((archive, document, style, shown(found.place.name()).to_owned()))
        }
    });
    let (mut archive, mut document, style, title) = match opened.await {
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
        let page = HtmlPage { dark: Some(&dark), fonts: &fonts, ..HtmlPage::default() };
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
