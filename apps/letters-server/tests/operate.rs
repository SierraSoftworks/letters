//! What whoever runs the server relies on: a log that can be read and sifted, a way to ask whether it runs,
//! and a state folder that a wrong build refuses and does not misread.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Command, Stdio};

use common::Scratch;
use letters_api::paths;
use letters_server::store::{FORMAT, Store, Table};

const PROGRAM: &str = env!("CARGO_BIN_EXE_letters-server");

/// Asks a server at `address` for `path`, and answers the status it gives.
fn status(address: SocketAddr, path: &str) -> u16 {
    let mut line = TcpStream::connect(address).expect("the server listens");
    write!(line, "GET {path} HTTP/1.1\r\nHost: letters.test\r\nConnection: close\r\n\r\n").unwrap();
    let mut answer = String::new();
    let _ = line.read_to_string(&mut answer);
    answer.split_whitespace().nth(1).and_then(|status| status.parse().ok()).expect("an answer")
}

/// Runs `letters-server` with `more` among its server settings, asks it for each of `paths` and stops
/// it: the statuses it gave, and every line of its log.
fn run(name: &str, more: &str, paths: &[&str]) -> (Vec<u16>, Vec<String>) {
    let scratch = Scratch::new(name);
    let settings = common::settings(&scratch.0, "https://id.example.test", "");
    let own = format!("name = \"Test\"\nlisten = \"127.0.0.1:0\"\n{more}");
    let file = scratch.0.join("server.toml");
    std::fs::write(&file, settings.replace("name = \"Test\"", &own)).unwrap();
    let mut program = Command::new(PROGRAM);
    program.arg("--config").arg(&file).env("LETTERS_TELEMETRY", "0").stderr(Stdio::piped());
    let mut program = program.spawn().expect("the server starts");
    let mut log = BufReader::new(program.stderr.take().unwrap()).lines().map(|line| line.expect("a line of text"));
    let mut lines = Vec::new();
    // It says where it listens once it does.
    let address = loop {
        let line = log.next().expect("the server says that it listens");
        let said = line.split_once("listening on ").map(|(_, rest)| rest.split(' ').next().unwrap().to_owned());
        lines.push(line);
        if let Some(address) = said {
            break address.parse().expect("an address");
        }
    };
    let statuses = paths.iter().map(|path| status(address, path)).collect();
    program.kill().unwrap();
    lines.extend(log);
    program.wait().unwrap();
    (statuses, lines)
}

#[test]
fn the_log_gives_each_line_its_time_and_kind_and_a_request_only_where_asked() {
    // A link nobody made, and an address the server has nothing at: each holds what a log must not keep.
    let secret = "hunter2hunter2";
    let asked = [paths::HEALTH.to_owned(), paths::link_page(secret), format!("/nowhere/{secret}")];
    let asked: Vec<&str> = asked.iter().map(String::as_str).collect();
    // What the settings say of the log, and whether requests are then written.
    let cases = [("", false), ("log = \"info\"", false), ("log = \"debug\"", true)];
    for (n, (setting, requests)) in cases.into_iter().enumerate() {
        let (statuses, lines) = run(&format!("operate-log-{n}"), setting, &asked);
        assert_eq!(statuses[0], 200, "whether it runs is told to whoever asks");
        assert_eq!(statuses[2], 404);

        for line in &lines {
            let mut parts = line.splitn(3, ' ');
            let (time, kind) = (parts.next().unwrap(), parts.next().unwrap_or_default());
            assert!(time.parse::<jiff::Timestamp>().is_ok(), "{setting}: a line starts with its time: {line}");
            assert!(["info", "warn", "debug"].contains(&kind), "{setting}: then its kind: {line}");
            assert!(!line.contains(secret), "{setting}: nothing out of an address is kept: {line}");
        }
        let kinds = |kind: &str| lines.iter().filter(|line| line.split(' ').nth(1) == Some(kind)).count();
        assert!(kinds("info") >= 1, "{setting}: {lines:?}");
        assert_eq!(kinds("debug"), if requests { asked.len() } else { 0 }, "{setting}: {lines:?}");
        if requests {
            let told = |words: &[&str]| lines.iter().any(|line| words.iter().all(|word| line.contains(word)));
            assert!(told(&["GET", paths::HEALTH, "200"]), "{lines:?}");
            assert!(told(&["GET", "/l/", "debug"]), "a link's route is named, not its link: {lines:?}");
            assert!(told(&["GET", "404"]), "{lines:?}");
        }
    }
}

#[test]
fn a_state_folder_of_a_later_build_is_refused_and_left_as_it_is() {
    let scratch = Scratch::new("operate-store");
    let (state, mark) = (scratch.0.join("state"), Table::<u32>::new("store"));
    let rows: Table<String> = Table::new("rows");
    // This build's own store opens again, with what it held.
    let store = Store::open(&state).expect("a new store opens");
    store.put(&rows, "kept", &"yes".to_owned()).unwrap();
    drop(store);
    let store = Store::open(&state).expect("its own store opens again");
    assert_eq!(store.get(&rows, "kept").unwrap().as_deref(), Some("yes"));

    // A later build marks it as its own.
    store.put(&mark, "format", &(FORMAT + 1)).unwrap();
    drop(store);
    let refusal = Store::open(&state).err().expect("a later build's store is refused").to_string();
    assert!(refusal.contains("newer") && refusal.contains(&(FORMAT + 1).to_string()), "{refusal}");
    // And still finds it so afterwards: the refusal changed nothing.
    assert!(Store::open(&state).is_err());

    // The program says as much, and does not serve.
    let settings = common::settings(&scratch.0, "https://id.example.test", "");
    let file = scratch.0.join("server.toml");
    std::fs::write(&file, settings.replace("name = \"Test\"", "name = \"Test\"\nlisten = \"127.0.0.1:0\"")).unwrap();
    let ran = Command::new(PROGRAM).arg("--config").arg(&file).env("LETTERS_TELEMETRY", "0").output().unwrap();
    assert!(!ran.status.success());
    assert!(String::from_utf8_lossy(&ran.stderr).contains("newer"), "{ran:?}");
}
