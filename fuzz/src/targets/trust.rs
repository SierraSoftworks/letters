//! The settings file that says what is trusted, and the addresses a document names: each is read or
//! refused, and what the app writes of them it reads back as the same.

use std::io;

use letters_trust::{HttpsUrl, Limits, Origin, Source, Store, Trust, TrustStore};

use crate::input::text;

/// A settings file in memory.
struct File(Option<String>);

impl Store for File {
    fn read(&mut self) -> io::Result<Option<String>> {
        Ok(self.0.clone())
    }
    fn write(&mut self, text: &str) -> io::Result<()> {
        self.0 = Some(text.to_owned());
        Ok(())
    }
}

pub fn run(data: &[u8]) {
    let text = text(data);
    let lines: Vec<&str> = text.lines().take(6).collect();
    let mut sources = Vec::new();
    for line in &lines {
        if let Ok(url) = HttpsUrl::parse(line) {
            assert_eq!(HttpsUrl::parse(&url.to_string()).as_ref(), Ok(&url), "an address is written as another");
            let _ = url.is_public();
        }
        if let Ok(source) = Source::parse(line) {
            assert_eq!(Source::parse(&source.to_string()).as_ref(), Ok(&source), "a source is written as another");
            sources.push(source);
        }
    }

    const HOME: &str = "/home/ana";
    if let Ok(trust) = Trust::parse(&text, Some(HOME)) {
        for origin in [Origin::Created, Origin::File("/home/ana/notes/a.let"), Origin::Outside] {
            let gate = trust.gate("document", origin).on_server(&sources);
            for line in &lines {
                let _ = (gate.image(line), gate.link(line), gate.merge(line), gate.maths_links());
                let _ = gate.fetch_on_request(line, Limits::default());
            }
        }
    }

    // Whatever the file holds, what the app adds to it is trusted from then on and can be taken back.
    let mut store = TrustStore::new(File(Some(text.to_string())), Some(HOME));
    if store.load().is_ok() {
        if store.trust_document("document").is_ok() {
            assert!(store.trust().trusts_document("document"), "a document that was trusted is not");
            store.forget_document("document").expect("a file that was just written is written again");
            assert!(!store.trust().trusts_document("document"), "a document that was forgotten is trusted");
        }
        for source in &sources {
            if store.trust_source(source).is_ok() {
                assert!(store.trust().sources().contains(source), "a source that was trusted is not");
            }
        }
        for line in &lines {
            if store.trust_folder(line).is_ok() {
                assert!(store.trust().trusts_folder(line), "a folder that was trusted is not");
            }
        }
    }
}

pub fn seeds() -> Vec<Vec<u8>> {
    let file = concat!(
        "# What Letters may load.\n[trust]\n",
        "documents = [\"8f14e45f-ceea-467f-a0e6-5b1d3c2a9f00\"]\n",
        "folders = [\"~/notes\", \"/srv/shared docs/\", 'C:\\Users\\ana']\n",
        "sources = [\"example.com\", \"https://cdn.example.com:8443/img/\"]\n",
    );
    let addresses = concat!(
        "https://example.com/a.png?x=1#f\nhttps://user@example.com/\nhttp://example.com\nhttps://[::1]:8443/a\n",
        "https://192.168.0.1/\nexample.com/img/\n~/notes\n/home/ana/notes/a.let\n",
    );
    vec![file.into(), addresses.into()]
}
