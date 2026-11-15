//! Every target runs on the inputs the fuzzer starts from, and on those that once failed.

use std::path::Path;

use letters_fuzz::TARGETS;

/// The inputs kept for a target in `fuzz/corpus/<target>/`, each with its file's name.
fn kept(target: &str) -> Vec<(String, Vec<u8>)> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus").join(target);
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let read = |entry: std::fs::DirEntry| {
        let bytes = std::fs::read(entry.path()).expect("a kept input is read");
        (entry.file_name().to_string_lossy().into_owned(), bytes)
    };
    entries.flatten().map(read).collect()
}

#[test]
fn every_target_passes_its_seeds() {
    for target in TARGETS {
        let seeds = (target.seeds)();
        assert!(!seeds.is_empty(), "the fuzzer has nothing to start {} from", target.name);
        for seed in &seeds {
            assert!(seed.len() <= letters_fuzz::MAX_INPUT, "a seed of {} is cut short", target.name);
            (target.run)(seed);
        }
    }
}

#[test]
fn every_target_passes_the_inputs_that_once_failed() {
    for target in TARGETS {
        for (name, input) in kept(target.name) {
            println!("{} on {name}", target.name);
            (target.run)(&input);
        }
    }
}

#[test]
fn every_kept_input_belongs_to_a_target() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let Ok(folders) = std::fs::read_dir(corpus) else { return };
    for folder in folders.flatten() {
        let name = folder.file_name().to_string_lossy().into_owned();
        assert!(letters_fuzz::target(&name).is_some(), "fuzz/corpus/{name} is for no target");
    }
}
