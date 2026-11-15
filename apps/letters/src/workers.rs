//! A few threads for work that must not hold the UI thread, and must not all happen at once.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

type Job = Box<dyn FnOnce() + Send>;

/// Threads that do the jobs they are given, in the order given and no more at a time than they are.
/// They start with the first job, so a launch that has none pays nothing for them.
pub struct Workers {
    most: usize,
    jobs: Option<Sender<Job>>,
}

impl Workers {
    /// At most `most` jobs at a time, and no more than the machine has cores for.
    pub const fn new(most: usize) -> Self {
        Self { most, jobs: None }
    }

    /// Does `job` on one of the threads, once those before it are under way.
    pub fn run(&mut self, job: impl FnOnce() + Send + 'static) {
        let most = self.most;
        let jobs = self.jobs.get_or_insert_with(|| start(most));
        // The threads live as long as this does, so the job is taken.
        let _ = jobs.send(Box::new(job));
    }
}

fn start(most: usize) -> Sender<Job> {
    let (jobs, waiting) = mpsc::channel::<Job>();
    let waiting = Arc::new(Mutex::new(waiting));
    let threads = thread::available_parallelism().map_or(1, usize::from).min(most).max(1);
    for _ in 0..threads {
        let waiting = waiting.clone();
        thread::spawn(move || {
            while let Some(job) = next(&waiting) {
                job();
            }
        });
    }
    jobs
}

/// The next job, once there is one. `None` when the workers are gone and no more will come.
fn next(waiting: &Mutex<Receiver<Job>>) -> Option<Job> {
    waiting.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).recv().ok()
}
