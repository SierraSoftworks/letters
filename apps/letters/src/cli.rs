//! The command line: `letters [FILE]`, plus the launch-time measurements.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
Usage: letters [OPTIONS] [FILE]

Opens FILE, or a new document when none is given.

Options:
      --launch-report      Print how long each stage of launch took, then exit after the first frame
      --bench-launch[=N]   Launch N times (default 20) and check the cold-launch budget
      --typing-report[=N]  Type N characters (default 200) and check the keystroke budget, saving nothing
      --headless           With the three above: paint into memory, without a window
  -V, --version
  -h, --help

Environment:
  LETTERS_RENDERER=cpu     Never use the GPU
  LETTERS_RENDERER=gpu     Use the GPU even when it is a software one
";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Run(Options),
    Version,
    Help,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub file: Option<PathBuf>,
    pub mode: Mode,
    /// Paints the first frame into memory. Only the measuring modes take it.
    pub headless: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Edit,
    LaunchReport,
    BenchLaunch {
        runs: u32,
    },
    TypingReport {
        keys: u32,
    },
}

/// Parses the arguments after the program name.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut options = Options::default();
    let mut only_files = false;
    for arg in args {
        let flag = arg.to_str().filter(|text| !only_files && text.starts_with('-') && text.len() > 1);
        match flag {
            Some("--") => only_files = true,
            Some("-V" | "--version") => return Ok(Command::Version),
            Some("-h" | "--help") => return Ok(Command::Help),
            Some("--launch-report") => options.mode = Mode::LaunchReport,
            Some("--bench-launch") => options.mode = Mode::BenchLaunch { runs: 20 },
            Some("--typing-report") => options.mode = Mode::TypingReport { keys: 200 },
            Some("--headless") => options.headless = true,
            Some(flag) => match flag.split_once('=').map(|(name, count)| (name, count.parse())) {
                Some(("--bench-launch", Ok(runs))) if runs > 0 => options.mode = Mode::BenchLaunch { runs },
                Some(("--typing-report", Ok(keys))) if keys > 0 => options.mode = Mode::TypingReport { keys },
                Some(("--bench-launch" | "--typing-report", _)) => {
                    return Err(format!("{flag}: expected a count above zero"));
                }
                _ => return Err(format!("unknown option {flag}")),
            },
            None if options.file.is_some() => return Err("only one file can be opened at a time".into()),
            None => options.file = Some(arg.into()),
        }
    }
    if options.headless && options.mode == Mode::Edit {
        return Err("--headless needs --launch-report, --bench-launch or --typing-report".into());
    }
    Ok(Command::Run(options))
}
