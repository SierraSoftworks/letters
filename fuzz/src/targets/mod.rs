//! The targets, one module each.

mod container;
mod docx;
mod editor;
mod export;
mod files;
mod history;
mod html;
mod journal;
mod journal_file;
mod json;
mod layout;
mod markdown;
mod maths;
mod model;
mod save;
mod spell;
mod trust;
#[cfg(feature = "shell")]
mod windows;

use crate::Target;

pub const TARGETS: &[Target] = &[
    Target {
        name: "container",
        about: "a .let file opens or is refused, and one that opens saves over itself and merges with itself",
        run: container::run,
        seeds: container::seeds,
    },
    Target {
        name: "docx",
        about: "a Word file imports into a sound document or is refused, and is written back as one that reads",
        run: docx::run,
        seeds: docx::seeds,
    },
    Target {
        name: "editor",
        about: "two writers typing, pasting and sharing keep a sound document and end on the same one",
        run: editor::run,
        seeds: editor::seeds,
    },
    Target {
        name: "export",
        about: "any document is written as a PDF, a Word file that reads back, and a web page",
        run: export::run,
        seeds: export::seeds,
    },
    Target {
        name: "files",
        about: "every reader of a file takes any bytes or refuses them",
        run: files::run,
        seeds: files::seeds,
    },
    Target {
        name: "history",
        about: "a history loads and merges or is refused, and a refusal changes nothing",
        run: history::run,
        seeds: history::seeds,
    },
    Target {
        name: "html",
        about: "any HTML reads into a sound document, and what Letters writes of it reads back",
        run: html::run,
        seeds: html::seeds,
    },
    Target {
        name: "journal",
        about: "what reached the journal before a crash is in the document the next session opens",
        run: journal::run,
        seeds: journal::seeds,
    },
    Target {
        name: "journal_file",
        about: "a journal file is read and replayed or refused, and a journal that opens on it goes on",
        run: journal_file::run,
        seeds: journal_file::seeds,
    },
    Target {
        name: "json",
        about: "document.json, style.json and notes.json are read or refused, and read back as written",
        run: json::run,
        seeds: json::seeds,
    },
    Target {
        name: "layout",
        about: "any document is laid out at any width, painted, paged and hit tested",
        run: layout::run,
        seeds: layout::seeds,
    },
    Target {
        name: "markdown",
        about: "any Markdown imports into a sound document that survives a round trip",
        run: markdown::run,
        seeds: markdown::seeds,
    },
    Target {
        name: "maths",
        about: "any TeX is typeset or refused, and what is typeset can be drawn and written",
        run: maths::run,
        seeds: maths::seeds,
    },
    Target {
        name: "model",
        about: "writers who have seen the same edits show the same document, whatever the edits and their order",
        run: model::run,
        seeds: model::seeds,
    },
    Target {
        name: "save",
        about: "writers saving one file to a shared folder lose nothing and end on what the file holds",
        run: save::run,
        seeds: save::seeds,
    },
    Target {
        name: "spell",
        about: "what is marked as misspelled lies on whole characters of its text",
        run: spell::run,
        seeds: spell::seeds,
    },
    Target {
        name: "trust",
        about: "the trust settings and the addresses a document names are read or refused, and written back the same",
        run: trust::run,
        seeds: trust::seeds,
    },
    #[cfg(feature = "shell")]
    Target {
        name: "windows",
        about: "windows of two copies of the app never share a journal, and unsaved work comes back after a crash",
        run: windows::run,
        seeds: windows::seeds,
    },
];
