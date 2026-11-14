//! Rebuilds `dictionaries/` from a SCOWL release, so the bundled English needs no throwaway script.
//!
//! Unzip the release's Hunspell archives for `en_US`, `en_GB-ise`, `en_GB-ize`, `en_AU`, `en_CA` and the
//! four `-large` ones into one folder (https://sourceforge.net/projects/wordlist/files/speller/), then run:
//!
//! ```sh
//! SCOWL=/that/folder cargo test -p letters-spell --test bundled -- --ignored
//! ```

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Each bundled variant: its name, the standard lists it accepts and the large list its accents come from.
const VARIANTS: [(&str, &[&str], &str); 4] = [
    ("en_US", &["en_US"], "en_US-large"),
    // British English is written with both "-ise" and "-ize", so it takes both lists.
    ("en_GB", &["en_GB-ise", "en_GB-ize"], "en_GB-large"),
    ("en_AU", &["en_AU"], "en_AU-large"),
    ("en_CA", &["en_CA"], "en_CA-large"),
];

#[test]
#[ignore = "rewrites dictionaries/ from the SCOWL release in $SCOWL"]
fn rebuild_the_bundled_dictionaries() {
    let source = PathBuf::from(std::env::var("SCOWL").expect("set SCOWL to the folder holding the release"));
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("dictionaries");
    let read = |name: &str| fs::read_to_string(source.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    // The entries of a `.dic` file, without its leading count.
    let entries = |name: &str| read(&format!("{name}.dic")).lines().skip(1).map(str::to_owned).collect::<Vec<_>>();

    let lists = VARIANTS.map(|(_, standard, large)| {
        let mut list: BTreeSet<String> = standard.iter().flat_map(|name| entries(name)).collect();
        // The standard lists hold one spelling of each word and so drop "café" for "cafe". The large lists
        // keep both: take their accented spellings of words the standard list already holds.
        let stems: BTreeSet<String> = list.iter().map(|entry| stem(entry).to_owned()).collect();
        list.extend(entries(large).into_iter().filter(|e| !e.is_ascii() && stems.contains(&unaccented(stem(e)))));
        list
    });

    let american = &lists[0];
    let whole = american.iter().fold(format!("{}\n", american.len()), |text, entry| text + entry + "\n");
    fs::write(target.join("en_US.dic"), whole).unwrap();
    for ((name, ..), list) in VARIANTS.iter().zip(&lists).skip(1) {
        let removed = american.difference(list).map(|entry| format!("-{entry}\n"));
        let added = list.difference(american).map(|entry| format!("+{entry}\n"));
        fs::write(target.join(format!("{name}.diff")), removed.chain(added).collect::<String>()).unwrap();
    }
    // The affix files differ only in the comment that names them.
    fs::write(target.join("en.aff"), read("en_US.aff")).unwrap();
    fs::write(target.join("SCOWL.txt"), read("README_en_US.txt")).unwrap();
}

/// The word of a `.dic` entry, without its flags.
fn stem(entry: &str) -> &str {
    entry.split('/').next().unwrap_or(entry)
}

fn unaccented(word: &str) -> String {
    const PLAIN: [(&str, char); 12] = [
        ("áàâäãå", 'a'),
        ("ç", 'c'),
        ("éèêë", 'e'),
        ("íìîï", 'i'),
        ("ñ", 'n'),
        ("óòôöõ", 'o'),
        ("úùûü", 'u'),
        ("ÁÀÂÄÅ", 'A'),
        ("ÉÈÊ", 'E'),
        ("ÓÖ", 'O'),
        ("ÚÜ", 'U'),
        ("Ç", 'C'),
    ];
    word.chars()
        .map(|c| PLAIN.iter().find(|(accented, _)| accented.contains(c)).map_or(c, |&(_, plain)| plain))
        .collect()
}
