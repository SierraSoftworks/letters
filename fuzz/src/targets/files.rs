//! The bytes of any file, handed to everything that reads one: each reader takes it or refuses it.

use std::io::Cursor;

use letters_convert::from_docx;
use letters_format::{Archive, Unsaved, WorkingCopy, journal_name};
use letters_model::{Model, Style};

use crate::checks::ids;
use crate::folder::Folder;
use crate::input::bounded;

pub fn run(data: &[u8]) {
    let data = bounded(data);
    if let Ok(mut archive) = Archive::open(Cursor::new(data.to_vec())) {
        let _ = archive.package();
        let _ = WorkingCopy::open(&mut archive, 1.into());
    }
    let _ = from_docx(data, &Style::default(), "2026-11-15T00:00:00Z", &mut ids());
    let _ = Model::load(data, 1.into());
    let (mut folder, name) = (Folder::default(), journal_name("fuzz"));
    folder.put(&name, data.to_vec());
    let _ = Unsaved::read(&mut folder, &name);
}

pub fn seeds() -> Vec<Vec<u8>> {
    let report: &[u8] = include_bytes!("../../../crates/letters-format/corpus/report.let");
    let mut seeds = vec![report.to_vec(), b"LETJ1\n".to_vec(), b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1".to_vec()];
    seeds.extend(super::history::seeds().into_iter().take(2));
    seeds
}
