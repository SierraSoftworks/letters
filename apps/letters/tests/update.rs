use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

use letters::update::{Resumed, Way, allowed, bundled, newest, resume, way};
use semver::Version;
use update_rs::{RESUME_FLAG, Release, ReleaseVariant};

fn release(version: &str, prerelease: bool, program: bool) -> Release {
    Release {
        id: format!("v{version}"),
        changelog: String::new(),
        version: version.parse().expect("a version"),
        prerelease,
        variant: program.then(|| ReleaseVariant { name: "letters".into(), sha256: None }),
    }
}

fn args(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

#[test]
fn the_release_offered_is_the_newest_this_system_can_run() {
    let releases = [
        release("0.2.0", false, true),
        release("0.3.0", false, true),
        release("0.4.0", false, false),
        release("0.5.0-rc.1", true, true),
    ];
    let cases = [("0.1.0", Some("v0.3.0")), ("0.2.5", Some("v0.3.0")), ("0.3.0", None), ("1.0.0", None)];
    for (from, expected) in cases {
        let from: Version = from.parse().expect("a version");
        let offered = newest(&releases, &from).map(|release| release.id.as_str());
        assert_eq!(offered, expected, "from {from}");
    }
}

#[test]
fn settings_can_turn_the_look_off() {
    let cases = [
        ("", true),
        ("[updates]\ncheck = true", true),
        ("[updates]\ncheck = false", false),
        ("[telemetry]\nenabled = false", true),
        ("not toml [", true),
    ];
    for (settings, expected) in cases {
        assert_eq!(allowed(settings), expected, "{settings:?}");
    }
}

#[test]
fn a_program_inside_a_macos_app_is_told_apart() {
    let cases = [
        ("/Applications/Letters.app/Contents/MacOS/letters", true),
        ("/Users/ada/Apps/Letters 2.app/Contents/MacOS/letters", true),
        ("/usr/local/bin/letters", false),
        ("/home/ada/MacOS/letters", false),
        ("/srv/Contents/MacOS/letters", false),
    ];
    for (program, expected) in cases {
        assert_eq!(bundled(Path::new(program)), expected, "{program}");
    }
}

#[test]
fn letters_replaces_only_a_program_it_may_write_beside() {
    let folder = std::env::temp_dir().join(format!("letters-update-way-{}", std::process::id()));
    let app = folder.join("Letters.app/Contents/MacOS");
    std::fs::create_dir_all(&app).expect("a folder");
    assert_eq!(way(&folder.join("letters")), Way::Itself);
    assert_eq!(way(&app.join("letters")), Way::Download);
    assert_eq!(way(&folder.join("missing/letters")), Way::Download);
    std::fs::remove_dir_all(&folder).expect("removed");
}

#[test]
fn a_usual_start_keeps_its_arguments() {
    for line in [&[][..], &["notes.let"], &["export", "--to", "pdf", "notes.let"]] {
        assert_eq!(resume(args(line)), Resumed::Run(args(line)), "{line:?}");
    }
}

#[test]
fn the_new_version_removes_what_was_fetched_and_opens_what_it_was_given() {
    let fetched = std::env::temp_dir().join(format!("letters-update-fetched-{}", std::process::id()));
    std::fs::write(&fetched, "the new program").expect("written");
    let state = serde_json::json!({ "phase": "cleanup", "update": fetched }).to_string();

    let resumed = resume(args(&[RESUME_FLAG, &state, "notes.let"]));

    assert_eq!(resumed, Resumed::Run(args(&["notes.let"])));
    let until = Instant::now() + Duration::from_secs(10);
    while fetched.exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!fetched.exists(), "what was fetched is still there");
}

#[test]
fn a_state_that_cannot_be_read_starts_letters_as_usual() {
    let resumed = resume(args(&[RESUME_FLAG, "{ not json", "notes.let"]));
    assert_eq!(resumed, Resumed::Run(args(&["notes.let"])));
}
