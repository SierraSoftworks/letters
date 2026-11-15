//! What the person set for themselves, kept in `~/.letters/settings.toml` beside what they trust.
//!
//! ```toml
//! [appearance]
//! theme = "dark"       # or "light". Left out, the window follows the system.
//! accent = "#1f8a5b"
//!
//! [writer]
//! name = "Ada Lovelace"
//! language = "en-GB"
//! ```
//!
//! Only these keys are touched. Whatever else the file holds, comments and all, stays as its writer had it.

use std::io;
use std::path::Path;

use letters_trust::Store;
use letters_ui::{Appearance, Settings};
use toml_edit::{DocumentMut, Item, Table, TomlError, value};

use crate::trust::Settings as File;

/// Reads the settings from the file's text. A key that is missing, or holds what makes no sense for it, is
/// as if it had never been set, and so is everything in text that is no TOML.
pub fn read(text: &str) -> Settings {
    let Ok(file) = text.parse::<DocumentMut>() else { return Settings::default() };
    let said = |table: &str, key: &str| {
        let said = file.get(table)?.get(key)?.as_str()?.trim();
        Some(said.to_owned()).filter(|said| !said.is_empty())
    };
    Settings {
        appearance: match said("appearance", "theme").as_deref() {
            Some("light") => Appearance::Light,
            Some("dark") => Appearance::Dark,
            _ => Appearance::System,
        },
        accent: said("appearance", "accent").and_then(|accent| colour(&accent)),
        name: said("writer", "name"),
        language: said("writer", "language"),
    }
}

/// The file's text with `settings` written into it. Text that is no TOML is refused with the reason, since
/// nothing could be written into it without losing what its writer meant.
pub fn written(text: &str, settings: &Settings) -> Result<String, TomlError> {
    let mut file = text.parse::<DocumentMut>()?;
    let theme = match settings.appearance {
        Appearance::System => None,
        Appearance::Light => Some("light".to_owned()),
        Appearance::Dark => Some("dark".to_owned()),
    };
    let accent = settings.accent.map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}"));
    let keys = [
        ("appearance", "theme", theme),
        ("appearance", "accent", accent),
        ("writer", "name", settings.name.clone()),
        ("writer", "language", settings.language.clone()),
    ];
    for (table, key, set) in keys {
        match set {
            Some(set) => {
                let table = file.entry(table).or_insert_with(|| Item::Table(Table::new()));
                // Something else under the table's name is not ours to write over.
                if let Some(table) = table.as_table_like_mut() {
                    table.insert(key, value(set));
                }
            }
            // What is no longer set goes from the file. Its table stays, with whatever else it holds.
            None => _ = file.get_mut(table).and_then(Item::as_table_like_mut).and_then(|table| table.remove(key)),
        }
    }
    Ok(file.to_string())
}

/// Reads `#rrggbb`.
fn colour(hex: &str) -> Option<[u8; 3]> {
    let digits = hex.strip_prefix('#')?;
    // A sign would parse as a number, and is no part of a colour.
    if digits.len() != 6 || !digits.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = u32::from_str_radix(digits, 16).ok()?;
    Some([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8])
}

/// The settings of the person whose home folder is `home`. No file, or one that cannot be read, is a person
/// who has set nothing.
pub fn load(home: &Path) -> Settings {
    File::of(home).read().ok().flatten().map(|text| read(&text)).unwrap_or_default()
}

/// Keeps `settings` in the file of the person whose home folder is `home`.
pub fn keep(home: &Path, settings: &Settings) -> io::Result<()> {
    let mut file = File::of(home);
    let text = file.read()?.unwrap_or_default();
    // A file that already says all this is left alone, down to its date.
    if read(&text) == *settings {
        return Ok(());
    }
    let text = written(&text, settings).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    file.write(&text)
}
