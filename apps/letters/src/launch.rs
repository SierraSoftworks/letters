//! Measures launch: how long it takes from starting the app to a document painted on screen.
//!
//! - [`Launch`] times the stages inside one run, for `--launch-report`.
//! - [`bench`] launches the app repeatedly and checks the result against the budget, for `--bench-launch`.

use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Cold launch to the first painted document, on the reference machine (a MacBook Pro with the M1 Pro).
pub const BUDGET: Duration = Duration::from_millis(150);

/// The line a `--launch-report` run prints the moment its first frame is on screen.
pub const FIRST_FRAME: &str = "first frame";

/// A stopwatch started as the first thing `main` does.
#[derive(Clone, Debug)]
pub struct Launch {
    start: Instant,
    stages: Vec<(&'static str, Duration)>,
}

impl Launch {
    pub fn start() -> Self {
        Self { start: Instant::now(), stages: Vec::new() }
    }

    /// Records that a stage has just finished.
    pub fn mark(&mut self, stage: &'static str) {
        self.stages.push((stage, self.start.elapsed()));
    }

    /// When the latest stage finished, counted from the start.
    pub fn total(&self) -> Duration {
        self.stages.last().map_or(Duration::ZERO, |(_, at)| *at)
    }

    /// One line per stage: when it finished, and how long it took since the stage before.
    pub fn report(&self) -> String {
        let mut out = String::new();
        let mut before = Duration::ZERO;
        for (stage, at) in &self.stages {
            let _ = writeln!(out, "{:>8.2} ms  {:<24} +{:.2} ms", ms(*at), stage, ms(at.saturating_sub(before)));
            before = *at;
        }
        let verdict = if self.total() <= BUDGET { "within" } else { "over" };
        let _ =
            writeln!(out, "{:>8.2} ms  in the process, {verdict} the {:.0} ms budget", ms(self.total()), ms(BUDGET));
        out
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

/// What a series of launches measured, from spawning the process to its first frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Bench {
    /// Sorted, fastest first.
    pub runs: Vec<Duration>,
}

impl Bench {
    pub fn median(&self) -> Duration {
        self.runs[self.runs.len() / 2]
    }

    pub fn within_budget(&self) -> bool {
        self.median() <= BUDGET
    }

    pub fn report(&self) -> String {
        let (fastest, slowest) = (self.runs[0], self.runs[self.runs.len() - 1]);
        let verdict = if self.within_budget() { "within" } else { "over" };
        format!(
            "{} launches: median {:.1} ms, fastest {:.1} ms, slowest {:.1} ms\nmedian is {verdict} the {:.0} ms budget\n",
            self.runs.len(),
            ms(self.median()),
            ms(fastest),
            ms(slowest),
            ms(BUDGET)
        )
    }
}

/// Launches `program` on `file` `runs` times, timing each from spawn to the first frame it reports.
///
/// This includes starting the process, which the stopwatch inside a run cannot see. The file cache is warm
/// after the first run, so the slowest run is the nearest thing here to a cold launch.
pub fn bench(program: &Path, file: Option<&Path>, headless: bool, runs: u32) -> io::Result<Bench> {
    let mut times = Vec::new();
    for _ in 0..runs {
        let mut command = Command::new(program);
        command.arg("--launch-report").args(headless.then_some("--headless")).arg("--").args(file);
        let start = Instant::now();
        let mut child = command.stdout(Stdio::piped()).spawn()?;
        let mut output = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut line = String::new();
        output.read_line(&mut line)?;
        let elapsed = start.elapsed();
        // The rest is the run's own report. Reading it lets the run finish writing.
        output.read_to_string(&mut String::new())?;
        let status = child.wait()?;
        if line.trim() != FIRST_FRAME || !status.success() {
            return Err(io::Error::other(format!("a launch ended without painting a frame ({status})")));
        }
        times.push(elapsed);
    }
    times.sort();
    Ok(Bench { runs: times })
}
