//! Pixel snapshots: a display list compared with a stored picture of what it draws.
//!
//! Each picture has a key beside it: a hash of the display list, its fonts and images, and the renderer.
//! While the key still fits, nothing can have changed, so [`compare`] skips rasterising. A key that no
//! longer fits only costs the time to draw, until the snapshot is next stored. For tests only.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::{fs, io};

use crate::cpu::{CpuRenderer, Frame};
use crate::image::Images;
use crate::{Color, DisplayList, damage};

/// A pixel differs when a channel is off by more than this. Antialiasing varies a little between machines.
const CHANNEL: u8 = 24;
/// The share of pixels that may differ.
const SHARE: f64 = 0.002;
/// Set in the environment to make [`assert`] store what is drawn as the new snapshot.
pub const BLESS: &str = "LETTERS_BLESS";

/// What a snapshot is a picture of.
#[derive(Clone, Copy)]
pub struct Shot<'a> {
    pub list: &'a DisplayList,
    pub images: &'a Images,
    /// The picture's size in physical pixels.
    pub size: (u32, u32),
    pub scale: f32,
    pub background: Color,
}

/// How a shot was found to match its snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The key fits: nothing that decides the pixels has changed, so none were drawn.
    Unchanged,
    /// The shot was drawn, and its pixels match the stored picture.
    Matches,
}

/// Compares a shot with the snapshot at `path`, a PNG. The error says what differs, and where the picture
/// that was drawn has been left to look at.
pub fn compare(path: &Path, shot: &Shot<'_>) -> Result<Outcome, String> {
    let key = key(shot);
    if key.is_some() && key == fs::read_to_string(path.with_extension("key")).ok() {
        return Ok(Outcome::Unchanged);
    }
    let frame = draw(shot);
    let name = path.file_name().unwrap_or_default();
    let drawn = std::env::temp_dir().join("letters-snapshots").join(name);
    let hint = |frame: &Frame| {
        let saved = fs::create_dir_all(drawn.parent().unwrap_or(&drawn)).and_then(|()| save(&drawn, frame));
        match saved {
            Ok(()) => format!("see {}, and set {BLESS}=1 to accept it", drawn.display()),
            Err(error) => format!("set {BLESS}=1 to accept what is drawn now ({error})"),
        }
    };
    let Ok(stored) = ::image::open(path).map(::image::DynamicImage::into_rgba8) else {
        return Err(format!("{} cannot be read: {}", path.display(), hint(&frame)));
    };
    if (stored.width(), stored.height()) != (frame.width(), frame.height()) {
        return Err(format!("{} has another size: {}", path.display(), hint(&frame)));
    }
    let pixels = stored.as_raw().as_chunks::<4>().0.iter().zip(frame.data().as_chunks::<4>().0);
    let differing = pixels.filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > CHANNEL)).count();
    let allowed = (f64::from(frame.width()) * f64::from(frame.height()) * SHARE) as usize;
    if differing > allowed {
        return Err(format!("{} changed in {differing} pixels of {allowed} allowed: {}", path.display(), hint(&frame)));
    }
    Ok(Outcome::Matches)
}

/// Stores what a shot draws as the snapshot at `path`, with its key.
pub fn store(path: &Path, shot: &Shot<'_>) -> io::Result<()> {
    save(path, &draw(shot))?;
    match key(shot) {
        Some(key) => fs::write(path.with_extension("key"), key),
        None => Ok(()),
    }
}

/// Panics unless a shot matches the snapshot at `path`. With [`BLESS`] set it stores the shot instead.
pub fn assert(path: &Path, shot: &Shot<'_>) {
    if std::env::var_os(BLESS).is_some() {
        store(path, shot).unwrap_or_else(|error| panic!("{} cannot be written: {error}", path.display()));
    } else if let Err(mismatch) = compare(path, shot) {
        panic!("{mismatch}");
    }
}

fn draw(shot: &Shot<'_>) -> Frame {
    let mut frame = Frame::new(shot.size.0, shot.size.1);
    CpuRenderer::new().render(shot.list, shot.images, shot.scale, shot.background, &mut frame);
    frame
}

fn save(path: &Path, frame: &Frame) -> io::Result<()> {
    ::image::save_buffer(path, frame.data(), frame.width(), frame.height(), ::image::ColorType::Rgba8)
        .map_err(io::Error::other)
}

/// The key of a shot, or `None` when the renderer's own sources are not there to be hashed.
fn key(shot: &Shot<'_>) -> Option<String> {
    static RENDERER: OnceLock<Option<u64>> = OnceLock::new();
    let renderer = (*RENDERER.get_or_init(renderer))?;
    let Color { r, g, b, a } = shot.background;
    let view = [shot.size.0, shot.size.1, shot.scale.to_bits(), u32::from_be_bytes([r, g, b, a])];
    let content = damage::fingerprint(shot.list, shot.images);
    let parts: [&[u8]; 3] = [&renderer.to_le_bytes(), bytemuck::cast_slice(&view), &content.to_le_bytes()];
    Some(format!("{:016x}\n", damage::hash(&parts)))
}

/// A hash of what decides how a list is drawn: this crate's sources, and the locked versions of everything
/// it is built from.
fn renderer() -> Option<u64> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources: Vec<PathBuf> = fs::read_dir(root.join("src")).ok()?.flatten().map(|entry| entry.path()).collect();
    sources.sort();
    let mut parts = Vec::new();
    for source in sources.iter().filter(|source| source.extension().is_some_and(|kind| kind == "rs")) {
        // A checkout may have changed the line endings.
        parts.push(fs::read_to_string(source).ok()?.replace('\r', ""));
    }
    let lock = root.ancestors().find_map(|folder| fs::read_to_string(folder.join("Cargo.lock")).ok())?;
    parts.extend(built_from(&lock, env!("CARGO_PKG_NAME")));
    Some(damage::hash(&parts.iter().map(String::as_bytes).collect::<Vec<_>>()))
}

/// The packages of a `Cargo.lock` that `package` is built from, each as its name, version and checksum.
fn built_from(lock: &str, package: &str) -> BTreeSet<String> {
    struct Locked<'a> {
        version: &'a str,
        checksum: &'a str,
        dependencies: Vec<&'a str>,
    }
    let mut packages: HashMap<&str, Vec<Locked<'_>>> = HashMap::new();
    for block in lock.split("[[package]]").skip(1) {
        let field = |name: &str| {
            let value = block.lines().find_map(|line| line.strip_prefix(name)?.strip_prefix(" = \""));
            value.map_or("", |value| value.trim_end_matches('"'))
        };
        let dependencies = block.split_once("dependencies = [").map_or("", |(_, list)| list);
        let dependencies = dependencies.split(']').next().unwrap_or_default();
        let dependencies =
            dependencies.lines().map(|line| line.trim().trim_matches(['"', ','])).filter(|line| !line.is_empty());
        packages.entry(field("name")).or_default().push(Locked {
            version: field("version"),
            checksum: field("checksum"),
            dependencies: dependencies.collect(),
        });
    }

    let mut found = BTreeSet::new();
    let mut pending = vec![package];
    while let Some(wanted) = pending.pop() {
        // A dependency is its name, with its version when the lock holds more than one.
        let mut words = wanted.split(' ');
        let (name, version) = (words.next().unwrap_or_default(), words.next());
        for locked in
            packages.get(name).into_iter().flatten().filter(|locked| version.is_none_or(|v| v == locked.version))
        {
            if found.insert(format!("{name} {} {}", locked.version, locked.checksum)) {
                pending.extend(&locked.dependencies);
            }
        }
    }
    found
}
