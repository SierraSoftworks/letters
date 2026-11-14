use std::ffi::OsString;
use std::path::PathBuf;

use letters::cli::{Command, Mode, Options, parse};

fn args(line: &str) -> Vec<OsString> {
    line.split_whitespace().map(OsString::from).collect()
}

#[test]
fn arguments_select_a_file_and_a_mode() {
    let file = |name: &str| Some(PathBuf::from(name));
    let run = |file, mode, headless| Ok(Command::Run(Options { file, mode, headless }));
    let cases = [
        ("", run(None, Mode::Edit, false)),
        ("notes.let", run(file("notes.let"), Mode::Edit, false)),
        ("--launch-report notes.let", run(file("notes.let"), Mode::LaunchReport, false)),
        ("notes.let --bench-launch --headless", run(file("notes.let"), Mode::BenchLaunch { runs: 20 }, true)),
        ("--bench-launch=5 notes.let", run(file("notes.let"), Mode::BenchLaunch { runs: 5 }, false)),
        ("--typing-report notes.let", run(file("notes.let"), Mode::TypingReport { keys: 200 }, false)),
        ("--typing-report=50 --headless", run(None, Mode::TypingReport { keys: 50 }, true)),
        ("-- --odd-name.let", run(file("--odd-name.let"), Mode::Edit, false)),
        ("--version", Ok(Command::Version)),
        ("notes.let -h", Ok(Command::Help)),
    ];
    for (line, expected) in cases {
        assert_eq!(parse(args(line)), expected, "{line:?}");
    }
}

#[test]
fn bad_arguments_are_refused() {
    for line in [
        "--frobnicate",
        "a.let b.let",
        "--bench-launch=0",
        "--bench-launch=many",
        "--typing-report=-1",
        "--headless a.let",
    ] {
        assert!(parse(args(line)).is_err(), "{line:?}");
    }
}
