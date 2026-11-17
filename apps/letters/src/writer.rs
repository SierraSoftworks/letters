//! Whose name the edits made here carry, when the person has set none: what their account calls them.
//!
//! The name goes into a document's history and leaves the machine with the file. It goes nowhere else,
//! and is never printed.

use std::env;
use std::process::Command;

/// The full name in an account's line of the user database, as `getent passwd` prints it.
pub fn full_name(account: &str) -> Option<String> {
    // The fifth field holds the name, then a room and telephone numbers after commas.
    proper(account.split(':').nth(4)?.split(',').next()?)
}

/// A name with something in it.
fn proper(name: &str) -> Option<String> {
    Some(name.trim().to_owned()).filter(|name| !name.is_empty())
}

fn said(command: &mut Command) -> Option<String> {
    let output = command.output().ok().filter(|output| output.status.success())?;
    String::from_utf8(output.stdout).ok()
}

/// What the account of the person at this machine calls them: its full name, or else the name they log in
/// with. This asks the system, so call it off the main thread.
pub fn account() -> Option<String> {
    let names = if cfg!(windows) { ["USERNAME", "USER"] } else { ["USER", "LOGNAME"] };
    let login = names.into_iter().find_map(|name| env::var(name).ok().and_then(|login| proper(&login)));
    let named = match (cfg!(target_os = "macos"), &login) {
        _ if cfg!(windows) => letters_frame::account_name().and_then(|name| proper(&name)),
        (true, _) => said(Command::new("id").arg("-F")).and_then(|name| proper(&name)),
        (false, Some(login)) => said(Command::new("getent").args(["passwd", login])).and_then(|line| full_name(&line)),
        (false, None) => None,
    };
    named.or(login)
}
