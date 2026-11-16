//! A finding is told by where it fails, so that many inputs for one fault are one finding.
#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use letters_fuzz::report::{Failure, place, told};

#[test]
fn a_run_is_told_by_where_it_failed() {
    let (well, aborted) = (ExitStatus::from_raw(0), ExitStatus::from_raw(6));
    let panic = "\nthread 'main' (19376) panicked at fuzz/src/checks.rs:42:9:\nb24 is on no block\nstack backtrace:\n";
    let caught = format!("thread 'main' (1) panicked at dep/src/a.rs:1:1:\ncaught\n{panic}");
    let stopped = format!("{panic}thread 'main' (1) panicked at /rustc/0ef1/library/core/src/panicking.rs:233:5:\n");
    let cases = [
        // (how it ended, what it printed, where it failed)
        (Some(well), "", None),
        (Some(well), panic, None),
        (Some(aborted), panic, Some("fuzz/src/checks.rs:42:9")),
        // A panic the target caught is not where the run failed.
        (Some(aborted), caught.as_str(), Some("fuzz/src/checks.rs:42:9")),
        // Nor is the one Rust adds when a second panic stops the first from unwinding.
        (Some(aborted), stopped.as_str(), Some("fuzz/src/checks.rs:42:9")),
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

#[test]
fn a_report_shows_what_letters_did_under_each_panic() {
    let frame = |n: usize, call: &str, file: &str| format!("  {n}: {call}\n             at {file}\n");
    let deep: String =
        (2..80).map(|n| frame(n, "decode", "/root/.cargo/registry/src/index/loro-1.0.0/src/a.rs:1:1")).collect();
    let panic = |words: &str, own: &str| {
        let top = frame(0, "rust_begin_unwind", "/rustc/0ef1/library/std/src/panicking.rs:676:5");
        format!("thread 'main' (7) panicked at dep/src/a.rs:1:1:\n{words}\nstack backtrace:\n{top}{deep}{own}")
    };
    let output = panic("caught", &frame(80, "check", "./crates/letters-model/src/model.rs:376:30"))
        + &panic("not caught", &frame(80, "replay", "./crates/letters-format/src/working.rs:382:28"));
    let failure = Failure { place: "dep/src/a.rs:1:1".into(), input: "none".into(), output, inputs: 1 };
    let told = told("journal_file", "a journal is read or refused", &[failure]);
    for shown in ["caught", "model.rs:376:30", "not caught", "working.rs:382:28", "loro-1.0.0/src/a.rs"] {
        assert!(told.contains(shown), "{shown} is not in:\n{told}");
    }
}
