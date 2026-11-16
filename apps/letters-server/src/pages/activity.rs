//! A document's access log, made ready for the Share dialog's Activity tab: its rows by day in the
//! server's time zone, which the tab names, each worded as the app's own Activity tab words it.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use letters_api::{Access, Action, Who};

use super::day;
use super::view::{Activity, Reached};
use crate::access::{KEPT_DAYS, LISTED};

/// What was done, after the name of who did it.
fn did(action: Action, refused: bool) -> &'static str {
    match (action, refused) {
        (Action::Open, false) => "opened the document",
        (Action::Edit, false) => "started editing",
        (Action::Comment, false) => "wrote a comment",
        (Action::Download, false) => "downloaded a copy",
        (Action::Print, false) => "printed the document",
        (Action::Other, false) => "reached the document",
        (Action::Open, true) => "was refused the document",
        (Action::Edit, true) => "was refused an edit",
        (Action::Comment, true) => "was refused a comment",
        (Action::Download, true) => "was refused a download",
        (Action::Print, true) => "was refused a print",
        (Action::Other, true) => "was refused",
    }
}

/// So many minutes, as they are said: "12 minutes", "1 hour 5 minutes".
fn lasting(minutes: u64) -> String {
    let count = |many: u64, one: &str| if many == 1 { format!("1 {one}") } else { format!("{many} {one}s") };
    match (minutes / 60, minutes % 60) {
        (0, minutes) => count(minutes, "minute"),
        (hours, 0) => count(hours, "hour"),
        (hours, minutes) => format!("{} {}", count(hours, "hour"), count(minutes, "minute")),
    }
}

fn reached(row: &Access, at: String) -> Reached {
    let (who, link) = match &row.who {
        // Someone signed in may have come by a link all the same.
        Who::Person { name, .. } => (name.as_str(), row.link.as_ref().map(|via| via.label.clone())),
        // Whoever comes by a link is not known by name: the link's label says who it was made for.
        Who::Link { label, .. } => ("Guest", Some(label.clone())),
    };
    // A session of edits is one row, which says how long it went on once that is a minute.
    let minutes = row.lasted.filter(|_| row.action == Action::Edit && !row.refused).map_or(0, |seconds| seconds / 60);
    let did = match minutes {
        0 => did(row.action, row.refused).to_owned(),
        minutes => format!("edited for {}", lasting(minutes)),
    };
    let from: Vec<&str> =
        [row.address.as_str(), row.browser.as_str()].into_iter().filter(|part| !part.is_empty()).collect();
    let (said, from) = (format!("{who} {did}"), from.join(", "));
    Reached { said, action: row.action, role: row.role, refused: row.refused, link, from, at }
}

/// What a zone is called for a reader who may be in another: its name, or how far from UTC it is where it
/// has none. A server that cannot tell its zone tells its times in UTC.
fn called(zone: &TimeZone, now: Timestamp) -> String {
    match zone.iana_name() {
        Some("UTC" | "Etc/UTC" | "Etc/Unknown") => "UTC".to_owned(),
        Some(name) => format!("{} time", name.replace('_', " ")),
        None => format!("UTC{}", zone.to_offset(now)),
    }
}

/// The tab for `rows`, which come newest first, at the time `now`. `older` says that they are not the
/// newest, and `before` makes the address that lists the rows from before a time.
pub fn listed(rows: &[Access], now: u64, older: bool, before: impl Fn(u64) -> String) -> Activity {
    let zone = TimeZone::system();
    let local = |at: u64| Timestamp::from_second(at as i64).ok().map(|time| time.to_zoned(zone.clone()));
    let date = |at: u64| local(at).map(|time| time.date());
    let today = date(now);
    let mut days = Vec::new();
    for rows in rows.chunk_by(|a, b| date(a.time) == date(b.time)) {
        let called = match (date(rows[0].time), today) {
            (Some(on), Some(today)) if on == today => "Today".to_owned(),
            (Some(on), Some(today)) if today.yesterday().ok() == Some(on) => "Yesterday".to_owned(),
            _ => day(rows[0].time, now),
        };
        let clock = |row: &Access| local(row.time).map(|time| time.strftime("%H:%M").to_string()).unwrap_or_default();
        days.push((called, rows.iter().map(|row| reached(row, clock(row))).collect()));
    }
    // A full page may not be all of it.
    let earlier = rows.last().filter(|_| rows.len() >= LISTED).map(|last| before(last.time));
    let zone = called(&zone, Timestamp::from_second(now as i64).unwrap_or(Timestamp::UNIX_EPOCH));
    Activity { days, kept: KEPT_DAYS, zone, earlier, older }
}
