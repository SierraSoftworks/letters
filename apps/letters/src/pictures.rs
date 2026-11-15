//! Pictures on their way into a document, from the clipboard or from files.

use std::fs;

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::metadata::Orientation;
use image::{ExtendedColorType, ImageDecoder, ImageEncoder, ImageReader};
use letters_editor::Picture;
use letters_model::Style;
use letters_render::image::{Image, MAX_PIXELS, MAX_SIDE};

use crate::files;

/// How many pixels across the widest picture on a page of `style` is drawn, in a window that shows
/// `scale` pixels to a point of the screen: the page's text is that wide, and no picture is wider. A
/// picture held for the window needs no more.
pub fn widest(style: &Style, scale: f64) -> u32 {
    let page = &style.page;
    // A page is measured in points, and a window draws it at 96 to the inch.
    let text = f64::from(page.width - page.margin_left - page.margin_right) * 96.0 / 72.0;
    (text * scale).ceil().max(1.0) as u32
}

/// How the files of the pictures a document keeps are named.
pub const ENDINGS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

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
    let (width, height) = size(&bytes)?;
    Some(Picture { bytes, mime, width, height })
}

/// How large a picture's file says it is, upright. Its size and which way up it is are in its first
/// bytes, so a paste does not wait for the pixels: those are decoded off the UI thread.
fn size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut decoder = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    if width.max(height) > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_PIXELS || width.min(height) == 0 {
        return None;
    }
    let turned = matches!(
        decoder.orientation().ok()?,
        Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH
    );
    Some(if turned { (height, width) } else { (width, height) })
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

/// Where the file of a picture to decode is.
#[derive(Clone, Debug)]
pub enum Source {
    /// Here, whole.
    Bytes(Vec<u8>),
    /// In the document saved at this path, under the picture's name.
    Document(PathBuf),
}

/// What is done with a picture once it is decoded: its name, and the picture unless it could not be had.
type Done = Box<dyn FnOnce(String, Option<Image>) + Send>;

struct Job {
    name: String,
    source: Source,
    within: u32,
    done: Done,
}

/// Decodes pictures off the thread that asks, a few at a time and in the order asked. A photo is tens of
/// megabytes while it is being decoded, so a page of them is not decoded all at once.
#[derive(Default)]
pub struct Decoder {
    /// The way to the threads that decode, which start with the first picture.
    jobs: Option<Sender<Job>>,
}

impl Decoder {
    /// The most pictures decoded at a time.
    const AT_ONCE: usize = 2;

    /// Decodes the picture `name` from `source`, held no wider than `within` pixels, and calls `done` with
    /// it on another thread.
    pub fn decode(
        &mut self,
        name: String,
        source: Source,
        within: u32,
        done: impl FnOnce(String, Option<Image>) + Send + 'static,
    ) {
        let jobs = self.jobs.get_or_insert_with(Self::start);
        // The threads live as long as this does, so the job is taken.
        let _ = jobs.send(Job { name, source, within, done: Box::new(done) });
    }

    fn start() -> Sender<Job> {
        let (jobs, waiting) = mpsc::channel::<Job>();
        let waiting = Arc::new(Mutex::new(waiting));
        let threads = thread::available_parallelism().map_or(1, usize::from).min(Self::AT_ONCE);
        for _ in 0..threads {
            let waiting = waiting.clone();
            thread::spawn(move || {
                while let Some(job) = next(&waiting) {
                    let bytes = match &job.source {
                        Source::Bytes(bytes) => Some(std::borrow::Cow::Borrowed(bytes.as_slice())),
                        Source::Document(path) => files::entry(path, &job.name).map(std::borrow::Cow::Owned),
                    };
                    let image = bytes.and_then(|bytes| Image::decode_within(&bytes, job.within).ok());
                    (job.done)(job.name, image);
                }
            });
        }
        jobs
    }
}

/// The next job, once there is one. `None` when the decoder is gone and no more will come.
fn next(waiting: &Mutex<Receiver<Job>>) -> Option<Job> {
    waiting.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).recv().ok()
}
