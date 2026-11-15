//! Opening, naming and moving a document: what the shell tells the window, and what the window asks of it.
//!
//! The window knows no file system. A path is a string the shell made, which it gets back unchanged.

/// What the window asks its shell to do. Collect them with `Window::take_requests` after each event.
///
/// More will come, so a shell matches the ones it can do and lets the rest go.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Request {
    /// Open the recent document with this path.
    Open(String),
    /// Let the person choose a document to open.
    OpenDialog,
    /// Start another new document.
    New,
    /// The caret left the first Heading 1 of a document that has no name of its own: it takes this one.
    Named(String),
    /// Call the document this. Answer with `Window::set_location`, or with `Window::set_notice` when it
    /// cannot be done.
    Rename(String),
    /// Move the document to the folder with this path. Answered as a rename is.
    Move(String),
    /// Let the person choose a folder that is not in the tree, and move the document there.
    MoveElsewhere,
    /// Make a folder called `name` in the folder with the path `parent`, and list it with
    /// `Window::set_location`.
    NewFolder { parent: String, name: String },
    /// Keep this as the person's default style, which their new documents start with: the text of a
    /// `style.json`. Answer with `Window::set_default_style` if it could not be kept.
    SaveDefaultStyle(String),
    /// Let the person choose a stylesheet, a `.letstyle` file, and hand its text to
    /// `Window::offer_stylesheet`.
    ImportStyle,
    /// Let the person choose where to write this stylesheet: the text of a `.letstyle` file.
    ExportStyle(String),
}

/// A document opened lately, as the landing card lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recent {
    pub path: String,
    pub name: String,
    /// The folder it is in, as the card shows it: `Family / House`.
    pub folder: String,
    /// When it was last opened, as RFC 3339 with the person's own offset.
    pub opened: String,
}

/// What the landing card offers while no document is open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Landing {
    /// The time now, as RFC 3339 with the person's own offset. The window has no clock of its own.
    pub now: String,
    /// The newest first. The card lists the first few.
    pub recents: Vec<Recent>,
}

/// Where a document is kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Location {
    /// The document's own name. Without one the bar shows its first Heading 1, or "Untitled", in grey.
    pub name: Option<String>,
    /// The path of the folder it is in, or will first be written to.
    pub folder: String,
    /// The folders it can move to, in tree order.
    pub folders: Vec<Folder>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
    pub path: String,
    pub name: String,
    /// How far down the tree it is. A folder at 0 is the space itself, or one outside it that holds the
    /// document.
    pub depth: u8,
}

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// A moment as RFC 3339 writes it.
struct Moment {
    /// The date on its writer's calendar.
    date: (i64, i64, i64),
    /// That date as days since 1970.
    day: i64,
    /// Seconds since 1970, wherever it was written.
    seconds: i64,
}

fn moment(text: &str) -> Option<Moment> {
    let number = |from: usize, to: usize| text.get(from..to)?.parse::<i64>().ok();
    let date = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    // What follows the seconds: perhaps a fraction, then Z or an offset of hours and minutes.
    let rest = text.get(19..)?;
    let offset = match rest.rfind(['+', '-']) {
        _ if rest.ends_with(['Z', 'z']) => 0,
        Some(at) => {
            let (hours, minutes) = (number(19 + at + 1, 19 + at + 3)?, number(19 + at + 4, 19 + at + 6)?);
            (hours * 3600 + minutes * 60) * if rest[at..].starts_with('-') { -1 } else { 1 }
        }
        None => return None,
    };
    let (year, month) = (if date.1 <= 2 { date.0 - 1 } else { date.0 }, (date.1 + 9) % 12);
    let (era, of_era) = (year.div_euclid(400), year.rem_euclid(400));
    let day = era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + (153 * month + 2) / 5 + date.2 - 1 - 719_468;
    Some(Moment { date, day, seconds: day * 86_400 + hour * 3600 + minute * 60 + second - offset })
}

/// How long ago `then` was, as the landing card says it: "2 minutes ago", "Yesterday", "Tuesday", "3 Nov".
/// Empty when either time cannot be read.
pub(crate) fn ago(now: &str, then: &str) -> String {
    let (Some(now), Some(then)) = (moment(now), moment(then)) else { return String::new() };
    let (minutes, days) = ((now.seconds - then.seconds) / 60, now.day - then.day);
    let count = |count: i64, unit: &str| format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" });
    let (year, month, day) = then.date;
    let month = MONTHS[(month - 1).clamp(0, 11) as usize];
    match (minutes, days) {
        (..1, _) => "Just now".into(),
        (..60, _) => count(minutes, "minute"),
        (_, ..1) => count(minutes / 60, "hour"),
        (_, 1) => "Yesterday".into(),
        (_, ..7) => WEEKDAYS[(then.day + 4).rem_euclid(7) as usize].into(),
        // A date without its year is plain for most of a year back.
        (_, ..300) => format!("{day} {month}"),
        _ => format!("{day} {month} {year}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_is_told_by_how_long_ago_it_was() {
        let now = "2026-11-14T10:30:00+01:00";
        let cases = [
            ("2026-11-14T10:29:40+01:00", "Just now"),
            ("2026-11-14T10:28:00+01:00", "2 minutes ago"),
            ("2026-11-14T09:28:30Z", "1 minute ago"),
            ("2026-11-14T07:10:00+01:00", "3 hours ago"),
            ("2026-11-13T23:50:00+01:00", "Yesterday"),
            ("2026-11-10T08:00:00+01:00", "Tuesday"),
            ("2026-11-08T08:00:00.250+01:00", "Sunday"),
            ("2026-11-03T08:00:00+01:00", "3 Nov"),
            ("2026-02-28T08:00:00-08:00", "28 Feb"),
            ("2025-09-12T08:00:00+01:00", "12 Sep 2025"),
            ("2026-11-14T10:45:00+01:00", "Just now"),
            ("yesterday", ""),
            ("", ""),
        ];
        for (then, said) in cases {
            assert_eq!(ago(now, then), said, "{then}");
        }
    }
}
