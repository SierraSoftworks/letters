use std::collections::HashSet;

use letters::platform::{Desktop, block_id, document_id, local, rfc3339};
use letters_editor::Platform;

#[test]
fn dates_are_rfc_3339_in_utc() {
    let cases = [
        (0, "1970-01-01T00:00:00Z"),
        (951_782_400, "2000-02-29T00:00:00Z"),
        (1_794_688_471, "2026-11-14T20:34:31Z"),
        (4_102_444_799, "2099-12-31T23:59:59Z"),
    ];
    for (seconds, expected) in cases {
        assert_eq!(rfc3339(seconds), expected);
    }
}

#[test]
fn a_local_time_is_the_same_moment_with_an_offset() {
    for utc in ["1970-01-01T00:00:00Z", "2026-03-29T01:30:00Z", "2026-11-14T20:34:31Z"] {
        let shown = local(utc);
        let moment = |text: &str| text.parse::<jiff::Timestamp>().unwrap();
        assert_eq!(moment(&shown), moment(utc), "{shown}");
        assert!(!shown.ends_with('Z'), "{shown} states no offset");
    }
}

#[test]
fn a_time_that_cannot_be_read_is_passed_on_as_it_is() {
    for text in ["", "yesterday", "2026-13-45T00:00:00Z"] {
        assert_eq!(local(text), text);
    }
}

#[test]
fn ids_are_unique_and_safe_to_store() {
    let mut platform = Desktop::new();
    let ids: HashSet<String> = (0..1_000).map(|_| platform.new_id()).collect();
    assert_eq!(ids.len(), 1_000);
    assert!(ids.iter().all(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())));
    assert_ne!(block_id(), block_id());
}

#[test]
fn a_document_id_is_a_version_4_uuid() {
    let id = document_id();
    let groups: Vec<&str> = id.split('-').collect();
    assert_eq!(groups.iter().map(|group| group.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12]);
    assert!(groups[2].starts_with('4'));
    assert!(id.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
    assert_ne!(id, document_id());
}

#[test]
fn the_clock_never_runs_backwards() {
    let platform = Desktop::new();
    let first = platform.now_ms();
    assert!(platform.now_ms() >= first);
}
