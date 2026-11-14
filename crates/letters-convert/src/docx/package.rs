//! The zip a `.docx` is: its parts, and the relationships that name them.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::{Cursor, Read};

use roxmltree::{Document, Node};
use zip::ZipArchive;

use super::DocxError;

/// No part of a sound document comes near this, and a zip can claim to hold far more than it weighs.
const PART_LIMIT: u64 = 256 << 20;

/// WordprocessingML, as Word writes it and as the strict form of the standard names it.
const W: [&str; 2] = [
    "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    "http://purl.oclc.org/ooxml/wordprocessingml/main",
];

pub(super) struct Package<'a> {
    zip: ZipArchive<Cursor<&'a [u8]>>,
}

impl<'a> Package<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self, DocxError> {
        if bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0]) {
            return Err(DocxError::Legacy);
        }
        let zip = ZipArchive::new(Cursor::new(bytes)).map_err(|error| DocxError::Damaged(error.to_string()))?;
        Ok(Self { zip })
    }

    /// Reads a part by name. `None` when the file lacks it.
    pub fn bytes(&mut self, name: &str) -> Result<Option<Vec<u8>>, DocxError> {
        // Part names compare without case.
        let index = self.zip.index_for_name(name).or_else(|| {
            let mut names = self.zip.file_names().filter_map(Result::ok);
            let other = names.find(|other| other.eq_ignore_ascii_case(name))?.into_owned();
            self.zip.index_for_name(&other)
        });
        let Some(index) = index else { return Ok(None) };
        let damaged = |error: &dyn Display| DocxError::Damaged(format!("{name}: {error}"));
        let file = self.zip.by_index(index).map_err(|error| damaged(&error))?;
        let mut bytes = Vec::new();
        file.take(PART_LIMIT + 1).read_to_end(&mut bytes).map_err(|error| damaged(&error))?;
        if bytes.len() as u64 > PART_LIMIT {
            return Err(DocxError::TooLarge(name.into()));
        }
        Ok(Some(bytes))
    }

    /// Reads an XML part as text. `None` when the file lacks it.
    pub fn text(&mut self, name: &str) -> Result<Option<String>, DocxError> {
        let Some(bytes) = self.bytes(name)? else { return Ok(None) };
        let wide = |order: fn([u8; 2]) -> u16| {
            let units = bytes[2..].as_chunks::<2>().0.iter().map(|pair| order(*pair));
            char::decode_utf16(units).map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER)).collect::<String>()
        };
        let text = match bytes.as_slice() {
            [0xff, 0xfe, ..] => wide(u16::from_le_bytes),
            [0xfe, 0xff, ..] => wide(u16::from_be_bytes),
            _ => String::from_utf8(bytes)
                .map_err(|_| DocxError::Xml { part: name.into(), message: "it is not UTF-8".into() })?,
        };
        Ok(Some(text))
    }

    /// The relationships of a part, or of the package itself for the empty name.
    pub fn rels(&mut self, part: &str) -> Result<Rels, DocxError> {
        let (folder, file) = part.rsplit_once('/').map_or(("", part), |(folder, file)| (folder, file));
        let name = [folder, "_rels", &format!("{file}.rels")].iter().filter(|step| !step.is_empty()).fold(
            String::new(),
            |mut path, step| {
                if !path.is_empty() {
                    path.push('/');
                }
                path.push_str(step);
                path
            },
        );
        let Some(text) = self.text(&name)? else { return Ok(Rels::default()) };
        let xml = parse(&name, &text)?;
        let mut rels = BTreeMap::new();
        for rel in xml.root_element().children().filter(|node| node.tag_name().name() == "Relationship") {
            let (Some(id), Some(target)) = (rel.attribute("Id"), rel.attribute("Target")) else { continue };
            let external = rel.attribute("TargetMode").is_some_and(|mode| mode.eq_ignore_ascii_case("external"));
            let kind = rel.attribute("Type").unwrap_or_default().rsplit('/').next().unwrap_or_default();
            let target = if external { target.to_owned() } else { resolve(part, target) };
            rels.insert(id.to_owned(), Rel { kind: kind.to_owned(), target, external });
        }
        Ok(Rels(rels))
    }
}

/// Parses a part, naming it in the error if it is not XML.
pub(super) fn parse<'t>(part: &str, text: &'t str) -> Result<Document<'t>, DocxError> {
    Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| DocxError::Xml { part: part.into(), message: error.to_string() })
}

/// The part a relationship of `part` points at, as the zip names it.
fn resolve(part: &str, target: &str) -> String {
    if let Some(rooted) = target.strip_prefix('/') {
        return rooted.to_owned();
    }
    let mut path: Vec<&str> = part.split('/').collect();
    path.pop();
    for step in target.split('/') {
        match step {
            "" | "." => {}
            ".." => _ = path.pop(),
            step => path.push(step),
        }
    }
    path.join("/")
}

/// Where one relationship leads.
pub(super) struct Rel {
    /// The last step of the relationship's type, such as `image` or `hyperlink`.
    pub kind: String,
    /// A part's name, or an address outside the file.
    pub target: String,
    pub external: bool,
}

/// The relationships of one part, by id.
#[derive(Default)]
pub(super) struct Rels(BTreeMap<String, Rel>);

impl Rels {
    pub fn get(&self, id: &str) -> Option<&Rel> {
        self.0.get(id)
    }

    /// The first part this one names of a kind.
    pub fn part(&self, kind: &str) -> Option<&str> {
        self.0.values().find(|rel| rel.kind == kind && !rel.external).map(|rel| rel.target.as_str())
    }

    /// Every part this one names of a kind.
    pub fn parts<'r>(&'r self, kind: &'r str) -> impl Iterator<Item = &'r str> {
        self.0.values().filter(move |rel| rel.kind == kind && !rel.external).map(|rel| rel.target.as_str())
    }
}

/// The few questions the readers ask of an element.
pub(super) trait Element<'a, 'input>: Sized {
    /// Whether this is the WordprocessingML element of this name.
    fn is(&self, name: &str) -> bool;
    /// The element's name, when it is WordprocessingML.
    fn word(&self) -> Option<&'a str>;
    /// The first WordprocessingML child of this name.
    fn child(&self, name: &str) -> Option<Self>;
    /// Every WordprocessingML child of this name.
    fn all(&self, name: &'static str) -> impl Iterator<Item = Self>;
    /// An attribute by its local name, whatever its prefix.
    fn attr(&self, name: &str) -> Option<&'a str>;
    /// The `val` attribute.
    fn val(&self) -> Option<&'a str> {
        self.attr("val")
    }
    /// An on/off property: set unless its value says otherwise.
    fn on(&self) -> bool {
        !matches!(self.val(), Some("0" | "false" | "off" | "none"))
    }
    /// The value of a child, such as the `X` of `<w:pStyle w:val="X"/>`.
    fn value(&self, child: &str) -> Option<&'a str> {
        self.child(child).and_then(|child| child.val())
    }
    /// A child's whole-number value.
    fn number<T: std::str::FromStr>(&self, child: &str) -> Option<T> {
        self.value(child).and_then(|value| value.trim().parse().ok())
    }
}

impl<'a, 'input> Element<'a, 'input> for Node<'a, 'input> {
    fn is(&self, name: &str) -> bool {
        self.word() == Some(name)
    }

    fn word(&self) -> Option<&'a str> {
        let tag = self.tag_name();
        tag.namespace().is_some_and(|namespace| W.contains(&namespace)).then(|| tag.name())
    }

    fn child(&self, name: &str) -> Option<Self> {
        self.children().find(|child| child.is(name))
    }

    fn all(&self, name: &'static str) -> impl Iterator<Item = Self> {
        self.children().filter(move |child| child.is(name))
    }

    fn attr(&self, name: &str) -> Option<&'a str> {
        self.attributes().find(|attribute| attribute.name() == name).map(|attribute| attribute.value())
    }
}
