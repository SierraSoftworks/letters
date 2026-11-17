//! The app as Windows has it: the program's file, and its window seen from outside.
//!
//! A Windows machine is rarely at hand, so what can be asked of the system is asked here, on the runner
//! that tests Windows each night. The window is found by its process and looked at with
//! `letters_frame::probe`, and what it says is read from the file `LETTERS_SAYS` names.
#![cfg(windows)]

mod common;

use std::fs;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use common::Scratch;
use letters::app::SAYS;
use letters_frame::probe::{self, Probe};

/// How long anything a window is waited for may take, on a runner that is slow and busy.
const PATIENCE: Duration = Duration::from_secs(60);

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

/// The app is ended when the test lets go of it, however the test ends.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn the_program_opens_no_console_and_says_what_it_is() {
    let program = fs::read(env!("CARGO_BIN_EXE_letters")).unwrap();
    // The header says which subsystem a program asks for: 2 is a window, 3 a console.
    let word = |at: usize| u16::from_le_bytes([program[at], program[at + 1]]);
    let header = u32::from_le_bytes(program[0x3c..0x40].try_into().unwrap()) as usize;
    assert_eq!(word(header + 24 + 68), 2, "the program asks for a console");

    // The version's fields are kept as UTF-16, and the manifest as it was written.
    let holds = |bytes: &[u8]| program.windows(bytes.len()).any(|window| window == bytes);
    let wide = |text: &str| text.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
    for field in ["ProductName", "FileDescription", "Sierra Softworks", env!("CARGO_PKG_VERSION")] {
        assert!(holds(&wide(field)), "the program's file does not say {field:?}");
    }
    assert!(holds(b"PerMonitorV2"), "the program has no manifest");
}

#[test]
fn the_bar_is_the_title_bar_and_the_system_keeps_the_frame() {
    let desk = Scratch::new("windows-frame");
    let (home, says) = (desk.0.join("Ada"), desk.0.join("says.txt"));
    fs::create_dir_all(home.join(".letters")).unwrap();
    fs::write(home.join(".letters/settings.toml"), "[writer]\nname = \"Ada\"\n").unwrap();
    let app = Command::new(env!("CARGO_BIN_EXE_letters"))
        .arg(desk.0.join("New.let"))
        .env("USERPROFILE", &home)
        .env("HOME", &home)
        .env(SAYS, &says)
        .env("LETTERS_RENDERER", "cpu")
        .env("LETTERS_TELEMETRY", "0")
        .env("LETTERS_UPDATES", "0")
        .stdout(Stdio::null())
        .spawn()
        .expect("the app starts");
    let mut app = Running(app);
    let said = || fs::read_to_string(&says).unwrap_or_default();
    // A line of its own, so that a button is not taken for a word in the document or in another's name.
    let says_now = |what: &str| said().lines().any(|line| line.trim() == what);
    let say = |what: &str| waited(&format!("the window to say {what:?}"), || says_now(what).then_some(()));

    // The bar carries the window's buttons, which it does only where it is the title bar.
    for button in ["Minimise", "Maximise", "Close window"] {
        say(button);
    }
    let window: Probe = waited("the app's window", || probe::window_of(app.0.id()));
    let scale = window.scale();
    let unit = |units: f64| (units * scale).round() as i32;

    // No title bar of the system's stands over it: the app draws from the window's top edge down. The
    // system still has the window as one it resizes, snaps and maximises.
    let (outer, inner) = (window.outer(), window.inner());
    assert!(inner.top - outer.top <= unit(2.0), "a title bar stands over the app: {outer:?}, {inner:?}");
    assert_eq!((inner.left, inner.right), (outer.left, outer.right), "the system frames the sides");
    assert!(window.framed(), "the system cannot resize or snap the window");

    // Maximised, it fills what the taskbar leaves and no more, and the bar offers the way back.
    window.maximise(true);
    say("Restore");
    assert_eq!(window.inner(), window.work_area());
    window.maximise(false);
    waited("the window to be put back", || Some(()).filter(|()| window.inner() == inner));
    waited("the bar to offer Maximise again", || (!says_now("Restore")).then_some(()));

    // Close is the last button of the bar, which is 44 units tall, and it ends the app.
    let width = inner.right - inner.left;
    window.click(width - unit(22.0), unit(22.0));
    waited("the app to end at a click on Close", || app.0.try_wait().ok().flatten());
}
