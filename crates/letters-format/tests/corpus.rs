//! The conformance corpus under `corpus/`, checked against both the published schemas and this build's readers.
//!
//! Each kind has canonical files, which Letters writes byte for byte, an `accepted.json` of looser forms every
//! reader must take, and a `refused.json` of breaks every reader must refuse. `report.let` is a whole file built
//! from the canonical ones. After changing one of them, rebuild it with
//! `cargo test -p letters-format --test corpus -- --ignored`.

mod common;

use std::path::{Path, PathBuf};

use jsonschema::{Retrieve, Uri, Validator};
use letters_format::schema;
use letters_model::{Document, JsonError, Notes, Style};
use serde_json::Value;

/// One public JSON entry: its schema, and this build's reader as "read, then write canonically".
struct Kind {
    name: &'static str,
    schema: &'static str,
    canonical: fn(&str) -> Result<String, JsonError>,
}

const KINDS: [Kind; 3] = [
    Kind { name: "document", schema: schema::DOCUMENT, canonical: |json| Ok(Document::from_json(json)?.to_json()) },
    Kind { name: "style", schema: schema::STYLE, canonical: |json| Ok(Style::from_json(json)?.to_json()) },
    Kind { name: "notes", schema: schema::NOTES, canonical: |json| Ok(Notes::from_json(json)?.to_json()) },
];

/// Resolves a reference from one schema to another without leaving the repository.
struct Published;

impl Retrieve for Published {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let all = [schema::MANIFEST, schema::DOCUMENT, schema::STYLE, schema::NOTES];
        let found = all.iter().map(|text| parse(text)).find(|schema| schema["$id"] == uri.as_str());
        found.ok_or_else(|| format!("{uri} is not a published schema").into())
    }
}

fn parse(json: &str) -> Value {
    serde_json::from_str(json).unwrap()
}

fn validator(schema: &str) -> Validator {
    jsonschema::options().with_retriever(Published).build(&parse(schema)).unwrap()
}

fn corpus(kind: &Kind) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus").join(kind.name)
}

fn report(kind: &Kind) -> String {
    std::fs::read_to_string(corpus(kind).join("report.json")).unwrap()
}

/// The cases listed in a kind's `accepted.json` or `refused.json`.
fn cases(kind: &Kind, list: &str) -> Vec<Value> {
    let cases = parse(&std::fs::read_to_string(corpus(kind).join(list)).unwrap());
    cases.as_array().unwrap().clone()
}

/// The schema's complaints about a value, empty when it is valid.
fn complaints(validator: &Validator, value: &Value) -> Vec<String> {
    validator.iter_errors(value).map(|error| format!("{}: {error}", error.instance_path())).collect()
}

#[test]
fn the_schemas_are_themselves_valid() {
    for text in [schema::MANIFEST, schema::DOCUMENT, schema::STYLE, schema::NOTES] {
        let schema = parse(text);
        assert!(jsonschema::meta::is_valid(&schema), "{}", schema["$id"]);
        validator(text);
    }
}

#[test]
fn canonical_files_validate_and_are_written_back_byte_for_byte() {
    for kind in KINDS {
        let validator = validator(kind.schema);
        let mut seen = 0;
        for entry in std::fs::read_dir(corpus(&kind)).unwrap() {
            let path = entry.unwrap().path();
            if ["accepted.json", "refused.json"].iter().any(|list| path.ends_with(list)) {
                continue;
            }
            let json = std::fs::read_to_string(&path).unwrap();
            assert_eq!(complaints(&validator, &parse(&json)), Vec::<String>::new(), "{}", path.display());
            assert_eq!((kind.canonical)(&json).unwrap(), json, "{}", path.display());
            seen += 1;
        }
        assert!(seen > 0, "no canonical {} files", kind.name);
    }
}

#[test]
fn looser_forms_are_accepted_by_schema_and_reader() {
    for kind in KINDS {
        let validator = validator(kind.schema);
        for case in cases(&kind, "accepted.json") {
            let why = format!("{}: {}", kind.name, case["why"]);
            assert_eq!(complaints(&validator, &case["json"]), Vec::<String>::new(), "{why}");
            let canonical =
                (kind.canonical)(&case["json"].to_string()).unwrap_or_else(|error| panic!("{why}: {error}"));
            // What this build writes back is valid too, and settles: writing it again changes nothing.
            assert_eq!(complaints(&validator, &parse(&canonical)), Vec::<String>::new(), "{why}");
            assert_eq!((kind.canonical)(&canonical).unwrap(), canonical, "{why}");
        }
    }
}

#[test]
fn breaks_are_refused_by_schema_and_reader() {
    for kind in KINDS {
        let validator = validator(kind.schema);
        for case in cases(&kind, "refused.json") {
            let why = format!("{}: {}", kind.name, case["why"]);
            let error = (kind.canonical)(&case["json"].to_string()).expect_err(&why);
            assert_eq!(error.path, case["at"].as_str().unwrap(), "{why}: {error}");
            // A few rules are beyond JSON Schema, and those cases say so.
            let schema_accepts = case["schema_accepts"] == true;
            assert_eq!(validator.is_valid(&case["json"]), schema_accepts, "{why}");
        }
    }
}

fn sample() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/report.let")
}

#[test]
#[ignore = "rewrites corpus/report.let from the canonical entries"]
fn rebuild_the_sample_file() {
    let mut package = common::open(&std::fs::read(sample()).unwrap()).package().unwrap();
    for kind in KINDS {
        package.entries.insert(format!("{}.json", kind.name), report(&kind).into_bytes());
    }
    package.document = Document::from_json(&report(&KINDS[0])).unwrap();
    std::fs::write(sample(), common::bytes(&package)).unwrap();
}

#[test]
fn the_sample_file_holds_the_canonical_entries() {
    let file = std::fs::read(sample()).unwrap();
    let mut archive = common::open(&file);
    for kind in KINDS {
        let entry = archive.read(&format!("{}.json", kind.name)).unwrap().unwrap();
        assert_eq!(String::from_utf8(entry).unwrap(), report(&kind), "{}", kind.name);
    }
    assert!(archive.names().any(|name| name.starts_with(letters_format::ASSETS)));
}

#[test]
fn the_manifest_of_a_written_file_validates() {
    let file = common::bytes(&common::package());
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(file)).unwrap();
    let manifest: Value = serde_json::from_reader(zip.by_name(letters_format::MANIFEST).unwrap()).unwrap();
    assert_eq!(complaints(&validator(schema::MANIFEST), &manifest), Vec::<String>::new());
}
