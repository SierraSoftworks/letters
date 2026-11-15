mod common;

use std::sync::mpsc;
use std::time::Duration;

use common::Scratch;
use letters::pictures::{self, Decoder, Source};

/// A PNG of `width` by `height` grey pixels.
fn png(width: u32, height: u32) -> Vec<u8> {
    pictures::from_rgba(width, height, &vec![0x80; (width * height * 4) as usize]).unwrap().bytes
}

#[test]
fn a_picture_is_told_by_its_first_bytes() {
    let cases: [(&[u8], Option<&str>); 7] = [
        (b"\x89PNG\r\n\x1a\n....", Some("image/png")),
        (b"\xff\xd8\xff\xe0..JFIF", Some("image/jpeg")),
        (b"GIF89a....", Some("image/gif")),
        (b"RIFF\x24\0\0\0WEBPVP8 ", Some("image/webp")),
        (b"RIFF\x24\0\0\0WAVEfmt ", None),
        (b"%PDF-1.7", None),
        (b"", None),
    ];
    for (bytes, mime) in cases {
        assert_eq!(pictures::mime(bytes), mime, "{:?}", String::from_utf8_lossy(bytes));
    }
}

#[test]
fn pixels_and_files_become_pictures_with_their_size() {
    let pasted = pictures::from_rgba(5, 3, &[0x40; 60]).expect("pixels make a picture");
    assert_eq!((pasted.mime.as_str(), pasted.width, pasted.height), ("image/png", 5, 3));
    assert!(pictures::from_rgba(5, 3, &[0x40; 59]).is_none(), "too few pixels for the size are no picture");

    let read = pictures::read(pasted.bytes.clone()).expect("a PNG file is a picture");
    assert_eq!(read, pasted, "a file reads as the picture it holds");
    for bytes in [b"not a picture".to_vec(), b"\x89PNG and then nothing a decoder can use".to_vec(), Vec::new()] {
        assert!(pictures::read(bytes.clone()).is_none(), "{:?}", String::from_utf8_lossy(&bytes));
    }
}

#[test]
fn copied_files_are_pasted_as_pictures_only_when_all_of_them_are() {
    let scratch = Scratch::new("pictures");
    let file = |name: &str, bytes: &[u8]| {
        let path = scratch.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    };
    let (one, two, text) = (file("one.png", &png(4, 4)), file("two.png", &png(8, 2)), file("notes.txt", b"words"));

    let sizes = |files: &[std::path::PathBuf]| -> Vec<(u32, u32)> {
        pictures::from_files(files).iter().map(|picture| (picture.width, picture.height)).collect()
    };
    assert_eq!(sizes(&[one.clone(), two.clone()]), [(4, 4), (8, 2)]);
    assert_eq!(sizes(&[one.clone(), text]), [], "a text file among them");
    assert_eq!(sizes(&[one, scratch.0.join("gone.png")]), [], "a file that is not there");
    assert_eq!(sizes(&[]), []);
}

#[test]
fn a_picture_is_held_as_wide_as_the_page_text_at_the_window_scale() {
    let style = letters_model::Style::default();
    let page = &style.page;
    let (text, paper) = (page.width - page.margin_left - page.margin_right, page.width);
    for scale in [1.0, 1.5, 2.0, 3.0] {
        let widest = f64::from(pictures::widest(&style, scale));
        // A window draws a point of the page as 96/72 of its own.
        let (least, most) = (f64::from(text) * scale * 96.0 / 72.0, f64::from(paper) * scale * 96.0 / 72.0);
        assert!(widest >= least.floor() && widest < most, "scale {scale}: {widest} outside {least} to {most}");
    }
}

#[test]
fn pictures_are_decoded_on_other_threads_no_wider_than_asked() {
    let mut decoder = Decoder::default();
    let (done, decoded) = mpsc::channel();
    // (name, the file's bytes, the width to hold it within, the size it is held at)
    let cases = [
        ("wide.png", png(8, 4), 4, Some((4, 2))),
        ("small.png", png(8, 4), 100, Some((8, 4))),
        ("broken.png", b"\x89PNG and nothing more".to_vec(), 100, None),
    ];
    for (name, bytes, within, _) in &cases {
        let done = done.clone();
        let tell = move |name, image: Option<letters_render::image::Image>| {
            done.send((name, image.map(|image| (image.width(), image.height())))).unwrap();
        };
        decoder.decode((*name).to_owned(), Source::Bytes(bytes.clone()), *within, tell);
    }
    // No such document, so no picture in it.
    let gone = Source::Document(Scratch::new("pictures-decoder").0.join("Gone.let"));
    let tell =
        move |name, image: Option<letters_render::image::Image>| done.send((name, image.map(|_| (0, 0)))).unwrap();
    decoder.decode("assets/gone.png".to_owned(), gone, 100, tell);

    let mut held: Vec<(String, Option<(u32, u32)>)> =
        (0..4).map(|_| decoded.recv_timeout(Duration::from_secs(30)).expect("every picture is answered")).collect();
    held.sort();
    let mut expected: Vec<(String, Option<(u32, u32)>)> =
        cases.iter().map(|(name, _, _, size)| ((*name).to_owned(), *size)).collect();
    expected.push(("assets/gone.png".to_owned(), None));
    expected.sort();
    assert_eq!(held, expected);
}
