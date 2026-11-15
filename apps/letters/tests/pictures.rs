mod common;

use common::Scratch;
use letters::pictures;

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
