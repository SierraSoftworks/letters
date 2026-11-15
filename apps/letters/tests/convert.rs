//! `letters export` and `letters import`, run as a user runs them.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image as Drawn, ImageDrawProps, InterpreterCache,
    InterpreterSettings, SoftMask, interpret_page,
};
use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, String as Bytes};
use hayro::hayro_syntax::page::Page;
use hayro::kurbo::{Affine, BezPath};
use letters::{fallback, trust};
use letters_format::{Archive, OPLOG, Package};
use letters_layout::Uncovered;
use letters_model::{
    Anchor, Block, BlockKind, Document, Mark, Model, Note, NoteKind, Notes, Op, Pos, ReplicaId, Run, Style,
};
use letters_trust::Mode;

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
        // The scratch folder is the home too, so that nothing of the person running the tests is read or
        // written: their style, or what they trust.
        let mut command = Command::new(env!("CARGO_BIN_EXE_letters"));
        command.args(args).current_dir(&self.0).env("HOME", &self.0).env("USERPROFILE", &self.0);
        let output = command.output().unwrap();
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
        "# Index\n\n[Plan](sub/plan.md#goals), [[My note]] and [web](https://example.com/a.md).\n\n<div>raw</div>\n",
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

/// How a document imported from `from` is treated when it is kept at `kept`, by a person who trusts the
/// folder `trusted`. All three are inside the scratch folder.
fn imported(name: &str, from: &str, kept: &str, trusted: &str) -> (Mode, Mode) {
    let folder = Folder::new(name);
    let source = folder.write(&format!("{from}/Trip.md"), "# Trip\n");
    fs::create_dir_all(folder.path(trusted)).unwrap();
    let mut trust = trust::store(&folder.0);
    trust.trust_folder(fs::canonicalize(folder.path(trusted)).unwrap().to_str().unwrap()).unwrap();

    let run = folder.letters(&["import", source.to_str().unwrap()]);
    assert!(run.ok, "{}", run.stderr);
    let made = source.with_extension("let");
    let id = open(&made).manifest().id.clone();
    let kept = folder.path(&format!("{kept}/Trip.let"));
    fs::create_dir_all(kept.parent().unwrap()).unwrap();
    fs::rename(&made, &kept).unwrap();

    trust.load().unwrap();
    let before = trust::mode(trust.trust(), &id, Some(&kept));
    trust.trust_document(&id).unwrap();
    (before, trust::mode(trust.trust(), &id, Some(&kept)))
}

#[test]
fn the_folder_a_file_is_imported_from_vouches_for_the_document() {
    // (name, imported from, kept in, trusted folder, how it opens)
    let cases = [
        ("from-trusted", "notes", "notes", "notes", Mode::Trusted),
        ("from-trusted-moved", "notes", "elsewhere", "notes", Mode::Trusted),
        ("from-below-trusted", "notes/2026", "notes/2026", "notes", Mode::Trusted),
        ("from-outside", "downloads", "downloads", "notes", Mode::Safe),
        ("from-outside-moved-in", "downloads", "notes", "notes", Mode::Safe),
    ];
    for (name, from, kept, trusted, opens) in cases {
        let (before, after) = imported(name, from, kept, trusted);
        assert_eq!(before, opens, "{name}");
        assert_eq!(after, Mode::Trusted, "{name}: the person trusts the document itself");
    }
}

#[test]
fn nothing_is_imported_when_the_import_cannot_be_recorded() {
    let folder = Folder::new("unrecorded");
    folder.write("Trip.md", "# Trip\n");
    // The settings cannot be written: a file lies where their folder goes.
    folder.write(".letters", "in the way");
    let run = folder.letters(&["import", "Trip.md"]);
    assert!(!run.ok, "{}", run.stdout);
    assert!(!folder.path("Trip.let").exists());
}

/// Collects the text of a PDF page in the order it is drawn.
#[derive(Default)]
struct Reader(String);

impl<'a> Device<'a> for Reader {
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {
        for glyph in run.glyphs() {
            match glyph.as_unicode() {
                Some(BfString::Char(character)) => self.0.push(character),
                Some(BfString::String(text)) => self.0.push_str(&text),
                None => {}
            }
        }
    }

    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Drawn<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// Text without its white space. A PDF holds no line ends, so this is what a page's text is compared as.
fn squeezed(text: &str) -> String {
    text.split_whitespace().collect()
}

/// The text a reader finds on a page when it is searched or copied, [`squeezed`].
fn text_on(page: &Page<'_>) -> String {
    let cache = InterpreterCache::default();
    let bounds = hayro::kurbo::Rect::new(0.0, 0.0, 1e4, 1e4);
    let mut context = Context::new(Affine::IDENTITY, bounds, &cache, page.xref(), InterpreterSettings::default());
    let mut reader = Reader::default();
    interpret_page(page, &mut context, &mut reader);
    squeezed(&reader.0)
}

fn pdf_string(dict: &Dict<'_>, key: &str) -> Option<String> {
    let bytes = dict.get::<Bytes<'_>>(key)?;
    Some(String::from_utf8_lossy(bytes.as_bytes()).into_owned())
}

/// The addresses of the links on a page.
fn links_on(page: &Page<'_>) -> Vec<String> {
    let links = page.raw().get::<Array<'_>>("Annots").map(|links| links.iter::<Dict<'_>>().collect::<Vec<_>>());
    links.unwrap_or_default().iter().filter_map(|link| pdf_string(&link.get::<Dict<'_>>("A")?, "URI")).collect()
}

/// The outline's titles with the page each leads to, indented by a space for each level below the first.
fn outline_of(pdf: &Pdf) -> Vec<(String, usize)> {
    fn walk(pdf: &Pdf, parent: &Dict<'_>, depth: usize, out: &mut Vec<(String, usize)>) {
        let mut next = parent.get::<Dict<'_>>("First");
        while let Some(entry) = next {
            let target = entry.get::<Array<'_>>("Dest").and_then(|place| place.flex_iter().next::<Dict<'_>>());
            let page =
                target.and_then(|page| pdf.pages().iter().position(|other| other.raw().obj_id() == page.obj_id()));
            let title = pdf_string(&entry, "Title").unwrap_or_default();
            out.push((format!("{}{title}", " ".repeat(depth)), page.unwrap_or(usize::MAX)));
            walk(pdf, &entry, depth + 1, out);
            next = entry.get::<Dict<'_>>("Next");
        }
    }
    let mut out = Vec::new();
    let root = pdf.xref().get::<Dict<'_>>(pdf.xref().root_id()).expect("a PDF has a catalogue");
    if let Some(outline) = root.get::<Dict<'_>>("Outlines") {
        walk(pdf, &outline, 0, &mut out);
    }
    out
}

#[test]
fn export_writes_a_pdf_a_reader_can_search_and_find_its_way_in() {
    let folder = Folder::new("pdf");
    let filler = "Every household on the scheme should check the notice that arrives in December. ".repeat(6);
    let markdown = format!(
        "# Rates\n\nRates change in January, as [the notice](https://example.com/rates?year=2027) and \
         [a trap](javascript:alert(1)) say.\n\n```rust\nlet rate = 4;\n```\n\n{}## Appeals\n\nAppeals close in March.\n",
        format!("{filler}\n\n").repeat(12)
    );
    folder.write("Rates.md", markdown);
    assert!(folder.letters(&["import", "Rates.md"]).ok);

    let run = folder.letters(&["export", "Rates.let", "--to", "pdf"]);
    assert!(run.ok, "{}", run.stderr);
    let pdf = Pdf::new(fs::read(folder.path("Rates.pdf")).unwrap()).expect("a reader can open the PDF");
    let pages = pdf.pages();
    let text: Vec<String> = pages.iter().map(text_on).collect();

    // The document runs over more than one sheet of the style's paper, which is A4.
    assert!(pages.len() > 1, "{} pages", pages.len());
    for page in pages.iter() {
        let (width, height) = page.render_dimensions();
        assert_eq!((width.round(), height.round()), (595.0, 842.0));
    }

    // Text is text, in reading order from the first page to the last.
    assert!(
        text[0].starts_with(&squeezed("Rates Rates change in January, as the notice and a trap say.")),
        "{}",
        text[0]
    );
    assert!(text[0].contains(&squeezed("let rate = 4;")), "{}", text[0]);
    assert!(text.last().unwrap().contains(&squeezed("Appeals close in March.")), "{:?}", text.last());
    assert_eq!(text.concat().matches(&squeezed("check the notice that arrives in December.")).count(), 72);

    // Links open what they name, unless that would run script.
    assert_eq!(links_on(&pages[0]), ["https://example.com/rates?year=2027"]);

    // The headings are the outline, each leading to the page it is on.
    let appeals = text.iter().position(|page| page.contains("Appealsclose")).unwrap();
    assert!(appeals > 0, "the last heading is on a later page");
    assert_eq!(outline_of(&pdf), [("Rates".to_owned(), 0), (" Appeals".to_owned(), appeals)]);
    let title = pdf.metadata().title.as_deref().map(|title| String::from_utf8_lossy(title).into_owned());
    assert_eq!(title.as_deref(), Some("Rates"));
    assert!(pdf.metadata().creation_date.is_some(), "the PDF says when it was written");
}

#[test]
fn a_pdf_sets_what_the_bundled_fonts_lack_in_a_face_of_the_system() {
    let folder = Folder::new("pdf-fallback");
    folder.write("Trip.md", "# Trip\n\nBy air to 東京, and back.\n");
    assert!(folder.letters(&["import", "Trip.md"]).ok);
    let run = folder.letters(&["export", "Trip.let", "--to", "pdf"]);
    assert!(run.ok, "{}", run.stderr);
    let pdf = Pdf::new(fs::read(folder.path("Trip.pdf")).unwrap()).expect("a reader can open the PDF");
    let text = text_on(&pdf.pages()[0]);
    assert!(text.contains("Byairto") && text.contains(",andback."), "what the fonts cover is set as ever: {text}");

    // Which faces a machine holds varies. Where it has one for these characters, they are text in the PDF.
    let lacking = ['東', '京'].map(|ch| Uncovered { ch, emoji: false, lang: None });
    if fallback::system_fonts(&lacking).faces.is_empty() {
        eprintln!("no face of this system draws Han, so only the export itself was checked");
    } else {
        assert!(text.contains("東京"), "{text}");
    }
}

#[test]
fn a_pdf_names_those_who_wrote_the_document() {
    let folder = Folder::new("authors");
    let blocks = vec![Block::text("p", BlockKind::Paragraph, "Rates")];
    let mut model = Model::new(Document { blocks, ..Document::default() }, ReplicaId::new("a device"));
    model.set_author("Ada Lovelace");
    model.apply([Op::InsertText { at: Pos::new("p", 5), text: " change in January.".into() }]).unwrap();
    let mut package = Package {
        id: "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f01".into(),
        created: "2026-11-14T20:00:00Z".into(),
        document: model.doc().clone(),
        entries: Default::default(),
    };
    package.entries.insert(OPLOG.into(), model.save());
    let mut out = Cursor::new(Vec::new());
    letters_format::write(&mut out, &package).unwrap();
    folder.write("Rates.let", out.into_inner());

    let run = folder.letters(&["export", "Rates.let", "--to", "pdf"]);
    assert!(run.ok, "{}", run.stderr);
    let pdf = Pdf::new(fs::read(folder.path("Rates.pdf")).unwrap()).expect("a reader can open the PDF");
    let author = pdf.metadata().author.as_deref().map(String::from_utf8_lossy).unwrap_or_default().into_owned();
    assert_eq!(author, "Ada Lovelace");
}

#[test]
fn a_clean_copy_is_the_document_without_its_history_and_notes() {
    let folder = Folder::new("clean");
    let shown = BlockKind::Figure {
        asset: Some("assets/shown.png".into()),
        src: None,
        alt: "A chart".into(),
        width: 40.0,
        height: 20.0,
        numbered: false,
    };
    let card = BlockKind::LinkPreview {
        url: "https://example.com/rates".into(),
        title: Some("Rates".into()),
        description: None,
        image: Some("assets/card.png".into()),
    };
    let blocks = vec![Block::text("p", BlockKind::Paragraph, "Rates"), Block::new("f", shown), Block::new("c", card)];
    let mut model = Model::new(Document { blocks, ..Document::default() }, ReplicaId::new("a device"));
    model.set_author("Ada Lovelace");
    model.apply([Op::InsertText { at: Pos::new("p", 5), text: " change in January.".into() }]).unwrap();
    let mut package = Package {
        id: "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f02".into(),
        created: "2026-11-14T20:00:00Z".into(),
        document: model.doc().clone(),
        entries: Default::default(),
    };
    package.entries.insert(OPLOG.into(), model.save());
    // A picture the page shows in a figure, one a link preview shows, and one only an earlier version did.
    package.entries.insert("assets/shown.png".into(), PICTURE.to_vec());
    package.entries.insert("assets/card.png".into(), PICTURE.to_vec());
    package.entries.insert("assets/removed.png".into(), PICTURE.to_vec());
    package.set_style(&Style::default());
    let mut out = Cursor::new(Vec::new());
    letters_format::write(&mut out, &package).unwrap();
    let original = folder.write("Rates.let", out.into_inner());
    comment(&original);
    let before = fs::read(&original).unwrap();

    let run = folder.letters(&["export", "Rates.let", "--to", "clean"]);
    assert!(run.ok, "{}", run.stderr);
    assert!(
        run.stderr.contains("history") && run.stderr.contains("1 comment"),
        "it says what it drops: {}",
        run.stderr
    );
    assert_eq!(fs::read(&original).unwrap(), before, "the original is left as it was");

    let mut copy = open(&folder.path("Rates clean.let"));
    let clean = copy.package().unwrap();
    assert_eq!(clean.document, package.document, "the document is the same");
    assert_ne!(clean.id, package.id, "under an id of its own");
    let names: Vec<&str> = clean.entries.keys().map(String::as_str).collect();
    assert!(!names.iter().any(|name| name.starts_with(letters_format::HISTORY)), "no history: {names:?}");
    assert!(clean.notes().unwrap().notes.is_empty(), "no notes");
    assert_eq!(pictures_held(&names), ["assets/card.png", "assets/shown.png"], "only the pictures the page shows");
    assert!(names.contains(&letters_format::STYLE), "and its style");

    // The copy opens and exports like any document.
    let again = folder.letters(&["export", "Rates clean.let", "--to", "md", "--output", "-"]);
    assert!(again.ok && again.stdout.contains("Rates change in January."), "{}{}", again.stdout, again.stderr);
}

/// The entries that are pictures.
fn pictures_held<'a>(names: &[&'a str]) -> Vec<&'a str> {
    names.iter().copied().filter(|name| name.starts_with(letters_format::ASSETS)).collect()
}

/// A picture of 40 by 20 pixels.
const PICTURE: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x28\x00\x00\x00\x14\x01\x00\x00\x00\x00\xd7\x3d\x42\x16\x00\x00\x00\x0c\x49\x44\x41\x54\x78\xda\x63\x60\x18\x18\x00\x00\x00\x78\x00\x01\x35\x33\x6e\xa4\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82";

/// The entries of the pictures a document shows, in order.
fn pictures(path: &Path) -> Vec<String> {
    let asset = |block: &Block| match &block.kind {
        BlockKind::Figure { asset, .. } => asset.clone(),
        _ => None,
    };
    open(path).document().unwrap().walk().filter_map(asset).collect()
}

#[test]
fn pictures_follow_markdown_in_and_out() {
    let folder = Folder::new("pictures");
    folder.write("notes/img/cat.png", PICTURE);
    // A picture outside what the import was asked for, which a file from elsewhere might name to carry it off.
    folder.write("private.png", PICTURE);
    folder.write("notes/Cat.md", "![A cat](img/cat.png)\n\n![Not ours](../private.png)\n\n![Gone](gone.png)\n");

    assert!(folder.letters(&["import", "notes"]).ok);
    let document = folder.path("notes/Cat.let");
    let held = pictures(&document);
    assert_eq!(held.len(), 1, "only the picture in the folder comes along: {held:?}");
    assert_eq!(open(&document).read(&held[0]).unwrap().as_deref(), Some(PICTURE));

    // On the way out the picture is written where the Markdown names it.
    folder.write("out/keep", "");
    let run = folder.letters(&["export", "notes/Cat.let", "--to", "md", "--output", "out/Cat.md"]);
    assert!(run.ok, "{}", run.stderr);
    let markdown = fs::read_to_string(folder.path("out/Cat.md")).unwrap();
    assert!(markdown.contains(&format!("![A cat]({})", held[0])), "{markdown}");
    assert_eq!(fs::read(folder.path("out").join(&held[0])).unwrap(), PICTURE);
    // A second export finds the picture there and leaves it.
    let again = folder.letters(&["export", "notes/Cat.let", "--to", "md", "--output", "out/Again.md"]);
    assert!(again.ok, "{}", again.stderr);
}

#[test]
fn import_reads_pictures_from_the_folders_it_is_given() {
    // What is added to the command, and how many of the two pictures the document then holds.
    let cases: [(&[&str], usize); 3] =
        [(&[], 1), (&["--pictures-from", "shared"], 2), (&["--pictures-from=shared", "--pictures-from", "none"], 2)];
    for (index, (added, expected)) in cases.into_iter().enumerate() {
        let folder = Folder::new(&format!("pictures-from-{index}"));
        folder.write("notes/cat.png", PICTURE);
        // The same picture with a byte after its end, so that the two are kept apart.
        folder.write("shared/owl.png", [PICTURE, &[0]].concat());
        folder.write("notes/Pets.md", "![A cat](cat.png)\n\n![An owl](../shared/owl.png)\n");

        let run = folder.letters(&[&["import", "notes/Pets.md"], added].concat());
        assert!(run.ok, "{added:?}: {}", run.stderr);
        assert_eq!(pictures(&folder.path("notes/Pets.let")).len(), expected, "{added:?}");
    }
}

#[test]
fn an_export_writes_no_picture_outside_its_folder() {
    let folder = Folder::new("climb");
    let asset = Some("../stolen.png".to_owned());
    let kind = BlockKind::Figure { asset, src: None, alt: "A cat".into(), width: 30.0, height: 15.0, numbered: false };
    let mut package = Package {
        id: "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f02".into(),
        created: "2026-11-14T20:00:00Z".into(),
        document: Document { blocks: vec![Block::text("f", kind, "")], ..Document::default() },
        entries: Default::default(),
    };
    package.entries.insert("../stolen.png".into(), PICTURE.to_vec());
    let mut out = Cursor::new(Vec::new());
    // A file that cannot even be written with such an entry cannot ask for it either.
    if letters_format::write(&mut out, &package).is_ok() {
        folder.write("docs/Cat.let", out.into_inner());
        let run = folder.letters(&["export", "docs/Cat.let", "--to", "md"]);
        assert!(run.ok, "{}", run.stderr);
    }
    assert!(!folder.path("stolen.png").exists());
}
