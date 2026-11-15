//! A finding is told by where it fails, so that many inputs for one fault are one finding.
#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use letters_fuzz::report::place;

#[test]
fn a_run_is_told_by_where_it_failed() {
    let (well, aborted) = (ExitStatus::from_raw(0), ExitStatus::from_raw(6));
    let panic = "\nthread 'main' (19376) panicked at fuzz/src/checks.rs:42:9:\nb24 is on no block\nstack backtrace:\n";
    let cases = [
        // (how it ended, what it printed, where it failed)
        (Some(well), "", None),
        (Some(well), panic, None),
        (Some(aborted), panic, Some("fuzz/src/checks.rs:42:9")),
        (Some(aborted), "thread 'main' has overflowed its stack\n", Some("a stack overflow")),
        (Some(aborted), "", Some("no panic")),
        (None, panic, Some("a hang")),
    ];
    for (ended, output, expected) in cases {
        let place = place(ended, output);
        match expected {
            Some(expected) => assert!(place.as_deref().is_some_and(|place| place.starts_with(expected)), "{place:?}"),
            None => assert_eq!(place, None, "a run that ended well on {output:?}"),
        }
    }
}

#[test]
fn inputs_that_fail_at_different_lines_are_told_apart() {
    let aborted = Some(ExitStatus::from_raw(6));
    let at = |line: u32| place(aborted, &format!("thread 'main' (7) panicked at src/a.rs:{line}:1:\nwords {line}\n"));
    assert_eq!(at(1), at(1));
    assert_ne!(at(1), at(2));
}
