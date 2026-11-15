//! Fuzz targets for Letters.
//!
//! A target takes any bytes and panics when Letters breaks a promise on them: a parser that fails where it
//! should refuse, writers that end on different documents, a save or a crash that loses work. `scripts/fuzz`
//! runs one under AFL++, and the scheduled workflow runs them all and files what they find.
//!
//! Built by cargo-afl, the binary is an AFL++ target. Built by anything else it reads one input and needs no
//! AFL++, which is how a finding is replayed and how the tests run every seed.
//!
//! An input that once failed goes into `fuzz/corpus/<target>/`, where it is a seed and a test from then on.

mod archive;
mod checks;
mod edits;
mod folder;
mod input;
pub mod report;
mod targets;

pub use input::MAX_INPUT;

/// One fuzz target.
pub struct Target {
    pub name: &'static str,
    /// What the target holds Letters to, for the report of a finding.
    pub about: &'static str,
    /// Runs one input. Returning means Letters kept its promises on it.
    pub run: fn(&[u8]),
    /// Inputs made of real documents, for the fuzzer to start from.
    pub seeds: fn() -> Vec<Vec<u8>>,
}

pub use targets::TARGETS;

pub fn target(name: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|target| target.name == name)
}
