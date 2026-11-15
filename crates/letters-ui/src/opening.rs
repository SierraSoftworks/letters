//! Opening, naming and moving a document: what the shell tells the window, and what the window asks of it.
//!
//! The window knows no file system. A path is a string the shell made, which it gets back unchanged.

use letters_model::{BlockId, Revision};
use letters_render::Point;

use crate::browsing::Place;
use crate::chrome::Edge;
use crate::copying::CopyFormat;
use crate::notes;
use crate::settings::Settings;

/// What the window asks its shell to do. Collect them with `Window::take_requests` after each event.
///
/// More will come, so a shell matches the ones it can do and lets the rest go.
#[derive(Clone, Debug, PartialEq)]
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
    /// `Window::set_location`. One the card that saves a copy asked for, while copies have a place of their
    /// own, is listed with `Window::set_copy_location`, or refused with `Window::refuse_copy_folder`.
    NewFolder {
        parent: String,
        name: String,
    },
    /// Save the document now. Asked only while the shell says it is [`SaveState::Edited`](crate::SaveState):
    /// by a press on the bar's save mark, from the palette, or with Ctrl or Cmd with S. Answer with
    /// `Window::set_save_state`.
    Save,
    /// Paste the clipboard, with no formatting when `plain` is set. Asked when a menu or the palette pastes
    /// and the shell had nothing to give at once, as a browser has outside a paste event: a shell that can
    /// read the clipboard later does, and has the editor paste it then. Others let it go.
    Paste {
        plain: bool,
    },
    /// Write a copy of the document called `name` in `format`, and leave this window on the document it
    /// has. `folder` is the path of the folder to write it to, or `None` to let the person choose where.
    /// A file of that name is in the way unless `replace` is set, which it is once the person has said so.
    /// Answer with `Window::set_copied`, or with `Window::refuse_copy` when it cannot be done.
    /// [`CopyFormat::Clean`] asks for a document of its own without the history and the notes.
    SaveCopy {
        name: String,
        format: CopyFormat,
        folder: Option<String>,
        replace: bool,
    },
    /// Write a copy of the document as it read at `revision`, a version the person picked in the history,
    /// which `Window::document_at` reads. The rest is as for [`Request::SaveCopy`], and so are the answers.
    /// A copy in [`CopyFormat::Letters`] is a document of its own that starts with that version's text.
    SaveVersion {
        revision: Revision,
        name: String,
        format: CopyFormat,
        folder: Option<String>,
        replace: bool,
    },
    /// Keep this as the person's default style, which their new documents start with: the text of a
    /// `style.json`. Answer with `Window::set_default_style` if it could not be kept.
    SaveDefaultStyle(String),
    /// Let the person choose a stylesheet, a `.letstyle` file, and hand its text to
    /// `Window::offer_stylesheet`.
    ImportStyle,
    /// Let the person choose where to write this stylesheet: the text of a `.letstyle` file.
    ExportStyle(String),
    /// Import the file the shell offered with `Window::offer_import`, reading pictures and linked files
    /// only from these folders, by the paths the offer gave them. Take the card away with
    /// `Window::offer_import(None)` once the document is open, or offer again when it could not be done.
    Import {
        folders: Vec<String>,
        /// Whether the file's comments come too.
        comments: bool,
    },
    /// The file that was offered is not wanted. The card is already gone.
    CancelImport,
    /// Keep these as the person's settings, and tell every other window of them with
    /// `Window::set_settings`. The window that asks already follows the accent. The rest is the shell's to
    /// follow: light or dark, the name edits are signed with, and the language of new documents.
    Settings(Settings),
    /// The person turned this shell's anonymous usage reports on or off in the Settings dialog, which has
    /// the switch once the shell has said `Window::set_usage_reports`. Keep it, follow it, and tell every
    /// other window.
    UsageReports(bool),
    /// Move the window with the pointer, which is held down on the bar: a bar that is the title bar asks
    /// (see `Window::set_chrome`).
    DragWindow,
    /// Resize the window from this edge with the pointer, which is held down on the grip of a floating
    /// window.
    ResizeWindow(Edge),
    /// Open the desktop's menu for the window at this point of it.
    WindowMenu(Point),
    Minimise,
    /// Maximise the window, or restore it. Also asked by a double click on the bar, which on macOS does
    /// what the person set their system to do.
    ToggleMaximise,
    /// Close the window.
    Close,
    /// The person asked to print the document, from Print view: hand what the PDF export writes to the
    /// system's print dialog, or to the browser's.
    Print,
    /// Let the person choose pictures, and hand them to `Window::insert_pictures` with no point.
    Pictures,
    /// Read this asset of the document, by its entry name, and hand it decoded to `Window::set_image`.
    /// Asked once for each picture the page comes to show, which keeps its place until then.
    Asset(String),
    /// The document links to this picture, by the address its figure has for a `src`. Fetch it if the
    /// person's trust lets it load when a document opens, and hand it to `Window::set_image` under the
    /// address as it is written here. Otherwise do nothing: the picture keeps its frame and cloud. Asked
    /// once for each address, and again for those not showing after a [`Request::TrustPictures`].
    LinkedPicture(String),
    /// The person asked for this linked picture this once. Fetch it and hand it to `Window::set_image`.
    LoadPicture(String),
    /// The person trusts the host of this linked picture from now on: keep that. The window then asks for
    /// every linked picture not yet showing with [`Request::LinkedPicture`] again.
    TrustPictures(String),
    /// The person wants this linked picture kept in the document. Fetch it and hand its bytes to
    /// `Window::embed_picture` with the figure.
    EmbedPicture {
        figure: BlockId,
        src: String,
    },
    /// The person trusts this document: keep that, treat the document as trusted from now on, and say so
    /// with `Window::set_safe_mode(None)`. Asked from the bar that `Window::set_safe_mode` shows.
    TrustDocument,
    /// The person trusts the folder this document is in, and every document in it. Answered as
    /// [`Request::TrustDocument`] is. Asked only when the shell named a folder.
    TrustFolder,
    /// Let the person choose a folder, and trust every document in it. Asked from the Settings dialog, which
    /// lists the folders once the shell said `Window::set_trusted_folders`. Answer with the new list.
    TrustFolderDialog,
    /// Stop trusting the folder with this path, as `Window::set_trusted_folders` gave it. Answer with the
    /// new list.
    ForgetFolder(String),
    /// Trust this source of linked pictures, as the person typed it: a host, or an address whose paths are
    /// meant. Answer with `Window::set_trusted_sources`, or with `Window::refuse_source` when it is none.
    TrustSource(String),
    /// Stop trusting this source, as `Window::set_trusted_sources` gave it. Answer with the new list.
    ForgetSource(String),
    /// Stop trusting the document that is open, and hold it in safe mode again. Asked from the Settings
    /// dialog after `Window::set_document_trusted(true)`. Answer with `Window::set_document_trusted(false)`
    /// and `Window::set_safe_mode`.
    ForgetDocument,
    /// Take this word out of the person's dictionary, and tell every other window with
    /// `Window::set_user_words`. The window that asks has dropped it already.
    ForgetWord(String),
    /// The person asked to open the link with this address, as the document writes it. The shell judges
    /// it by what the person trusts: it opens the link, or has the window show the whole address and ask
    /// with `Window::ask_link`, or says why not with `Window::refuse_link`. `confirmed` is set once the
    /// person has read the address on that card and said yes, which is enough for a link that asks, and
    /// changes nothing for one that is refused. An address that starts with `#` is a place in the
    /// document and is never asked about.
    OpenLink {
        href: String,
        confirmed: bool,
    },
    /// Merge every conflict copy last told of with `Window::set_conflict_copies` into the document, and
    /// move each to the trash once its edits are in. The shell judges the files first: it merges them, or
    /// has the window name them and ask with `Window::ask_merge`, or says why not with
    /// `Window::refuse_merge`. `confirmed` is set once the person has read that card and said yes. The
    /// button waits for one of the three: `Window::set_conflict_copies` says which copies are left.
    MergeConflicts {
        confirmed: bool,
    },
    /// Merge another file of this document into it, and leave that file where it is. `path` is `None` for
    /// the shell to let the person choose the file. Once the person has said yes to a `Window::ask_merge`
    /// it is the path of the file on that card, with `confirmed` set. Answered as
    /// [`Request::MergeConflicts`] is, or not at all when the person chose no file.
    MergeFrom {
        path: Option<String>,
        confirmed: bool,
    },
    /// Merge this document, a conflict copy, into the document it is a copy of, which was told with
    /// `Window::set_conflict_original`, and go on with that document in this window. The shell merges, or
    /// has the window name the original and ask with `Window::ask_merge_into`, or says why not with
    /// `Window::refuse_merge`. `confirmed` is set once the person has read that card and said yes. The
    /// button waits for one of the three: `Window::set_conflict_original` says what the window holds then.
    MergeIntoOriginal {
        confirmed: bool,
    },
    /// The person asked for what the page at this address says of itself, for a link preview: they put
    /// the preview in, or pressed its "Fetch preview". Read the page's title, its description and the
    /// picture it gives for itself, and hand them to `Window::fill_preview`, or say why not with
    /// `Window::fail_preview`. Each request is answered once, by its address: what waits for the address
    /// says that it is being fetched until then, and no second request is made for it. Never asked when a
    /// document opens: what a preview shows is kept in the document.
    FetchPreview {
        url: String,
    },
    /// The person wants the server asked again, after it ended the session: see
    /// [`Connection::Refused`](crate::Connection). Answer with `Window::set_connection`.
    Connect,
    /// The Share dialog opened on a document a server keeps: who is it shared with? Answer with
    /// `Window::set_shares`, or with `Window::refuse_share` and the server's sentence. Asked again after a
    /// refusal, since the dialog then shows what may not be so.
    Shares,
    /// The document's owner changed who it is shared with: these are all its people from here on, as the
    /// server's `PUT` of a document's shares takes them. The dialog shows them already. Answer as for
    /// [`Request::Shares`].
    SharePeople(Vec<letters_api::Grant>),
    /// The owner asked for a link. Answer with `Window::link_made` and what the server made, which is the
    /// one time its address is told, or with `Window::refuse_share`.
    MakeLink(letters_api::NewLink),
    /// The owner revoked the link with this id. The dialog has dropped it already. Answer only when it could
    /// not be done, with `Window::refuse_share`.
    RevokeLink(String),
    /// Someone who may share the document turned to the Share dialog's Activity tab: who reached it
    /// through a share?
    /// Answer with `Window::set_access_log` and the server's rows, newest first, or with
    /// `Window::refuse_share`.
    AccessLog,
    /// The Activity tab's "Show earlier" was pressed: the rows of the access log from before this time,
    /// which is that of the last row the window holds (the server's `?before=`). Answer with
    /// `Window::add_access_log`, or with `Window::refuse_share`.
    AccessLogBefore(u64),
    /// Sign in to the server at this address, as the person typed it, through the system's browser. Say
    /// how far it has come with `Window::set_sign_in`. Once it is done, say who is signed in where with
    /// `Window::set_servers`, and then that no sign-in is under way with `Window::set_sign_in(None)`.
    SignIn {
        server: String,
    },
    /// Give up the sign-in that is under way.
    CancelSignIn,
    /// Sign out of the server at this [`Server::address`](crate::Server::address), and say who is then
    /// signed in where with `Window::set_servers`.
    SignOut {
        server: String,
    },
    /// Say what a place of the server at this address holds, with `Window::set_listing`, or why it could
    /// not be read with `Window::refuse_browse`. The card that opens a server's documents asks it.
    Browse {
        server: String,
        place: Place,
    },
    /// Open the document with this id that the server at this address keeps, as `Open` opens a file.
    OpenFrom {
        server: String,
        id: String,
    },
    /// Start a document in a folder of a space the server at this address keeps, and open it, as `New`
    /// starts one on this device. `path` is the folder's path in the space, empty for the space itself.
    /// The card that opens a server's documents asks it, only of a space the person edits.
    NewAt {
        server: String,
        space: String,
        path: String,
    },
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
    /// Whether `folders` are all the places there are for it, as for a document a server keeps. Nothing
    /// then offers "Other location", which asks for a folder of the device.
    pub closed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
    pub path: String,
    pub name: String,
    /// How far down the tree it is. A folder at 0 is the space itself, or one outside it that holds the
    /// document.
    pub depth: u8,
}

pub(crate) const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
pub(crate) const MONTHS: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// A moment on the person's clock, which is `zone` minutes ahead of UTC.
pub(crate) fn local(seconds: u64, zone: i32) -> i64 {
    i64::try_from(seconds).unwrap_or(i64::MAX).saturating_add(i64::from(zone) * 60)
}

/// The day a moment falls on where the person is, counted from 1970.
pub(crate) fn day(seconds: u64, zone: i32) -> i64 {
    local(seconds, zone).div_euclid(86_400)
}

/// The time of day a moment falls at where the person is: "09:14".
pub(crate) fn clock(seconds: u64, zone: i32) -> String {
    let of_day = local(seconds, zone).rem_euclid(86_400);
    format!("{:02}:{:02}", of_day / 3_600, of_day / 60 % 60)
}

/// What a day is called on `today`: "Today", "Yesterday", the weekday for a week back, and the date before
/// that.
pub(crate) fn day_called(day: i64, today: Option<i64>) -> String {
    match today.map(|today| today - day) {
        Some(0) => "Today".to_owned(),
        Some(1) => "Yesterday".to_owned(),
        Some(2..7) => WEEKDAYS[(day + 4).rem_euclid(7) as usize].to_owned(),
        _ => date_called(day, today),
    }
}

/// A day as a date: "6 Nov", with its year where that is not plain.
pub(crate) fn date_called(day: i64, today: Option<i64>) -> String {
    let (year, month, date) = notes::date(u64::try_from(day).unwrap_or_default());
    let month = MONTHS[(month.clamp(1, 12) - 1) as usize];
    match today.map(|today| today - day) {
        // A date without its year is plain for most of a year back.
        Some(0..300) => format!("{date} {month}"),
        _ => format!("{date} {month} {year}"),
    }
}

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
