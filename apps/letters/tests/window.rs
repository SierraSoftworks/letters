//! The app in a real window, for what only a window shows.
//!
//! Each test starts the app on a display, types into its window as a person would, and reads what the
//! window says from the file `LETTERS_SAYS` names. That takes a display and `xdotool`, which a test run
//! does not have, so `./scripts/check.sh slow` runs these by name each night, on a display in memory.

mod common;

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::thread::sleep;
use std::time::{Duration, Instant};

use common::Scratch;
use letters::app::SAYS;
use letters::journal::{self, Journals};
use letters::{files, strangers};
use letters_format::{Archive, OPLOG, Package, is_journal};
use letters_model::{Block, BlockKind, Document};

/// How long anything a window is waited for may take, on a runner that is slow and busy.
const PATIENCE: Duration = Duration::from_secs(60);

/// One window has the keyboard at a time.
static KEYBOARD: Mutex<()> = Mutex::new(());

/// A machine for one test: a folder of documents, and a home folder for each person who uses it.
struct Desk {
    folder: Scratch,
    _keyboard: MutexGuard<'static, ()>,
}

impl Desk {
    fn new(name: &str) -> Self {
        // A test that failed with the keyboard leaves it as usable as it was.
        let keyboard = KEYBOARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Self { folder: Scratch::new(&format!("window-{name}")), _keyboard: keyboard }
    }

    fn document(&self, name: &str) -> PathBuf {
        let folder = self.folder.0.join("documents");
        fs::create_dir_all(&folder).unwrap();
        folder.join(name)
    }

    /// The home folder of the person called `who`, whose edits carry that name.
    fn home(&self, who: &str) -> PathBuf {
        let home = self.folder.0.join(who);
        let settings = home.join(".letters/settings.toml");
        if !settings.exists() {
            fs::create_dir_all(settings.parent().unwrap()).unwrap();
            fs::write(settings, format!("[writer]\nname = \"{who}\"\n")).unwrap();
        }
        home
    }

    /// What `who` has said they trust.
    fn settings(&self, who: &str) -> String {
        fs::read_to_string(self.home(who).join(".letters/settings.toml")).unwrap_or_default()
    }

    /// Starts the app as `who` on `file`, and waits for its window.
    fn open(&self, who: &str, file: &Path) -> Shown {
        let says = self.folder.0.join(format!("{who}-says.txt"));
        let _ = fs::remove_file(&says);
        let app = Command::new(env!("CARGO_BIN_EXE_letters"))
            .arg(file)
            .env("HOME", self.home(who))
            .env("USER", who)
            .env(SAYS, &says)
            .env("LETTERS_RENDERER", "cpu")
            .env("LETTERS_TELEMETRY", "0")
            .stdout(Stdio::null())
            .spawn()
            .expect("the app starts");
        let mut shown = Shown { app, says, window: String::new() };
        let pid = shown.app.id().to_string();
        shown.window = waited("the app's window", || {
            let found = xdotool(&["search", "--onlyvisible", "--pid", &pid]);
            found.lines().last().map(str::to_owned)
        });
        shown
    }
}

/// The app with a window on the display. It is ended when the test lets go of it.
struct Shown {
    app: Child,
    says: PathBuf,
    window: String,
}

impl Shown {
    /// Everything the window names or holds now, a line for each.
    fn says(&self) -> String {
        fs::read_to_string(&self.says).unwrap_or_default()
    }

    /// Waits for the window to say `what` somewhere.
    fn wait_for(&self, what: &str) {
        let said =
            waited(&format!("the window to say {what:?}"), || Some(self.says()).filter(|says| says.contains(what)));
        assert!(said.contains(what));
    }

    /// Types at the end of the document, as a person does.
    fn write(&self, text: &str) {
        xdotool(&["windowfocus", "--sync", &self.window]);
        xdotool(&["mousemove", "--window", &self.window, "550", "400", "click", "1"]);
        xdotool(&["key", "--window", &self.window, "ctrl+End"]);
        xdotool(&["type", "--window", &self.window, "--delay", "20", text]);
        self.wait_for(text);
    }

    fn running(&mut self) -> bool {
        self.app.try_wait().expect("the app can be asked").is_none()
    }
}

impl Drop for Shown {
    fn drop(&mut self) {
        let _ = self.app.kill();
        let _ = self.app.wait();
    }
}

fn xdotool(arguments: &[&str]) -> String {
    let ran =
        Command::new("xdotool").args(arguments).output().expect("xdotool is installed, and DISPLAY names a display");
    String::from_utf8_lossy(&ran.stdout).into_owned()
}

/// Asks until there is an answer, and fails the test when there is none in time.
fn waited<T>(what: &str, mut asked: impl FnMut() -> Option<T>) -> T {
    let started = Instant::now();
    loop {
        if let Some(answer) = asked() {
            return answer;
        }
        assert!(started.elapsed() < PATIENCE, "waited too long for {what}");
        sleep(Duration::from_millis(100));
    }
}

/// Waits for a save to have put `text` in the document's file, and returns the document as the file has it.
fn saved(file: &Path, text: &str) -> Document {
    let holds = |document: &Document| document.blocks.iter().any(|block| block.plain_text().contains(text));
    waited(&format!("a save with {text:?}"), || {
        let opened = files::open(file, Default::default).ok()?;
        Some(opened.document).filter(|document| file.is_file() && holds(document))
    })
}

const SAFE_MODE: &str = "Safe mode";

#[test]
#[ignore = "needs a display and xdotool: ./scripts/check.sh slow"]
fn a_document_made_by_naming_a_new_file_is_trusted_when_it_is_opened_again() {
    let desk = Desk::new("made-here");
    let file = desk.document("New.let");

    let first = desk.open("Ada", &file);
    first.write("Made by naming a new file.");
    saved(&file, "Made by naming a new file.");
    // Its first save records it as one the person trusts, wherever its file goes.
    waited("the document to be recorded as trusted", || {
        Some(()).filter(|()| desk.settings("Ada").contains("documents"))
    });
    drop(first);

    let again = desk.open("Ada", &file);
    again.wait_for("Made by naming a new file.");
    // A save shows that the file was read, which is when a window says how it treats its document.
    again.write(" And written on.");
    saved(&file, "And written on.");
    assert!(!again.says().contains(SAFE_MODE), "{}", again.says());
}

#[test]
#[ignore = "needs a display and xdotool: ./scripts/check.sh slow"]
fn a_history_that_would_end_the_app_opens_in_safe_mode_and_is_written_anew() {
    // As a stranger writes them: Loro panics on the first two, and cannot write the third out again.
    let harmful: [(&str, &[u8]); 3] = [
        ("panics", include_bytes!("../../../fuzz/corpus/history/a-history-loro-panics-on.bin")),
        ("panics too", include_bytes!("../../../fuzz/corpus/history/a-second-history-loro-panics-on.bin")),
        ("unwritable", include_bytes!("../../../fuzz/corpus/history/a-history-loro-cannot-write-out.bin")),
    ];
    let desk = Desk::new("harmful");
    for (number, (why, history)) in harmful.into_iter().enumerate() {
        let file = desk.document(&format!("Stranger {number}.let"));
        let package = Package {
            id: format!("8f14e45f-ceea-467f-a0e6-5b1d3c2a9f0{number}"),
            created: "2026-11-14T20:00:00Z".into(),
            document: Document {
                blocks: vec![Block::text("p1", BlockKind::Paragraph, "Rates change in January.")],
                ..Document::default()
            },
            entries: [(OPLOG.to_owned(), history.to_vec())].into(),
        };
        let mut out = Cursor::new(Vec::new());
        letters_format::write(&mut out, &package).unwrap();
        fs::write(&file, out.into_inner()).unwrap();

        let mut shown = desk.open("Ada", &file);
        shown.wait_for("Rates change in January.");
        shown.wait_for(SAFE_MODE);
        // The document can be written on, and its save has a history that stands.
        shown.write(" Still here.");
        saved(&file, "Still here.");
        assert!(shown.running(), "{why}");
        let history = files::entry(&file, OPLOG).expect("the save wrote a history");
        assert!(strangers::check_by(Path::new(env!("CARGO_BIN_EXE_letters")), &history).is_ok(), "{why}");
    }
}

#[test]
#[ignore = "needs a display and xdotool: ./scripts/check.sh slow"]
fn a_document_says_who_else_wrote_since_the_person_last_had_it_open() {
    let desk = Desk::new("last-visit");
    let file = desk.document("Rates.let");

    let ada = desk.open("Ada", &file);
    ada.write("Rates for the year.");
    saved(&file, "Rates for the year.");
    drop(ada);

    let grace = desk.open("Grace", &file);
    grace.wait_for("Rates for the year.");
    grace.write(" Grace was here.");
    saved(&file, "Grace was here.");
    assert!(!grace.says().contains("Edited by"), "her own writing is no news to her: {}", grace.says());
    drop(grace);

    let ada = desk.open("Ada", &file);
    ada.wait_for("Edited by Grace");
    // Seen now, it is no news the next time.
    ada.write(" Noted.");
    saved(&file, "Noted.");
    drop(ada);
    let mut ada = desk.open("Ada", &file);
    ada.write(" Again.");
    saved(&file, "Again.");
    assert!(ada.running() && !ada.says().contains("Edited by"), "{}", ada.says());
}

#[test]
#[ignore = "needs a display and xdotool: ./scripts/check.sh slow"]
fn a_journal_the_app_ended_at_is_set_aside_and_its_document_opens_as_saved() {
    let desk = Desk::new("journal-aside");
    let file = desk.document("Rates.let");
    let first = desk.open("Ada", &file);
    first.write("Rates change in January.");
    saved(&file, "Rates change in January.");
    drop(first);

    // A session left an edit that Loro panics on (found by fuzzing a journal, issue #4), and the one
    // after it ended as it brought that back.
    let edit = include_bytes!("../../../fuzz/corpus/history/a-first-edit-that-builds-on-one-before-it.bin");
    let unsaved = desk.home("Ada").join(".letters/unsaved");
    let _ = fs::remove_dir_all(&unsaved);
    let (id, created) = {
        let archive = Archive::open(Cursor::new(fs::read(&file).unwrap())).unwrap();
        (archive.manifest().id.clone(), archive.manifest().created.clone())
    };
    let mut journals = Journals::at(&unsaved);
    let claim = journal::claim(&unsaved, &id).expect("nobody has the document's journal");
    journals.open(claim, &id, &created, Some(&file)).edits(edit.to_vec());
    drop(journals);
    let journal = |name: &String| is_journal(name).then(|| fs::read(unsaved.join(name)).unwrap());
    let [left] = &desk.folder.names("Ada/.letters/unsaved").iter().filter_map(journal).collect::<Vec<_>>()[..] else {
        panic!("the session left one journal")
    };
    let claims = Journals::at(&unsaved).orphans();
    assert!(std::panic::catch_unwind(|| claims[0].replay(|_| panic!("the app ends here"))).is_err());
    drop(claims);

    // The next launch does not read it again. It keeps it, and shows the document as its file has it.
    let mut shown = desk.open("Ada", &file);
    shown.wait_for("Rates change in January.");
    let kept = |name: &String| !is_journal(name) && fs::read(unsaved.join(name)).is_ok_and(|held| &held == left);
    let names = desk.folder.names("Ada/.letters/unsaved");
    assert!(names.iter().any(kept), "the journal is kept under a name of its own: {names:?}");
    assert!(shown.running());
    drop(shown);

    // Nothing is left to stop at, and the document is written on as any other.
    let mut again = desk.open("Ada", &file);
    again.wait_for("Rates change in January.");
    again.write(" Still here.");
    saved(&file, "Still here.");
    assert!(again.running());
    assert!(desk.folder.names("Ada/.letters/unsaved").iter().any(kept), "it is still kept");
}
