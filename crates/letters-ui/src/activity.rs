//! The Activity tab of the Share dialog: the server's log of who reached the document through a share, by
//! day, newest first.

use accesskit::Role;
use letters_api::{Access as Reach, Action, Who};

use crate::element::Element;
use crate::icon::{self, Icon};
use crate::opening::{clock, day, day_called};
use crate::sharing::{cap, marks, named, quiet, said};
use crate::style::{Align, Edges, Justify};
use crate::text::TextStyle;
use crate::tokens;
use crate::window::Message;

/// The most the tab lists. A server keeps far more than anyone reads through.
const MOST: usize = 200;
const MARK: f32 = 24.0;

/// The log as the tab shows it. `now` is the time by the person's clock, which is `zone` minutes ahead of
/// UTC: the days are named from it.
pub(crate) fn view(log: &[Reach], now: Option<u64>, zone: i32) -> Element<Message> {
    if log.is_empty() {
        return said("Nobody has reached this document through a share yet.");
    }
    let about = "Everything that reached this document through a share, newest first.";
    let mut list =
        Element::column().gap(tokens::SPACE_LG).child(Element::paragraph(about, quiet()).fg(tokens::FG_MUTED));
    let today = now.map(|now| day(now, zone));
    // The rows come newest first, so each day's are together.
    for rows in log[..log.len().min(MOST)].chunk_by(|a, b| day(a.time, zone) == day(b.time, zone)) {
        let called = day_called(day(rows[0].time, zone), today);
        // The heading reads as a side panel's. A screen reader hears the day as the name of its list.
        let style = TextStyle::control().size(tokens::TEXT_XS).weight(600);
        let heading = Element::text(called.to_uppercase(), style).fg(tokens::FG_MUTED).decorative();
        let entries = rows.iter().enumerate().map(|(at, reach)| entry(reach, zone, at + 1 < rows.len()));
        let entries = Element::column().gap(tokens::SPACE_SM).role(Role::List).label(called).children(entries);
        list = list.child(Element::column().gap(tokens::SPACE_SM).child(heading).child(entries));
    }
    match log.len().saturating_sub(MOST) {
        0 => list,
        1 => list.child(said("One earlier entry is not listed.")),
        more => list.child(said(&format!("{more} earlier entries are not listed."))),
    }
}

/// One row of the log: what was done under the icon of doing it, in the colour of the role it was done
/// with, or struck and in amber when the server refused it. A line joins it to the day's next one.
fn entry(reach: &Reach, zone: i32, joined: bool) -> Element<Message> {
    let (who, link) = match &reach.who {
        Who::Person { name, .. } => (name.as_str(), None),
        // Whoever comes by a link is not known by name: the link's label says who it was made for.
        Who::Link { label, .. } => ("Guest", Some(label.as_str())),
    };
    let did = did(reach.action, reach.refused);
    let (_, ink, soft) = marks(reach.role);
    let (ink, soft) = if reach.refused { (tokens::WARN, tokens::WARN_SOFT) } else { (ink, soft) };
    let strike = reach.refused.then(|| Element::icon(icon::SLASH, 14.0).overlay());
    let mark = Element::row()
        .width(MARK)
        .height(MARK)
        .radius(MARK / 2.0)
        .bg(soft)
        .fg(ink)
        .align(Align::Center)
        .justify(Justify::Center)
        .child(Element::row().child(Element::icon(glyph(reach.action), 14.0)).children(strike));
    let joint = joined.then(|| Element::row().width(2.0).grow(1.0).radius(1.0).bg(ink));
    let side = Element::column()
        .align(Align::Center)
        .gap(tokens::SPACE_XS)
        .align_self(Align::Stretch)
        .child(mark)
        .children(joint)
        .decorative();
    let title = Element::text(format!("{who} {did}"), TextStyle::control().weight(600)).shrink();
    let from: Vec<&str> =
        [reach.address.as_str(), reach.browser.as_str()].into_iter().filter(|part| !part.is_empty()).collect();
    let from = from.join(", ");
    let via = link.map(|label| {
        Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_XS)
            .padding(Edges::xy(tokens::SPACE_SM, 2.0))
            .border(Edges::all(1.0), tokens::LINE_STRONG)
            .radius(tokens::RADIUS_SM)
            .child(Element::icon(icon::LINK, 14.0).fg(tokens::FG_MUTED))
            .child(Element::text(label, quiet().weight(600)))
    });
    let meta = Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .child(cap(reach.role))
        .children(via)
        .child(Element::text(from.as_str(), quiet()).fg(tokens::FG_MUTED).shrink());
    let when = clock(reach.time, zone);
    // A screen reader is told the row whole, in the order it is read across.
    let by = link.map(|label| format!("by the link for {label}"));
    let rest: Vec<&str> = [Some(named(reach.role)), by.as_deref(), Some(from.as_str()), Some(when.as_str())]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect();
    Element::row()
        .align(Align::Start)
        .gap(tokens::SPACE_MD)
        .role(Role::ListItem)
        .label(format!("{who} {did}"))
        .description(rest.join(", "))
        .child(side)
        .child(Element::column().grow(1.0).gap(tokens::SPACE_2XS).child(title).child(meta))
        .child(Element::text(when.as_str(), quiet()).fg(tokens::FG_MUTED))
}

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

fn glyph(action: Action) -> Icon {
    match action {
        Action::Open | Action::Other => icon::EYE,
        Action::Edit => icon::PENCIL,
        Action::Comment => icon::MESSAGE_SQUARE,
        Action::Download => icon::DOWNLOAD,
        Action::Print => icon::PRINTER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_thing_done_or_refused_is_told_apart() {
        let actions = [Action::Open, Action::Edit, Action::Comment, Action::Download, Action::Print, Action::Other];
        let mut told = Vec::new();
        for action in actions {
            for refused in [false, true] {
                let words = did(action, refused);
                assert_eq!(words.contains("refused"), refused, "{action:?}: {words}");
                assert!(!told.contains(&words), "{action:?} reads as another: {words}");
                told.push(words);
            }
        }
    }
}
