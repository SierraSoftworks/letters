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

/// One place Letters fails at, with the smallest input that gets there.
pub struct Failure {
    pub place: String,
    pub input: PathBuf,
    /// What the run printed as it failed.
    pub output: String,
    /// How many of the inputs fail here.
    pub inputs: usize,
}

/// Where a run failed, from how it ended and what it printed: the line of code that panicked, since the
/// words of a panic change with its input. `None` for a run that ended well.
pub fn place(ended: Option<ExitStatus>, output: &str) -> Option<String> {
    let Some(status) = ended else { return Some(format!("a hang of more than {} seconds", PATIENCE.as_secs())) };
    if status.success() {
        return None;
    }
    let panicked = output.lines().find_map(|line| line.split_once(" panicked at ")).map(|(_, at)| at);
    let overflowed = output.contains("has overflowed its stack").then_some("a stack overflow");
    let named = panicked.map(|at| at.trim_end_matches(':')).or(overflowed);
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
        // The start says what failed, and a backtrace's far end says little.
        let cut = failure.output.char_indices().nth(OUTPUT).map_or(failure.output.len(), |(at, _)| at);
        out += &format!("\n{}\n", failure.output[..cut].trim_end());
    }
    out += &format!("\nReplay an input with: cargo run -p letters-fuzz -- {target} <file>\n");
    out += &format!("Once it is fixed, keep the input in fuzz/corpus/{target}/ and the tests run it from then on.\n");
    out
}
