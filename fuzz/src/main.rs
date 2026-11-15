//! `letters-fuzz <target>` runs a target: under AFL++ when cargo-afl built it, and otherwise once on the
//! input on stdin. `letters-fuzz <target> <file>...` replays inputs, `--seeds <target> <folder>` writes a
//! target's seeds, `--report <target> <folder>...` tells where the inputs AFL++ kept there fail, and `--list`
//! names the targets.

use std::path::Path;
use std::process::ExitCode;

use letters_fuzz::report::{failures, told};
use letters_fuzz::{TARGETS, Target, target};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["--list"] => {
            for target in TARGETS {
                println!("{}\t{}", target.name, target.about);
            }
        }
        ["--seeds", name, folder] => return with(name, |target| seeds(target, Path::new(folder))),
        ["--report", name, folders @ ..] => return with(name, |target| report(target, folders)),
        [name] => return with(name, fuzz),
        [name, files @ ..] => return with(name, |target| replay(target, files)),
        [] => {
            eprintln!("usage: letters-fuzz <target> [file...] | --seeds <target> <folder>");
            eprintln!("       letters-fuzz --report <target> <folder>... | --list");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

fn with(name: &str, then: impl FnOnce(&Target) -> std::io::Result<()>) -> ExitCode {
    let Some(target) = target(name) else {
        eprintln!("there is no fuzz target \"{name}\": --list names them");
        return ExitCode::from(2);
    };
    match then(target) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("letters-fuzz: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(fuzzing)]
fn fuzz(target: &Target) -> std::io::Result<()> {
    let run = target.run;
    // Not `fuzz!`, whose hook ends the process at a panic, before a target that catches one gets to. A
    // panic that leaves the target still ends the process, and is a finding.
    afl::fuzz_nohook!(|data: &[u8]| run(data));
    Ok(())
}

#[cfg(not(fuzzing))]
fn fuzz(target: &Target) -> std::io::Result<()> {
    use std::io::Read;
    let mut data = Vec::new();
    std::io::stdin().read_to_end(&mut data)?;
    (target.run)(&data);
    Ok(())
}

fn replay(target: &Target, files: &[&str]) -> std::io::Result<()> {
    for file in files {
        (target.run)(&std::fs::read(file)?);
    }
    Ok(())
}

fn seeds(target: &Target, folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    for (n, seed) in (target.seeds)().iter().enumerate() {
        std::fs::write(folder.join(format!("seed-{n:02}")), seed)?;
    }
    Ok(())
}

/// Tells what the inputs in `folders` find, and fails when any of them fails.
fn report(target: &Target, folders: &[&str]) -> std::io::Result<()> {
    let folders: Vec<&Path> = folders.iter().map(Path::new).collect();
    let found = failures(target.name, &folders)?;
    if found.is_empty() {
        return Ok(());
    }
    print!("{}", told(target.name, target.about, &found));
    Err(std::io::Error::other(format!("the \"{}\" target found failures", target.name)))
}
