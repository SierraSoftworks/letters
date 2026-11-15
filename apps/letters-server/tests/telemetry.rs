//! What the server reports of its use, and that reporting holds nothing up.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::post;
use letters_server::telemetry::{Event, Telemetry, Through};

/// Every event there is.
fn events() -> Vec<Event> {
    let through = [Through::Browser, Through::Desktop, Through::Link];
    let mut events =
        vec![Event::DocumentMade, Event::ShareChanged, Event::LinkMade, Event::LinkRevoked, Event::ConflictsMerged];
    events.extend(through.map(Event::SignedIn));
    events.extend(through.map(Event::SyncJoined));
    events
}

/// An analytics server that keeps what it is sent.
async fn listening() -> (String, Arc<Mutex<Vec<String>>>) {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let keep =
        |heard: Arc<Mutex<Vec<String>>>| post(move |body: String| async move { heard.lock().unwrap().push(body) });
    let routes = Router::new().route("/track/hit", keep(heard.clone())).route("/track/exception", keep(heard.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, routes).await });
    (address, heard)
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
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_reports_that_it_started_what_was_asked_and_that_it_ended() {
    let (address, heard) = listening().await;
    let telemetry = Telemetry::default();
    // Before the session there is nobody to tell.
    telemetry.record(Event::LinkMade);
    telemetry.start(&address);
    telemetry.record(Event::SyncJoined(Through::Desktop));
    telemetry.stop(Duration::from_secs(10)).await;
    let heard = heard.lock().unwrap().join("\n");
    assert!(heard.contains("\"load\"") && heard.contains("\"unload\""), "{heard}");
    assert!(heard.contains("letters.app/server"), "{heard}");
    assert!(heard.contains("sync_joined") && heard.contains("desktop"), "{heard}");
    assert!(!heard.contains("link_made"), "{heard}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_analytics_server_that_is_not_there_holds_nothing_up() {
    let telemetry = Telemetry::default();
    telemetry.start("http://127.0.0.1:9");
    let asked = Instant::now();
    telemetry.record(Event::DocumentMade);
    assert!(asked.elapsed() < Duration::from_millis(50));
    telemetry.stop(Duration::from_secs(1)).await;
    assert!(asked.elapsed() < Duration::from_secs(3));
}
