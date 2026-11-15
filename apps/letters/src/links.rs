//! Opening a link from a document.
//!
//! `letters_trust` says what a click on a link may do. This turns that into the next step for a window,
//! and hands an address the person may visit to the system, which knows what opens it.

use std::io;
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
    command.arg(href.trim()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // The opener hands over and ends. It is waited for elsewhere, so that it leaves nothing behind.
    let mut opener = command.spawn()?;
    std::thread::spawn(move || drop(opener.wait()));
    Ok(())
}
