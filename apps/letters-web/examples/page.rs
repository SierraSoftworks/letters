//! Writes the harness page as a server would send it: the template with the welcome document in it, as
//! HTML to read at once and as the editor opens it.
//!
//! `cargo run -p letters-web --example page -- www/index.html > dist/index.html`

use std::{env, fs, io};

use letters_model::Style;
use letters_web::page;

fn main() -> io::Result<()> {
    let template = fs::read_to_string(env::args().nth(1).expect("the path of the page's template"))?;
    let mut blocks = 0;
    let document = page::welcome(&mut || {
        blocks += 1;
        format!("welcome{blocks}")
    });
    let html = letters_convert::to_html(&document, &Style::default(), "Letters in the browser", &mut |_| None);
    print!("{}", page::compose(&template, &html, &document.to_json()));
    Ok(())
}
