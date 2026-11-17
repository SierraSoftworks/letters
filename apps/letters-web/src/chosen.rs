//! The fonts a writer chose for a document's types, beyond the bundled ones, as a page comes by them.
//!
//! A page has no fonts of a system. What it sets text in is in the document: the font files its style
//! lists. For whoever writes a trusted document of a server, a family of the Google Fonts library is
//! fetched through that server, one face at a time, and stored in the document, so that no reader
//! fetches anything. This module only decides: the shell fetches.

use letters_fonts::{DISPLAY, MONO, SANS, library};
use letters_layout::Named;
use letters_model::StyleFont;
use sha2::{Digest, Sha256};

/// The families every page has already.
pub const BUNDLED: [&str; 3] = [SANS, MONO, DISPLAY];

/// A face of a family of the library, as the library has it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Want {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
}

/// How far something that is fetched has come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrival {
    Coming,
    Here,
    /// It could not be had, and is not asked for again.
    Lost,
}

/// What a page knows of its document's fonts.
pub struct Known<'a> {
    /// The font files the style lists.
    pub listed: &'a [StyleFont],
    /// Every face the style can ask for.
    pub faces: &'a [Named],
    /// The faces its text asked for so far.
    pub named: &'a [Named],
    /// The library's families, for whoever may fetch them and keep them with the document. `None` for
    /// one who only reads, for a document that is not trusted, and on a page of no server.
    pub library: Option<&'a str>,
    /// How far a listed font file has come, by its name. `None` for one never asked for.
    pub file: &'a dyn Fn(&str) -> Option<Arrival>,
    /// How far a face of the library has come.
    pub face: &'a dyn Fn(&Want) -> Option<Arrival>,
}

/// What is to be done about them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// The listed font files to read, each asked for now and never again.
    pub read: Vec<String>,
    /// The faces to fetch from the library.
    pub fetch: Vec<Want>,
    /// The families the text asks for that nothing has.
    pub lacking: Vec<String>,
    /// The families of the library that could not be fetched.
    pub unfetched: Vec<String>,
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn bundled(family: &str) -> bool {
    BUNDLED.iter().any(|own| same(own, family))
}

/// The face of the library that stands for `face`, when its family is of the library and not bundled.
fn wanted(library: &str, face: &Named) -> Option<Want> {
    let family = library::find_in(library, &face.family).filter(|_| !bundled(&face.family))?;
    Some(Want {
        family: family.name.to_owned(),
        weight: family.weight(face.weight),
        italic: face.italic && family.italic,
    })
}

/// Decides what to read and to fetch, and which families to give up on.
pub fn plan(known: &Known<'_>) -> Plan {
    let mut plan = Plan::default();
    let unasked = known.listed.iter().filter(|font| (known.file)(&font.asset).is_none());
    plan.read = unasked.map(|font| font.asset.clone()).collect();
    plan.read.dedup();

    // A listed file that could not be had is as good as not listed.
    let held = |font: &&StyleFont| (known.file)(&font.asset) != Some(Arrival::Lost);
    let lists = |want: &Want| {
        let it = |font: &&StyleFont| {
            same(&font.family, &want.family) && (font.weight, font.italic) == (want.weight, want.italic)
        };
        known.listed.iter().filter(held).any(|font| it(&font))
    };
    for want in known.faces.iter().filter_map(|face| wanted(known.library?, face)) {
        if !lists(&want) && (known.face)(&want).is_none() && !plan.fetch.contains(&want) {
            plan.fetch.push(want);
        }
    }

    let mut families: Vec<&str> = Vec::new();
    for named in known.named.iter().filter(|named| !bundled(&named.family)) {
        if !families.iter().any(|family| same(family, &named.family)) {
            families.push(&named.family);
        }
    }
    for family in families {
        if known.listed.iter().filter(held).any(|font| same(&font.family, family)) {
            continue;
        }
        // A family of the library is on its way, unless a face of it was lost.
        let of_library = known.library.and_then(|library| library::find_in(library, family));
        let faces = known.faces.iter().filter(|face| same(&face.family, family));
        let mut wants = faces.filter_map(|face| wanted(known.library?, face));
        match of_library {
            Some(of) if wants.any(|want| (known.face)(&want) == Some(Arrival::Lost)) => {
                plan.unfetched.push(of.name.to_owned())
            }
            Some(_) => {}
            None => plan.lacking.push(family.to_owned()),
        }
    }
    plan
}

/// What the style should list once `arrived` faces of the library are stored under these names: what it
/// lists and those, less the families no type names any more. `None` when that is what it lists.
pub fn kept(listed: &[StyleFont], faces: &[Named], arrived: &[(Want, String)]) -> Option<Vec<StyleFont>> {
    let mut list = listed.to_vec();
    for (want, asset) in arrived {
        let it = |font: &StyleFont| {
            same(&font.family, &want.family) && (font.weight, font.italic) == (want.weight, want.italic)
        };
        if !list.iter().any(it) {
            let (family, asset) = (want.family.clone(), asset.clone());
            list.push(StyleFont { family, weight: want.weight, italic: want.italic, asset });
        }
    }
    list.retain(|font| faces.iter().any(|face| same(&face.family, &font.family)));
    list.sort();
    list.dedup();
    let mut before = listed.to_vec();
    before.sort();
    (list != before).then_some(list)
}

/// The name a font file is kept under among a document's assets, which is the same for the same bytes.
pub fn asset_name(bytes: &[u8]) -> String {
    let hash: String = Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    format!("assets/{hash}.ttf")
}
