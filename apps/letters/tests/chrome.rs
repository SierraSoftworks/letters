use letters::chrome::{Button, Buttons};

const ALL: [Button; 3] = [Button::Minimise, Button::Maximise, Button::Close];

#[test]
fn a_button_layout_gives_the_buttons_their_order_and_side() {
    let cases: [(&str, &[Button], bool); 9] = [
        ("appmenu:minimize,maximize,close", &ALL, false),
        ("appmenu:close", &[Button::Close], false),
        ("close,minimize,maximize:", &[Button::Close, Button::Minimise, Button::Maximise], true),
        ("close,minimize,maximize:appmenu", &[Button::Close, Button::Minimise, Button::Maximise], true),
        (":maximize,close", &[Button::Maximise, Button::Close], false),
        ("icon:minimize, maximize ,close", &ALL, false),
        // Buttons on both sides: the bar has one group, and it goes where Close is.
        ("minimize:close", &[Button::Close], false),
        ("close:minimize,maximize", &[Button::Close], true),
        // Someone who asked for no buttons gets none.
        ("appmenu:", &[], false),
    ];
    for (layout, order, at_start) in cases {
        let buttons = Buttons::parse(layout);
        assert_eq!((buttons.order.as_slice(), buttons.at_start), (order, at_start), "{layout}");
    }
}

#[test]
fn a_layout_that_makes_no_sense_never_panics() {
    for layout in ["", ":", ",,,", "close", "a:b:c", "close:close", "🙂"] {
        let buttons = Buttons::parse(layout);
        assert!(buttons.order.len() <= 3, "{layout}: {buttons:?}");
    }
    assert_eq!(Buttons::parse("close").order, [Button::Close], "a layout with no colon is all at the end");
}

#[test]
fn a_desktop_that_cannot_be_asked_gets_its_usual_buttons() {
    let cases: [(&str, &[Button]); 6] = [
        ("GNOME", &[Button::Close]),
        ("GNOME-Classic:GNOME", &[Button::Close]),
        ("ubuntu:GNOME", &ALL),
        ("KDE", &ALL),
        ("XFCE", &ALL),
        ("", &ALL),
    ];
    for (desktop, order) in cases {
        let buttons = Buttons::default_for(desktop);
        assert_eq!(buttons.order, order, "{desktop}");
        assert!(!buttons.at_start, "{desktop}");
    }
}

#[test]
fn asking_a_desktop_with_no_settings_portal_gives_up_quietly() {
    // Whatever this machine has, asking must come back, and with nothing or something usable.
    let started = std::time::Instant::now();
    let asked = Buttons::ask();
    assert!(asked.is_none_or(|buttons| buttons.order.len() <= 3));
    assert!(started.elapsed() < std::time::Duration::from_secs(30), "asking hung");
}
