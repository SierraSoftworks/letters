//! What a run of the fuzzer found, told for a person: every input it kept is run again, and the inputs
//! are grouped by where they fail, since many inputs find the same fault.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How long one input may run. An input is a few kilobytes at most, so a longer run is a hang.
const PATIENCE: Duration = Duration::from_secs(20);
/// How many of the kept inputs are run again, smallest first.
const REPLAYED: usize = 200;
/// How many places are shown with what was printed there. The others are named.
const SHOWN: usize = 5;
/// The longest output shown for one place, and the largest input written out with it.
const OUTPUT: usize = 6000;
const INPUT: usize = 2048;
/// How many calls nearest to a panic are shown whoever's code they are in.
const NEAR: usize = 3;
/// The longest name of a call that is shown whole.
const CALL: usize = 100;

/// One place Letters fails at, with the smallest input that gets there.
pub struct Failure {
    pub place: String,
    pub input: PathBuf,
    /// What the run printed as it failed.
    pub output: String,
    /// How many of the inputs fail here.
    pub inputs: usize,
}

/// One panic of a run, as it was printed.
struct Panic<'a> {
    /// The line of code that panicked.
    place: &'a str,
    /// The line that names it, and its words.
    said: Vec<&'a str>,
    /// The calls beneath it, nearest first, each with its place in its file.
    frames: Vec<(&'a str, &'a str)>,
}

/// Code of Rust itself, which panics for whoever called it.
fn runtime(file: &str) -> bool {
    file.starts_with("/rustc/")
}

/// Code of a crate Letters depends on.
fn dependency(file: &str) -> bool {
    file.contains("/registry/src/")
}

/// The panics of a run, in the order they were printed.
fn panics(output: &str) -> Vec<Panic<'_>> {
    let mut found: Vec<Panic> = Vec::new();
    let (mut backtrace, mut call) = (false, None);
    for line in output.lines() {
        if let Some((_, at)) = line.split_once(" panicked at ") {
            found.push(Panic { place: at.trim_end_matches(':'), said: vec![line], frames: Vec::new() });
            (backtrace, call) = (false, None);
            continue;
        }
        let Some(panic) = found.last_mut() else { continue };
        let text = line.trim_start();
        if text == "stack backtrace:" {
            backtrace = true;
        } else if !backtrace {
            panic.said.push(line);
        } else if let Some(file) = text.strip_prefix("at ") {
            panic.frames.extend(call.take().map(|call| (call, file)));
        } else if text.split_once(": ").is_some_and(|(n, _)| n.parse::<u32>().is_ok()) {
            call = Some(text);
        }
    }
    found
}

/// A panic in few lines: what it said, the calls nearest to it, and every call of Letters' own code. The
/// rest of a backtrace is a dependency's or Rust's, and says little of what Letters did.
fn brief(panic: &Panic) -> String {
    let mut out = panic.said.join("\n").trim_end().to_owned() + "\n";
    let mut near = NEAR;
    // This binary's own calls are under every panic alike.
    let told = |file: &str| !runtime(file) && !file.contains("fuzz/src/main.rs");
    for (call, file) in panic.frames.iter().filter(|(_, file)| told(file)) {
        if dependency(file) && near == 0 {
            continue;
        }
        near = near.saturating_sub(1);
        // The name of a generic call runs to hundreds of characters.
        let call = call.char_indices().nth(CALL).map_or(*call, |(at, _)| &call[..at]);
        out += &format!("  {call}\n      at {file}\n");
    }
    out
}

/// Where a run failed, from how it ended and what it printed: the line of code that panicked, since the
/// words of a panic change with its input. `None` for a run that ended well.
///
/// The last panic counts, since a target may catch one and go on. One of Rust's own, as where a second
/// panic stops the first from unwinding, counts only when there is no other.
pub fn place(ended: Option<ExitStatus>, output: &str) -> Option<String> {
    let Some(status) = ended else { return Some(format!("a hang of more than {} seconds", PATIENCE.as_secs())) };
    if status.success() {
        return None;
    }
    let panics = panics(output);
    let panicked = panics.iter().rev().find(|panic| !runtime(panic.place)).or(panics.last()).map(|panic| panic.place);
    let overflowed = output.contains("has overflowed its stack").then_some("a stack overflow");
    let named = panicked.or(overflowed);
    Some(named.map_or_else(|| format!("no panic, {status}"), str::to_owned))
}

/// Runs `target` on `input` in a process of its own, as a failure ends the process.
fn replay(target: &str, input: &Path) -> std::io::Result<(Option<ExitStatus>, String)> {
    let mut child = Command::new(std::env::current_exe()?)
        .args([target.as_ref(), input.as_os_str()])
        .env("RUST_BACKTRACE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    // Read on the side, so that a run that prints a lot is never held up.
    let mut stderr = child.stderr.take().expect("the output is piped");
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stderr.read_to_end(&mut output);
        String::from_utf8_lossy(&output).into_owned()
    });
    let started = Instant::now();
    let ended = loop {
        match child.try_wait()? {
            Some(status) => break Some(status),
            None if started.elapsed() > PATIENCE => {
                child.kill()?;
                child.wait()?;
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    Ok((ended, reader.join().unwrap_or_default()))
}

/// The places `target` fails at on the inputs in `folders`, in the order they were found.
pub fn failures(target: &str, folders: &[&Path]) -> std::io::Result<Vec<Failure>> {
    let entries = folders.iter().filter_map(|folder| std::fs::read_dir(folder).ok()).flatten().flatten();
    // AFL++ names what it keeps by id, beside a note of its own.
    let kept = entries.filter(|entry| entry.file_name().to_string_lossy().starts_with("id"));
    let mut inputs: Vec<(u64, PathBuf)> =
        kept.filter_map(|entry| Some((entry.metadata().ok()?.len(), entry.path()))).collect();
    inputs.sort();
    let mut found: Vec<Failure> = Vec::new();
    for (_, input) in inputs.into_iter().take(REPLAYED) {
        let (ended, output) = replay(target, &input)?;
        let Some(place) = place(ended, &output) else { continue };
        match found.iter_mut().find(|failure| failure.place == place) {
            Some(failure) => failure.inputs += 1,
            None => found.push(Failure { place, input, output, inputs: 1 }),
        }
    }
    Ok(found)
}

/// Bytes in base64, for an input that is pasted from a report into a file.
fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().fold(0u32, |bits, byte| bits << 8 | u32::from(*byte)) << (8 * (3 - chunk.len()));
        for digit in 0..4 {
            let shown = digit <= chunk.len();
            out.push(if shown { DIGITS[(bits >> (18 - 6 * digit) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// The report of what `target` fails at, for a terminal or an issue.
pub fn told(target: &str, about: &str, failures: &[Failure]) -> String {
    let inputs: usize = failures.iter().map(|failure| failure.inputs).sum();
    let mut out = format!("The \"{target}\" target fails at {} place(s), on {inputs} input(s).\n", failures.len());
    out += &format!("It holds Letters to this: {about}.\n");
    for (n, failure) in failures.iter().enumerate() {
        out += &format!("\n== {} of {}: {} ({} input(s))\n", n + 1, failures.len(), failure.place, failure.inputs);
        if n >= SHOWN {
            continue;
        }
        let bytes = std::fs::read(&failure.input).unwrap_or_default();
        out += &format!("The smallest input is {} ({} bytes).\n", failure.input.display(), bytes.len());
        if bytes.len() <= INPUT {
            out += "To have it as a file, give this to `base64 -d`:\n";
            let encoded = base64(&bytes);
            for line in encoded.as_bytes().chunks(76) {
                out += &format!("{}\n", String::from_utf8_lossy(line));
            }
        }
        // Every panic of the run in brief, since the earlier ones were caught or are what the last came of.
        let panics = panics(&failure.output);
        let printed = panics.iter().map(brief).collect::<Vec<_>>().join("\n");
        let printed = if panics.is_empty() { failure.output.as_str() } else { printed.as_str() };
        // The far end of a long run says little.
        let cut = printed.char_indices().nth(OUTPUT).map_or(printed.len(), |(at, _)| at);
        out += &format!("\n{}\n", printed[..cut].trim_end());
    }
    out += &format!("\nReplay an input with: cargo run -p letters-fuzz -- {target} <file>\n");
    out += &format!("Once it is fixed, keep the input in fuzz/corpus/{target}/ and the tests run it from then on.\n");
    out
}
