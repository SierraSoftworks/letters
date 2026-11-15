//! Pictures on their way into a document, from the clipboard or from files.

use std::fs;
use std::path::PathBuf;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};
use letters_editor::Picture;
use letters_render::image::Image;

/// The media type of a picture in one of the formats a document keeps, told by its first bytes.
pub fn mime(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

/// A picture from the bytes of its file. `None` for what is no picture Letters can show.
pub fn read(bytes: Vec<u8>) -> Option<Picture> {
    let mime = mime(&bytes)?.to_owned();
    // The size it is shown at is its upright one, which only decoding it tells.
    let image = Image::decode(&bytes).ok()?;
    Some(Picture { bytes, mime, width: image.width(), height: image.height() })
}

/// A picture from bare pixels, four bytes each: red, green, blue and alpha. It is kept as a PNG, which
/// loses nothing.
pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Picture> {
    // The encoder takes anything else for a mistake of the program's, and stops it.
    if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
        return None;
    }
    let mut bytes = Vec::new();
    // Made while the person waits for a paste, so quickly and not as small as it could be.
    let encoder = PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, FilterType::Adaptive);
    encoder.write_image(rgba, width, height, ExtendedColorType::Rgba8).ok()?;
    Some(Picture { bytes, mime: "image/png".into(), width, height })
}

/// The pictures among copied files, if every one of them is a picture. Files of other kinds are no
/// pictures to paste, and neither is a list with one of them in it.
pub fn from_files(files: &[PathBuf]) -> Vec<Picture> {
    let pictures: Vec<Picture> = files.iter().map_while(|file| read(fs::read(file).ok()?)).collect();
    if pictures.len() == files.len() { pictures } else { Vec::new() }
}
