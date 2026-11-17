//! The faces of the fonts a writer chose: kept with the document, fetched from the library for one who
//! writes, and taken from the system's own for the rest.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::Scratch;
use letters::chosen::{Ask, Chosen, Fetch, Found, asset_name};
use letters_format::Package;
use letters_layout::Named;
use letters_model::{Document, Style, StyleFont};
use letters_ui::FontSource;

/// A family of the library, and one that only a system has: a font of the formulas.
const LIBRARY: &str = "Lora";
const SYSTEM: &str = "KaTeX_Main";

/// A font file that stands for whatever the library serves.
const FILE: &[u8] = include_bytes!("../../../crates/letters-math/fonts/KaTeX_AMS-Regular.ttf");

/// A folder that holds the system's family, in its four faces.
fn system(name: &str) -> Scratch {
    let scratch = Scratch::new(name);
    let fonts = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/letters-math/fonts");
    for face in ["Regular", "Italic", "Bold", "BoldItalic"] {
        let file = format!("KaTeX_Main-{face}.ttf");
        std::fs::copy(format!("{fonts}/{file}"), scratch.0.join(&file)).unwrap();
    }
    scratch
}

/// A library that answers as Google's does, and counts what it is asked. One that is `down` answers
/// nothing.
fn library(down: bool) -> (Fetch, Arc<AtomicUsize>) {
    let asked = Arc::new(AtomicUsize::new(0));
    let count = asked.clone();
    let fetch = move |address: &str| {
        count.fetch_add(1, Ordering::Relaxed);
        match address {
            _ if down => None,
            sheet if sheet.starts_with("https://fonts.googleapis.com/css2?") => {
                let face = sheet.replace(|c: char| !c.is_ascii_alphanumeric(), "");
                Some(
                    format!("@font-face {{ src: url(https://fonts.gstatic.com/s/{face}.ttf) format('truetype'); }}")
                        .into_bytes(),
                )
            }
            face if face.starts_with("https://fonts.gstatic.com/s/") => Some(FILE.to_vec()),
            _ => None,
        }
    };
    (Box::new(fetch), asked)
}

/// A style whose body is set in `family`.
fn style(family: &str) -> Style {
    let mut style = Style::default();
    style.types.get_mut("body").unwrap().family = family.into();
    style
}

/// What a window asks for a document in `style` that may be written, with what it has stored so far.
fn written(style: &Style, stored: &[(String, Vec<u8>)]) -> Ask {
    let faces = letters_layout::faces(style);
    let bundled = |face: &Named| letters::fallback::BUNDLED.contains(&face.family.as_str());
    Ask {
        named: faces.iter().filter(|face| !bundled(face)).cloned().collect(),
        listed: style.fonts(),
        unsaved: stored.iter().cloned().collect(),
        file: None,
        keep: Some(faces),
    }
}

/// The same for one who only reads it, from the file at `file`.
fn read(style: &Style, file: Option<PathBuf>) -> Ask {
    Ask { keep: None, unsaved: HashMap::new(), file, ..written(style, &[]) }
}

/// The style as it is once the window has listed what was found.
fn kept(mut style: Style, found: &Found) -> Style {
    style.set_fonts(found.listed.as_deref().expect("there is something new to list"));
    style
}

#[test]
fn a_writer_can_choose_from_the_library_and_the_system_but_not_the_bundled_families_again() {
    let fonts = system("chosen-offers");
    let chosen = Chosen::of(&[&fonts.0], None, library(true).0);
    let offers = chosen.offers();
    let from = |family: &str| offers.iter().find(|offer| offer.family == family).map(|offer| offer.source);
    assert_eq!(from(LIBRARY), Some(FontSource::Library));
    assert_eq!(from(SYSTEM), Some(FontSource::Device));
    assert_eq!(from("Inter"), Some(FontSource::Library), "the window leaves out what is bundled by itself");
}

#[test]
fn a_family_of_the_library_is_fetched_once_and_kept_with_the_document() {
    let (fetch, asked) = library(false);
    let mut chosen = Chosen::of(&[], None, fetch);
    let style = style(LIBRARY);

    let found = chosen.answer(&written(&style, &[]));
    assert_eq!(found.stored.len(), 4, "the body and bold in it, upright and slanted");
    assert!(found.stored.iter().all(|(name, bytes)| *name == asset_name(bytes) && bytes == FILE));
    assert_eq!(found.faces.len(), 4, "and the text is set in them");
    assert!(found.lacking.is_empty() && found.unfetched.is_empty());
    let style = kept(style, &found);
    let listed: Vec<(u16, bool)> = style.fonts().iter().map(|font| (font.weight, font.italic)).collect();
    assert_eq!(listed, [(400, false), (400, true), (700, false), (700, true)]);
    assert!(style.fonts().iter().all(|font| font.family == LIBRARY && style.assets().contains(font.asset.as_str())));

    // Asked again as the document stands now, there is nothing to fetch, store or list.
    let fetches = asked.load(Ordering::Relaxed);
    let again = chosen.answer(&written(&style, &found.stored));
    assert_eq!((again.stored.len(), again.listed, again.faces.len()), (0, None, 4));
    assert_eq!(asked.load(Ordering::Relaxed), fetches);
}

#[test]
fn what_was_fetched_is_kept_on_the_device_for_the_next_document() {
    let kept = Scratch::new("chosen-kept");
    let mut online = Chosen::of(&[], Some(kept.0.clone()), library(false).0);
    assert_eq!(online.answer(&written(&style(LIBRARY), &[])).stored.len(), 4);

    let mut offline = Chosen::of(&[], Some(kept.0.clone()), library(true).0);
    let found = offline.answer(&written(&style(LIBRARY), &[]));
    assert_eq!((found.stored.len(), found.unfetched.len()), (4, 0), "a new document gets them with no network");
}

#[test]
fn a_reader_is_given_what_the_document_holds_and_fetches_nothing() {
    let scratch = Scratch::new("chosen-reader");
    let (fetch, asked) = library(false);
    let mut chosen = Chosen::of(&[], None, fetch);

    // The document as a writer left it, in a file.
    let found = Chosen::of(&[], None, library(false).0).answer(&written(&style(LIBRARY), &[]));
    let style = kept(style(LIBRARY), &found);
    let mut package = Package {
        id: "d".into(),
        created: "2026-11-17T00:00:00Z".into(),
        document: Document::default(),
        entries: found.stored.iter().cloned().collect(),
    };
    package.set_style(&style);
    let file = scratch.0.join("letter.let");
    letters_format::write(std::fs::File::create(&file).unwrap(), &package).unwrap();

    let held = chosen.answer(&read(&style, Some(file)));
    assert_eq!((held.faces.len(), held.stored.len(), held.listed), (4, 0, None));
    assert!(held.lacking.is_empty());

    // One that names a family and holds no face of it, as a writer with no network leaves it.
    let bare = chosen.answer(&read(&self::style(LIBRARY), None));
    assert_eq!((bare.faces.len(), bare.lacking), (0, vec![LIBRARY.to_owned()]));
    assert_eq!(asked.load(Ordering::Relaxed), 0, "nothing was asked of the library");
}

#[test]
fn a_family_of_the_system_is_set_in_its_faces_and_never_stored() {
    let fonts = system("chosen-system");
    let mut chosen = Chosen::of(&[&fonts.0], None, library(false).0);
    // Who asks, and the families nothing has for them.
    let cases = [("a writer", written(&style(SYSTEM), &[])), ("a reader", read(&style(SYSTEM), None))];
    for (who, ask) in cases {
        let found = chosen.answer(&ask);
        assert_eq!(found.faces.len(), 4, "{who}: every face the system has of it");
        assert!(found.stored.is_empty() && found.listed.is_none(), "{who}: nothing of it goes into the document");
        assert!(found.lacking.is_empty(), "{who}");
    }
    let elsewhere = Chosen::of(&[], None, library(false).0).answer(&read(&style(SYSTEM), None));
    assert_eq!(elsewhere.lacking, [SYSTEM], "a device without the family says so");
}

#[test]
fn a_library_that_cannot_be_reached_is_told_apart_from_a_family_nobody_has() {
    let fonts = system("chosen-down");
    let mut style = style(LIBRARY);
    style.types.get_mut("quote").unwrap().family = "Calibri".into();
    let found = Chosen::of(&[&fonts.0], None, library(true).0).answer(&written(&style, &[]));
    assert_eq!((found.unfetched, found.lacking), (vec![LIBRARY.to_owned()], vec!["Calibri".to_owned()]));
    assert!(found.stored.is_empty() && found.listed.is_none());
}

#[test]
fn a_family_no_type_names_any_more_leaves_the_list() {
    let mut chosen = Chosen::of(&[], None, library(false).0);
    let found = chosen.answer(&written(&style(LIBRARY), &[]));
    let mut style = kept(style(LIBRARY), &found);
    style.types.get_mut("body").unwrap().family = "Inter".into();
    let after = chosen.answer(&written(&style, &found.stored));
    assert_eq!(after.listed, Some(Vec::new()));
    assert!(after.stored.is_empty());
}
