//! What the desktop lends the editor: a clock, the clipboard and identifiers.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use letters_editor::{Clip, Platform};

use crate::pictures;
use crate::words::Words;

pub struct Desktop {
    start: Instant,
    /// Connected on first use, since reaching the display server would slow every launch.
    clipboard: Option<arboard::Clipboard>,
    /// Where a word the person teaches the app is kept.
    words: Option<Words>,
    /// The assets the editor handed over since they were last taken.
    stored: Vec<(String, Vec<u8>)>,
}

impl Desktop {
    pub fn new() -> Self {
        Self { start: Instant::now(), clipboard: None, words: None, stored: Vec::new() }
    }

    /// Keeps the words the person adds in `words`. Without this they last as long as their window.
    pub fn with_words(self, words: Words) -> Self {
        Self { words: Some(words), ..self }
    }

    /// The moment the editor's clock reads `ms`.
    pub fn instant(&self, ms: u64) -> Instant {
        self.start + Duration::from_millis(ms)
    }

    /// The assets the editor has stored since this was last asked, for the window whose document names
    /// them: its file, its journal and its screen all need them.
    pub fn take_assets(&mut self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.stored)
    }

    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }
}

impl Default for Desktop {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for Desktop {
    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn wall_time(&self) -> Option<u64> {
        SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|since| since.as_secs())
    }

    fn clipboard_read(&mut self) -> Option<Clip> {
        let clipboard = self.clipboard()?;
        // Copied picture files are pasted as their pictures, not as the paths that come with them.
        let files = clipboard.get().file_list().unwrap_or_default();
        let pictures = pictures::from_files(&files);
        if !pictures.is_empty() {
            return Some(Clip { pictures, ..Clip::default() });
        }
        let html = clipboard.get().html().ok().filter(|html| !html.is_empty());
        let text = clipboard.get_text().ok().filter(|text| !text.is_empty());
        if text.is_none() && html.is_none() {
            // Only a picture, as a screenshot is.
            let image = clipboard.get_image().ok()?;
            let picture = pictures::from_rgba(image.width as u32, image.height as u32, &image.bytes)?;
            return Some(Clip { pictures: vec![picture], ..Clip::default() });
        }
        Some(Clip { text: text.unwrap_or_default(), html, ..Clip::default() })
    }

    fn clipboard_write(&mut self, clip: Clip) {
        let Some(clipboard) = self.clipboard() else { return };
        // A picture alone, as a code block copied as one is, goes as a picture.
        let picture = clip.pictures.first().filter(|_| clip.text.is_empty() && clip.html.is_none());
        if let Some(image) = picture.and_then(pictures::to_rgba) {
            let _ = clipboard.set_image(image);
            return;
        }
        // A failed copy has nowhere useful to be reported: the clipboard simply keeps what it had.
        let _ = match clip.html {
            Some(html) => clipboard.set_html(html, Some(clip.text)),
            None => clipboard.set_text(clip.text),
        };
    }

    fn new_id(&mut self) -> String {
        block_id()
    }

    fn store_asset(&mut self, name: &str, bytes: &[u8]) {
        self.stored.push((name.to_owned(), bytes.to_vec()));
    }

    fn learn_word(&mut self, word: &str) {
        if let Some(words) = &self.words
            && let Err(error) = words.learn(word)
        {
            eprintln!("letters: could not keep the word {word:?}: {error}");
        }
    }
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).expect("the system has a source of randomness");
    bytes
}

/// A block id: 80 random bits as 16 characters, short enough to keep `document.json` readable.
pub fn block_id() -> String {
    letters_editor::block_id(random())
}

/// A document id: a random (version 4) UUID.
pub fn document_id() -> String {
    let mut b = random::<16>();
    b[6] = b[6] & 0x0f | 0x40;
    b[8] = b[8] & 0x3f | 0x80;
    let hex: String = b.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// The current time as RFC 3339, in UTC.
pub fn now_rfc3339() -> String {
    rfc3339(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs()))
}

/// How far the person's clock is ahead of UTC right now, in minutes. Behind UTC it is negative.
pub fn utc_offset() -> i32 {
    jiff::Zoned::now().offset().seconds() / 60
}

/// A time written in UTC, as RFC 3339 with the offset of the person's own time zone. What is kept on disk
/// stays in UTC: this is for what a window shows, which has no clock or time zone of its own.
pub fn local(utc: &str) -> String {
    let Ok(time) = utc.parse::<jiff::Timestamp>() else { return utc.to_owned() };
    time.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

/// Seconds since the Unix epoch as RFC 3339, in UTC.
pub fn rfc3339(unix_seconds: u64) -> String {
    let (days, seconds) = (unix_seconds / 86_400, unix_seconds % 86_400);
    // Days to a civil date, after Howard Hinnant's `civil_from_days`. Years here start in March.
    let z = days + 719_468;
    let (era, day_of_era) = (z / 146_097, z % 146_097);
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    let (hour, minute, second) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}
