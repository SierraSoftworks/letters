//! System faces for what the bundled fonts cannot draw: emoji, and the scripts they do not cover.
//!
//! A document is measured in the bundled fonts alone, so it reads the same on every machine. A system
//! face is looked for only for a character none of them has, where the choice is between that face and
//! an empty box. The system's fonts are first read when the first such character is asked for, on a
//! thread of their own, so that opening the app never touches them.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;

use fontdb::{Database, ID, Source, Stretch, Style};
use letters_fonts::{DISPLAY, MONO, SANS};
use letters_layout::{Fonts, Uncovered};
use letters_render::{Font, FontBytes};
use skrifa::{FontRef, MetadataProvider};

/// The families every window has already, whatever the system holds of them.
pub const BUNDLED: &[&str] = &[SANS, MONO, DISPLAY];

/// Which of the faces that share the Han characters draws them, since each draws them as its own
/// language writes them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Han {
    #[default]
    Simplified,
    Traditional,
    Japanese,
    Korean,
}

impl Han {
    /// What a language asks for, named as a document names it (`ja`, `zh-Hant`) or as a locale does
    /// (`ja_JP.UTF-8`). `None` for a language that writes no Han characters.
    pub fn of_language(language: &str) -> Option<Self> {
        let language = language.to_ascii_lowercase().replace('-', "_");
        let language = language.split(['.', '@']).next().unwrap_or_default();
        let mut parts = language.split('_');
        match parts.next()? {
            "ja" => Some(Self::Japanese),
            "ko" => Some(Self::Korean),
            "zh" if parts.any(|part| matches!(part, "tw" | "hk" | "mo" | "hant")) => Some(Self::Traditional),
            "zh" => Some(Self::Simplified),
            _ => None,
        }
    }

    /// What the person's locale asks for, where the environment tells it.
    pub fn of_system() -> Self {
        let set = ["LC_ALL", "LC_CTYPE", "LANG"].into_iter().find_map(|name| std::env::var(name).ok());
        set.and_then(|locale| Self::of_language(&locale)).unwrap_or_default()
    }

    /// The same, unless the text says more: its language, or else characters only one language writes,
    /// which are kana and Hangul.
    pub fn told(self, uncovered: &[Uncovered]) -> Self {
        let kana = |c: char| matches!(c as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F);
        let hangul = |c: char| matches!(c as u32, 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF);
        let language = uncovered.iter().find_map(|one| one.lang.as_deref().and_then(Self::of_language));
        match language {
            Some(language) => language,
            None if uncovered.iter().any(|one| kana(one.ch)) => Self::Japanese,
            None if uncovered.iter().any(|one| hangul(one.ch)) => Self::Korean,
            None => self,
        }
    }

    /// The families that draw Han characters this way, best first, as the start of their names.
    fn families(self) -> &'static [&'static str] {
        match self {
            Self::Simplified => &[
                "pingfang sc",
                "microsoft yahei",
                "noto sans cjk sc",
                "noto sans sc",
                "source han sans sc",
                "wenquanyi",
            ],
            Self::Traditional => {
                &["pingfang tc", "microsoft jhenghei", "noto sans cjk tc", "noto sans tc", "source han sans tc"]
            }
            Self::Japanese => &[
                "hiragino sans",
                "yu gothic",
                "meiryo",
                "noto sans cjk jp",
                "noto sans jp",
                "source han sans jp",
                "ipa",
            ],
            Self::Korean => &[
                "apple sd gothic neo",
                "malgun gothic",
                "noto sans cjk kr",
                "noto sans kr",
                "source han sans kr",
                "nanum",
            ],
        }
    }

    /// Every family known to draw Han characters, with this way's own first: a face that draws them as
    /// another language writes them is still better than one that was not made for them.
    fn all(self) -> impl Iterator<Item = &'static str> {
        let others = [Self::Simplified, Self::Traditional, Self::Japanese, Self::Korean];
        let others = others.into_iter().filter(move |other| *other != self);
        std::iter::once(self).chain(others).flat_map(|han| han.families().iter().copied())
    }
}

/// The families each system sets its own interface's other scripts in, best first, as the start of their
/// names. Any other family comes after them, and those for Han characters are [`Han`]'s to name.
#[cfg(target_os = "macos")]
const TEXT: &[&str] = &[
    "geeza pro",
    "arial hebrew",
    "thonburi",
    "kohinoor",
    "noto sans",
    "apple symbols",
    "stix two",
    "arial unicode ms",
];
#[cfg(windows)]
const TEXT: &[&str] = &["segoe ui", "leelawadee ui", "nirmala ui", "ebrima", "gadugi", "cambria math"];
#[cfg(not(any(target_os = "macos", windows)))]
const TEXT: &[&str] = &["noto sans", "dejavu sans", "liberation sans", "freesans", "droid sans fallback"];

/// The families that draw emoji in colour, best first. Any other with "emoji" in its name follows.
const EMOJI: &[&str] = &["apple color emoji", "segoe ui emoji", "noto color emoji"];

/// Families that draw nearly everything, each character as a grid of dots: better than an empty box and
/// worse than any other face.
const DOTTED: &[&str] = &["unifont"];

/// Families that have a glyph for every character and so say nothing of any.
const BLANK: &[&str] = &["lastresort", "last resort", "adobe blank"];

/// What kind of face a character is best drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Written to show as an emoji.
    Picture,
    /// Han characters and what is written with them: kana, Hangul, their punctuation and wide forms.
    Han,
    Text,
}

impl Kind {
    fn of(one: &Uncovered) -> Self {
        let han = matches!(
            one.ch as u32,
            0x1100..=0x11FF
                | 0x2E80..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xA960..=0xA97F
                | 0xAC00..=0xD7FF
                | 0xF900..=0xFAFF
                | 0xFE30..=0xFE4F
                | 0xFF00..=0xFFEF
                | 0x20000..=0x3FFFF
        );
        match () {
            () if one.emoji => Self::Picture,
            () if han => Self::Han,
            () => Self::Text,
        }
    }
}

/// One family of the shelf, by the face of it that is nearest to regular and upright. Bold and slanted
/// text is drawn from it too, as it is for a bundled family whose own bold has not arrived.
struct Family {
    face: ID,
    /// In lower case.
    name: String,
    /// Whether every character of it is as wide as the next, which reads oddly among text that is not.
    mono: bool,
}

/// The fonts of some folders, or of the system, to take faces from.
pub struct Shelf {
    fonts: Database,
    families: Vec<Family>,
    /// The files mapped so far. A collection is mapped once for all its faces, and a face handed out twice
    /// is the same bytes, which is how the layout knows it has it and the renderer when it is let go.
    files: HashMap<PathBuf, FontBytes>,
    /// The characters no face here has, so that they are looked for once.
    lacking: HashSet<char>,
}

impl Shelf {
    /// The system's fonts, but for the families in `but`: the bundled ones, which are there already.
    pub fn system(but: &[&str]) -> Self {
        let mut fonts = Database::new();
        fonts.load_system_fonts();
        Self::new(fonts, but)
    }

    /// The fonts in `folders` and the folders inside them, but for the families in `but`.
    pub fn of(folders: &[&Path], but: &[&str]) -> Self {
        let mut fonts = Database::new();
        for folder in folders {
            fonts.load_fonts_dir(folder);
        }
        Self::new(fonts, but)
    }

    fn new(fonts: Database, but: &[&str]) -> Self {
        // How far a face is from the one a family is known by.
        let off = |face: &fontdb::FaceInfo| (face.stretch != Stretch::Normal, face.weight.0.abs_diff(400));
        let mut nearest: HashMap<String, (ID, bool, (bool, u16))> = HashMap::new();
        for face in fonts.faces().filter(|face| face.style == Style::Normal) {
            let Some((name, _)) = face.families.first() else { continue };
            // A name that starts with a dot is the system's own, and not for documents.
            let hidden = name.starts_with('.') || BLANK.iter().any(|blank| name.eq_ignore_ascii_case(blank));
            if hidden || but.iter().any(|but| name.eq_ignore_ascii_case(but)) {
                continue;
            }
            let found = (face.id, face.monospaced, off(face));
            let held = nearest.entry(name.to_lowercase()).or_insert(found);
            if found.2 < held.2 {
                *held = found;
            }
        }
        let family = |(name, (face, mono, _))| Family { face, name, mono };
        let mut families: Vec<Family> = nearest.into_iter().map(family).collect();
        // The same order on every run, whatever order the folders were listed in.
        families.sort_by(|a, b| a.name.cmp(&b.name));
        Self { fonts, families, files: HashMap::new(), lacking: HashSet::new() }
    }

    /// The faces that draw what is `uncovered`: at most one for each character, and none for a character
    /// no font here has. Emoji are taken from the emoji faces first, Han characters from the faces of
    /// their own language, or of the one `han` names where they tell none, and other text from the
    /// families the system itself falls back on, before the rest.
    pub fn find(&mut self, uncovered: &[Uncovered], han: Han) -> Vec<Font> {
        // A page may hold Han characters of several languages, each to be drawn its own way.
        let own = |one: &Uncovered| one.lang.as_deref().and_then(Han::of_language).unwrap_or(han);
        let ways = [Han::Simplified, Han::Traditional, Han::Japanese, Han::Korean].map(|way| (Kind::Han, way));
        let mut found: Vec<Font> = Vec::new();
        for (kind, han) in [(Kind::Picture, han)].into_iter().chain(ways).chain([(Kind::Text, han)]) {
            let wanted = |one: &&Uncovered| {
                Kind::of(one) == kind && (kind != Kind::Han || own(one) == han) && !self.lacking.contains(&one.ch)
            };
            let mut left: Vec<char> = uncovered.iter().filter(wanted).map(|one| one.ch).collect();
            left.sort_unstable();
            left.dedup();
            let order = if left.is_empty() { Vec::new() } else { self.order(kind, han) };
            for face in order {
                if left.is_empty() {
                    break;
                }
                let has = |data: &[u8], index: u32| {
                    let Ok(font) = FontRef::from_index(data, index) else { return Vec::new() };
                    let map = font.charmap();
                    left.iter().copied().filter(|c| map.map(*c).is_some_and(|glyph| glyph.to_u32() != 0)).collect()
                };
                let covered: Vec<char> = self.fonts.with_face_data(face, has).unwrap_or_default();
                if covered.is_empty() {
                    continue;
                }
                if let Some(font) = self.read(face) {
                    left.retain(|c| !covered.contains(c));
                    // A face that draws characters of two kinds is still one face.
                    if !found.contains(&font) {
                        found.push(font);
                    }
                }
            }
            self.lacking.extend(left);
        }
        found
    }

    /// The families' faces, best first for characters of one kind.
    fn order(&self, kind: Kind, han: Han) -> Vec<ID> {
        let place = |names: &[&str], name: &str| names.iter().position(|start| name.starts_with(start));
        let for_han: Vec<&str> = han.all().collect();
        let rank = |family: &Family| {
            let name = family.name.as_str();
            let emoji = name.contains("emoji");
            let han = place(&for_han, name);
            // Faces made for another kind of character come last: an emoji face holds digits and signs,
            // and a Han face holds the signs of maths as wide as its own characters.
            let (apart, known) = match kind {
                Kind::Picture => (!emoji, place(EMOJI, name)),
                Kind::Han => (emoji, han.or_else(|| place(TEXT, name).map(|at| at + for_han.len()))),
                Kind::Text => (emoji || han.is_some(), place(TEXT, name)),
            };
            let dotted = DOTTED.iter().any(|dotted| name.contains(dotted));
            // The bundled faces are sans serif and proportional, and what stands in for them should be.
            let serif = name.contains("serif") && !name.contains("sans");
            (dotted, apart, family.mono, known.unwrap_or(usize::MAX), serif)
        };
        let mut order: Vec<&Family> = self.families.iter().collect();
        // Stable, so that families of one rank stay in the order of their names.
        order.sort_by_key(|family| rank(family));
        order.into_iter().map(|family| family.face).collect()
    }

    /// A face with its file's bytes, mapped once however many of the file's faces are taken.
    fn read(&mut self, face: ID) -> Option<Font> {
        let (source, index) = self.fonts.face(face).map(|face| (face.source.clone(), face.index))?;
        let path = match source {
            Source::File(path) | Source::SharedFile(path, _) => path,
            Source::Binary(_) => return None,
        };
        if !self.files.contains_key(&path) {
            // Mapped and not read: a face then costs the memory of the glyphs that are drawn, where the
            // whole file of an emoji face may be hundreds of megabytes.
            let mapped = letters_frame::map_font(&path).ok()?;
            self.files.insert(path.clone(), FontBytes::lent(Arc::new(mapped)));
        }
        self.files.get(&path).map(|data| Font::of(data.clone(), index))
    }
}

/// The system's faces for what is `uncovered`, found here and now: for an export, which has no window
/// to wait in. It reads the system's fonts each time it is called.
pub fn system_fonts(uncovered: &[Uncovered]) -> Fonts {
    if uncovered.is_empty() {
        return Fonts { faces: Vec::new() };
    }
    let han = Han::of_system().told(uncovered);
    Fonts { faces: Shelf::system(BUNDLED).find(uncovered, han) }
}

/// What the finder's thread runs: it answers what is asked until nobody can ask any more.
type Run<K> = Box<dyn FnOnce(mpsc::Receiver<(K, Vec<Uncovered>)>) + Send>;

/// Finds faces off the thread that draws, for whoever asks: a window, by its id.
pub struct Finder<K> {
    /// What starts the thread, until the first question does.
    start: Option<Run<K>>,
    asked: Option<mpsc::Sender<(K, Vec<Uncovered>)>>,
}

impl<K: Clone + Eq + Hash + Send + 'static> Finder<K> {
    /// A finder among the system's fonts. `found` is called on the finder's own thread with the faces for
    /// what one asker asked, and not at all when there are none.
    pub fn system(found: impl Fn(K, Vec<Font>) + Send + 'static) -> Self {
        Self::over(|| Shelf::system(BUNDLED), Han::of_system(), found)
    }

    /// A finder among the fonts of the shelf `shelf` makes, which it is asked for on the finder's thread.
    /// Han characters are drawn as `han` says until an asker's text says otherwise.
    pub fn over(
        shelf: impl FnOnce() -> Shelf + Send + 'static,
        han: Han,
        found: impl Fn(K, Vec<Font>) + Send + 'static,
    ) -> Self {
        let run = move |asked: mpsc::Receiver<(K, Vec<Uncovered>)>| {
            let mut shelf = shelf();
            // What each asker's text has told of its language so far, which a later character keeps to.
            let mut told: HashMap<K, Han> = HashMap::new();
            for (asker, uncovered) in asked {
                let han = told.entry(asker.clone()).or_insert(han);
                *han = han.told(&uncovered);
                let faces = shelf.find(&uncovered, *han);
                if !faces.is_empty() {
                    found(asker, faces);
                }
            }
        };
        Self { start: Some(Box::new(run)), asked: None }
    }

    /// Asks for faces that draw what is `uncovered`. The answer comes later, through `found`.
    pub fn ask(&mut self, asker: K, uncovered: Vec<Uncovered>) {
        if let Some(start) = self.start.take() {
            let (ask, asked) = mpsc::channel();
            // Without a thread nothing is found, and the characters stay as they are drawn.
            if thread::Builder::new().name("fonts".into()).spawn(move || start(asked)).is_ok() {
                self.asked = Some(ask);
            }
        }
        if let Some(asked) = &self.asked {
            let _ = asked.send((asker, uncovered));
        }
    }
}
