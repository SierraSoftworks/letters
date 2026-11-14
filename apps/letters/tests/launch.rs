//! The app itself, run without a window.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use letters::measure::FIRST_FRAME;
use letters_format::Package;
use letters_model::{Block, BlockKind, Document};

fn letters(args: &[&str], file: Option<&Path>) -> Output {
    Command::new(env!("CARGO_BIN_EXE_letters")).args(args).args(file).output().unwrap()
}

/// A document of a few pages, in a scratch folder of its own.
fn document(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("letters-launch-{}-{name}", std::process::id()));
    fs::create_dir_all(&folder).unwrap();
    let block =
        |n: usize| Block::text(format!("p{n}").as_str(), BlockKind::Paragraph, "Rates change in January. ".repeat(8));
    let package = Package {
        id: "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f00".into(),
        created: "2026-11-14T20:00:00Z".into(),
        document: Document { blocks: (0..100).map(block).collect(), ..Document::default() },
        entries: Default::default(),
    };
    let mut out = Cursor::new(Vec::new());
    letters_format::write(&mut out, &package).unwrap();
    let path = folder.join("Rates.let");
    fs::write(&path, out.into_inner()).unwrap();
    path
}

#[test]
fn a_launch_reports_its_first_frame_and_what_led_to_it() {
    let path = document("report");
    let output = letters(&["--launch-report", "--headless"], Some(&path));
    let report = String::from_utf8_lossy(&output.stdout);

    assert!(report.starts_with(FIRST_FRAME), "the first frame is announced before anything else:\n{report}");
    assert!(report.contains("document read") && report.contains("laid out"), "{report}");
    assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn a_bench_sets_the_app_beside_an_empty_window() {
    let path = document("bench");
    let original = fs::read(&path).unwrap();
    let output = letters(&["--bench-launch=2", "--headless"], Some(&path));
    let report = String::from_utf8_lossy(&output.stdout);

    // Whether a debug build makes the budget is not this test's business, so the exit code is not either.
    assert!(report.contains("empty window") && report.contains("Letters adds"), "{report}");
    assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn a_typing_report_times_each_keystroke_and_leaves_the_file_alone() {
    let path = document("typing");
    let original = fs::read(&path).unwrap();
    let output = letters(&["--typing-report=5", "--headless"], Some(&path));
    let report = String::from_utf8_lossy(&output.stdout);

    assert!(report.contains("5 keystrokes"), "{report}{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn what_cannot_run_says_why_and_fails() {
    let folder = document("refused");
    let missing = folder.with_file_name("no such folder").join("Rates.let");
    let not_a_document = folder.with_file_name("plain.let");
    fs::write(&not_a_document, "just text").unwrap();

    let cases: [(&str, &[&str], Option<&Path>); 4] = [
        ("a file in a folder that is not there", &["--launch-report", "--headless"], Some(&missing)),
        ("a file that is not a document", &["--launch-report", "--headless"], Some(&not_a_document)),
        ("a flag nobody knows", &["--no-such-flag"], None),
        ("headless with nothing to measure", &["--headless"], None),
    ];
    for (case, args, file) in cases {
        let output = letters(args, file);
        assert!(!output.status.success(), "{case}");
        assert!(output.stdout.is_empty(), "{case}");
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("letters: "), "{case}");
    }
}
