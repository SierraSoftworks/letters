//! What a page does about the fonts a writer chose: reads those its document holds, fetches a family of
//! the library for whoever writes, and says which families nothing has.

use std::collections::HashMap;

use letters_model::{Style, StyleFont};
use letters_web::chosen::{Arrival, Known, Plan, Want, kept, plan};

/// A listing as the server hands it over.
const LIBRARY: &str = "# families\nInter\t100-900\t1\nLobster\t400\t0\nLora\t400-700\t1\n";

/// A style whose body is set in `family`.
fn style(family: &str) -> Style {
    let mut style = Style::default();
    style.types.get_mut("body").unwrap().family = family.into();
    style
}

fn font(family: &str, weight: u16, italic: bool) -> StyleFont {
    let slant = if italic { "i" } else { "" };
    StyleFont { family: family.into(), weight, italic, asset: format!("assets/{family}{weight}{slant}.ttf") }
}

fn want(family: &str, weight: u16, italic: bool) -> Want {
    Want { family: family.into(), weight, italic }
}

/// The plan for a document in `style` whose text asked for all its faces.
fn planned(style: &Style, library: Option<&str>, files: &[(&str, Arrival)], faces: &[(Want, Arrival)]) -> Plan {
    let (files, faces): (HashMap<_, _>, HashMap<_, _>) =
        (files.iter().copied().collect(), faces.iter().cloned().collect());
    let all = letters_layout::faces(style);
    let listed = style.fonts();
    plan(&Known {
        listed: &listed,
        faces: &all,
        named: &all,
        library,
        file: &|name| files.get(name).copied(),
        face: &|want| faces.get(want).copied(),
    })
}

#[test]
fn a_writer_is_fetched_each_face_a_family_of_the_library_has_and_no_bundled_one() {
    // The family, and the faces fetched for it.
    let cases = [
        (
            "Lora",
            vec![want("Lora", 400, false), want("Lora", 400, true), want("Lora", 700, false), want("Lora", 700, true)],
        ),
        ("Lobster", vec![want("Lobster", 400, false)]),
        ("Inter", vec![]),
        ("Calibri", vec![]),
    ];
    for (family, expected) in cases {
        let mut fetched = planned(&style(family), Some(LIBRARY), &[], &[]).fetch;
        fetched.sort_by_key(|want| (want.weight, want.italic));
        fetched.retain(|want| want.family == family);
        assert_eq!(fetched, expected, "{family}");
    }
    let asked = [(want("Lobster", 400, false), Arrival::Coming)];
    assert_eq!(planned(&style("Lobster"), Some(LIBRARY), &[], &asked).fetch, [], "a face is asked for once");
}

#[test]
fn a_reader_fetches_nothing_and_reads_what_the_document_lists() {
    let mut listing = style("Lobster");
    listing.set_fonts(&[font("Lobster", 400, false)]);
    let asset = "assets/Lobster400.ttf";
    // What is known of the file, and what is read and given up on.
    let cases = [
        (None, vec![asset.to_owned()], vec![]),
        (Some(Arrival::Coming), vec![], vec![]),
        (Some(Arrival::Here), vec![], vec![]),
        (Some(Arrival::Lost), vec![], vec!["Lobster".to_owned()]),
    ];
    for (arrival, read, lacking) in cases {
        let files: Vec<_> = arrival.map(|arrival| (asset, arrival)).into_iter().collect();
        let plan = planned(&listing, None, &files, &[]);
        assert_eq!((plan.read, plan.lacking, plan.fetch), (read, lacking, vec![]), "{arrival:?}");
    }
    let unlisted = planned(&style("Lobster"), None, &[], &[]);
    assert_eq!((unlisted.fetch, unlisted.lacking), (vec![], vec!["Lobster".to_owned()]));
}

/// A family, what came of its faces, and the families said to be lacking and unfetched.
type Case<'a> = (&'a str, &'a [(Want, Arrival)], &'a [&'a str], &'a [&'a str]);

#[test]
fn a_family_nothing_has_is_told_apart_from_one_that_could_not_be_fetched() {
    let lost = [(want("Lobster", 400, false), Arrival::Lost)];
    // The family, what came of its faces, and what is said: nothing, lacking or unfetched.
    let none: &[&str] = &[];
    let cases: [Case<'_>; 3] =
        [("Lobster", &[], none, none), ("Lobster", &lost, none, &["Lobster"]), ("Calibri", &[], &["Calibri"], none)];
    for (family, faces, lacking, unfetched) in cases {
        let plan = planned(&style(family), Some(LIBRARY), &[], faces);
        assert_eq!(plan.lacking, lacking, "{family}: lacking");
        assert_eq!(plan.unfetched, unfetched, "{family}: unfetched");
    }
}

#[test]
fn the_style_lists_what_arrived_and_drops_a_family_no_type_names() {
    let faces = letters_layout::faces(&style("Lobster"));
    let arrived = [(want("Lobster", 400, false), "assets/a.ttf".to_owned())];
    let listed = kept(&[font("Lora", 400, false)], &faces, &arrived).expect("the list changes");
    assert_eq!(
        listed,
        [StyleFont { family: "Lobster".into(), weight: 400, italic: false, asset: "assets/a.ttf".into() }]
    );
    assert_eq!(kept(&listed, &faces, &arrived), None, "what is listed already is no news");
}
