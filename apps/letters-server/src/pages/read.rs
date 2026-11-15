//! A document as plain HTML: its whole text, pictures and maths in one page that runs nothing, for a
//! browser with no editor and for reading on a phone.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use letters_api::Action;
use letters_convert::{HtmlPage, WebFont, to_html_with};
use letters_fonts::{DISPLAY, MONO, SANS};
use letters_ui::{Scheme, palette};

use super::statics::{FACES, statics};
use super::{Opener, guarded, refused};
use crate::Shared;
use crate::api::blocking;
use crate::error::Refusal;
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

/// What the page may load: its own styles, the pictures written into it, and the server's fonts.
const POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src 'self'; \
                      form-action 'none'; base-uri 'none'; frame-ancestors 'none'";

/// `GET /d/{id}/read`.
pub async fn page(State(app): State<Shared>, Opener(who): Opener, Path(id): Path<String>) -> Response {
    let written = blocking({
        let (app, who) = (app.clone(), who.clone());
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
            let urls = FAMILIES.map(|(_, file, _)| format!("{}/{FACES}/{file}", statics().base));
            let fonts: Vec<WebFont> = FAMILIES
                .iter()
                .zip(&urls)
                .map(|((family, _, italic), url)| WebFont { family, url, italic: *italic })
                .collect();
            // Dark where the reader asks for it, in the colours the editor's own dark page has.
            let dark = palette(Scheme::Dark);
            let page = HtmlPage { dark: Some(&dark), fonts: &fonts, ..HtmlPage::default() };
            let mut assets = |name: &str| archive.read(name).ok().flatten();
            Ok(to_html_with(&document, &style, shown(found.place.name()), &mut assets, &page))
        }
    });
    match written.await {
        Ok(html) => (StatusCode::OK, guarded(POLICY), html).into_response(),
        Err(refusal) => refused(&app, Some(&who), &refusal).await,
    }
}
