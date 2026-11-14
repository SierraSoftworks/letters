//! `letters export` and `letters import`, run as a user runs them.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use letters_format::{Archive, Package};
use letters_model::{Anchor, Block, BlockKind, Document, Mark, Note, NoteKind, Notes, Run, Style};

/// A scratch folder that is removed when the test ends.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("letters-convert-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Writes a file, making the folders above it.
    fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    /// Runs the app in this folder.
    fn letters(&self, args: &[&str]) -> Ran {
        let output = Command::new(env!("CARGO_BIN_EXE_letters")).args(args).current_dir(&self.0).output().unwrap();
        Ran::from(output)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// How a run of the app ended.
struct Ran {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl From<Output> for Ran {
    fn from(output: Output) -> Self {
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        Self { ok: output.status.success(), stdout: text(&output.stdout), stderr: text(&output.stderr) }
    }
}

fn open(path: &Path) -> Archive<Cursor<Vec<u8>>> {
    Archive::open(Cursor::new(fs::read(path).unwrap())).unwrap()
}

/// A document about rates with a code block, a styled heading and a block this build does not know.
fn rates(folder: &Folder) -> PathBuf {
    let mut chart = Block::new("c", BlockKind::Unknown { type_name: "chart".into() });
    chart.extra.insert("fallback".into(), "Chart: rates by year".into());
    let mut odd = Block::text("o", BlockKind::Paragraph, "");
    odd.body = letters_model::Body::Text(vec![Run::marked("Sparkling", [Mark::Other("sparkle".into())])]);
    let document = Document {
        blocks: vec![
            Block::text("h", BlockKind::Heading { level: 1 }, "Rates"),
            Block::text("p", BlockKind::Paragraph, "Rates change in January."),
            Block::text("r", BlockKind::Code { language: Some("rust".into()) }, "let rate = 4;"),
            chart,
            odd,
        ],
        ..Document::default()
    };
    let mut style = Style::default();
    style.types.get_mut("heading1").unwrap().color = "#aa3300".into();
    let mut package = Package {
        id: "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f00".into(),
        created: "2026-11-14T20:00:00Z".into(),
        document,
        entries: Default::default(),
    };
    package.set_style(&style);
    let mut out = Cursor::new(Vec::new());
    letters_format::write(&mut out, &package).unwrap();
    folder.write("Rates.let", out.into_inner())
}

#[test]
fn export_writes_markdown_and_html_beside_the_document() {
    let folder = Folder::new("export");
    rates(&folder);

    let run = folder.letters(&["export", "Rates.let", "--to", "md"]);
    assert!(run.ok, "{}", run.stderr);
    let markdown = fs::read_to_string(folder.path("Rates.md")).unwrap();
    assert!(markdown.starts_with("# Rates\n\nRates change in January.\n\n```rust\nlet rate = 4;\n```"), "{markdown}");
    assert!(run.stdout.contains("Rates.md"), "{}", run.stdout);

    let run = folder.letters(&["export", "Rates.let", "--to", "html"]);
    assert!(run.ok, "{}", run.stderr);
    let html = fs::read_to_string(folder.path("Rates.html")).unwrap();
    for part in [
        "<title>Rates</title>",
        "<h1>Rates</h1>",
        "<span class=\"tok-keyword\">let</span>",
        // The document's own style, not the built-in one.
        "color: #aa3300;",
    ] {
        assert!(html.contains(part), "the page lacks {part:?}:\n{html}");
    }
}

#[test]
fn export_says_what_it_simplifies() {
    let folder = Folder::new("simplify");
    rates(&folder);
    let run = folder.letters(&["export", "Rates.let", "--to", "md", "--output", "-"]);
    assert!(run.ok, "{}", run.stderr);
    // The document goes to standard output alone, and what it lost is named beside it.
    assert!(run.stdout.starts_with("# Rates\n") && run.stdout.contains("Chart: rates by year"), "{}", run.stdout);
    assert!(run.stderr.contains("1 block") && run.stderr.contains("1 passage"), "{}", run.stderr);
    assert!(!folder.path("Rates.md").exists());
}

#[test]
fn export_never_replaces_a_file_unasked() {
    let folder = Folder::new("replace");
    rates(&folder);
    folder.write("Rates.md", "my own notes");

    let run = folder.letters(&["export", "Rates.let", "--to", "md"]);
    assert!(!run.ok);
    assert!(run.stderr.contains("--force"), "{}", run.stderr);
    assert_eq!(fs::read_to_string(folder.path("Rates.md")).unwrap(), "my own notes");

    let elsewhere = folder.letters(&["export", "Rates.let", "--to", "md", "-o", "copy.md"]);
    assert!(elsewhere.ok && folder.path("copy.md").exists(), "{}", elsewhere.stderr);
    let forced = folder.letters(&["export", "Rates.let", "--to", "md", "--force"]);
    assert!(forced.ok, "{}", forced.stderr);
    assert!(fs::read_to_string(folder.path("Rates.md")).unwrap().starts_with("# Rates"));
}

#[test]
fn export_fails_on_what_is_not_a_document() {
    let folder = Folder::new("not-a-document");
    folder.write("notes.let", "plain text");
    for file in ["notes.let", "missing.let"] {
        let run = folder.letters(&["export", file, "--to", "html"]);
        assert!(!run.ok, "{file}");
        assert!(run.stderr.contains(file), "{}", run.stderr);
        assert!(!folder.path(file).with_extension("html").exists(), "{file}");
    }
}

/// A folder of notes that link to each other, with files an import must pass over.
fn notes(folder: &Folder) {
    folder.write(
        "notes/index.md",
        "# Index\n\n[Plan](sub/plan.md#goals), [[My note]] and [web](https://example.com/a.md).\n\n$$x^2$$\n",
    );
    folder.write("notes/My note.md", "Back to [the index](index.md) or [a file](todo.txt).\n");
    folder.write("notes/sub/plan.markdown", "# Plan\n\nUp to [the index](../index.md).\n");
    folder.write("notes/sub/data.csv", "a,b\n");
    folder.write("notes/.trash/old.md", "# Old\n");
}

/// The addresses of a document's links, in order.
fn links(path: &Path) -> Vec<String> {
    let document = open(path).document().unwrap();
    document.walk().flat_map(|block| block.body.runs()).filter_map(|run| run.link.clone()).collect()
}

#[test]
fn import_makes_a_document_beside_each_markdown_file() {
    let folder = Folder::new("import");
    notes(&folder);
    let original = fs::read(folder.path("notes/index.md")).unwrap();

    let run = folder.letters(&["import", "notes", "--recursive"]);
    assert!(run.ok, "{}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 3, "one line per file: {}", run.stdout);
    for (line, source) in lines.iter().zip(["My note.md", "index.md", "plan.markdown"]) {
        assert!(line.contains(source) && line.contains(".let"), "{line}");
    }
    // Only what the import simplified is counted, on the file it happened in.
    assert!(lines[1].contains("1 note") && !lines[0].contains("note)"), "{}", run.stdout);

    // Originals are untouched, and what is hidden or not Markdown is passed over.
    assert_eq!(fs::read(folder.path("notes/index.md")).unwrap(), original);
    assert!(!folder.path("notes/.trash/old.let").exists() && !folder.path("notes/sub/data.let").exists());

    // The document holds the text, a style, and the import's notes on the blocks they are about.
    let mut index = open(&folder.path("notes/index.let"));
    let document = index.document().unwrap();
    assert_eq!(document.blocks[0].plain_text(), "Index");
    assert!(index.names().any(|name| name == letters_format::STYLE));
    let notes = index.notes().unwrap().notes;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].kind, NoteKind::Import);
    assert!(document.block(&notes[0].anchor.block).is_some());

    // Links between the imported files follow them to the documents. Other links stay as written.
    assert_eq!(
        links(&folder.path("notes/index.let")),
        ["sub/plan.let#goals", "My note.let", "https://example.com/a.md"]
    );
    assert_eq!(links(&folder.path("notes/My note.let")), ["index.let", "todo.txt"]);
    assert_eq!(links(&folder.path("notes/sub/plan.let")), ["../index.let"]);
}

#[test]
fn import_stays_in_a_folder_unless_told_to_go_deeper() {
    let folder = Folder::new("shallow");
    notes(&folder);
    let run = folder.letters(&["import", "notes"]);
    assert!(run.ok, "{}", run.stderr);
    assert_eq!(run.stdout.lines().count(), 2, "{}", run.stdout);
    assert!(folder.path("notes/index.let").exists() && !folder.path("notes/sub/plan.let").exists());
}

#[test]
fn import_reports_each_file_as_json() {
    let folder = Folder::new("json");
    notes(&folder);
    folder.write("notes/broken.md", [0xff, 0xfe, 0xfd]);
    folder.write("notes/sub/plan.let", "already here");

    let run = folder.letters(&["import", "--json", "-r", "notes", "missing.md"]);
    // A failure fails the run, without stopping the files after it.
    assert!(!run.ok);
    let lines: Vec<serde_json::Value> = run.stdout.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    let status = |name: &str| {
        let line = lines.iter().find(|line| line["source"].as_str().is_some_and(|source| source.ends_with(name)));
        line.unwrap_or_else(|| panic!("no line for {name}: {}", run.stdout))
    };
    assert_eq!(lines.len(), 5, "{}", run.stdout);
    assert_eq!(status("index.md")["status"], "imported");
    assert_eq!(status("index.md")["notes"], 1);
    assert!(status("index.md")["output"].as_str().unwrap().ends_with("index.let"));
    assert_eq!(status("My note.md")["status"], "imported");
    assert_eq!(status("broken.md")["status"], "failed");
    assert_eq!(status("missing.md")["status"], "failed");
    assert!(status("broken.md")["error"].is_string());
    // A document that is already there is never replaced.
    assert_eq!(status("plan.markdown")["status"], "skipped");
    assert_eq!(fs::read_to_string(folder.path("notes/sub/plan.let")).unwrap(), "already here");
    assert!(!folder.path("notes/broken.let").exists());
}

#[test]
fn an_imported_document_exports_back_to_its_markdown() {
    let folder = Folder::new("round-trip");
    let source = "# Title\n\nSome **bold** text.\n\n- one\n- two\n";
    folder.write("note.md", source);
    folder.write("empty.md", "");
    assert!(folder.letters(&["import", "note.md", "empty.md"]).ok);
    let run = folder.letters(&["export", "note.let", "--to", "md", "-o", "-"]);
    assert_eq!(run.stdout, source);
    // An empty file still makes a document that opens.
    assert_eq!(open(&folder.path("empty.let")).document().unwrap().blocks.len(), 1);
}

/// Adds a comment by Ada on the first word of the document's second block.
fn comment(path: &Path) {
    let mut package = open(path).package().unwrap();
    let note = Note {
        id: "n1".into(),
        kind: NoteKind::Comment,
        anchor: Anchor::range(package.document.blocks[1].id.clone(), 0..5),
        author: "Ada".into(),
        created: "2026-11-14T20:00:00Z".into(),
        resolved: false,
        content: vec![Run::plain("Which rates?")],
        replies: Vec::new(),
        extra: Default::default(),
    };
    package.set_notes(&Notes { notes: vec![note], ..Notes::default() });
    let mut out = Cursor::new(Vec::new());
    letters_format::write(&mut out, &package).unwrap();
    fs::write(path, out.into_inner()).unwrap();
}

#[test]
fn a_document_goes_to_word_and_back() {
    let folder = Folder::new("word");
    comment(&rates(&folder));
    folder.write("word/notes.md", "# Notes\n");
    // A document is made once for a name, whichever file of that name comes first.
    folder.write("word/Rates.md", "# Rates in Markdown\n");

    let run = folder.letters(&["export", "Rates.let", "--to", "docx", "-o", "word/Rates.docx"]);
    assert!(run.ok, "{}", run.stderr);
    assert!(run.stdout.contains("Rates.docx"), "{}", run.stdout);
    assert!(run.stderr.contains("1 block") && run.stderr.contains("1 passage"), "{}", run.stderr);
    let again = folder.letters(&["export", "Rates.let", "--to", "docx", "-o", "word/Rates.docx"]);
    assert!(!again.ok && again.stderr.contains("--force"), "{}", again.stderr);

    let run = folder.letters(&["import", "word"]);
    assert!(run.ok, "{}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 3, "one line per file: {}", run.stdout);
    // People's comments are not something the import simplified.
    assert!(
        lines[0].contains("Rates.docx") && lines[0].contains("Rates.let") && !lines[0].contains("note)"),
        "{}",
        lines[0]
    );
    assert!(folder.path("word/notes.let").exists());

    let mut back = open(&folder.path("word/Rates.let"));
    let document = back.document().unwrap();
    assert_eq!(
        (&document.blocks[0].kind, document.blocks[0].plain_text().as_str()),
        (&BlockKind::Heading { level: 1 }, "Rates")
    );
    assert_eq!(document.blocks[2].kind, BlockKind::Code { language: None });
    // The document's style went into Word's styles and came back out of them.
    assert_eq!(back.style().unwrap().types["heading1"].color, "#aa3300");
    let notes = back.notes().unwrap().notes;
    let [note] = notes.as_slice() else { panic!("one comment, got {notes:?}") };
    assert_eq!((&note.kind, note.author.as_str()), (&NoteKind::Comment, "Ada"));
    let on = document.block(&note.anchor.block).unwrap().plain_text();
    assert_eq!(&on[note.anchor.range.clone().unwrap()], "Rates");
}

#[test]
fn import_refuses_what_only_looks_like_word() {
    let folder = Folder::new("not-word");
    folder.write("old.doc", [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
    folder.write("torn.docx", b"PK\x03\x04 and then nothing");
    let run = folder.letters(&["import", "--json", "old.doc", "torn.docx"]);
    assert!(!run.ok);
    let lines: Vec<serde_json::Value> = run.stdout.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(lines.len(), 2, "{}", run.stdout);
    for line in &lines {
        assert_eq!(line["status"], "failed", "{line}");
    }
    // An older file says how to get one that can be read.
    assert!(lines[0]["error"].as_str().unwrap().contains(".docx"), "{}", lines[0]);
    assert!(!folder.path("old.let").exists() && !folder.path("torn.let").exists());
}
