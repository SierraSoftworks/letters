//! The English dictionaries compiled into every build, so English is checked without touching the system.
//!
//! They are SCOWL's Hunspell lists (see `dictionaries/SCOWL.txt` for their licence). The four differ in a
//! few thousand of their 47,000 entries, so American English is stored whole and the others as their
//! differences from it. `tests/bundled.rs` rebuilds the files from a SCOWL release.

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::Dictionary;

const AFF: &str = include_str!("../dictionaries/en.aff");
const AMERICAN: &str = include_str!("../dictionaries/en_US.dic");

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

    /// Parsed on first use, then shared by every [`crate::Speller`].
    pub fn dictionary(self) -> &'static Dictionary {
        static PARSED: [OnceLock<Dictionary>; 4] = [const { OnceLock::new() }; 4];
        PARSED[self as usize].get_or_init(|| {
            let dic = match self {
                Self::American => AMERICAN.to_owned(),
                Self::British => patched(include_str!("../dictionaries/en_GB.diff")),
                Self::Australian => patched(include_str!("../dictionaries/en_AU.diff")),
                Self::Canadian => patched(include_str!("../dictionaries/en_CA.diff")),
            };
            Dictionary::parse(AFF, &dic).expect("the bundled dictionaries are valid")
        })
    }
}

/// The American `.dic` file with a variant's differences applied: `-` lines leave and `+` lines join.
fn patched(diff: &str) -> String {
    let removed: HashSet<&str> = diff.lines().filter_map(|line| line.strip_prefix('-')).collect();
    let kept = AMERICAN.lines().skip(1).filter(|entry| !removed.contains(entry));
    let entries: Vec<&str> = kept.chain(diff.lines().filter_map(|line| line.strip_prefix('+'))).collect();
    format!("{}\n{}\n", entries.len(), entries.join("\n"))
}
