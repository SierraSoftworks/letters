//! Decoding the image formats a document may hold, and refusing what cannot be drawn.

mod common;

use std::io::Cursor;

use common::*;
use image::{ImageFormat, Rgba, RgbaImage};
use letters_render::image::{Image, ImageError, Images, MAX_SIDE};
use letters_render::{Color, Item, Rect};

/// A picture that is red on the left and blue on the right, encoded in `format`.
fn encoded(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
    common::encoded(width, height, format, |x, _| if x < width / 2 { RED } else { BLUE })
}

/// Draws an image across a frame of its own size and returns the colours of its left and right halves.
fn halves(image: Image) -> (Color, Color) {
    let (width, height) = (image.width(), image.height());
    let mut images = Images::default();
    images.insert("picture", image);
    let item = Item::Image { rect: Rect::new(0.0, 0.0, width as f32, height as f32), asset: "picture".into() };
    let frame = render_with(&list([item]), &images, width, height, 1.0);
    let probe = |x| frame.pixel(x, height / 2).expect("the probe is inside the frame");
    (probe(width / 4), probe(width * 3 / 4))
}

#[test]
fn every_supported_format_decodes_to_its_pixels() {
    // JPEG is lossy, so its colours are only close.
    let formats = [(ImageFormat::Png, 0), (ImageFormat::Gif, 0), (ImageFormat::WebP, 0), (ImageFormat::Jpeg, 12)];
    for (format, tolerance) in formats {
        let image = Image::decode(&encoded(32, 16, format)).unwrap_or_else(|error| panic!("{format:?}: {error}"));

        assert_eq!((image.width(), image.height()), (32, 16), "{format:?}");
        let (left, right) = halves(image);
        assert!(close(left, RED, tolerance), "{format:?} left is {left:?}");
        assert!(close(right, BLUE, tolerance), "{format:?} right is {right:?}");
    }
}

#[test]
fn a_photo_taken_on_its_side_is_stood_upright() {
    // Orientation 6 means the stored picture must be turned a quarter clockwise to view.
    let photo = oriented(&encoded(32, 16, ImageFormat::Jpeg), 6);
    let image = Image::decode(&photo).expect("the photo decodes");

    assert_eq!((image.width(), image.height()), (16, 32));
}

#[test]
fn transparency_survives_decoding() {
    let picture = RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 128]));
    let mut bytes = Cursor::new(Vec::new());
    picture.write_to(&mut bytes, ImageFormat::Png).expect("PNG encodes");
    let (left, _) = halves(Image::decode(&bytes.into_inner()).expect("the PNG decodes"));

    assert!(close(left, Color::rgb(255, 127, 127), 2), "half red over white, got {left:?}");
}

#[test]
fn images_that_cannot_be_drawn_are_refused() {
    let png = encoded(32, 16, ImageFormat::Png);
    // A GIF whose header claims a canvas far past the pixel limit.
    let mut vast = encoded(2, 2, ImageFormat::Gif);
    vast[6..10].copy_from_slice(&[0x60, 0xea, 0x60, 0xea]);
    type Check = fn(&ImageError) -> bool;
    let cases: [(&str, Vec<u8>, Check); 6] = [
        ("no data", Vec::new(), |error| *error == ImageError::Unsupported),
        ("text", b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(), |error| *error == ImageError::Unsupported),
        ("a format left out", [b"BM".as_slice(), &[0; 64]].concat(), |error| *error == ImageError::Unsupported),
        ("a truncated file", png[..png.len() / 2].to_vec(), |error| matches!(error, ImageError::Invalid(_))),
        ("a header with no picture", png[..8].to_vec(), |error| matches!(error, ImageError::Invalid(_))),
        ("a vast picture", vast, |error| *error == ImageError::TooLarge),
    ];
    for (name, bytes, expected) in cases {
        let error = Image::decode(&bytes).expect_err(name);

        assert!(expected(&error), "{name} gave {error:?}");
        assert!(!error.to_string().is_empty(), "{name} explains itself");
    }
}

#[test]
fn raw_pixels_must_match_their_size() {
    let cases = [
        ("too few bytes", 4, 4, 60, ImageError::Invalid(String::new())),
        ("no pixels", 0, 4, 0, ImageError::Invalid(String::new())),
        ("too wide", MAX_SIDE + 1, 1, (MAX_SIDE as usize + 1) * 4, ImageError::TooLarge),
    ];
    for (name, width, height, bytes, expected) in cases {
        let error = Image::from_rgba8(width, height, vec![0; bytes]).expect_err(name);

        assert_eq!(std::mem::discriminant(&error), std::mem::discriminant(&expected), "{name} gave {error:?}");
    }
    assert!(Image::from_rgba8(4, 4, vec![0; 64]).is_ok());
}
