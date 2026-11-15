//! Whole scenes compared with stored pictures, to catch changes no single probe would.

mod common;

use std::fs;
use std::path::PathBuf;

use common::*;
use letters_render::image::Images;
use letters_render::snapshot::{self, Outcome};
use letters_render::{Color, Font, Item, Point, Rect};

#[test]
fn scenes_match_their_snapshots() {
    let images = scene_images();
    for (name, scene, scale) in SCENES.into_iter().chain([EMOJI]) {
        let (width, height) = ((240.0 * scale) as u32, (160.0 * scale) as u32);

        snapshot::assert(&snapshot_path(name), &shot(&list(scene()), &images, width, height, scale));
    }
}

/// A snapshot of its own for one test, stored from a list of one fill.
fn stored(test: &str) -> (PathBuf, Vec<Item>) {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("snapshots").join(test);
    fs::create_dir_all(&folder).expect("the test's folder can be made");
    let (path, items) = (folder.join("square.png"), vec![fill(4.0, 4.0, 12.0, 12.0, RED)]);
    snapshot::store(&path, &shot(&list(items.clone()), &Images::default(), 24, 24, 1.0)).expect("it can be stored");
    (path, items)
}

#[test]
fn a_snapshot_is_only_drawn_again_when_something_changed() {
    let (path, items) = stored("skipped");
    let none = Images::default();
    let same = list(items.clone());
    let key = path.with_extension("key");

    assert_eq!(snapshot::compare(&path, &shot(&same, &none, 24, 24, 1.0)), Ok(Outcome::Unchanged));

    // Whatever decides the pixels is part of the key, so each of these is drawn and compared.
    let recoloured = list([fill(4.0, 4.0, 12.0, 12.0, BLUE)]);
    let nudged = list([fill(4.0, 4.0, 12.0, 12.0, Color::rgb(221, 30, 30))]);
    assert!(snapshot::compare(&path, &shot(&recoloured, &none, 24, 24, 1.0)).is_err(), "another colour");
    assert!(snapshot::compare(&path, &shot(&same, &none, 24, 24, 2.0)).is_err(), "another scale");
    assert!(snapshot::compare(&path, &shot(&same, &none, 32, 24, 1.0)).is_err(), "another size");
    assert_eq!(snapshot::compare(&path, &shot(&nudged, &none, 24, 24, 1.0)), Ok(Outcome::Matches), "a near match");

    // A key from another renderer, or none, falls back to the pixels.
    fs::write(&key, "0000000000000000\n").expect("the key can be replaced");
    assert_eq!(snapshot::compare(&path, &shot(&same, &none, 24, 24, 1.0)), Ok(Outcome::Matches));
    fs::remove_file(&key).expect("the key can be removed");
    assert_eq!(snapshot::compare(&path, &shot(&same, &none, 24, 24, 1.0)), Ok(Outcome::Matches));
}

#[test]
fn a_snapshot_names_the_fonts_and_images_it_draws_by_their_content() {
    let (path, _) = stored("content");
    let swatch = |color| {
        let mut images = Images::default();
        images.insert("swatch", image(4, 4, move |_, _| color));
        images
    };
    let picture = || list([Item::Image { rect: Rect::new(2.0, 2.0, 20.0, 20.0), asset: "swatch".into() }]);
    let words = |font: &Font| list([text(font, 12.0, BLACK, Point { x: 2.0, y: 16.0 }, "Ag")]);
    let none = Images::default();

    snapshot::store(&path, &shot(&picture(), &swatch(RED), 24, 24, 1.0)).expect("it can be stored");
    let again = snapshot::compare(&path, &shot(&picture(), &swatch(RED), 24, 24, 1.0));
    assert_eq!(again, Ok(Outcome::Unchanged), "the same pixels, decoded again");
    assert!(snapshot::compare(&path, &shot(&picture(), &swatch(BLUE), 24, 24, 1.0)).is_err(), "another image");

    snapshot::store(&path, &shot(&words(&sans(400.0)), &none, 24, 24, 1.0)).expect("it can be stored");
    let again = snapshot::compare(&path, &shot(&words(&sans(400.0)), &none, 24, 24, 1.0));
    assert_eq!(again, Ok(Outcome::Unchanged), "the same font, loaded again");
    assert!(snapshot::compare(&path, &shot(&words(&display(400.0)), &none, 24, 24, 1.0)).is_err(), "another font");
}

#[test]
fn a_missing_or_damaged_snapshot_fails_with_a_way_forward() {
    let (path, items) = stored("damaged");
    let none = Images::default();
    let same = list(items);
    fs::remove_file(path.with_extension("key")).expect("the key can be removed");

    for damage in [|path: &PathBuf| fs::write(path, b"not a picture"), |path: &PathBuf| fs::remove_file(path)] {
        damage(&path).expect("the snapshot can be damaged");

        let mismatch = snapshot::compare(&path, &shot(&same, &none, 24, 24, 1.0)).expect_err("nothing to match");

        assert!(mismatch.contains(snapshot::BLESS), "{mismatch}");
    }
}
