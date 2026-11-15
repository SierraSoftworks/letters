use letters::chrome::{self, Button, Buttons, Host};
use letters_ui::{Chrome, Frame, WindowButton};

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

/// A stand-in for the desktop's settings portal, answering the way GNOME's does.
#[cfg(all(unix, not(target_os = "macos")))]
mod portal {
    use zbus::fdo;
    use zbus::zvariant::{OwnedValue, Value};

    pub struct Settings;

    #[zbus::interface(name = "org.freedesktop.portal.Settings")]
    impl Settings {
        fn read_one(&self, namespace: &str, key: &str) -> fdo::Result<OwnedValue> {
            let value = match (namespace, key) {
                ("org.gnome.desktop.wm.preferences", "button-layout") => Value::from("close,minimize:appmenu"),
                ("org.gnome.desktop.interface", "enable-animations") => Value::from(false),
                _ => return Err(fdo::Error::Failed(format!("no setting {namespace} {key}"))),
            };
            value.try_into().map_err(|error: zbus::zvariant::Error| fdo::Error::Failed(error.to_string()))
        }
    }
}

#[test]
#[cfg(all(unix, not(target_os = "macos")))]
#[ignore = "needs a session bus of its own: dbus-run-session -- cargo test -p letters --test chrome -- --ignored"]
fn the_portal_is_asked_for_the_layout_and_for_less_motion() {
    let served = zbus::blocking::connection::Builder::session()
        .and_then(|bus| bus.name("org.freedesktop.portal.Desktop"))
        .and_then(|bus| bus.serve_at("/org/freedesktop/portal/desktop", portal::Settings))
        .and_then(|bus| bus.build());
    let _portal = served.expect("a session bus to serve the portal on");

    let buttons = Buttons::ask().expect("the portal knows the layout");
    assert_eq!((buttons.order.as_slice(), buttons.at_start), (&[Button::Close, Button::Minimise][..], true));
    assert_eq!(chrome::reduced_motion(), Some(true), "no animations is less motion");
}

#[test]
fn a_window_is_framed_as_its_desktop_expects() {
    let buttons = Buttons::parse("close,minimize:");
    let shown = [WindowButton::Close, WindowButton::Minimise];
    // The host, whether the window is maximised and full screen, then its frame and how many buttons.
    let cases = [
        (Host::Linux, false, false, Frame::Floating, 2),
        (Host::Linux, true, false, Frame::Filled, 2),
        (Host::Linux, false, true, Frame::Filled, 2),
        (Host::Mac, false, false, Frame::Unified, 0),
        (Host::Mac, true, false, Frame::Unified, 0),
        (Host::Other, false, false, Frame::System, 0),
    ];
    for (host, maximized, full_screen, frame, count) in cases {
        let chrome = host.chrome(&buttons, maximized, full_screen);
        let case = format!("{host:?}, maximised {maximized}, full screen {full_screen}");
        assert_eq!(chrome.frame, frame, "{case}");
        assert_eq!(chrome.buttons, shown[..count], "{case}");
        assert_eq!(chrome.buttons_at_start, count > 0, "{case}");
        assert_eq!(chrome.maximized, maximized && host != Host::Other, "{case}");
    }
}

#[test]
fn the_bar_leaves_room_for_the_lights_macos_draws() {
    let none = Buttons::parse("");
    assert!(Host::Mac.chrome(&none, false, false).inset_start > 0.0);
    assert_eq!(Host::Mac.chrome(&none, false, true).inset_start, 0.0, "a full screen window has no lights");
    assert_eq!(Host::Linux.chrome(&none, false, false).inset_start, 0.0);
    assert_eq!(Host::Other.chrome(&none, true, true), Chrome::default());
}
