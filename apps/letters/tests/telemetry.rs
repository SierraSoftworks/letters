use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use letters::telemetry::{self, Event, Merged, Page, Scope, Source, Telemetry};
use letters_ui::{CopyFormat, Request};

/// Every event there is.
fn events() -> Vec<Event> {
    use CopyFormat::{Clean, Letters, Markdown, Pdf, WebPage, Word};
    let mut events = vec![
        Event::DocumentImported,
        Event::DocumentPrinted,
        Event::PictureAdded { linked: true },
        Event::PictureAdded { linked: false },
        Event::SignInStarted,
        Event::ShareChanged,
        Event::LinkMade,
        Event::LinkRevoked,
        Event::SettingsChanged,
    ];
    events.extend([Source::New, Source::Listed, Source::Chosen].map(Event::DocumentOpened));
    events.extend([Letters, Clean, Word, Pdf, WebPage, Markdown].map(Event::DocumentCopied));
    events.extend([Merged::Copies, Merged::File, Merged::Original].map(Event::MergeAsked));
    events.extend([Scope::Document, Scope::Folder, Scope::Host].map(Event::TrustGranted));
    events
}

/// A server that takes whatever is sent to it and hands on each request's text.
fn server() -> (String, Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (heard, hears) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let (Ok(mut stream), heard) = (stream, heard.clone()) else { continue };
            thread::spawn(move || {
                let mut text = Vec::new();
                let mut part = [0; 4096];
                // One request a connection is all the test needs: its body is short and comes whole.
                while let Ok(read) = stream.read(&mut part) {
                    text.extend_from_slice(&part[..read]);
                    let whole = String::from_utf8_lossy(&text).into_owned();
                    if read == 0 || whole.split_once("\r\n\r\n").is_some_and(|(_, body)| body.ends_with('}')) {
                        let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
                        let _ = heard.send(whole);
                        break;
                    }
                }
            });
        }
    });
    (address, hears)
}

fn heard(hears: &Receiver<String>) -> Vec<String> {
    hears.try_iter().collect()
}

#[test]
fn what_is_said_is_plain_words_from_a_closed_list() {
    let plain = |word: &str| !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'_');
    for event in events() {
        assert!(plain(event.name()), "{event:?}");
        if let Some((key, value)) = event.property() {
            assert!(plain(key) && plain(value), "{event:?}");
        }
    }
    for page in Page::ALL {
        assert!(page.path().strip_prefix('/').is_some_and(plain), "{page:?}");
    }
}

#[test]
fn a_request_says_which_feature_was_used_and_nothing_it_carries() {
    let secret = "Ada's diary";
    let cases = [
        (Request::Print, Some(Event::DocumentPrinted)),
        (Request::LinkedPicture(secret.into()), Some(Event::PictureAdded { linked: true })),
        (Request::TrustPictures(secret.into()), Some(Event::TrustGranted(Scope::Host))),
        (Request::SignIn { server: secret.into() }, Some(Event::SignInStarted)),
        (
            Request::SaveCopy {
                name: secret.into(),
                format: CopyFormat::Pdf,
                folder: Some(secret.into()),
                replace: false,
            },
            Some(Event::DocumentCopied(CopyFormat::Pdf)),
        ),
        (Request::MergeConflicts { confirmed: false }, Some(Event::MergeAsked(Merged::Copies))),
        // The yes on the card is the same merge, counted when it was asked for.
        (Request::MergeConflicts { confirmed: true }, None),
        (Request::Rename(secret.into()), None),
        (Request::Named(secret.into()), None),
    ];
    for (request, expected) in cases {
        let event = Event::asked(&request);
        assert_eq!(event, expected, "{request:?}");
        let said = event.map(|event| format!("{} {:?}", event.name(), event.property())).unwrap_or_default();
        assert!(!said.contains("Ada"), "{request:?}");
    }
}

#[test]
fn the_settings_file_turns_reporting_off() {
    let cases = [
        ("", true),
        ("[writer]\nname = \"Ada Lovelace\"\n", true),
        ("[telemetry]\nenabled = true\n", true),
        ("[telemetry]\nenabled = false\n", false),
        // Only a plain no is a no.
        ("[telemetry]\nenabled = \"sometimes\"\n", true),
        ("not toml [", true),
    ];
    for (file, expected) in cases {
        assert_eq!(telemetry::allowed(file), expected, "{file:?}");
    }
}

#[test]
fn a_session_reports_its_pages_and_events() {
    let (address, hears) = server();
    let telemetry = Telemetry::start(address, true);
    telemetry.page(Page::Editor);
    telemetry.record(Event::DocumentCopied(CopyFormat::Pdf));
    telemetry.stop(Duration::from_secs(10));
    let heard = heard(&hears).join("\n");
    assert!(heard.contains("/editor"), "{heard}");
    assert!(heard.contains("document_copied") && heard.contains("pdf"), "{heard}");
}

#[test]
fn nothing_is_sent_once_the_person_says_no() {
    let (address, hears) = server();
    let telemetry = Telemetry::start(address, false);
    telemetry.page(Page::Landing);
    telemetry.record(Event::DocumentPrinted);
    telemetry.stop(Duration::from_secs(10));
    assert_eq!(heard(&hears), Vec::<String>::new());
}

#[test]
fn a_server_that_is_not_there_holds_nothing_up() {
    // Nothing listens here, and the session is over before any send could time out.
    let telemetry = Telemetry::start("http://127.0.0.1:9", true);
    let asked = Instant::now();
    telemetry.record(Event::DocumentPrinted);
    assert!(asked.elapsed() < Duration::from_millis(50));
    telemetry.stop(Duration::from_millis(300));
    assert!(asked.elapsed() < Duration::from_secs(2));
    Telemetry::off().record(Event::DocumentPrinted);
}
