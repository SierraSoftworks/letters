//! Whose name the edits made here carry.
//!
//! The name goes into a document's history and leaves the machine with the file. It goes nowhere else,
//! and is never printed.

use std::path::Path;
use std::process::Command;
use std::{env, fs};

/// The name a settings file gives its writer: `name` in the `[writer]` table.
pub fn named(settings: &str) -> Option<String> {
    let settings = settings.parse::<toml_edit::DocumentMut>().ok()?;
    proper(settings.get("writer")?.get("name")?.as_str()?)
}

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

/// The name of the person whose home folder is `home`: the one their settings give, or else their
/// account's full name, or else the name they log in with. This asks the system, so call it off the main
/// thread.
pub fn find(home: &Path) -> Option<String> {
    let settings = fs::read_to_string(home.join(".letters").join("settings.toml")).ok();
    let login = env::var("USER").or_else(|_| env::var("LOGNAME")).ok().and_then(|login| proper(&login));
    let account = || match (cfg!(target_os = "macos"), &login) {
        (true, _) => said(Command::new("id").arg("-F")).and_then(|name| proper(&name)),
        (false, Some(login)) => said(Command::new("getent").args(["passwd", login])).and_then(|line| full_name(&line)),
        (false, None) => None,
    };
    settings.and_then(|settings| named(&settings)).or_else(account).or(login)
}
