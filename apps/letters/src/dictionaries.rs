//! The system's Hunspell dictionaries, for text in languages other than English, which is built in.
//!
//! A dictionary is an `.aff` file and a `.dic` file of one name, such as `de_DE` or `pt_BR`. They are large
//! and slow to read, so only those a document names are looked for, away from the thread that paints. A
//! language the system has no dictionary for is simply left unchecked.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::{env, fs, io};

use letters_model::Document;
use letters_spell::Dictionary;

/// A file larger than this is not read: no word list is worth that much memory.
const LARGEST: u64 = 64 << 20;

/// The parts of a language tag or of a dictionary's file name that choose a dictionary.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Tag {
    /// In lower case.
    language: String,
    /// In upper case.
    region: Option<String>,
    /// Whether the name says more than a language and a region, as `de_DE_frami` and `sr-Latn-RS` do.
    more: bool,
}

impl Tag {
    /// Reads a BCP 47 tag such as `pt-BR`, or a file name such as `pt_BR`.
    fn parse(name: &str) -> Option<Self> {
        let alphabetic = |part: &str| part.bytes().all(|b| b.is_ascii_alphabetic());
        let mut parts = name.trim().split(['-', '_']);
        let language = parts.next().filter(|part| matches!(part.len(), 2 | 3) && alphabetic(part))?;
        let rest: Vec<&str> = parts.collect();
        let region = rest.iter().find(|part| part.len() == 2 && alphabetic(part));
        Some(Self {
            language: language.to_ascii_lowercase(),
            region: region.map(|region| region.to_ascii_uppercase()),
            more: rest.len() > usize::from(region.is_some()),
        })
    }

    fn written(&self) -> String {
        match &self.region {
            Some(region) => format!("{}-{region}", self.language),
            None => self.language.clone(),
        }
    }
}

/// The languages a document names that a system dictionary could serve: its own, and those of its blocks
/// and runs, as `de` or `de-CH`.
pub fn languages(document: &Document) -> BTreeSet<String> {
    let mut named = BTreeSet::new();
    let mut note = |tag: &str| {
        if let Some(tag) = Tag::parse(tag).filter(|tag| tag.language != "en") {
            named.insert(tag.written());
        }
    };
    note(&document.language);
    for block in document.walk() {
        block.lang.as_deref().map(&mut note);
        block.body.runs().iter().filter_map(|run| run.lang.as_deref()).for_each(&mut note);
    }
    named
}

/// Where Hunspell dictionaries are kept on this system, the person's own folders first. `DICPATH`, which
/// Hunspell itself reads, comes before them all.
pub fn system_folders(home: &Path) -> Vec<PathBuf> {
    let listed = |name: &str| env::var_os(name).map(|paths| env::split_paths(&paths).collect::<Vec<_>>());
    let mut folders = listed("DICPATH").unwrap_or_default();
    if cfg!(target_os = "macos") {
        folders.extend([home.join("Library/Spelling"), "/Library/Spelling".into()]);
    } else if cfg!(unix) {
        let own = env::var_os("XDG_DATA_HOME").map_or_else(|| home.join(".local/share"), PathBuf::from);
        let shared = listed("XDG_DATA_DIRS").unwrap_or_else(|| vec!["/usr/local/share".into(), "/usr/share".into()]);
        for data in std::iter::once(own).chain(shared) {
            folders.extend(["hunspell", "myspell", "myspell/dicts"].map(|inside| data.join(inside)));
        }
    }
    folders.retain(|folder| folder.is_absolute());
    folders
}

/// A dictionary found on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The name its two files share, such as `de_DE`.
    pub name: String,
    pub aff: PathBuf,
    pub dic: PathBuf,
}

/// Finds the dictionary that best serves `language`, a tag such as `de` or `de-CH`: one for its region, else
/// one for the language alone, else that of the country the language is named after, else any. A plain
/// name is preferred to a variant such as `de_DE_frami`, and an earlier folder to a later one.
pub fn find(folders: &[PathBuf], language: &str) -> Option<Found> {
    let wanted = Tag::parse(language)?;
    let rank = |tag: &Tag| match &tag.region {
        region if *region == wanted.region => 0,
        None => 1,
        Some(region) if region.eq_ignore_ascii_case(&wanted.language) => 2,
        Some(_) => 3,
    };
    let mut found: Vec<_> = Vec::new();
    for (place, folder) in folders.iter().enumerate() {
        for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
            let aff = entry.path();
            let name = aff.file_stem().and_then(|name| name.to_str()).map(str::to_owned);
            let Some(name) = name.filter(|_| aff.extension().is_some_and(|extension| extension == "aff")) else {
                continue;
            };
            let Some(tag) = Tag::parse(&name).filter(|tag| tag.language == wanted.language) else { continue };
            let dic = aff.with_extension("dic");
            if dic.is_file() {
                found.push(((rank(&tag), tag.more, place, name.clone()), Found { name, aff, dic }));
            }
        }
    }
    found.into_iter().min_by(|a, b| a.0.cmp(&b.0)).map(|(_, found)| found)
}

/// Reads the dictionary that best serves `language`, with the name it has on disk. `None` when there is
/// none, or it cannot be read: the language is then not checked, which is no error to show anyone.
pub fn load(folders: &[PathBuf], language: &str) -> Option<(String, Dictionary)> {
    let found = find(folders, language)?;
    let parsed = read(&found.aff).and_then(|aff| {
        let dic = read(&found.dic)?;
        let (aff, dic) = decode(&aff, &dic);
        Dictionary::parse(&aff, &dic).map_err(io::Error::other)
    });
    match parsed {
        Ok(dictionary) => Some((found.name, dictionary)),
        Err(error) => {
            eprintln!("letters: the dictionary {} cannot be used: {error}", found.aff.display());
            None
        }
    }
}

fn read(path: &Path) -> io::Result<Vec<u8>> {
    if fs::metadata(path)?.len() > LARGEST {
        return Err(io::Error::other("it is too large"));
    }
    fs::read(path)
}

/// The text of a dictionary's two files. Older dictionaries are in the encoding their `SET` line names.
fn decode(aff: &[u8], dic: &[u8]) -> (String, String) {
    if let (Ok(aff), Ok(dic)) = (str::from_utf8(aff), str::from_utf8(dic)) {
        return (aff.to_owned(), dic.to_owned());
    }
    let set = aff.split(|byte| *byte == b'\n').find_map(|line| line.strip_prefix(b"SET"));
    let label = set.map(|label| String::from_utf8_lossy(label).trim().to_ascii_lowercase()).unwrap_or_default();
    // Hunspell's names for encodings are nearly the web's.
    let label = match label.strip_prefix("microsoft-").unwrap_or(&label) {
        label if label.starts_with("tis620") => "tis-620",
        label => label,
    };
    // Hunspell reads a file that names no encoding as Latin-1.
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252);
    let text = |bytes| encoding.decode_without_bom_handling(bytes).0.into_owned();
    (text(aff), text(dic))
}
