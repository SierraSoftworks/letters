//! Writes the word lists of the scripts written without spaces into a folder, each under the name the
//! editor fetches it by. The editor leaves them out of its wasm, and fetches one when a document needs it.
//!
//! `cargo run -p letters-web --example words -- dist/words`

use std::path::PathBuf;
use std::{env, fs, io};

use letters_layout::WordList;

fn main() -> io::Result<()> {
    let folder = PathBuf::from(env::args().nth(1).expect("the folder to write the lists in"));
    fs::create_dir_all(&folder)?;
    WordList::ALL.into_iter().try_for_each(|list| fs::write(folder.join(list.file()), list.data()))
}
