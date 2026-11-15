//! What a person set for themselves that is theirs wherever they write: the words they taught the spelling
//! check, and the style their new documents start with.
//!
//! Both are files in `.letters/` of the person's own space, `dictionary.txt` with a word a line and
//! `style.json`, as the desktop app keeps them in a home folder. A device that has the space as its home
//! folder therefore shares them with the server, and they outlive the server's state. The folder is hidden,
//! as every name with a leading dot is: no listing shows it, and nothing in it is found as a document.
//!
//! A server that gives nobody a space of their own has nowhere to put them, and keeps them in its store.

use std::collections::HashSet;
use std::hash::{BuildHasher, RandomState};
use std::sync::{Mutex, MutexGuard, PoisonError};

use letters_api::Settings;
use letters_format::Files;
use letters_model::Style;
use serde_json::Value;

use crate::error::{Answer, Refusal};
use crate::session::Signed;
use crate::spaces::{self, Folder, Root};
use crate::store::Table;
use crate::{App, config};

/// What each person set, by their id. The words and the style are here only for whoever has no space of
/// their own, and until they are moved out for whoever has.
pub(crate) const SETTINGS: Table<Settings> = Table::new("settings");

const DICTIONARY: &str = "dictionary.txt";
const STYLE: &str = "style.json";
/// How much of either file is read. A person may write them by hand, at any length.
const MOST: u64 = 4 << 20;
/// The longest word, in characters, and how many a person may keep.
const WORD: usize = 64;
const WORDS: usize = 10_000;

/// One writer at a time for what a person keeps. Two people seldom share a lock, and nothing is kept for
/// each of them.
pub struct Homes {
    locks: [Mutex<()>; 16],
    hasher: RandomState,
}

impl Default for Homes {
    fn default() -> Self {
        Self { locks: Default::default(), hasher: RandomState::new() }
    }
}

impl Homes {
    fn hold(&self, person: &str) -> MutexGuard<'_, ()> {
        let at = self.hasher.hash_one(person) as usize % self.locks.len();
        self.locks[at].lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The words of a dictionary file, in their order and each once.
fn listed(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let words = text.lines().map(str::trim).filter(|word| !word.is_empty());
    words.filter(|word| seen.insert(*word)).map(str::to_owned).collect()
}

/// The dictionary file as far as it is read, and whether that is all of it.
fn dictionary(folder: Option<&Folder>) -> Answer<(String, bool)> {
    let Some(bytes) = folder.map(|folder| folder.start(DICTIONARY, MOST)).transpose()?.flatten() else {
        return Ok((String::new(), true));
    };
    let whole = (bytes.len() as u64) < MOST;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    // What was cut may have been cut within a word.
    if !whole {
        text.truncate(text.rfind('\n').map_or(0, |at| at + 1));
    }
    Ok((text, whole))
}

/// Adds lines to the dictionary file. A file someone edited by hand may not end its last line.
fn learn(folder: &mut Folder, known: &str, words: &[&str]) -> Answer<()> {
    let gap = if known.is_empty() || known.ends_with('\n') { "" } else { "\n" };
    let lines: String = words.iter().flat_map(|word| [word, "\n"]).collect();
    Ok(Files::append(folder, DICTIONARY, format!("{gap}{lines}").as_bytes())?)
}

/// Writes `bytes` as the file `name`, beside it first: whoever reads meanwhile finds the old file or the
/// new one.
fn replace(folder: &mut Folder, name: &str, bytes: &[u8]) -> Answer<()> {
    let beside = format!("{name}.new");
    Files::write(folder, &beside, bytes)?;
    Ok(Files::rename(folder, &beside, name)?)
}

/// The style file, when it is there and reads as a style.
fn style(folder: Option<&Folder>) -> Answer<Option<Value>> {
    let Some(bytes) = folder.map(|folder| folder.start(STYLE, MOST)).transpose()?.flatten() else {
        return Ok(None);
    };
    let read = String::from_utf8(bytes).ok().filter(|text| Style::from_json(text).is_ok());
    Ok(read.and_then(|text| serde_json::from_str(&text).ok()))
}

impl App {
    /// The person's own space, on a server that gives everyone one. A name that cannot name a folder has
    /// none, as it has no space.
    fn home(&self, who: &Signed) -> Answer<Option<Root>> {
        let own = self.config.spaces.iter().find(|space| config::Space::personal(space));
        match own.filter(|_| spaces::username_ok(&who.person.username)) {
            Some(space) => Ok(Some(self.root(who, space)?)),
            None => Ok(None),
        }
    }

    /// Holds the person's lock and opens their own space, with whatever an earlier build kept in the
    /// store for them written out to its files first.
    fn kept(&self, who: &Signed) -> Answer<(MutexGuard<'_, ()>, Option<Root>)> {
        let held = self.homes.hold(&who.person.id);
        let home = self.home(who)?;
        if let Some(root) = &home {
            self.move_out(who, root)?;
        }
        Ok((held, home))
    }

    /// Writes the words and the style of the person's row in the store to their files, once: the row has
    /// neither afterwards. A word joins those the file has, and a style gives way to one the file has.
    fn move_out(&self, who: &Signed, root: &Root) -> Answer<()> {
        let row = self.store.get(&SETTINGS, &who.person.id)?.unwrap_or_default();
        if row.dictionary.is_empty() && row.default_style.is_none() {
            return Ok(());
        }
        let Some(mut folder) = root.kept(true)? else { return Err(Refusal::not_found()) };
        let (text, _) = dictionary(Some(&folder))?;
        let known: HashSet<&str> = text.lines().map(str::trim).collect();
        let new: Vec<&str> = row.dictionary.iter().map(String::as_str).filter(|word| !known.contains(word)).collect();
        if !new.is_empty() {
            learn(&mut folder, &text, &new)?;
        }
        if let Some(theirs) = &row.default_style
            && style(Some(&folder))?.is_none()
        {
            replace(&mut folder, STYLE, theirs.to_string().as_bytes())?;
        }
        self.store.write(|batch| {
            let mut row = batch.get(&SETTINGS, &who.person.id)?.unwrap_or_default();
            (row.dictionary, row.default_style) = (Vec::new(), None);
            batch.put(&SETTINGS, &who.person.id, &row)
        })?;
        Ok(())
    }

    /// What `who` set for themselves.
    pub fn settings(&self, who: &Signed) -> Answer<Settings> {
        let (_held, home) = self.kept(who)?;
        let mut settings = self.store.get(&SETTINGS, &who.person.id)?.unwrap_or_default();
        if let Some(root) = home {
            let folder = root.kept(false)?;
            settings.dictionary = listed(&dictionary(folder.as_ref())?.0);
            settings.default_style = style(folder.as_ref())?;
        }
        Ok(settings)
    }

    /// Keeps what `who` set, in place of what they had set, and answers what is kept. The words are no
    /// part of it: they change one at a time ([`App::word`]). What a whole list sent here would do is
    /// bring back a word that another device took out, so the words kept stay as they are.
    pub fn set_settings(&self, who: &Signed, mut settings: Settings) -> Answer<Settings> {
        if let Some(style) = &settings.default_style
            && let Err(error) = Style::from_json(&style.to_string())
        {
            return Err(Refusal::invalid(format!("The default style cannot be read: {error}.")));
        }
        let (_held, home) = self.kept(who)?;
        let Some(root) = home else {
            return Ok(self.store.write(|batch| {
                settings.dictionary = batch.get(&SETTINGS, &who.person.id)?.unwrap_or_default().dictionary;
                batch.put(&SETTINGS, &who.person.id, &settings)?;
                Ok(settings)
            })?);
        };
        // The file is written only when the style is another, and is not left behind without one.
        let theirs = settings.default_style.take();
        let mut folder = root.kept(theirs.is_some())?;
        if let Some(folder) = &mut folder
            && style(Some(folder))? != theirs
        {
            match &theirs {
                Some(style) => replace(folder, STYLE, style.to_string().as_bytes())?,
                None => Files::remove(folder, STYLE).or_else(|error| match error.kind() {
                    std::io::ErrorKind::NotFound => Ok(()),
                    _ => Err(error),
                })?,
            }
        }
        settings.dictionary = Vec::new();
        self.store.put(&SETTINGS, &who.person.id, &settings)?;
        settings.dictionary = listed(&dictionary(folder.as_ref())?.0);
        settings.default_style = theirs;
        Ok(settings)
    }

    /// The name `who` signs what they write with: the one they chose, or the one they sign in under.
    pub fn signs_as(&self, who: &Signed) -> Answer<String> {
        let settings = self.store.get(&SETTINGS, &who.person.id)?.unwrap_or_default();
        Ok(settings.name.unwrap_or_else(|| who.person.name.clone()))
    }

    /// Adds a word to the person's own, or takes it out, and answers the words as they are then. One
    /// read and write under the person's lock, so two devices that change words at once both have their
    /// way.
    pub fn word(&self, who: &Signed, word: &str, add: bool) -> Answer<Vec<String>> {
        // Only a word going in is held to the rule: whatever is there can be taken out.
        let fits = !word.is_empty() && word.chars().count() <= WORD;
        if add && !(fits && word.chars().all(|letter| !letter.is_whitespace() && !letter.is_control())) {
            return Err(Refusal::invalid(format!(
                "A word in your dictionary is a single word of up to {WORD} characters, with no spaces."
            )));
        }
        let full = || {
            Refusal::invalid(format!("Your dictionary already holds {WORDS} words. Remove some before adding more."))
        };
        let (_held, home) = self.kept(who)?;
        let Some(root) = home else {
            let words = self.store.write(|batch| {
                let mut settings = batch.get(&SETTINGS, &who.person.id)?.unwrap_or_default();
                let at = settings.dictionary.iter().position(|known| known == word);
                match (add, at) {
                    (true, None) if settings.dictionary.len() >= WORDS => return Ok(None),
                    (true, None) => settings.dictionary.push(word.to_owned()),
                    (false, Some(at)) => drop(settings.dictionary.remove(at)),
                    _ => return Ok(Some(settings.dictionary)),
                }
                batch.put(&SETTINGS, &who.person.id, &settings)?;
                Ok(Some(settings.dictionary))
            })?;
            return words.ok_or_else(full);
        };
        let Some(mut folder) = root.kept(add)? else { return Ok(Vec::new()) };
        let (text, whole) = dictionary(Some(&folder))?;
        let mut words = listed(&text);
        match (add, words.iter().position(|known| known == word)) {
            (true, None) if words.len() >= WORDS => return Err(full()),
            (true, None) => {
                learn(&mut folder, &text, &[word])?;
                words.push(word.to_owned());
            }
            (false, Some(at)) => {
                // Written again from what was read, which must then be all of it.
                if !whole {
                    return Err(Refusal::invalid("Your dictionary file is too long for the server to change."));
                }
                // The other lines stay as they are, whatever a hand wrote in them.
                let stays: String =
                    text.lines().filter(|line| line.trim() != word).flat_map(|line| [line, "\n"]).collect();
                replace(&mut folder, DICTIONARY, stays.as_bytes())?;
                words.remove(at);
            }
            _ => {}
        }
        Ok(words)
    }

    /// The style the new documents of `who` start with: their own, or the built-in one.
    pub(crate) fn new_style(&self, who: &Signed) -> Answer<Style> {
        let style = self.settings(who)?.default_style;
        Ok(style.and_then(|style| Style::from_json(&style.to_string()).ok()).unwrap_or_default())
    }
}
