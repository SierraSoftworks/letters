//! The fonts a writer chose for a document's types, beyond the bundled ones.
//!
//! A style names a family and nothing more, so the faces are found here, in this order:
//!
//! - In the document. A family of the Google Fonts library is kept with the document that uses it, so
//!   that whoever reads it later fetches nothing: its style lists the font files among its assets.
//! - In the library, for a document that may be written and is trusted. The faces its style can ask for
//!   are fetched once, kept on this device for the next document, and stored in this one.
//! - On the system. Its fonts are never stored in a document, since most may not be passed on, so a
//!   reader without one sees the text in a bundled family.
//!
//! All of it runs on a thread of its own, started after a window's first frame.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;

use fontdb::{Database, Source};
use letters_fonts::library;
use letters_format::Archive;
use letters_layout::Named;
use letters_model::{Style, StyleFont};
use letters_render::{Font, FontBytes};
use letters_ui::{FontOffer, FontSource};
use skrifa::raw::TableProvider;
use ureq::Agent;
use ureq::config::AutoHeaderValue;
use ureq::tls::{RootCerts, TlsConfig};

use crate::fallback::BUNDLED;
use crate::remote::LIMIT;

/// The most a font file of the library may weigh. Its largest, for Chinese, are some tens of megabytes.
const MOST: u64 = 64 << 20;

/// What a window asks: the faces for its document as it stands.
#[derive(Clone, Debug, Default)]
pub struct Ask {
    /// The faces its text asked for that are none of the bundled families'.
    pub named: Vec<Named>,
    /// The font files its style lists.
    pub listed: Vec<StyleFont>,
    /// Those of them no save has written yet, by name.
    pub unsaved: HashMap<String, Vec<u8>>,
    /// The document's file, which holds the rest.
    pub file: Option<PathBuf>,
    /// Every face its style can ask for, when the library's may be fetched and kept with the document.
    /// `None` for a document that is only read, or is not trusted.
    pub keep: Option<Vec<Named>>,
}

/// What was found for a window.
#[derive(Clone, Debug, Default)]
pub struct Found {
    /// The faces to set its text in.
    pub faces: Vec<Font>,
    /// Font files to keep among the document's assets, by name.
    pub stored: Vec<(String, Vec<u8>)>,
    /// What its style should list from now on, when that is not what it lists.
    pub listed: Option<Vec<StyleFont>>,
    /// The families nothing here has.
    pub lacking: Vec<String>,
    /// The families of the library that could not be fetched.
    pub unfetched: Vec<String>,
    /// The font files its style lists that are not to be read here. A document's server may have them.
    pub absent: Vec<String>,
}

/// Fetches what is at an address of the library, or nothing.
pub type Fetch = Box<dyn Fn(&str) -> Option<Vec<u8>> + Send>;

/// Where faces are found.
pub struct Chosen {
    system: Database,
    /// The system's files mapped so far, each once.
    files: HashMap<PathBuf, FontBytes>,
    /// The face of each font file handed out, so that the same file is the same bytes every time.
    held: HashMap<String, Font>,
    /// The folder that keeps what was fetched from the library.
    kept: Option<PathBuf>,
    fetch: Fetch,
}

impl Chosen {
    /// The system's fonts, and the library over the network. What is fetched is kept under `home`.
    pub fn system(home: Option<&Path>) -> Self {
        let mut system = Database::new();
        system.load_system_fonts();
        Self::new(system, home.map(|home| home.join(".letters").join("fonts")), Box::new(web()))
    }

    /// The fonts in `folders` for a system, and a library that `fetch` answers for.
    pub fn of(folders: &[&Path], kept: Option<PathBuf>, fetch: Fetch) -> Self {
        let mut system = Database::new();
        folders.iter().for_each(|folder| system.load_fonts_dir(folder));
        Self::new(system, kept, fetch)
    }

    fn new(system: Database, kept: Option<PathBuf>, fetch: Fetch) -> Self {
        Self { system, files: HashMap::new(), held: HashMap::new(), kept, fetch }
    }

    /// The families a writer can choose from: the library's, then the system's own. A family of both is
    /// the library's, which every reader can have.
    pub fn offers(&self) -> Vec<FontOffer> {
        let library = library::families().map(|family| (family.name.to_owned(), FontSource::Library));
        let mut device: BTreeSet<&str> = BTreeSet::new();
        for face in self.system.faces() {
            let Some((name, _)) = face.families.first() else { continue };
            // A name that starts with a dot is the system's own, and not for documents.
            if !name.starts_with('.') && !BUNDLED.iter().any(|own| name.eq_ignore_ascii_case(own)) {
                device.insert(name);
            }
        }
        let device = device.into_iter().map(|name| (name.to_owned(), FontSource::Device));
        library.chain(device).map(|(family, source)| FontOffer { family, source }).collect()
    }

    /// Finds the faces for a document.
    pub fn answer(&mut self, ask: &Ask) -> Found {
        let mut found = Found::default();
        // What the document holds, by the family, weight and slant it is listed for.
        let mut has: Vec<&StyleFont> = Vec::new();
        for font in &ask.listed {
            match self.stored(ask, &font.asset) {
                Some(face) => {
                    found.faces.push(face);
                    has.push(font);
                }
                None => found.absent.push(font.asset.clone()),
            }
        }
        let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        let mut listed: Vec<StyleFont> = has.iter().map(|font| (*font).clone()).collect();

        for face in ask.keep.iter().flatten() {
            // The library has the bundled families too, which every window has already.
            let bundled = BUNDLED.iter().any(|own| own.eq_ignore_ascii_case(&face.family));
            let Some(family) = library::find(&face.family).filter(|_| !bundled) else { continue };
            let (weight, italic) = (family.weight(face.weight), face.italic && family.italic);
            let here =
                |font: &StyleFont| same(&font.family, family.name) && (font.weight, font.italic) == (weight, italic);
            if listed.iter().any(here) || found.unfetched.iter().any(|name| name == family.name) {
                continue;
            }
            let Some(bytes) = self.fetched(&family, weight, italic) else {
                found.unfetched.push(family.name.to_owned());
                continue;
            };
            let asset = asset_name(&bytes);
            let face = Font::new(bytes.as_slice().into(), 0);
            found.faces.push(self.held.entry(asset.clone()).or_insert(face).clone());
            found.stored.push((asset.clone(), bytes));
            listed.push(StyleFont { family: family.name.to_owned(), weight, italic, asset });
        }
        if let Some(keep) = &ask.keep {
            // A family no type names any more leaves the list. Its files stay for the history's sake.
            listed.retain(|font| keep.iter().any(|face| same(&face.family, &font.family)));
            listed.sort();
            let mut before = ask.listed.clone();
            before.sort();
            found.listed = (listed != before).then(|| listed.clone());
        }

        // The system's own, for a family the document holds no face of.
        let mut families: Vec<&str> = Vec::new();
        for named in &ask.named {
            let known = listed.iter().any(|font| same(&font.family, &named.family));
            if !known && !families.iter().any(|family| same(family, &named.family)) {
                families.push(&named.family);
            }
        }
        for family in families {
            let faces = self.installed(family);
            let fetchable = found.unfetched.iter().any(|name| same(name, family));
            if faces.is_empty() && !fetchable {
                found.lacking.push(family.to_owned());
            }
            found.faces.extend(faces);
        }
        found
    }

    /// The face of a font file the document holds: unsaved with its window, or in its file.
    fn stored(&mut self, ask: &Ask, asset: &str) -> Option<Font> {
        if let Some(face) = self.held.get(asset) {
            return Some(face.clone());
        }
        let read = || {
            let mut archive = Archive::open(BufReader::new(fs::File::open(ask.file.as_ref()?).ok()?)).ok()?;
            archive.read(asset).ok()?
        };
        let bytes = ask.unsaved.get(asset).cloned().or_else(read)?;
        // A file that is no font would be drawn as nothing at all.
        skrifa::FontRef::new(&bytes).ok()?;
        let face = Font::new(bytes.into(), 0);
        self.held.insert(asset.to_owned(), face.clone());
        Some(face)
    }

    /// One face of the library: as this device kept it, or fetched now and kept.
    fn fetched(&self, family: &library::Family, weight: u16, italic: bool) -> Option<Vec<u8>> {
        let slant = if italic { "-italic" } else { "" };
        let kept = self.kept.as_ref().map(|folder| folder.join(format!("{}-{weight}{slant}.ttf", family.name)));
        if let Some(bytes) = kept.as_ref().and_then(|file| fs::read(file).ok()) {
            return Some(bytes);
        }
        let sheet = (self.fetch)(&family.stylesheet(weight, italic))?;
        let bytes = (self.fetch)(library::face(std::str::from_utf8(&sheet).ok()?)?)?;
        skrifa::FontRef::new(&bytes).ok()?;
        if let Some(file) = kept {
            // Kept for the next document. One that cannot be kept is fetched again then.
            let _ = file.parent().map(fs::create_dir_all);
            let _ = fs::write(file, &bytes);
        }
        Some(bytes)
    }

    /// Every face the system has of a family.
    fn installed(&mut self, family: &str) -> Vec<Font> {
        let of = |face: &&fontdb::FaceInfo| face.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family));
        let faces: Vec<(Source, u32)> =
            self.system.faces().filter(of).map(|face| (face.source.clone(), face.index)).collect();
        let mut found = Vec::new();
        for (source, index) in faces {
            let (Source::File(path) | Source::SharedFile(path, _)) = source else { continue };
            if !self.files.contains_key(&path) {
                let Ok(mapped) = letters_frame::map_font(&path) else { continue };
                self.files.insert(path.clone(), FontBytes::lent(Arc::new(mapped)));
            }
            found.extend(self.files.get(&path).map(|data| Font::of(data.clone(), index)));
        }
        found
    }
}

/// The faces a document is printed in, beyond the bundled ones: the font files it holds, then the
/// system's own for a family it holds none of. `system` is asked for only then, since reading what a
/// system has takes a while.
pub fn printed(
    style: &Style,
    read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    system: impl FnOnce() -> Chosen,
) -> Vec<Font> {
    let listed = style.fonts();
    let stored = listed.iter().filter_map(|font| read(&font.asset));
    let mut faces: Vec<Font> =
        stored.filter(|bytes| skrifa::FontRef::new(bytes).is_ok()).map(|bytes| Font::new(bytes.into(), 0)).collect();
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let mut families: Vec<String> = Vec::new();
    for named in letters_layout::faces(style) {
        let known = BUNDLED.iter().any(|own| same(own, &named.family))
            || listed.iter().any(|font| same(&font.family, &named.family))
            || families.iter().any(|family| same(family, &named.family));
        if !known {
            families.push(named.family);
        }
    }
    if !families.is_empty() {
        let mut system = system();
        let installed = families.iter().flat_map(|family| system.installed(family));
        faces.extend(installed.filter(may_embed));
    }
    faces
}

/// Whether a font lets itself be put into a file that is passed on. One that forbids it is not printed
/// with, and its text is set in a bundled family.
fn may_embed(font: &Font) -> bool {
    let read = skrifa::FontRef::from_index(&font.data, font.index).ok();
    let kind = read.and_then(|face| face.os2().ok().map(|table| table.fs_type()));
    // The low four bits say how far a font may travel, and 2 alone says nowhere.
    kind.is_none_or(|kind| kind & 0xF != 2)
}

/// The name a font file is kept under among a document's assets, which is the same for the same bytes.
pub fn asset_name(bytes: &[u8]) -> String {
    let hash = letters_format::hash(bytes);
    format!("assets/{}.ttf", hash.strip_prefix("sha256:").unwrap_or(&hash))
}

/// Fetches from the library's two hosts and nowhere else: no redirect is followed, nothing tells one
/// person from another, and no browser is named, so that whole TrueType files are served.
fn web() -> impl Fn(&str) -> Option<Vec<u8>> + Send {
    let config = Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .user_agent("Letters")
        .accept_encoding(AutoHeaderValue::None)
        .timeout_global(Some(LIMIT))
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .build();
    let agent = Agent::new_with_config(config);
    move |address: &str| {
        let host = address.strip_prefix("https://")?.split('/').next()?;
        if host != library::STYLESHEETS && host != library::FACES {
            return None;
        }
        let answer = agent.get(address).call().ok()?;
        if answer.status().as_u16() != 200 {
            return None;
        }
        let mut bytes = Vec::new();
        answer.into_body().into_reader().take(MOST + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= MOST).then_some(bytes)
    }
}

/// Finds faces off the thread that draws, for whoever asks: a window, by its id.
pub struct Chooser<K> {
    asked: Option<mpsc::Sender<(K, Ask)>>,
}

impl<K: Send + 'static> Chooser<K> {
    /// A chooser that does nothing until it is started.
    pub fn idle() -> Self {
        Self { asked: None }
    }

    pub fn started(&self) -> bool {
        self.asked.is_some()
    }

    /// Starts the thread, which makes its `shelves` first and tells `offered` what a writer can choose
    /// from. `found` is then called with the answer to each ask. Both run on the chooser's thread.
    pub fn start(
        &mut self,
        shelves: impl FnOnce() -> Chosen + Send + 'static,
        offered: impl FnOnce(Vec<FontOffer>) + Send + 'static,
        found: impl Fn(K, Found) + Send + 'static,
    ) {
        let (ask, asked) = mpsc::channel::<(K, Ask)>();
        let run = move || {
            let mut chosen = shelves();
            offered(chosen.offers());
            for (asker, ask) in asked {
                found(asker, chosen.answer(&ask));
            }
        };
        // Without a thread nothing is found, and text stays in the bundled families.
        if thread::Builder::new().name("chosen fonts".into()).spawn(run).is_ok() {
            self.asked = Some(ask);
        }
    }

    /// Asks for a document's faces. The answer comes later, through `found`.
    pub fn ask(&self, asker: K, ask: Ask) {
        if let Some(asked) = &self.asked {
            let _ = asked.send((asker, ask));
        }
    }
}

impl<K> std::fmt::Debug for Chooser<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Chooser").field("started", &self.asked.is_some()).finish()
    }
}
