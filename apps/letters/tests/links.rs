//! What a click on a link does, by what the person trusts of its document.

use letters::links::{self, Step};
use letters_trust::{Origin, Trust};

#[test]
fn a_link_is_opened_asked_about_or_refused_by_where_it_leads_and_what_is_trusted() {
    let trust = Trust::parse("", None).unwrap();
    let (trusted, held) = (trust.gate("made-here", Origin::Created), trust.gate("from-outside", Origin::Outside));
    // The address, and whether it opens at once from a trusted document and from one in safe mode.
    // `None` is a link that is never opened.
    let cases = [
        ("https://harland.example/quotes?ref=letter", Some((true, false))),
        ("mailto:ada@harland.example", Some((true, false))),
        // It goes to the host after the `@`, so the whole address is shown first.
        ("https://bank.example@elsewhere.example/", Some((false, false))),
        // Another app's address.
        ("zoommtg://harland.example/join", Some((false, false))),
        ("javascript:alert(1)", None),
        ("file:///etc/passwd", None),
        // With no scheme the system would look for a file on this device.
        ("../run.sh", None),
        ("www.harland.example", None),
        ("C:\\Windows\\notepad.exe", None),
        ("", None),
    ];
    for (href, opens) in cases {
        for (gate, at_once) in [(&trusted, opens.map(|opens| opens.0)), (&held, opens.map(|opens| opens.1))] {
            let (clicked, confirmed) = (links::step(gate, href, false), links::step(gate, href, true));
            match at_once {
                Some(true) => assert_eq!((clicked, confirmed), (Step::Open, Step::Open), "{href}"),
                // A yes to the whole address is enough.
                Some(false) => assert_eq!((clicked, confirmed), (Step::Ask, Step::Open), "{href}"),
                // And changes nothing for a link that is refused.
                None => {
                    assert!(matches!(clicked, Step::Refuse(_)), "{href}: {clicked:?}");
                    assert!(matches!(confirmed, Step::Refuse(_)), "{href}: {confirmed:?}");
                }
            }
        }
    }
}
