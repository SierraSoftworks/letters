//! A stranger's history is tried in a process of its own.
//!
//! A history from elsewhere may be one the history library panics on, and a panic ends the app, whose
//! panics abort. So the app runs itself a second time with [`ARGUMENT`], hands that process the history and
//! nothing else, and takes it in only if the process says it stands. Whatever else happens there, a crash
//! or a history that never finishes reading, is a refusal, and the app and its documents are as they were.
//!
//! This costs a process for each history, so it is kept off the UI thread and off the launch path: only a
//! document nobody vouches for, a file that is merged, and a server's file are read this way.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use letters_model::{HistoryError, Model};

/// The argument that makes the app try the history on its standard input and do nothing else. It is the
/// app's own business, so the usage does not list it.
pub const ARGUMENT: &str = "--check-history";

/// How long a history may take to read before it is refused, and how much longer for each megabyte of it.
/// Far more than an honest one needs on a slow machine: this is for the one that never ends.
const LIMIT: Duration = Duration::from_secs(10);
const LIMIT_PER_MB: Duration = Duration::from_secs(5);

static PROGRAM: OnceLock<PathBuf> = OnceLock::new();

/// Names the app's program, for a process that is not the app itself, as a test is. The first name stays.
pub fn run_with(program: impl Into<PathBuf>) {
    let _ = PROGRAM.set(program.into());
}

/// What the app does when it was started with [`ARGUMENT`]: reads a history from its standard input and
/// ends with success if [`Model::check`] has it stand. It reads and writes nothing else.
pub fn child() -> ExitCode {
    // A panic means the history does not stand. Ending here says so without a crash for the system to
    // report or a trace on the terminal.
    std::panic::set_hook(Box::new(|_| std::process::exit(1)));
    let mut history = Vec::new();
    match io::stdin().lock().read_to_end(&mut history) {
        Ok(_) if Model::check(&history).is_ok() => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

/// Tries a whole history in a process of its own, as a wary copy's [`letters_format::Check`]. Refused
/// unless that process says the history stands: so is one it crashed on or did not finish in time, and
/// every history when the process cannot be started.
pub fn check(history: &[u8]) -> Result<(), HistoryError> {
    let program = match PROGRAM.get() {
        Some(program) => Ok(program.clone()),
        None => std::env::current_exe(),
    };
    said(program.and_then(|program| tried(&program, history)))
}

/// [`check`] by the app at `program`.
pub fn check_by(program: &Path, history: &[u8]) -> Result<(), HistoryError> {
    said(tried(program, history))
}

/// Whether [`check`] has the history stand.
pub fn stands(history: &[u8]) -> bool {
    check(history).is_ok()
}

fn said(tried: io::Result<bool>) -> Result<(), HistoryError> {
    match tried {
        Ok(true) => Ok(()),
        Ok(false) => Err(HistoryError::Unreadable("it did not stand where it was tried".into())),
        Err(error) => Err(HistoryError::Unreadable(format!("it could not be tried: {error}"))),
    }
}

fn tried(program: &Path, history: &[u8]) -> io::Result<bool> {
    let mut command = Command::new(program);
    command.arg(ARGUMENT).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up beside the app.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn()?;
    let pipe = child.stdin.take();
    let megabytes = u32::try_from(history.len() >> 20).unwrap_or(u32::MAX);
    let until = Instant::now() + LIMIT + LIMIT_PER_MB.saturating_mul(megabytes);
    thread::scope(|scope| {
        // Written beside the wait, so that a process that reads nothing cannot hold this one up. The pipe
        // closes as the writing ends, which is how the process knows it has the whole history.
        scope.spawn(move || pipe.map(|mut pipe| pipe.write_all(history)));
        let mut pause = Duration::from_micros(200);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status.success()),
                Ok(None) if Instant::now() < until => {
                    thread::sleep(pause);
                    pause = (pause * 2).min(Duration::from_millis(10));
                }
                waited => {
                    // Ending it also frees a writer that waits on a full pipe.
                    let _ = child.kill();
                    let _ = child.wait();
                    return waited.map(|_| false);
                }
            }
        }
    })
}
