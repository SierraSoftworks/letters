//! The system's Hunspell dictionaries, for text in languages other than English, which is built in.
//!
//! A dictionary is an `.aff` file and a `.dic` file of one name, such as `de_DE` or `pt_BR`. They are large
//! and slow to read, so only those a document names are looked for, away from the thread that paints. A
//! language the system has no dictionary for is asked of the server a document is kept at, if it is kept
//! at one, and else simply left unchecked.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::{env, fs, io};

use letters_model::Document;
use letters_spell::{Dictionary, best_dictionary, dictionary_language};

use crate::session::Session;

/// A file larger than this is not read: no word list is worth that much memory.
const LARGEST: u64 = 64 << 20;

/// The tag a dictionary's name or a passage's language stands for, as `de` or `de-CH`, by the rule the
/// server shares (`letters_spell::dictionary_language`). `None` for English: it is built in, and never
/// read from the system.
fn other(name: &str) -> Option<String> {
    dictionary_language(name).filter(|tag| tag != "en" && !tag.starts_with("en-"))
}

/// The languages a document names that a system dictionary could serve: its own, and those of its blocks
/// and runs, as `de` or `de-CH`.
pub fn languages(document: &Document) -> BTreeSet<String> {
    let mut named = BTreeSet::new();
    let mut note = |tag: &str| named.extend(other(tag));
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

/// The languages the system has dictionaries for, as tags such as `de-CH`, each once. English is left out:
/// it is built in, and never read from the system.
pub fn installed(folders: &[PathBuf]) -> BTreeSet<String> {
    let mut languages = BTreeSet::new();
    for entry in folders.iter().flat_map(|folder| fs::read_dir(folder).into_iter().flatten().flatten()) {
        let aff = entry.path();
        let named = aff.extension().is_some_and(|extension| extension == "aff") && aff.with_extension("dic").is_file();
        languages.extend(aff.file_stem().and_then(|name| name.to_str()).filter(|_| named).and_then(other));
    }
    languages
}

/// A dictionary found on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The name its two files share, such as `de_DE`.
    pub name: String,
    pub aff: PathBuf,
    pub dic: PathBuf,
}

/// Finds the dictionary that best serves `language`, a tag such as `de` or `de-CH`, by the rule the server
/// shares (`letters_spell::best_dictionary`): one for its region, else one for the language alone, else
/// that of the country the language is named after, else any. A plain name is preferred to a variant such
/// as `de_DE_frami`, and an earlier folder to a later one.
pub fn find(folders: &[PathBuf], language: &str) -> Option<Found> {
    let mut found = Vec::new();
    for folder in folders {
        let mut here = Vec::new();
        for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
            let aff = entry.path();
            let name = aff.file_stem().and_then(|name| name.to_str()).map(str::to_owned);
            let Some(name) = name.filter(|_| aff.extension().is_some_and(|extension| extension == "aff")) else {
                continue;
            };
            let dic = aff.with_extension("dic");
            if dic.is_file() {
                here.push(Found { name, aff, dic });
            }
        }
        // A folder is read in no order, and of two that serve alike the earlier is taken.
        here.sort_by(|a, b| a.name.cmp(&b.name));
        found.append(&mut here);
    }
    let best = best_dictionary(found.iter().map(|found| found.name.as_str()), language)?;
    Some(found.swap_remove(best))
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

/// The dictionaries among `held`, a server's by their tags, that text in `languages` is checked against:
/// each by the server's tag for it, with the tag a speller takes it under. The one a region the server has
/// none for falls back to goes under the language alone, where a speller looks for such a region's, so
/// that the speller and the server choose alike. The browser asks by the same rule.
pub fn served(languages: &BTreeSet<String>, held: &[String]) -> BTreeMap<String, String> {
    let best = |tag: &str| best_dictionary(held.iter().map(String::as_str), tag);
    let mut wanted = BTreeMap::new();
    for tag in languages {
        let Some(at) = best(tag) else { continue };
        let language = tag.split('-').next().unwrap_or(tag);
        let under = if best(language) == Some(at) { language.to_owned() } else { held[at].clone() };
        wanted.insert(held[at].clone(), under);
    }
    wanted
}

/// Reads the dictionary a server has for the language `tag`, one of those it lists. `None` when it
/// cannot be had or used: the language is then not checked, which is no error to show anyone. This waits
/// on the network.
pub fn fetch(session: &Session, tag: &str) -> Option<Dictionary> {
    let parsed = session
        .dictionary(tag, LARGEST)
        .map_err(|refused| refused.to_string())
        .and_then(|(aff, dic)| Dictionary::parse(&aff, &dic).map_err(|error| error.to_string()));
    parsed.inspect_err(|error| eprintln!("letters: the server's dictionary for {tag} cannot be used: {error}")).ok()
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
