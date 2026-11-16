//! The toolkit, driven the way a person drives it: through the pointer, the keyboard and the accessibility
//! tree of a small window that uses every widget.

use std::cell::Cell;
use std::rc::Rc;

use letters_editor::{InputEvent, Key, KeyEvent, Modifiers, Platform};
use letters_render::cpu::{CpuRenderer, Frame};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Item, Point, Rect, Size};
use letters_test::{Harness, Role, WINDOW};
use letters_ui::color::{fade, same};
use letters_ui::widgets::{Button, Menu, MenuItem, Popover, SidePanel, TextField, TopBar};
use letters_ui::{Align, App, Edges, Element, Paint, Ring, Scheme, Sides, TextStyle};

#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Name(String),
    Submit,
    Bold,
    Chose(&'static str),
    Share(bool),
    Email(String),
    Notes(bool),
    Typed(InputEvent),
}

/// A window with a bar, a field, a page and a panel. It keeps every message it was sent.
#[derive(Default)]
struct Desk {
    name: String,
    email: String,
    bold: bool,
    sharing: bool,
    notes: bool,
    log: Vec<Msg>,
}

struct Blank;

impl Paint for Blank {
    fn paint(&self, _rect: Rect, _visible: Rect, _scheme: Scheme, _out: &mut DisplayList) {}
}

impl App for Desk {
    type Msg = Msg;

    fn title(&self) -> String {
        "Desk".into()
    }

    fn view(&mut self, _window: Size) -> Element<Msg> {
        let file = Menu::new()
            .label("File")
            .item(MenuItem::new("New").on_select(Msg::Chose("New")))
            .item(MenuItem::new("Open").on_select(Msg::Chose("Open")))
            .item(MenuItem::new("Print").disabled(true).on_select(Msg::Chose("Print")))
            .item(MenuItem::new("Delete").danger().on_select(Msg::Chose("Delete")));
        let share = Element::column()
            .gap(8.0)
            .child(TextField::new("Email", &self.email).on_input(Msg::Email))
            .child(Button::new("Done").on_press(Msg::Share(false)));
        let share = Popover::new(share).label("Share").modal().open(self.sharing).on_close(Msg::Share(false));
        let bar = TopBar::new()
            .start(Button::new("File").menu(file))
            .middle(Button::new("Bold").pressed(self.bold).on_press(Msg::Bold))
            .middle(Button::new("Print").disabled(true).on_press(Msg::Chose("Print")))
            .end(Button::new("Notes").pressed(self.notes).on_press(Msg::Notes(!self.notes)))
            .end(Button::new("Share").on_press(Msg::Share(true)).popover(share));

        let field =
            TextField::new("Name", &self.name).placeholder("Your name").on_input(Msg::Name).on_submit(Msg::Submit);
        let page = Element::canvas(Rc::new(Blank)).grow(1.0).role(Role::Document).label("Page").default_focus();
        let page = page.on_text(Msg::Typed);
        let mut body = Element::row().grow(1.0).child(page);
        if self.notes {
            let rows = (1..=40).map(|n| Element::text(format!("Note {n}"), TextStyle::body()));
            body =
                body.child(SidePanel::new("Notes").on_close(Msg::Notes(false)).child(Element::column().children(rows)));
        }
        Element::column().child(bar).child(Element::row().padding(Edges::all(8.0)).child(field)).child(body)
    }

    fn update(&mut self, message: Msg, _platform: &mut dyn Platform) {
        self.log.push(message.clone());
        match message {
            Msg::Name(name) => self.name = name,
            Msg::Email(email) => self.email = email,
            Msg::Bold => self.bold = !self.bold,
            Msg::Share(open) => self.sharing = open,
            Msg::Notes(open) => self.notes = open,
            Msg::Submit | Msg::Chose(_) | Msg::Typed(_) => {}
        }
    }
}

fn desk() -> Harness<Desk> {
    Harness::new(Desk::default())
}

fn named(name: &str) -> Harness<Desk> {
    Harness::new(Desk { name: name.into(), ..Desk::default() })
}

/// Runs keys and text written as `"End Backspace 'x y' Mod+A"`: quoted parts are typed, the rest are keys.
fn play(desk: &mut Harness<Desk>, script: &str) {
    let mut rest = script.trim();
    while !rest.is_empty() {
        if let Some(quoted) = rest.strip_prefix('\'') {
            let (text, after) = quoted.split_once('\'').expect("a closing quote");
            desk.type_text(text);
            rest = after.trim_start();
        } else {
            let (key, after) = rest.split_once(' ').unwrap_or((rest, ""));
            desk.key(key);
            rest = after.trim_start();
        }
    }
}

fn focus_name(desk: &mut Harness<Desk>) -> String {
    desk.focused().map(|found| found.name).unwrap_or_default()
}

// Buttons

#[test]
fn a_button_is_pressed_by_click_enter_and_space() {
    for press in ["click", "Enter", "Space"] {
        let mut desk = desk();
        match press {
            "click" => desk.click(Role::Button, "Bold"),
            key => desk.key("Tab").key("Tab").key(key),
        };
        assert_eq!(desk.app().log, [Msg::Bold], "{press}");
        assert_eq!(desk.find(Role::Button, "Bold").unwrap().toggled, Some(true), "{press}");
    }
}

#[test]
fn a_disabled_button_does_nothing_and_says_so() {
    let mut desk = desk();
    desk.click(Role::Button, "Print");
    assert!(desk.app().log.is_empty());
    assert!(desk.find(Role::Button, "Print").unwrap().disabled);
}

#[test]
fn a_press_that_leaves_the_button_is_called_off() {
    let mut desk = desk();
    let bold = desk.find(Role::Button, "Bold").unwrap().center();
    desk.press_at(bold).release_at(Point { x: bold.x, y: 400.0 });
    assert!(desk.app().log.is_empty());
}

// Menus

#[test]
fn a_menu_opens_from_its_button_and_lists_its_items() {
    let mut desk = desk();
    assert_eq!(desk.find(Role::Button, "File").unwrap().expanded, Some(false));
    desk.click(Role::Button, "File");
    desk.assert_open(Role::Menu, "File");
    assert_eq!(desk.find(Role::Button, "File").unwrap().expanded, Some(true));
    for item in ["New", "Open", "Print", "Delete"] {
        desk.assert_open(Role::MenuItem, item);
    }
    let (button, menu) =
        (desk.find(Role::Button, "File").unwrap().bounds, desk.find(Role::Menu, "File").unwrap().bounds);
    assert!(menu.y >= button.y + button.height, "the menu hangs below its button");
}

#[test]
fn choosing_an_item_sends_its_message_and_closes_the_menu() {
    // How the item is chosen, and what focus rests on afterwards.
    let ways = [("click", "Page"), ("keys", "File")];
    for (way, focus) in ways {
        let mut desk = desk();
        match way {
            "click" => desk.click(Role::Button, "File").click(Role::MenuItem, "Open"),
            _ => desk.key("Tab").key("Enter").key("Down").key("Enter"),
        };
        assert_eq!(desk.app().log, [Msg::Chose("Open")], "{way}");
        desk.assert_closed(Role::Menu, "File");
        assert_eq!(focus_name(&mut desk), focus, "{way}: focus goes back to where it was");
    }
}

#[test]
fn arrow_keys_walk_a_menu_past_disabled_items_and_wrap() {
    let mut desk = desk();
    desk.key("Tab").key("Enter");
    let mut seen = vec![focus_name(&mut desk)];
    for _ in 0..3 {
        desk.key("Down");
        seen.push(focus_name(&mut desk));
    }
    assert_eq!(seen, ["New", "Open", "Delete", "New"]);
    desk.key("End");
    assert_eq!(focus_name(&mut desk), "Delete");
    desk.key("Up");
    assert_eq!(focus_name(&mut desk), "Open");
}

#[test]
fn the_first_key_in_a_menu_opened_by_the_pointer_goes_where_it_says() {
    for (key, lands) in [("Down", "New"), ("Home", "New"), ("Up", "Delete"), ("End", "Delete")] {
        let mut desk = desk();
        desk.click(Role::Button, "File").key(key);
        assert_eq!(focus_name(&mut desk), lands, "{key}");
    }
}

#[test]
fn a_menu_closes_without_choosing() {
    for way in ["Escape", "outside", "button"] {
        let mut desk = desk();
        desk.click(Role::Button, "File");
        match way {
            "Escape" => desk.key("Escape"),
            "outside" => desk.click(Role::Document, "Page"),
            _ => desk.click(Role::Button, "File"),
        };
        desk.assert_closed(Role::Menu, "File");
        assert!(desk.app().log.is_empty(), "{way}");
    }
}

#[test]
fn a_disabled_item_cannot_be_chosen() {
    let mut desk = desk();
    desk.click(Role::Button, "File").click(Role::MenuItem, "Print");
    assert!(desk.app().log.is_empty());
    desk.assert_open(Role::Menu, "File");
}

// Text fields

#[test]
fn a_text_field_edits_its_value() {
    // (value, script, result)
    let cases = [
        ("", "'Ana'", "Ana"),
        ("Ana", "End Backspace", "An"),
        ("Ana", "Home Delete", "na"),
        ("Ana", "Home Right 'd'", "Adna"),
        ("Ana", "Mod+A 'Bo'", "Bo"),
        ("Ana", "Home Shift+Right 'E'", "Ena"),
        ("Ana", "End Shift+Home Backspace", ""),
        ("Ana Li", "End Left Left 'de '", "Ana de Li"),
        ("né", "End Backspace", "n"),
        ("Ana", "'a\tb'", "Anaa b"),
    ];
    for (value, script, result) in cases {
        let mut desk = named(value);
        desk.click(Role::TextInput, "Name");
        play(&mut desk, &format!("End {script}"));
        assert_eq!(desk.app().name, result, "{value:?} then {script}");
        assert_eq!(desk.find(Role::TextInput, "Name").unwrap().value.unwrap_or_default(), result);
    }
}

#[test]
fn a_text_field_cuts_copies_and_pastes() {
    let mut desk = named("Ana Li");
    desk.click(Role::TextInput, "Name").key("Mod+A").key("Mod+C");
    assert_eq!(desk.platform.clipboard.as_ref().map(|clip| clip.text.as_str()), Some("Ana Li"));
    desk.key("Mod+X");
    assert_eq!(desk.app().name, "");
    desk.key("Mod+V").key("Mod+V");
    assert_eq!(desk.app().name, "Ana LiAna Li");
}

#[test]
fn a_text_field_moves_by_word_and_by_line_with_each_systems_keys() {
    // (macOS keys, script from the end of "Ana de Li", result)
    let cases = [
        (false, "Mod+Left 'x'", "Ana de xLi"),
        (false, "Mod+Backspace", "Ana de "),
        (false, "Home 'x'", "xAna de Li"),
        (true, "Alt+Left 'x'", "Ana de xLi"),
        (true, "Alt+Backspace", "Ana de "),
        (true, "Mod+Left 'x'", "xAna de Li"),
        (true, "Mod+Backspace", ""),
    ];
    for (mac, script, result) in cases {
        let mut desk = if mac { named("Ana de Li").mac() } else { named("Ana de Li") };
        desk.click(Role::TextInput, "Name");
        play(&mut desk, &format!("End {script}"));
        assert_eq!(desk.app().name, result, "{script} with {} keys", if mac { "macOS" } else { "standard" });
    }
}

#[test]
fn a_double_click_selects_a_word_to_replace() {
    let mut desk = named("Ana Li");
    desk.double_click(Role::TextInput, "Name").type_text("Bo");
    assert_ne!(desk.app().name, "Ana Li");
    assert_eq!(desk.app().name.split(' ').count(), 2, "one word was replaced: {:?}", desk.app().name);
}

#[test]
fn composed_text_shows_at_once_and_joins_the_value_when_committed() {
    let mut desk = named("ab");
    desk.click(Role::TextInput, "Name").key("End");
    desk.input(InputEvent::SetComposition { text: "にほ".into(), selection: None });
    assert_eq!(desk.app().name, "ab", "the value waits for the commit");
    assert_eq!(desk.find(Role::TextInput, "Name").unwrap().value.as_deref(), Some("abにほ"));
    desk.input(InputEvent::Commit("日本".into()));
    assert_eq!(desk.app().name, "ab日本");
    assert_eq!(desk.find(Role::TextInput, "Name").unwrap().value.as_deref(), Some("ab日本"));
}

#[test]
fn enter_submits_a_text_field() {
    let mut desk = named("Ana");
    desk.click(Role::TextInput, "Name").key("Enter");
    assert_eq!(desk.app().log, [Msg::Submit]);
}

// Focus

#[test]
fn focus_starts_on_the_page_and_typing_goes_there() {
    let mut desk = desk();
    assert_eq!(focus_name(&mut desk), "Page");
    desk.type_text("a");
    assert_eq!(desk.app().log, [Msg::Typed(InputEvent::Insert("a".into()))]);
}

#[test]
fn tab_visits_every_control_in_order_and_wraps() {
    let order = ["File", "Bold", "Notes", "Share", "Name", "Page"];
    let mut desk = desk();
    for (key, expected) in [("Tab", order.to_vec()), ("Shift+Tab", order.iter().rev().skip(1).copied().collect())] {
        for name in expected {
            desk.key(key);
            assert_eq!(focus_name(&mut desk), name, "{key}");
        }
    }
}

#[test]
fn clicking_a_button_leaves_focus_where_the_writer_was() {
    let mut desk = desk();
    desk.click(Role::Button, "Bold");
    assert_eq!(focus_name(&mut desk), "Page");
    desk.click(Role::TextInput, "Name");
    assert_eq!(focus_name(&mut desk), "Name");
    desk.click(Role::Button, "Bold");
    assert_eq!(focus_name(&mut desk), "Name");
    desk.key("Escape");
    assert_eq!(focus_name(&mut desk), "Page", "Escape hands focus back to the page");
}

#[test]
fn a_focus_ring_shows_for_the_keyboard_only() {
    let ring = |desk: &mut Harness<Desk>| {
        let bold = desk.find(Role::Button, "Bold").unwrap().bounds;
        let around = |rect: &Rect| rect.x < bold.x && rect.x + rect.width > bold.x + bold.width && rect.y < bold.y;
        desk.paint()
            .items
            .iter()
            .any(|item| matches!(item, Item::Stroke { rect, .. } if around(rect) && rect.width < 2.0 * bold.width))
    };
    let mut desk = desk();
    desk.click(Role::Button, "Bold");
    assert!(!ring(&mut desk), "a click shows no ring");
    desk.key("Tab").key("Tab");
    assert_eq!(focus_name(&mut desk), "Bold");
    assert!(ring(&mut desk), "Tab shows one");
}

// Popovers

#[test]
fn a_modal_popover_takes_focus_and_keeps_it() {
    let mut desk = desk();
    desk.click(Role::Button, "Share");
    desk.assert_open(Role::Dialog, "Share");
    let mut seen = vec![focus_name(&mut desk)];
    for _ in 0..2 {
        desk.key("Tab");
        seen.push(focus_name(&mut desk));
    }
    assert_eq!(seen, ["Email", "Done", "Email"]);
    desk.type_text("a@b.c");
    assert_eq!(desk.app().email, "a@b.c");
}

#[test]
fn a_modal_popover_shields_the_window_and_closes_three_ways() {
    for way in ["Escape", "Done", "backdrop"] {
        let mut desk = desk();
        desk.click(Role::Button, "Share");
        match way {
            "Escape" => desk.key("Escape"),
            "Done" => desk.click(Role::Button, "Done"),
            _ => desk.click(Role::Button, "Bold"),
        };
        desk.assert_closed(Role::Dialog, "Share");
        assert!(!desk.app().log.contains(&Msg::Bold), "{way}: nothing beneath was pressed");
        assert_eq!(focus_name(&mut desk), "Page", "{way}");
    }
}

#[test]
fn a_dialog_sits_in_the_middle_of_the_window() {
    let mut desk = desk();
    desk.click(Role::Button, "Share");
    let dialog = desk.find(Role::Dialog, "Share").unwrap();
    let center = dialog.center();
    assert!(
        (center.x - WINDOW.width / 2.0).abs() <= 1.0 && (center.y - WINDOW.height / 2.0).abs() <= 1.0,
        "{dialog:?}"
    );
}

// Layout and scrolling

#[test]
fn the_bar_spans_the_window_and_the_panel_takes_its_side() {
    for width in [1000.0, 640.0] {
        let mut desk = desk();
        desk.resize(width, 500.0).click(Role::Button, "Notes");
        let bar = desk.find(Role::Toolbar, "Document").unwrap().bounds;
        assert_eq!((bar.x, bar.y, bar.width), (0.0, 0.0, width));
        let panel = desk.find(Role::Complementary, "Notes").unwrap().bounds;
        assert_eq!(panel.x + panel.width, width, "the panel is at the right edge");
        assert_eq!(panel.y + panel.height, 500.0, "and reaches the bottom");
        let page = desk.find(Role::Document, "Page").unwrap().bounds;
        assert_eq!(page.x + page.width, panel.x, "the page gets the rest");
        let share = desk.find(Role::Button, "Share").unwrap().bounds;
        assert!(share.x + share.width <= width, "the bar's end stays in the window");
    }
}

#[test]
fn a_long_panel_scrolls_under_the_wheel_and_stops_at_its_ends() {
    let mut desk = desk();
    desk.click(Role::Button, "Notes");
    let top = |desk: &mut Harness<Desk>| desk.find(Role::Label, "Note 1").unwrap().bounds.y;
    let (start, over) = (top(&mut desk), desk.find(Role::Complementary, "Notes").unwrap().center());
    desk.scroll_at(over, 100.0);
    assert_eq!(top(&mut desk), start - 100.0);
    desk.scroll_at(over, -500.0);
    assert_eq!(top(&mut desk), start, "it does not scroll past the top");
    desk.scroll_at(over, 1e6);
    let last = desk.find(Role::Label, "Note 40").unwrap().bounds;
    assert!(last.y + last.height <= WINDOW.height, "the last row is in view at the end: {last:?}");
}

/// A window that is one long list, each row built only while it is near what shows. A press on a row
/// picks it, and the button under the list picks the last. The row that is picked asks to be in view.
struct Ledger {
    rows: usize,
    picked: usize,
    /// How many times a row was built.
    built: Rc<Cell<usize>>,
    /// How many times the window was built, when its rows are to be others each time.
    restless: Option<usize>,
}

const LEDGER_ROW: f32 = 24.0;

impl App for Ledger {
    type Msg = usize;

    fn title(&self) -> String {
        "Ledger".into()
    }

    fn view(&mut self, _window: Size) -> Element<usize> {
        let builds = self.restless.as_mut().map(|builds| std::mem::replace(builds, *builds + 1));
        let rows = (0..self.rows).map(|n| {
            let built = self.built.clone();
            let row = move || {
                built.set(built.get() + 1);
                Element::text(format!("Row {n}"), TextStyle::body()).on_click(n)
            };
            Element::later(LEDGER_ROW, row).key((n, builds)).in_view(n == self.picked)
        });
        let list = Element::column().grow(1.0).scroll().role(Role::List).label("Rows").children(rows);
        Element::column().child(list).child(Button::new("Last").on_press(self.rows - 1))
    }

    fn update(&mut self, picked: usize, _platform: &mut dyn Platform) {
        self.picked = picked;
    }
}

fn ledger(rows: usize) -> (Harness<Ledger>, Rc<Cell<usize>>) {
    let built = Rc::new(Cell::new(0));
    (Harness::new(Ledger { rows, picked: 0, built: built.clone(), restless: None }), built)
}

/// The rows of the ledger that are built, by their numbers, and the part of the window its list has with
/// the middle of that.
fn ledger_rows(ledger: &mut Harness<Ledger>) -> (Vec<usize>, Rect, Point) {
    let list = ledger.find(Role::List, "Rows").unwrap();
    let rows = ledger.controls().into_iter().filter_map(|found| found.name.strip_prefix("Row ")?.parse().ok());
    (rows.collect(), list.bounds, list.center())
}

#[test]
fn a_long_list_builds_the_rows_near_what_shows_and_keeps_the_place_of_the_rest() {
    let (mut ledger, built) = ledger(10_000);
    let (rows, list, middle) = ledger_rows(&mut ledger);
    let in_view = (list.height / LEDGER_ROW).ceil() as usize;
    assert_eq!(rows.first(), Some(&0));
    assert!(rows.len() >= in_view && rows.len() <= 3 * in_view, "{} rows are built for {in_view} in view", rows.len());
    assert!(built.get() <= 3 * in_view, "no row was built that is far from view: {}", built.get());

    // The rows just out of view are there before they scroll in, so the window is not built for them.
    let before = built.get();
    ledger.scroll_at(middle, 3.0 * LEDGER_ROW);
    assert!(ledger.find(Role::Label, &format!("Row {}", in_view + 2)).is_some());
    assert_eq!(built.get(), before);

    // A row that scrolls into view is there, where its place was kept.
    ledger.scroll_at(middle, 1_997.0 * LEDGER_ROW);
    let (rows, ..) = ledger_rows(&mut ledger);
    assert!(rows.contains(&2_000) && !rows.contains(&0), "the rows built are those in view now: {rows:?}");
    assert_eq!(ledger.find(Role::Label, "Row 2000").unwrap().bounds.y, list.y);

    // The list is as tall as all of its rows, so its end is the last row.
    ledger.scroll_at(middle, 1e9);
    let last = ledger.find(Role::Label, "Row 9999").expect("the last row at the end of the list").bounds;
    assert_eq!(last.y + LEDGER_ROW, list.y + list.height);
}

#[test]
fn a_row_far_from_view_that_asks_to_be_in_view_is_built_there() {
    let (mut ledger, _) = ledger(10_000);
    ledger.click(Role::Button, "Last");
    let (rows, list, _) = ledger_rows(&mut ledger);
    let last = ledger.find(Role::Label, "Row 9999").expect("the row that was picked").bounds;
    assert!(last.y >= list.y && last.y + last.height <= list.y + list.height, "it is in view: {last:?}");
    assert!(rows.len() < 200, "and only the rows near it are built with it: {}", rows.len());

    // A press on a row that was built reaches it.
    ledger.click(Role::Label, "Row 9990");
    assert_eq!(ledger.app().picked, 9_990);
}

#[test]
fn rows_that_are_others_at_every_build_do_not_have_the_window_built_without_end() {
    let built = Rc::new(Cell::new(0));
    let mut ledger = Harness::new(Ledger { rows: 100, picked: 0, built: built.clone(), restless: Some(0) });
    assert!(!ledger.paint().items.is_empty());
    assert!(built.get() <= 300, "the rows were built {} times", built.get());
}

#[test]
fn the_window_repaints_only_when_something_changed() {
    let mut desk = desk();
    assert!(!desk.paint().items.is_empty());
    assert!(!desk.ui().needs_paint());
    desk.hover(Role::Button, "Bold");
    assert!(desk.ui().needs_paint(), "a hovered button looks different");
    desk.paint();
    desk.hover(Role::Button, "Bold");
    assert!(!desk.ui().needs_paint(), "the pointer staying put changes nothing");
}

#[test]
fn the_caret_blinks_while_a_field_has_focus() {
    let mut desk = named("Ana");
    desk.paint();
    assert_eq!(desk.ui().animate(0), None, "nothing blinks without a caret");
    desk.click(Role::TextInput, "Name").paint();
    let next = desk.ui().animate(1000).expect("a field's caret blinks");
    assert!(next > 1000);
    desk.paint();
    desk.ui().animate(next);
    assert!(desk.ui().needs_paint(), "the caret went out");
    desk.ui().focus(false);
    desk.paint();
    assert_eq!(desk.ui().animate(next + 1), None, "a window without the keyboard shows no caret");
}

// A field the app draws

/// What a slip was sent: a key its field used, or text typed into the page or into the field.
#[derive(Clone, Debug, PartialEq)]
enum Wrote {
    Key(KeyEvent),
    Page(InputEvent),
    Field(InputEvent),
}

/// A page with a field that the app draws itself, in a popover that hangs beside the page. It keeps what
/// it was sent, and every key the app was offered.
#[derive(Default)]
struct Slip {
    sent: Vec<Wrote>,
    offered: Vec<KeyEvent>,
}

const PAGE_CARET: Rect = Rect { x: 10.0, y: 10.0, width: 1.0, height: 20.0 };

impl App for Slip {
    type Msg = Wrote;

    fn title(&self) -> String {
        "Slip".into()
    }

    fn view(&mut self, _window: Size) -> Element<Wrote> {
        // The field uses the keys that edit what is written in it, and leaves the rest.
        let edits = |event: &KeyEvent| matches!(event.key, Key::Left | Key::Char('b'));
        let field = Element::canvas(Rc::new(Blank))
            .width(200.0)
            .height(24.0)
            .role(Role::TextInput)
            .label("Comment")
            .field("Well said")
            .caret(Some(Rect::new(4.0, 2.0, 1.0, 16.0)))
            .on_text(Wrote::Field)
            .on_key(move |event| edits(event).then(|| Wrote::Key(event.clone())));
        let note = Popover::new(field).label("Note").beside().stays().open(true);
        let page = Element::canvas(Rc::new(Blank)).grow(1.0).role(Role::Document).label("Page").default_focus();
        page.caret(Some(PAGE_CARET)).on_text(Wrote::Page).popover(note)
    }

    fn update(&mut self, message: Wrote, _platform: &mut dyn Platform) {
        self.sent.push(message);
    }

    fn key(&mut self, event: &KeyEvent, _platform: &mut dyn Platform) -> bool {
        self.offered.push(event.clone());
        true
    }

    fn describe(&self, canvas: &mut accesskit::Node, _nodes: &mut Vec<(accesskit::NodeId, accesskit::Node)>, _: bool) {
        canvas.set_description("The app's own");
    }
}

fn chord(key: Key, primary: bool) -> KeyEvent {
    KeyEvent { key, mods: Modifiers { primary, ..Modifiers::default() } }
}

#[test]
fn a_field_the_app_draws_has_its_keys_and_text_before_the_app() {
    let mut slip = Harness::new(Slip::default());
    // On the page, every key is the app's and what is typed is the page's.
    slip.key("Left").key("Mod+B").type_text("a");
    assert_eq!(slip.app().offered, [chord(Key::Left, false), chord(Key::Char('b'), true)]);
    assert_eq!(slip.app().sent, [Wrote::Page(InputEvent::Insert("a".into()))]);
    assert!(slip.ui().types_in_canvas());

    // In the field, the keys it uses are its own, and of the rest the app hears the shortcuts.
    slip.click(Role::TextInput, "Comment");
    assert_eq!(focus_of(&mut slip), "Comment");
    slip.key("Left").key("Mod+B").key("Down").key("Mod+S").type_text("b");
    let field = [Wrote::Key(chord(Key::Left, false)), Wrote::Key(chord(Key::Char('b'), true))];
    assert_eq!(slip.app().sent[1..], [&field[..], &[Wrote::Field(InputEvent::Insert("b".into()))]].concat());
    assert_eq!(slip.app().offered[2..], [chord(Key::Char('s'), true)]);
    assert!(!slip.ui().types_in_canvas(), "a shell asks the page nothing about text typed in the field");
}

#[test]
fn a_field_the_app_draws_keeps_its_caret_to_itself_and_says_what_is_written_in_it() {
    let mut slip = Harness::new(Slip::default());
    // While the page has focus the caret that shows is the page's, though the field hangs beside it.
    let page = slip.find(Role::Document, "Page").expect("the page");
    let caret = slip.ui().caret_rect().expect("the page's caret");
    assert_eq!((caret.x - page.bounds.x, caret.height), (PAGE_CARET.x, PAGE_CARET.height));

    let field = slip.find(Role::TextInput, "Comment").expect("the field");
    assert_eq!(field.value.as_deref(), Some("Well said"));
    assert_eq!(field.description, None, "the app is asked to describe its own canvas only");
    assert_eq!(page.description.as_deref(), Some("The app's own"));

    slip.click(Role::TextInput, "Comment");
    let caret = slip.ui().caret_rect().expect("the field's caret");
    assert!(field.bounds.contains(Point { x: caret.x, y: caret.y }), "{caret:?} in {:?}", field.bounds);
}

fn focus_of<A: App>(window: &mut Harness<A>) -> String {
    window.focused().map(|found| found.name).unwrap_or_default()
}

/// A dialog of buttons that go when they are pressed, over a button of the window. The first in the dialog
/// is the heir of the keyboard once no more than `heir` of them are left, and the one under it always is.
struct Roll {
    left: Vec<&'static str>,
    heir: usize,
}

impl App for Roll {
    type Msg = &'static str;

    fn title(&self) -> String {
        "Roll".into()
    }

    fn view(&mut self, _window: Size) -> Element<&'static str> {
        let rows = self.left.iter().enumerate().map(|(at, name)| {
            let row = Element::from(Button::new(*name).on_press(*name)).key(name);
            if at == 0 && self.left.len() <= self.heir { row.heir() } else { row }
        });
        let roll = Popover::new(Element::column().children(rows)).label("Roll").modal().open(true);
        let under = Element::from(Button::new("Under").on_press("Under")).heir();
        Element::column().child(under).child(Element::row().overlay().popover(roll))
    }

    fn update(&mut self, message: &'static str, _platform: &mut dyn Platform) {
        self.left.retain(|name| *name != message);
    }
}

#[test]
fn the_keyboard_goes_to_an_heir_in_the_dialog_when_what_had_it_goes_and_nowhere_without_one() {
    let roll = |heir| Harness::new(Roll { left: vec!["One", "Two", "Three"], heir });
    // With the keyboard on none of its buttons, the dialog itself is said to have it.
    let nowhere = "Roll";
    for (heir, then) in [(3, "One"), (0, nowhere)] {
        let mut roll = roll(heir);
        roll.key("Tab");
        assert_eq!(focus_of(&mut roll), "Two");
        roll.key("Enter");
        assert_eq!(roll.app().left, ["One", "Three"]);
        assert_eq!(focus_of(&mut roll), then, "an heir from {heir} left: never the one under the dialog");
    }

    // An heir takes the keyboard from nothing that still has it.
    let mut kept = roll(3);
    kept.key("Tab").key("Tab");
    kept.click(Role::Button, "Two");
    assert_eq!(focus_of(&mut kept), "Three");

    // And it is the heir of what went, not of a keyboard that was nowhere before it came.
    let mut late = roll(1);
    late.key("Tab").key("Enter");
    late.click(Role::Button, "Three");
    assert_eq!(late.app().left, ["One"]);
    assert_eq!(focus_of(&mut late), nowhere, "nothing is pressed by an Enter the person did not aim");
}

/// Four boxes with corners this round, joined to something on these sides: one filled, one with a line round
/// it, one cut about what it holds, and one with the keyboard on it.
struct Joined(Sides, f32);

impl App for Joined {
    type Msg = ();

    fn title(&self) -> String {
        "Joined".into()
    }

    fn view(&mut self, _window: Size) -> Element<()> {
        // An ink that shows where it was laid on twice.
        let ink = fade(same(200, 30, 30), 0.5);
        let shape = |name: &str| {
            Element::row().width(80.0).height(60.0).radius(self.1).square(self.0).role(Role::Group).label(name)
        };
        // What the third holds runs out past it on every side, so that only the cut shapes it.
        let held = Element::row().width(120.0).height(100.0).bg(ink);
        let held = Element::column().width(80.0).height(100.0).align(Align::Center).child(held);
        Element::column()
            .padding(Edges::all(20.0))
            .gap(20.0)
            .child(shape("Filled").bg(ink))
            .child(shape("Lined").border(Edges::all(4.0), ink))
            .child(shape("Cut").align(Align::Center).clip().child(held))
            .child(shape("Ringed").focusable().ring(Ring::Outline(-4.0)))
    }

    fn update(&mut self, _message: (), _platform: &mut dyn Platform) {}
}

#[test]
fn a_box_is_square_at_the_corners_of_the_sides_it_is_joined_on_and_round_at_the_rest() {
    let both = Sides { left: true, ..Sides::TOP };
    let every = Sides { top: true, right: true, bottom: true, left: true };
    let round =
        [Sides::NONE, Sides::TOP, Sides::RIGHT, Sides::BOTTOM, Sides::LEFT, both, every].map(|sides| (sides, 16.0));
    // A box that is round nowhere is as square joined as not.
    for (sides, radius) in round.into_iter().chain([(Sides::NONE, 0.0), (both, 0.0)]) {
        let mut window = Harness::new(Joined(sides, radius));
        window.key("Tab");
        let list = window.paint();
        let mut frame = Frame::new(WINDOW.width as u32, WINDOW.height as u32);
        CpuRenderer::new().render(&list, &Images::default(), 1.0, Color::rgb(255, 255, 255), &mut frame);
        let at = |x: f32, y: f32| frame.pixel(x as u32, y as u32);
        let paper = at(1.0, 1.0);
        for name in ["Filled", "Lined", "Cut", "Ringed"] {
            let shape = window.find(Role::Group, name).expect(name).bounds;
            // A little inside each edge, where a line or a ring runs, and half way along each side.
            let (left, top) = (shape.x + 2.0, shape.y + 2.0);
            let (right, bottom) = (shape.x + shape.width - 3.0, shape.y + shape.height - 3.0);
            let (across, down) = (shape.x + shape.width / 2.0, shape.y + shape.height / 2.0);
            let ink = at(across, top);
            assert_ne!(ink, paper, "{name} joined on {sides:?}: drawn at all");
            // Every side is drawn the whole way, joined or not, in one ink, and nothing is drawn beyond it.
            for (x, y) in [(right, down), (across, bottom), (left, down)] {
                assert_eq!(at(x, y), ink, "{name} joined on {sides:?}: the side at {x}, {y}");
            }
            let beyond = [(across, top - 5.0), (right + 5.0, down), (across, bottom + 5.0), (left - 5.0, down)];
            for (x, y) in beyond {
                assert_eq!(at(x, y), paper, "{name} joined on {sides:?}: beyond it at {x}, {y}");
            }
            // A corner is drawn into where a side it lies on is joined, once, and is left out where it is round.
            let corners = [
                (left, top, sides.left || sides.top),
                (right, top, sides.right || sides.top),
                (right, bottom, sides.right || sides.bottom),
                (left, bottom, sides.left || sides.bottom),
            ];
            for (x, y, square) in corners {
                let wanted = if square || radius == 0.0 { ink } else { paper };
                assert_eq!(at(x, y), wanted, "{name} joined on {sides:?}: the corner at {x}, {y}");
            }
        }
    }
}
