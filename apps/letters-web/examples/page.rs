//! Writes the harness page as a server would send it: the template with the welcome document in it, as
//! HTML to read at once and as the editor opens it, under the shapes of the window's controls.
//!
//! `cargo run -p letters-web --example page -- www/index.html > dist/index.html`
//!
//! A `document.json` named after the template is shown in place of the welcome document.

use std::{env, fs, io};

use letters_model::{Document, ReplicaId};
use letters_ui::{Scheme, palette};
use letters_web::page::{self, Skeleton};

const TITLE: &str = "Letters in the browser";

fn main() -> io::Result<()> {
    let template = fs::read_to_string(env::args().nth(1).expect("the path of the page's template"))?;
    let mut blocks = 0;
    let document = match env::args().nth(2) {
        Some(path) => Document::from_json(&fs::read_to_string(path)?).map_err(io::Error::other)?,
        None => page::welcome(&mut || {
            blocks += 1;
            format!("welcome{blocks}")
        }),
    };
    let json = document.to_json();
    // The window as the editor will open it.
    let mut window = page::window(document.clone(), ReplicaId::new("page"));
    let style = window.style().clone();
    let html = letters_convert::to_html_themed(&document, &style, TITLE, &mut |_| None, &palette(Scheme::Dark));
    let skeleton = Skeleton::of(&mut window);
    print!("{}", page::compose(&template, &page::first_paint(&skeleton, &style, "fonts/", &html), &json));
    Ok(())
}
