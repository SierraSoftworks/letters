//! Writes the harness page as a server would send it: the template with the welcome document in it, as
//! HTML to read at once and as the editor opens it, under the shape of how the window's bar starts.
//!
//! `cargo run -p letters-web --example page -- www/index.html > dist/index.html`
//!
//! A `document.json` named after the template is shown in place of the welcome document.
//!
//! `cargo run -p letters-web --example page -- --served <base> <opened.json> [document.json]`
//!
//! writes the page a server sends for a document it keeps, whose files are under `base`: the browser
//! tests open it to stand where a server's person would.

use std::{env, fs, io};

use letters_api::Opened;
use letters_model::{Document, ReplicaId};
use letters_web::page::{self, Skeleton};

const TITLE: &str = "Letters in the browser";

/// The document at `path`, or the welcome document.
fn document(path: Option<String>) -> io::Result<Document> {
    let mut blocks = 0;
    Ok(match path {
        Some(path) => Document::from_json(&fs::read_to_string(path)?).map_err(io::Error::other)?,
        None => page::welcome(&mut || {
            blocks += 1;
            format!("welcome{blocks}")
        }),
    })
}

fn main() -> io::Result<()> {
    let mut given = env::args().skip(1);
    let first = given.next().expect("the path of the page's template, or --served");
    if first == "--served" {
        let base = given.next().expect("where the page's files are");
        let opened = fs::read_to_string(given.next().expect("the path of what the server says"))?;
        let opened: Opened = serde_json::from_str(&opened).map_err(io::Error::other)?;
        print!("{}", page::served(&document(given.next())?, &opened, &base));
        return Ok(());
    }
    let template = fs::read_to_string(first)?;
    let document = document(given.next())?;
    let json = document.to_json();
    // The window as the editor will open it.
    let mut window = page::window(document.clone(), ReplicaId::new("page"));
    let style = window.style().clone();
    let html = page::export(&document, &style, TITLE, "fonts/");
    let first = page::first_paint(&Skeleton::of(&mut window), &html);
    print!("{}", page::compose(&template, &page::preloads(&document, ""), &first, &json));
    Ok(())
}
