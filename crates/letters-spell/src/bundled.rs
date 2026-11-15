//! The English dictionaries compiled into every build, so English is checked without touching the system.
//!
//! They are SCOWL's Hunspell lists (see `dictionaries/SCOWL.txt` for their licence). The four differ in a
//! few thousand of their 47,000 entries, so American English is stored whole and the others as their
//! differences from it. `tests/bundled.rs` rebuilds the files from a SCOWL release.
//!
//! A build with the `provided` feature leaves them out for the shell to hand over, as the browser shell
//! does: a page shows its first frame before it has spelling to mark.

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::Dictionary;

/// The files in `dictionaries/`, in the order [`provide`] takes them: the affix rules, American English
/// whole, and what British, Australian and Canadian English change in it.
pub const FILES: [&str; 5] = ["en.aff", "en_US.dic", "en_GB.diff", "en_AU.diff", "en_CA.diff"];

/// What the shell handed over, for a build that carries no dictionaries.
static PROVIDED: OnceLock<[String; 5]> = OnceLock::new();

/// Hands over the contents of [`FILES`], in that order, for a build with the `provided` feature. They are
/// taken once, and English is unchecked until then. A build that carries its dictionaries has no use for them.
pub fn provide(files: Vec<String>) {
    if cfg!(feature = "provided")
        && let Ok(files) = <[String; 5]>::try_from(files)
    {
        let _ = PROVIDED.set(files);
    }
}

/// The contents of [`FILES`], when the build has them.
#[cfg(not(feature = "provided"))]
fn files() -> Option<[&'static str; 5]> {
    Some([
        include_str!("../dictionaries/en.aff"),
        include_str!("../dictionaries/en_US.dic"),
        include_str!("../dictionaries/en_GB.diff"),
        include_str!("../dictionaries/en_AU.diff"),
        include_str!("../dictionaries/en_CA.diff"),
    ])
}

#[cfg(feature = "provided")]
fn files() -> Option<[&'static str; 5]> {
    PROVIDED.get().map(|files| std::array::from_fn(|i| files[i].as_str()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum English {
    American,
    British,
    Australian,
    Canadian,
}

impl English {
    /// The dictionary for English as written in `region`: American where none is named, and British for
    /// every region without a dictionary of its own, since most of them follow British spelling.
    pub fn of(region: Option<&str>) -> Self {
        match region {
            None | Some("US" | "PH") => Self::American,
            Some("CA") => Self::Canadian,
            Some("AU") => Self::Australian,
            Some(_) => Self::British,
        }
    }

    /// Parsed on first use, then shared by every [`crate::Speller`]. `None` until the build has its files.
    pub fn dictionary(self) -> Option<&'static Dictionary> {
        static PARSED: [OnceLock<Dictionary>; 4] = [const { OnceLock::new() }; 4];
        let [aff, american, british, australian, canadian] = files()?;
        Some(PARSED[self as usize].get_or_init(|| {
            let dic = match self {
                Self::American => american.to_owned(),
                Self::British => patched(american, british),
                Self::Australian => patched(american, australian),
                Self::Canadian => patched(american, canadian),
            };
            Dictionary::parse(aff, &dic).expect("the bundled dictionaries are valid")
        }))
    }
}

/// The American `.dic` file with a variant's differences applied: `-` lines leave and `+` lines join.
fn patched(american: &str, diff: &str) -> String {
    let removed: HashSet<&str> = diff.lines().filter_map(|line| line.strip_prefix('-')).collect();
    let kept = american.lines().skip(1).filter(|entry| !removed.contains(entry));
    let entries: Vec<&str> = kept.chain(diff.lines().filter_map(|line| line.strip_prefix('+'))).collect();
    format!("{}\n{}\n", entries.len(), entries.join("\n"))
}
