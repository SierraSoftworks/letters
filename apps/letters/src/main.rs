//! The Letters desktop app.

use std::process::ExitCode;

use letters::cli::{self, Command, Mode};
use letters::measure::{self, Launch};

fn main() -> ExitCode {
    let launch = Launch::start();
    // A file in a folder synced on demand is downloaded as it is read, also when Letters was not started
    // from Finder. Without this macOS refuses the read (EDEADLK).
    if !letters_frame::fetch_cloud_files() {
        eprintln!("letters: files kept in the cloud may not be read: the system would not set the policy");
    }
    let options = match cli::parse(std::env::args_os().skip(1)) {
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
