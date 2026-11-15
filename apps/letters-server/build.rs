//! Compiles the browser's editor into the server, when the build is told where it is.
//!
//! `LETTERS_WEB_DIST` names the folder `apps/letters-web/build.sh` made. Each of its files goes in as
//! brotli, made here once so that no request waits for it. Built without the variable, the server has no
//! editor and shows its documents to read.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

/// What is not compiled in: the page, which is made for each document, and the large faces, which the
/// server reads from its state directory.
const LEFT_OUT: [&str; 2] = ["index.html", "fallback"];

fn main() {
    println!("cargo:rerun-if-env-changed=LETTERS_WEB_DIST");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo says where a build writes"));
    let mut table = String::from("&[\n");
    if let Some(dist) = env::var_os("LETTERS_WEB_DIST").filter(|dist| !dist.is_empty()) {
        let dist = PathBuf::from(dist);
        println!("cargo:rerun-if-changed={}", dist.display());
        let mut names = Vec::new();
        list(&dist, "", &mut names);
        names.sort();
        let whole = ["start.js", "letters.js", "letters_bg.wasm"].iter().all(|name| names.iter().any(|at| at == name));
        assert!(whole, "LETTERS_WEB_DIST: {} is not what apps/letters-web/build.sh makes", dist.display());
        for (at, name) in names.iter().enumerate() {
            let bytes = fs::read(dist.join(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
            let small = brotli(&bytes);
            // A file that brotli does not make smaller goes in as it is.
            let (bytes, compressed) = if small.len() < bytes.len() { (small, true) } else { (bytes, false) };
            let kept = out.join(format!("editor-{at}"));
            fs::write(&kept, bytes).unwrap_or_else(|error| panic!("{}: {error}", kept.display()));
            writeln!(table, "    ({name:?}, include_bytes!({:?}), {compressed}),", kept.display().to_string()).unwrap();
        }
    }
    table.push(']');
    fs::write(out.join("editor.rs"), table).expect("the build's own folder can be written");
}

/// Every file under `folder`, named from `dist` with a `/` between folders.
fn list(folder: &Path, under: &str, names: &mut Vec<String>) {
    let entries = fs::read_dir(folder).unwrap_or_else(|error| panic!("{}: {error}", folder.display()));
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if under.is_empty() && LEFT_OUT.contains(&name.as_str()) {
            continue;
        }
        let named = format!("{under}{name}");
        match entry.path().is_dir() {
            true => list(&entry.path(), &format!("{named}/"), names),
            false => names.push(named),
        }
    }
}

/// `bytes` in brotli, as small as it goes: the build does this once, and every browser fetches the result.
fn brotli(bytes: &[u8]) -> Vec<u8> {
    let mut small = Vec::new();
    let mut writer = brotli::CompressorWriter::new(&mut small, 1 << 16, 11, 24);
    writer.write_all(bytes).expect("memory takes what is written");
    drop(writer);
    small
}
