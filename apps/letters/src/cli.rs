//! The command line: `letters [FILE]`, plus the launch-time measurements.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
Usage: letters [OPTIONS] [FILE]

Opens FILE, or a new document when none is given.

Options:
      --launch-report      Print how long each stage of launch took, then exit after the first frame
      --bench-launch[=N]   Launch N times (default 20) and check the cold-launch budget
      --headless           With the two above: paint into memory, without a window
  -V, --version
  -h, --help
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
            Some("--headless") => options.headless = true,
            Some(flag) => match flag.strip_prefix("--bench-launch=").map(str::parse) {
                Some(Ok(runs)) if runs > 0 => options.mode = Mode::BenchLaunch { runs },
                Some(_) => return Err("--bench-launch takes a number of runs above zero".into()),
                None => return Err(format!("unknown option {flag}")),
            },
            None if options.file.is_some() => return Err("only one file can be opened at a time".into()),
            None => options.file = Some(arg.into()),
        }
    }
    if options.headless && options.mode == Mode::Edit {
        return Err("--headless needs --launch-report or --bench-launch".into());
    }
    Ok(Command::Run(options))
}
