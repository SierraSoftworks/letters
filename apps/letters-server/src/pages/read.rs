//! A document as plain HTML: its whole text, pictures and maths in one page that runs nothing, for a
//! browser with no editor and for reading on a phone.
//!
//! It is also all that a link for a reader shows: the document as it stands, with nothing of its past
//! and no line to its room. What such a link does not let its reader take away, the page withholds as
//! far as a page can: the editor is not there to do it.
//!
//! Above the text the page offers the document to take away: its file, and each format the server
//! writes it in, with what that format will simplify.

use std::fmt::Write as _;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use letters_api::{Action, Allows, paths};
use letters_convert::{HtmlPage, WebFont, to_html_with};
use letters_fonts::{DISPLAY, MONO, SANS};
use letters_model::{Block, BlockKind, Body};
use letters_ui::{Scheme, palette};

use super::statics::{FACES, statics};
use super::{Opener, escape, guarded, refused};
use crate::Shared;
use crate::api::blocking;
use crate::export::{Offer, linked};
use crate::fetch::Fetched;

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
/// The rules of what the document is offered to take away in: a button that opens a list, in the page's
/// own colours. The list is no list of the document's, whose rules it would take. Paper gets none of it.
const BAR: &str = ".taken { padding: 12px 16px 0; font: 500 13px/1.4 Inter, system-ui, sans-serif; text-align: end; }\n\
                   .taken summary { display: inline-block; padding: 5px 12px; border-radius: 6px; cursor: pointer; \
                   border: 1px solid color-mix(in srgb, currentColor 22%, transparent); opacity: 0.72; list-style: none; }\n\
                   .taken summary::-webkit-details-marker { display: none; }\n\
                   .taken summary:hover, .taken summary:focus-visible, .taken[open] summary { opacity: 1; }\n\
                   .taken nav { margin-top: 4px; }\n\
                   .taken nav div { padding: 4px 0; }\n\
                   .taken a { color: inherit; }\n\
                   .taken small { display: block; font-weight: 400; font-size: 12px; opacity: 0.64; }\n\
                   @media print { .taken { display: none; } }\n";

/// What the page of the document `id` adds to its text for what its reader may take away: the elements
/// above the document, which offer its file and each of `offers`, and the rules that come with them and
/// that withhold what a link does not allow.
fn offered(id: &str, allows: Allows, offers: &[Offer]) -> (String, String) {
    let (mut above, mut css) = (String::new(), String::new());
    if allows.download {
        css.push_str(BAR);
        above.push_str("<details class=\"taken\"><summary>Download</summary>\n<nav aria-label=\"Formats\">\n");
        let _ = writeln!(above, "<div><a href=\"{}\" download>Letters file</a></div>", escape(&paths::file(id)));
        for offer in offers {
            let _ = write!(above, "<div><a href=\"{}\" download>{}</a>", escape(&offer.href), escape(offer.label));
            for said in &offer.simplified {
                let _ = write!(above, "<small>{}</small>", escape(said));
            }
            above.push_str("</div>\n");
        }
        above.push_str("</nav>\n</details>\n");
    }
    css.push_str(if allows.copy { "" } else { NO_COPY });
    css.push_str(if allows.print { "" } else { NO_PRINT });
    (above, css)
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
            let source = app.source(&found)?;
            app.note_opened(&who, &id)?;
            app.note(&who, &found, &id, Action::Open, false);
            Ok((source, found.allows))
        }
    });
    let (mut source, allows) = match opened.await {
        Ok(opened) => opened,
        Err(refusal) => return refused(&app, Some(&who), &refusal).await,
    };
    // A picture that is still a link is fetched here where its source is trusted, so that its site
    // learns nothing of who reads. The page carries it as it carries the file's own.
    let fetched = app.linked_pictures(&who, &id, linked(&source.document)).await;
    let fetched: Vec<(String, Fetched)> = fetched.into_iter().collect();
    let written = blocking(move || {
        // A PDF is handed the linked pictures this page shows, so those are none of what it simplifies.
        let shows = |address: &str| fetched.iter().any(|(url, _)| url == address);
        let offers = if allows.download { source.offers(&id, &shows) } else { Vec::new() };
        let (above, css) = offered(&id, allows, &offers);
        let name = |at: usize| format!("{FETCHED}{at}");
        show(&mut source.document.blocks, &|src| fetched.iter().position(|(url, _)| url == src).map(name));
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
            None => source.file.read(name).ok().flatten(),
        };
        Ok(to_html_with(&source.document, &source.style, &source.title, &mut assets, &page))
    });
    match written.await {
        Ok(html) => (StatusCode::OK, guarded(POLICY), html).into_response(),
        Err(refusal) => refused(&app, Some(&who), &refusal).await,
    }
}
