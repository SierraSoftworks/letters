//! The Styles panel: each named style as it looks, the person's default, page setup and stylesheets.
//!
//! The panel shows and asks. A style changes through the editor, and files are the shell's to read and write.

use accesskit::Role;
use letters_layout::Palette;
use letters_model::{Page, Style, TextStyle as TypeStyle};

use crate::color::same;
use crate::element::Element;
use crate::icon;
use crate::restyle;
use crate::style::{Align, Edges, Justify, Look, Ring};
use crate::text::{Family, TextStyle};
use crate::tokens;
use crate::widgets::{Button, ControlSize, Menu, MenuItem, SidePanel, TextField};
use crate::window::{FAMILIES, Message, PT, WEIGHTS, hex_color};

/// The papers offered, by their short and long sides in points.
const PAPERS: [(&str, f32, f32); 4] =
    [("A4", 595.0, 842.0), ("A5", 420.0, 595.0), ("Letter", 612.0, 792.0), ("Legal", 612.0, 1008.0)];
/// Points in a millimetre.
const MM: f32 = 72.0 / 25.4;
/// The least text a page keeps between its margins, in millimetres.
const LEAST_TEXT: f32 = 20.0;
/// The biggest a type's name is shown in its row.
const BIGGEST: f32 = 28.0;

/// What the Styles panel sends.
#[derive(Clone, Debug, PartialEq)]
pub enum Styling {
    /// Opens the panel.
    Show,
    /// Has the bar show this type's style, until the caret moves.
    Pick(&'static str),
    /// Keeps the document's style as the person's default, or the stylesheet on offer when there is one.
    SaveDefault,
    /// Gives the document the person's default style.
    Reset,
    /// Asks the shell for a stylesheet to bring in.
    Import,
    /// Asks the shell to write the document's style out as a stylesheet.
    Export,
    /// Gives the document the stylesheet on offer.
    Apply,
    /// Puts the stylesheet on offer away, unused.
    Decline,
    /// Changes the paper to one of the papers offered, the same way up.
    Paper(usize),
    Landscape(bool),
    /// What is typed into the margins field, before it is taken.
    Margins(String),
    /// Takes the margins that were typed, or drops them if the paper has no room for them.
    TakeMargins,
    ForgetMargins,
}

/// What the Styles panel keeps between frames.
#[derive(Default)]
pub(crate) struct Styles {
    /// The person's default style, once the shell has said what it is.
    pub default: Option<Style>,
    /// A stylesheet the person chose to bring in, with its file's name. It is shown and changes nothing yet.
    pub offered: Option<(String, Style)>,
    /// Why the file last chosen is no stylesheet.
    pub refused: Option<String>,
    /// What is being typed into the margins field.
    pub margins: Option<String>,
}

impl Styles {
    /// Takes a file the person chose as the stylesheet on offer, or says why it is none.
    pub fn offer(&mut self, name: &str, json: &str) {
        (self.offered, self.refused) = match Style::from_json(json) {
            Ok(style) => (Some((name.to_owned(), style)), None),
            Err(error) => (None, Some(format!("{name} is not a stylesheet Letters can read: {error}"))),
        };
    }

    /// Drops what the panel was in the middle of, as it closes.
    pub fn leave(&mut self) {
        (self.offered, self.refused, self.margins) = (None, None, None);
    }

    /// Whether the document's style is no longer the person's default.
    pub fn differs(&self, style: &Style) -> bool {
        self.default.as_ref().is_some_and(|default| default != style)
    }

    /// The panel's content for a document's style. `open` is the type the bar shows, and `palette` holds
    /// the colours of the page.
    pub fn view(&self, style: &Style, open: Option<&str>, palette: &Palette) -> Element<Message> {
        // A stylesheet on offer is shown in the document's place, to be looked at and not changed.
        let offered = self.offered.as_ref();
        let shown = offered.map_or(style, |(_, offered)| offered);
        let mut body = Element::column().padding(Edges::top(tokens::SPACE_MD));
        if let Some((name, _)) = offered {
            body = body.child(offer(name));
        } else if self.differs(style) {
            body = body.child(differs());
        }
        let rows = Style::TYPES.iter().filter_map(|element| {
            let open = offered.is_none() && open == Some(*element);
            Some(row(element, shown.types.get(*element)?, open, offered.is_none(), palette))
        });
        body.child(Element::column().gap(tokens::SPACE_2XS).children(rows))
            .child(SidePanel::section("Page"))
            .child(if offered.is_some() { page_as_read(&shown.page) } else { self.page(&shown.page) })
            .child(SidePanel::section("Stylesheet"))
            .child(self.sheet())
    }

    /// The paper, which way up it is, and its margins, each changed where it stands.
    fn page(&self, page: &Page) -> Element<Message> {
        let name = paper_name(page);
        let paper = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .height(tokens::CONTROL_HEIGHT_SM)
            .padding(Edges::xy(tokens::SPACE_SM, 0.0))
            .border(Edges::all(1.0), tokens::LINE_STRONG)
            .radius(tokens::RADIUS_CONTROL)
            .hover(Look::bg(tokens::TONE_1))
            .ring(Ring::Outline(2.0))
            .role(Role::Button)
            .label("Paper")
            .description(name.clone())
            .child(Element::text(name, quiet()))
            .child(Element::icon(icon::CHEVRON_DOWN, 14.0).fg(tokens::FG_MUTED))
            .popover(papers(page).popover());

        let typed = self.margins.as_ref();
        let shown = margins(page).map(|margins| margins.to_string()).unwrap_or_default();
        let field = TextField::new("Margins", typed.cloned().unwrap_or(shown))
            .size(ControlSize::Small)
            .placeholder("Mixed")
            .invalid(typed.is_some_and(|typed| read_margins(page, typed).is_none()))
            .on_input(|typed| Message::Styles(Styling::Margins(typed)))
            .on_submit(Message::Styles(Styling::TakeMargins))
            .on_leave(Message::Styles(Styling::TakeMargins))
            .on_cancel(Message::Styles(Styling::ForgetMargins));
        let margins = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .child(Element::text("Margins", quiet()).fg(tokens::FG_MUTED).decorative())
            .child(Element::from(field).width(56.0))
            .child(Element::text("mm", quiet()).fg(tokens::FG_MUTED));
        Element::row().align(Align::Center).justify(Justify::Between).gap(tokens::SPACE_SM).child(paper).child(margins)
    }

    /// Stylesheets in and out, which the shell reads and writes.
    fn sheet(&self) -> Element<Message> {
        let buttons = Element::row()
            .gap(tokens::SPACE_XS)
            .child(Button::new("Import").icon(icon::FILE_UP).small().on_press(Message::Styles(Styling::Import)))
            .child(Button::new("Export").icon(icon::FILE_DOWN).small().on_press(Message::Styles(Styling::Export)));
        let note = match &self.refused {
            Some(why) => Element::paragraph(why, quiet()).fg(tokens::DANGER).role(Role::Alert).label(why),
            None => Element::paragraph("Share this look as a .letstyle file", quiet()).fg(tokens::FG_MUTED),
        };
        Element::column().gap(tokens::SPACE_SM).child(buttons).child(note)
    }
}

/// The mark of a style that is not the person's default, here and on the bar's type label.
pub(crate) fn dot() -> Element<Message> {
    Element::row().width(8.0).height(8.0).radius(4.0).bg(tokens::INFO).decorative()
}

/// The page with one of the papers offered, the same way up.
pub(crate) fn with_paper(page: &Page, paper: usize) -> Option<Page> {
    let (_, short, long) = *PAPERS.get(paper)?;
    let (width, height) = if landscape(page) { (long, short) } else { (short, long) };
    Some(Page { width, height, ..page.clone() })
}

/// The page turned on its side, or upright again.
pub(crate) fn turned(page: &Page, to_landscape: bool) -> Page {
    let turn = landscape(page) != to_landscape;
    let (width, height) = if turn { (page.height, page.width) } else { (page.width, page.height) };
    Page { width, height, ..page.clone() }
}

/// The page with the margins that were typed on every side. Text that is no margin the paper has room for
/// gives none, and so do the margins the page already has.
pub(crate) fn with_margins(page: &Page, typed: &str) -> Option<Page> {
    let typed = read_margins(page, typed).filter(|typed| Some(*typed) != margins(page))?;
    let side = typed * MM;
    Some(Page { margin_top: side, margin_right: side, margin_bottom: side, margin_left: side, ..page.clone() })
}

/// Margins typed in millimetres, if they leave the paper some room for text.
fn read_margins(page: &Page, typed: &str) -> Option<f32> {
    let most = (mm(page.width.min(page.height)) - LEAST_TEXT) / 2.0;
    typed.trim().parse().ok().filter(|typed| (0.0..=most).contains(typed))
}

/// Points as millimetres, to the tenth that people type.
fn mm(points: f32) -> f32 {
    (points / MM * 10.0).round() / 10.0
}

/// The margin all four sides share, in millimetres.
fn margins(page: &Page) -> Option<f32> {
    let sides = [page.margin_top, page.margin_right, page.margin_bottom, page.margin_left].map(mm);
    sides.iter().all(|side| *side == sides[0]).then_some(sides[0])
}

fn landscape(page: &Page) -> bool {
    page.width > page.height
}

/// Which of the papers offered a page is, either way up.
fn paper(page: &Page) -> Option<usize> {
    let (short, long) = (page.width.min(page.height), page.width.max(page.height));
    PAPERS.iter().position(|(_, width, height)| (width - short).abs() < 1.0 && (height - long).abs() < 1.0)
}

/// The papers offered and the two ways up, with the page's own ticked. A choice changes the page.
pub(crate) fn papers(page: &Page) -> Menu<Message> {
    let mut papers = Menu::new().label("Paper");
    for (index, (name, ..)) in PAPERS.iter().enumerate() {
        let pick = Message::Styles(Styling::Paper(index));
        papers = papers.item(MenuItem::new(*name).checked(paper(page) == Some(index)).on_select(pick));
    }
    papers = papers.heading("Orientation");
    for (name, to) in [("Portrait", false), ("Landscape", true)] {
        let pick = Message::Styles(Styling::Landscape(to));
        papers = papers.item(MenuItem::new(name).checked(landscape(page) == to).on_select(pick));
    }
    papers
}

/// A page's paper and which way up it is, as the panel says it: `A4, portrait`.
pub(crate) fn paper_name(page: &Page) -> String {
    let way = if landscape(page) { "landscape" } else { "portrait" };
    match paper(page) {
        Some(paper) => format!("{}, {way}", PAPERS[paper].0),
        None => format!("{} × {} mm, {way}", mm(page.width), mm(page.height)),
    }
}

/// The page of a stylesheet on offer, to be read.
fn page_as_read(page: &Page) -> Element<Message> {
    let margins = margins(page).map_or_else(|| "Margins vary".to_owned(), |margins| format!("Margins {margins} mm"));
    Element::row()
        .align(Align::Center)
        .justify(Justify::Between)
        .gap(tokens::SPACE_SM)
        .child(Element::text(paper_name(page), TextStyle::control()))
        .child(Element::text(margins, quiet()).fg(tokens::FG_MUTED))
}

fn quiet() -> TextStyle {
    TextStyle::control().size(tokens::TEXT_SM)
}

/// A note with what can be done about it, set off from the rows under it.
fn card(note: Element<Message>, actions: Element<Message>) -> Element<Message> {
    let card = Element::column()
        .gap(tokens::SPACE_SM)
        .padding(Edges::all(tokens::SPACE_MD))
        .radius(tokens::RADIUS_CONTROL)
        .bg(tokens::TONE_1)
        .child(note)
        .child(actions);
    Element::column().padding(Edges::bottom(tokens::SPACE_MD)).child(card)
}

/// Says that the document's style is not the person's default, and offers to make either the other.
fn differs() -> Element<Message> {
    let note = Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .child(dot())
        .child(Element::text("Differs from your default", quiet()));
    let save = Button::new("Save as my default").small().on_press(Message::Styles(Styling::SaveDefault));
    let reset = Button::new("Reset to my default").small().subtle().on_press(Message::Styles(Styling::Reset));
    card(note, Element::row().gap(tokens::SPACE_XS).child(save).child(reset))
}

/// Names the stylesheet on offer, and asks what to do with it.
fn offer(name: &str) -> Element<Message> {
    let title = Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .child(Element::icon(icon::FILE_UP, 14.0).fg(tokens::FG_MUTED))
        .child(Element::text(name, quiet().weight(500)).shrink());
    let said = "Shown as it will look. Nothing has changed yet.";
    let note = Element::column()
        .gap(tokens::SPACE_XS)
        .child(title)
        .child(Element::paragraph(said, quiet()).fg(tokens::FG_MUTED));
    let apply = Button::new("Apply").small().primary().on_press(Message::Styles(Styling::Apply));
    let save = Button::new("Save as my default").small().on_press(Message::Styles(Styling::SaveDefault));
    let cancel = Button::new("Cancel").small().subtle().on_press(Message::Styles(Styling::Decline));
    card(note, Element::row().gap(tokens::SPACE_XS).child(apply).child(save).child(cancel))
}

/// One named style: its name set as text of that type looks, and its font in words. A row that can be
/// picked has the bar show its style.
fn row(element: &'static str, style: &TypeStyle, open: bool, pick: bool, palette: &Palette) -> Element<Message> {
    let face = FAMILIES.iter().find(|(family, _)| *family == style.family).map_or(Family::Sans, |found| found.1);
    let ink = palette.recolor(hex_color(&style.color).unwrap_or(palette.text));
    let look = TextStyle { italic: style.italic, ..TextStyle::control().family(face).weight(style.weight) };
    let (name, font) = (restyle::name(element), font(style));
    let row = Element::row()
        .align(Align::Center)
        .justify(Justify::Between)
        .gap(tokens::SPACE_MD)
        .padding(Edges::xy(tokens::SPACE_MD, tokens::SPACE_SM))
        .radius(tokens::RADIUS_CONTROL)
        .child(Element::text(name.clone(), look.size(preview(style.size))).fg(same(ink.r, ink.g, ink.b)).shrink())
        .child(Element::text(font.clone(), TextStyle::control().size(tokens::TEXT_XS)).fg(tokens::FG_MUTED));
    if !pick {
        return row;
    }
    let row = if open { row.bg(tokens::NEUTRAL_SOFT) } else { row };
    row.hover(Look::bg(tokens::NEUTRAL_SOFT))
        .ring(Ring::Outline(2.0))
        .role(Role::Button)
        .label(name)
        .description(font)
        .pressed(open)
        .on_press(Message::Styles(Styling::Pick(element)))
}

/// The size a type's name is shown at: its own up to the size of the panel's text, and beyond that growing
/// slowly enough for the biggest to keep its row.
fn preview(points: f32) -> f32 {
    let (actual, plain) = (points * PT, tokens::TEXT_MD);
    if actual <= plain { actual.max(tokens::TEXT_XS) } else { (plain + (actual - plain) * 0.4).min(BIGGEST) }
}

/// A style's font in words: `Montserrat Light, 30`.
fn font(style: &TypeStyle) -> String {
    let weight = WEIGHTS.iter().find(|(weight, _)| *weight == style.weight).map(|found| found.1.to_owned());
    let weight = weight.unwrap_or_else(|| style.weight.to_string());
    let mut face = style.family.clone();
    for part in
        [(style.weight != 400).then_some(weight.as_str()), style.italic.then_some("Italic")].into_iter().flatten()
    {
        face = format!("{face} {part}");
    }
    format!("{face}, {}", style.size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_says_its_paper_and_which_way_up_it_is() {
        let a4 = Page::default();
        let cases = [
            (a4.clone(), "A4, portrait"),
            (turned(&a4, true), "A4, landscape"),
            (with_paper(&a4, 2).expect("a paper on offer"), "Letter, portrait"),
            (with_paper(&turned(&a4, true), 3).expect("a paper on offer"), "Legal, landscape"),
            (Page { width: 300.0, height: 500.0, ..a4.clone() }, "105.8 × 176.4 mm, portrait"),
        ];
        for (page, expected) in cases {
            assert_eq!(paper_name(&page), expected);
        }
        assert_eq!(with_paper(&a4, 9), None, "a paper that is not on offer changes nothing");
        assert_eq!(turned(&turned(&a4, true), false), a4, "turning it back gives the page it was");
    }

    #[test]
    fn margins_are_taken_in_millimetres_when_the_paper_has_room() {
        let a4 = Page::default();
        // What is typed, and the margins then in force in millimetres. None leaves the page as it was.
        let cases = [
            ("20", Some(20.0)),
            (" 12.5 ", Some(12.5)),
            ("0", Some(0.0)),
            ("25", None),
            ("-1", None),
            ("100", None),
            ("wide", None),
        ];
        for (typed, expected) in cases {
            let after = with_margins(&a4, typed);
            assert_eq!(after.as_ref().and_then(margins), expected, "{typed:?}");
            let sides = after.map(|page| (page.width, page.height));
            assert!(sides.is_none_or(|sides| sides == (a4.width, a4.height)), "the paper is as it was");
        }
    }

    #[test]
    fn a_font_is_said_in_words() {
        let body = TypeStyle::default();
        let cases = [
            (body.clone(), "Inter, 11"),
            (TypeStyle { italic: true, ..body.clone() }, "Inter Italic, 11"),
            (
                TypeStyle { family: "Montserrat".into(), weight: 300, size: 30.0, ..body.clone() },
                "Montserrat Light, 30",
            ),
            (TypeStyle { weight: 600, italic: true, size: 10.5, ..body.clone() }, "Inter Semibold Italic, 10.5"),
            (TypeStyle { weight: 450, ..body }, "Inter 450, 11"),
        ];
        for (style, expected) in cases {
            assert_eq!(font(&style), expected);
        }
    }

    #[test]
    fn bigger_type_is_shown_bigger_and_keeps_its_row() {
        let sizes = [6.0, 9.0, 11.0, 16.0, 20.0, 30.0, 400.0].map(preview);
        assert!(sizes.is_sorted(), "{sizes:?}");
        assert!(sizes.iter().all(|size| (tokens::TEXT_XS..=BIGGEST).contains(size)), "{sizes:?}");
        assert!(preview(30.0) > preview(11.0), "a title stands out from body text");
    }
}
