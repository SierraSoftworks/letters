//! Keeping Letters current. Once a window is up, GitHub is asked for the releases of [`REPOSITORY`]. A newer
//! one is fetched to `~/.letters/update`, checked against the digest GitHub gives for it, and the person is
//! told that it is ready. When they say so, Letters saves, closes and comes back as the new version, with
//! the document they were in.
//!
//! The replacing is `update-rs`'s: the new program is started and puts itself where the old one was, then
//! starts that, which removes what was fetched ([`resume`]). It replaces one file, so it is not used for the
//! macOS app, whose signature seals the whole bundle, nor where Letters may not write its own program
//! ([`Way::Download`]). The person is shown the release's page there.
//!
//! Only release builds look. The person turns it off with `LETTERS_UPDATES=0`, or with this in
//! `~/.letters/settings.toml`:
//!
//! ```toml
//! [updates]
//! check = false
//! ```

use std::ffi::OsString;
use std::future::Future;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, thread};

use letters_ui::Notice;
use semver::Version;
use sha2::{Digest, Sha256};
use toml_edit::DocumentMut;
use update_rs::{
    DefaultLauncher, GitHubSource, RESUME_FLAG, Release, Source, UpdateManager, UpdatePhase, UpdateState, naming,
};

/// Where releases are published. Each carries one program per system, named `letters-<target>`.
pub const REPOSITORY: &str = "SierraSoftworks/letters";

/// The page of the newest release, for a person who fetches it themselves.
pub const PAGE: &str = "https://github.com/SierraSoftworks/letters/releases/latest";

/// What a window's notice of a release is known by.
pub const KEY: &str = "letters:update";

/// Whether this run may look for a release at all, whatever the person set: a release build, without
/// `LETTERS_UPDATES=0` around it. Builds for development and their tests ask nobody.
pub fn here() -> bool {
    !cfg!(debug_assertions) && std::env::var_os("LETTERS_UPDATES").is_none_or(|said| said != "0")
}

/// Whether the settings file's text lets Letters look. It does unless it says otherwise.
pub fn allowed(settings: &str) -> bool {
    let Ok(file) = settings.parse::<DocumentMut>() else { return true };
    file.get("updates").and_then(|table| table.get("check")).and_then(|check| check.as_bool()) != Some(false)
}

/// How a newer release reaches this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    /// Letters replaces its own program and starts again.
    Itself,
    /// The person fetches it from the release's page.
    Download,
}

/// Whether a program is the one inside a macOS app, as `Letters.app/Contents/MacOS/letters` is.
pub fn bundled(program: &Path) -> bool {
    let mut above = program.ancestors().skip(1);
    above.next().is_some_and(|folder| folder.ends_with("Contents/MacOS"))
        && above.nth(1).is_some_and(|app| app.extension().is_some_and(|kind| kind == "app"))
}

/// How the program at this path can be brought up to date.
pub fn way(program: &Path) -> Way {
    // A file can be put in its folder: the old program is removed and the new one made there.
    let writable = program.parent().is_some_and(|folder| {
        let probe = folder.join(format!(".letters-{}", std::process::id()));
        let made = fs::File::create(&probe).is_ok();
        let _ = fs::remove_file(probe);
        made
    });
    if writable && !bundled(program) { Way::Itself } else { Way::Download }
}

/// The release to move to from version `from`: the highest that is no prerelease, has a program for this
/// system and is newer.
pub fn newest<'a>(releases: &'a [Release], from: &Version) -> Option<&'a Release> {
    let fits = |release: &&Release| !release.prerelease && release.variant.is_some() && release.version > *from;
    releases.iter().filter(fits).max_by(|a, b| a.version.cmp(&b.version))
}

/// A newer release, and with [`Way::Itself`] its program, fetched and checked.
#[derive(Clone, Debug)]
pub struct Found {
    pub version: Version,
    pub way: Way,
    fetched: PathBuf,
}

impl Found {
    /// What a window says of it. The press comes back under [`KEY`].
    pub fn notice(&self) -> Notice {
        let (sentence, button) = match self.way {
            Way::Itself => (format!("Letters {} is ready to install.", self.version), "Restart to update"),
            Way::Download => (format!("Letters {} is available.", self.version), "Download"),
        };
        Notice { sentence, button: button.into(), key: KEY.into() }
    }

    /// Starts the fetched program, which waits for this one to end and takes its place. `reopen` is what
    /// the new version is started with. Called last of all: everything must be saved by now.
    pub fn install(&self, reopen: Option<&Path>) -> Result<(), String> {
        let state = UpdateState {
            target_application: Some(std::env::current_exe().map_err(|error| error.to_string())?),
            temporary_application: Some(self.fetched.clone()),
            trace_context: None,
            phase: UpdatePhase::Prepare,
        };
        let rest = reopen.map(|path| path.as_os_str().to_owned()).into_iter().collect();
        wait(manager(rest).resume(&state)).and_then(|started| started.map_err(|error| error.to_string())).map(drop)
    }
}

/// Looks for a newer release on a thread of its own, and hands `tell` what it found. Nothing is said
/// when there is none, or when GitHub cannot be reached: the next start looks again.
pub fn look(home: &Path, tell: impl FnOnce(Found) + Send + 'static) {
    let folder = home.join(".letters").join("update");
    thread::spawn(move || match wait(found(&folder)) {
        Ok(Ok(Some(found))) => tell(found),
        Ok(Ok(None)) => {}
        Ok(Err(error)) => eprintln!("letters: no newer version was looked for: {error}"),
        Err(error) => eprintln!("letters: no newer version was looked for: {error}"),
    });
}

async fn found(folder: &Path) -> Result<Option<Found>, String> {
    let current: Version = env!("CARGO_PKG_VERSION").parse().map_err(|error| format!("{error}"))?;
    let source = source();
    let releases = source.get_releases().await.map_err(|error| error.to_string())?;
    let Some(release) = newest(&releases, &current) else {
        // What an earlier run fetched has been installed, or is no longer wanted.
        let _ = fs::remove_dir_all(folder);
        return Ok(None);
    };
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    let way = way(&program);
    let fetched = folder.join(format!("letters-{}{}", release.id, std::env::consts::EXE_SUFFIX));
    if way == Way::Itself {
        fetch(&source, release, folder, &fetched).await?;
    }
    Ok(Some(Found { version: release.version.clone(), way, fetched }))
}

/// Has the release's program at `fetched`, alone in `folder`. One an earlier run fetched is kept when it
/// still has the digest GitHub gives.
async fn fetch(source: &GitHubSource, release: &Release, folder: &Path, fetched: &Path) -> Result<(), String> {
    let Some(variant) = release.get_variant() else { return Err("the release has no program".into()) };
    if variant.sha256.as_deref().is_some_and(|digest| digest_of(fetched).is_ok_and(|has| has == digest)) {
        return Ok(());
    }
    let said = |error: io::Error| format!("{}: {error}", folder.display());
    let _ = fs::remove_dir_all(folder);
    fs::create_dir_all(folder).map_err(said)?;
    let mut file = fs::File::create(fetched).map_err(said)?;
    // The digest is checked as it is read. A file that fails is not left to be started.
    if let Err(error) = source.get_binary(release, variant, &mut file).await {
        drop(file);
        let _ = fs::remove_file(fetched);
        return Err(error.to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(fetched, fs::Permissions::from_mode(0o755)).map_err(said)?;
    }
    Ok(())
}

/// A file's SHA-256 in lower-case hex, as GitHub writes an asset's.
fn digest_of(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let (mut hasher, mut buffer) = (Sha256::new(), vec![0; 1 << 16]);
    loop {
        match file.read(&mut buffer)? {
            0 => break,
            read => hasher.update(&buffer[..read]),
        }
    }
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

fn source() -> GitHubSource {
    GitHubSource::new(REPOSITORY, naming::rust("letters")).with_release_tag_prefix("v")
}

/// `update-rs`'s manager, which starts the next program with `rest` after its own arguments.
fn manager(rest: Vec<OsString>) -> UpdateManager {
    UpdateManager::new(source()).with_launcher(Box::new(DefaultLauncher::new().with_args(rest)))
}

/// Runs a future to its end on this thread. The network is only ever waited for off the main thread.
fn wait<F: Future>(future: F) -> Result<F::Output, String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
    Ok(runtime.map_err(|error| error.to_string())?.block_on(future))
}

/// What a start of the program is for, as far as updating goes.
#[derive(Debug, PartialEq, Eq)]
pub enum Resumed {
    /// Run as usual with these arguments: no update was under way, or it is done with.
    Run(Vec<OsString>),
    /// This start was a step of an update, and another program carries on from here.
    Leave,
}

/// Takes the part of an update this start was made for, before anything else reads the arguments.
/// `update-rs` starts the program with [`RESUME_FLAG`] and its state. What follows those is what the
/// person's Letters is to be started with at the end.
pub fn resume(mut arguments: Vec<OsString>) -> Resumed {
    let Some(at) = arguments.iter().position(|argument| argument == RESUME_FLAG) else {
        return Resumed::Run(arguments);
    };
    let rest = arguments.split_off((at + 2).min(arguments.len()));
    let state = arguments.get(at + 1).and_then(|state| state.to_str()).unwrap_or_default().to_owned();
    arguments.truncate(at);
    arguments.extend(rest.iter().cloned());
    let Ok(read) = serde_json::from_str::<UpdateState>(&state) else {
        eprintln!("letters: the update's state could not be read");
        return Resumed::Run(arguments);
    };
    let resumed =
        move |rest| wait(manager(rest).resume_from_arg(&state)).and_then(|done| done.map_err(|e| e.to_string()));
    match read.phase {
        // This is the new version, in its place. What was fetched is removed while the window opens:
        // the program that was started from it may take a moment to end.
        UpdatePhase::Cleanup => {
            thread::spawn(move || resumed(Vec::new()).map_err(|error| eprintln!("letters: {error}")));
            Resumed::Run(arguments)
        }
        UpdatePhase::NoUpdate => Resumed::Run(arguments),
        // This is the fetched program. If it cannot take the old one's place, the old one comes back.
        UpdatePhase::Prepare | UpdatePhase::Replace => {
            if let Err(error) = resumed(rest.clone()) {
                eprintln!("letters: {error}");
                if let Some(old) = read.target_application.filter(|old| old.is_file()) {
                    let _ = Command::new(old).args(rest).spawn();
                }
            }
            Resumed::Leave
        }
    }
}
