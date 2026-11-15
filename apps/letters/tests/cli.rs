use std::ffi::OsString;
use std::path::PathBuf;

use letters::cli::{Command, Export, Format, Import, Mode, Options, Output, parse};

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
        ("--baseline", run(None, Mode::Baseline, false)),
        ("--baseline --headless", run(None, Mode::Baseline, true)),
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

#[test]
fn export_and_import_are_subcommands() {
    let export = |file: &str, to, output, force| Ok(Command::Export(Export { file: file.into(), to, output, force }));
    let import_with = |paths: &[&str], pictures: &[&str], recursive, json| {
        let list = |paths: &[&str]| paths.iter().map(PathBuf::from).collect();
        Ok(Command::Import(Import { paths: list(paths), pictures: list(pictures), recursive, json }))
    };
    let import = |paths: &[&str], recursive, json| import_with(paths, &[], recursive, json);
    let to_file = |name: &str| Some(Output::File(name.into()));
    let cases = [
        ("export a.let --to md", export("a.let", Format::Markdown, None, false)),
        ("export --to=html a.let", export("a.let", Format::Html, None, false)),
        ("export a.let --to docx", export("a.let", Format::Docx, None, false)),
        ("export a.let --to html -o out/a.html --force", export("a.let", Format::Html, to_file("out/a.html"), true)),
        ("export a.let --to md --output=-", export("a.let", Format::Markdown, Some(Output::Stdout), false)),
        ("export a.let --to pdf", export("a.let", Format::Pdf, None, false)),
        ("export --to md -o - -- --odd.let", export("--odd.let", Format::Markdown, Some(Output::Stdout), false)),
        ("import notes", import(&["notes"], false, false)),
        ("import notes a.md --recursive --json", import(&["notes", "a.md"], true, true)),
        ("import -r -- --odd.md", import(&["--odd.md"], true, false)),
        (
            "import notes --pictures-from shared --pictures-from=../art",
            import_with(&["notes"], &["shared", "../art"], false, false),
        ),
        ("import notes --help", Ok(Command::Help)),
        // Only the first argument names a subcommand, so files of those names still open.
        ("-- export", Ok(Command::Run(Options { file: Some("export".into()), ..Options::default() }))),
        ("./import", Ok(Command::Run(Options { file: Some("./import".into()), ..Options::default() }))),
    ];
    for (line, expected) in cases {
        assert_eq!(parse(args(line)), expected, "{line:?}");
    }
}

#[test]
fn bad_subcommand_arguments_are_refused() {
    for line in [
        "export",
        "export a.let",
        "export --to md",
        "export a.let --to odt",
        "export a.let --to",
        "export a.let b.let --to md",
        "export a.let --to md --recursive",
        "import",
        "import --json",
        "import notes --to md",
        "import notes --headless",
        "import notes --pictures-from",
    ] {
        assert!(parse(args(line)).is_err(), "{line:?}");
    }
}
