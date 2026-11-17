//! The Letters desktop app.

// Windows opens a console beside a program that asks for one, and the app asks for none. The commands
// that print write to the console they were started from (see `attach_console`).
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;

use letters::cli::{self, Command, Mode};
use letters::measure::{self, Launch};
use letters::update::{self, Resumed};

fn main() -> ExitCode {
    let launch = Launch::start();
    // Without a console of its own, `letters --version` and the like say nothing unless they take the one
    // they were typed into. Started from anywhere else there is none, and nothing to say it to.
    let _ = letters_frame::attach_console();
    // A file in a folder synced on demand is downloaded as it is read, also when Letters was not started
    // from Finder. Without this macOS refuses the read (EDEADLK).
    if !letters_frame::fetch_cloud_files() {
        eprintln!("letters: files kept in the cloud may not be read: the system would not set the policy");
    }
    // A start that is a step of an update reads its own arguments first, and may be all this run is for.
    let arguments = match update::resume(std::env::args_os().skip(1).collect()) {
        Resumed::Run(arguments) => arguments,
        Resumed::Leave => return ExitCode::SUCCESS,
    };
    let options = match cli::parse(arguments) {
        Ok(Command::Run(options)) => options,
        Ok(Command::Export(export)) => return letters::convert::export(&export),
        Ok(Command::Import(import)) => return letters::convert::import(&import),
        Ok(Command::CheckHistory) => return letters::strangers::child(),
        Ok(Command::Version) => {
            println!("letters {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(Command::Help) => {
            print!("{}", cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprint!("letters: {message}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };

    let result = match options.mode {
        Mode::BenchLaunch { runs } => std::env::current_exe()
            .and_then(|program| measure::bench(&program, options.file.as_deref(), options.headless, runs))
            .map_err(Into::into)
            .map(|bench| {
                print!("{}", bench.report());
                bench.within_budget()
            }),
        Mode::Baseline => letters::baseline::run(options.headless, launch).map(|()| true),
        _ if options.headless => letters::app::run_headless(&options, launch),
        _ => letters::app::run(options, launch).map(|()| true),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("letters: {error}");
            ExitCode::FAILURE
        }
    }
}
