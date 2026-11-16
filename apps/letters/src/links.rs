//! Opening a link from a document.
//!
//! `letters_trust` says what a click on a link may do. This turns that into the next step for a window,
//! and hands an address the person may visit to the system, which knows what opens it. The system is
//! also what shows a file where it lies.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use letters_trust::{Gate, LinkDecision};

/// Why a link that would run something is not opened.
const RUNS: &str = "It would run something on this device instead of visiting a page.";
/// Why a link with no scheme is not opened.
const NOWHERE: &str = "It is not a full address, such as one that starts with https://.";

/// What a window does about a link that was clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Hand it to the system.
    Open,
    /// Show the whole address and ask first.
    Ask,
    /// Say why it is not opened.
    Refuse(&'static str),
}

/// The step for the link `href` in a document held to `gate`. `confirmed` says the person has read the
/// whole address and said yes, which is enough for a link that asks and changes nothing for one refused.
pub fn step(gate: &Gate<'_>, href: &str, confirmed: bool) -> Step {
    match gate.link(href) {
        LinkDecision::Refuse => Step::Refuse(RUNS),
        // The system would take an address with no scheme for a file on this device, and open or run it.
        _ if !addressed(href) => Step::Refuse(NOWHERE),
        LinkDecision::Ask if !confirmed => Step::Ask,
        LinkDecision::Open | LinkDecision::Ask => Step::Open,
    }
}

/// Whether an address names its scheme. One letter is a drive on Windows, not a scheme.
fn addressed(href: &str) -> bool {
    let Some((scheme, _)) = href.trim().split_once(':') else { return false };
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.');
    scheme.len() > 1 && scheme.starts_with(|c: char| c.is_ascii_alphabetic()) && scheme.chars().all(plain)
}

/// Hands an address to the system to open, with whatever the person uses for its kind. Only for a link
/// whose [`step`] is [`Step::Open`]: such an address starts with its scheme, so it cannot be read as an
/// option, and it is passed as one argument, with no shell to read it.
pub fn open(href: &str) -> io::Result<()> {
    let mut command = match () {
        () if cfg!(target_os = "macos") => Command::new("open"),
        () if cfg!(windows) => {
            let mut command = Command::new("rundll32");
            command.arg("url.dll,FileProtocolHandler");
            command
        }
        () => Command::new("xdg-open"),
    };
    command.arg(href.trim());
    hand_over(command)
}

fn hand_over(mut command: Command) -> io::Result<()> {
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // The opener hands over and ends. It is waited for elsewhere, so that it leaves nothing behind.
    let mut opener = command.spawn()?;
    std::thread::spawn(move || drop(opener.wait()));
    Ok(())
}

/// Shows `file` in the system's file manager, picked out among what lies beside it where the system can
/// do that. `file` is a whole path of the app's own making. This may wait for the file manager to
/// answer, so call it off the main thread.
pub fn reveal(file: &Path) -> io::Result<()> {
    #[cfg(all(unix, not(target_os = "macos")))]
    if file_manager::show(file).is_some() {
        return Ok(());
    }
    let command = match () {
        () if cfg!(target_os = "macos") => {
            let mut command = Command::new("open");
            command.arg("-R").arg(file);
            command
        }
        () if cfg!(windows) => {
            let mut command = Command::new("explorer");
            command.arg("/select,").arg(file);
            command
        }
        // No file manager says it can pick a file out: the folder is opened by whatever opens folders.
        () => {
            let mut command = Command::new("xdg-open");
            command.arg(file.parent().unwrap_or(file));
            command
        }
    };
    hand_over(command)
}

#[cfg(all(unix, not(target_os = "macos")))]
mod file_manager {
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use zbus::blocking::{Connection, Proxy};

    /// Asks the desktop's file manager to show `file`. `None` when none answers to the name they share.
    pub fn show(file: &Path) -> Option<()> {
        let plain = |byte: &u8| byte.is_ascii_alphanumeric() || b"-._~/".contains(byte);
        let escaped = file.as_os_str().as_bytes().iter().map(|byte| match plain(byte) {
            true => char::from(*byte).to_string(),
            false => format!("%{byte:02X}"),
        });
        let address = format!("file://{}", escaped.collect::<String>());
        let connection = Connection::session().ok()?;
        let name = "org.freedesktop.FileManager1";
        let manager = Proxy::new(&connection, name, "/org/freedesktop/FileManager1", name).ok()?;
        manager.call::<_, _, ()>("ShowItems", &(vec![address], "")).ok()
    }
}
