//! Measures the app against its performance budgets.
//!
//! - [`Launch`] times the stages from starting the app to a painted document, for `--launch-report`.
//! - [`bench`] launches the app repeatedly and checks the result against the budget, for `--bench-launch`.
//! - [`Typing`] collects how long each keystroke takes to reach a frame, for `--typing-report`.

use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Cold launch to the first painted document, on the reference machine (a MacBook Pro with the M1 Pro).
pub const BUDGET: Duration = Duration::from_millis(150);

/// A keystroke to its frame submitted, at the median.
pub const KEYSTROKE: Duration = Duration::from_millis(8);
/// The same, for the slowest keystroke in a hundred.
pub const KEYSTROKE_P99: Duration = Duration::from_millis(16);

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

/// How long keystrokes took to reach a submitted frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Typing {
    samples: Vec<Duration>,
}

impl Typing {
    pub fn record(&mut self, keystroke: Duration) {
        let at = self.samples.partition_point(|sample| *sample <= keystroke);
        self.samples.insert(at, keystroke);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The keystroke that `percent` of the others were at least as fast as. Zero with no samples.
    pub fn percentile(&self, percent: usize) -> Duration {
        let rank = (self.samples.len() * percent).div_ceil(100).max(1);
        self.samples.get(rank - 1).copied().unwrap_or_default()
    }

    pub fn within_budget(&self) -> bool {
        self.percentile(50) <= KEYSTROKE && self.percentile(99) <= KEYSTROKE_P99
    }

    /// `renderer` says what painted the frames, such as `CPU`.
    pub fn report(&self, renderer: &str) -> String {
        let verdict = |within| if within { "within" } else { "over" };
        let (median, p99) = (self.percentile(50), self.percentile(99));
        format!(
            "{} keystrokes on the {renderer}: median {:.2} ms, p99 {:.2} ms, slowest {:.2} ms\n\
             median is {} the {:.0} ms budget, p99 is {} the {:.0} ms budget\n",
            self.len(),
            ms(median),
            ms(p99),
            ms(self.percentile(100)),
            verdict(median <= KEYSTROKE),
            ms(KEYSTROKE),
            verdict(p99 <= KEYSTROKE_P99),
            ms(KEYSTROKE_P99),
        )
    }
}
