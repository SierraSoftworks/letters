//! PDF export: what the shell passes in reaches the file, and its links lead somewhere. The pages themselves
//! are tested through the CLI.

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict};
use letters_convert::{PdfInfo, PdfSources, to_pdf, to_pdf_with};
use letters_layout::{Fonts, WordList};
use letters_model::{Block, BlockId, BlockKind, Body, Document, Mark, Run, Style};

fn written(doc: &Document, info: &PdfInfo) -> Pdf {
    let fonts = Fonts { faces: letters_fonts::bundled() };
    let bytes = to_pdf(doc, &Style::default(), fonts, &mut |_| None, info).expect("the document exports");
    Pdf::new(bytes).expect("a reader can open the PDF")
}

fn fonts() -> Fonts {
    Fonts { faces: letters_fonts::bundled() }
}

/// How many pictures the first page of a PDF draws from.
fn pictures(bytes: Vec<u8>) -> usize {
    let pdf = Pdf::new(bytes).expect("a reader can open the PDF");
    pdf.pages()[0].resources().x_objects.keys().count()
}

fn export(info: &PdfInfo) -> Pdf {
    let doc = Document { blocks: vec![Block::text("a", BlockKind::Paragraph, "Rates")], ..Document::default() };
    written(&doc, info)
}

fn text(field: &Option<Box<[u8]>>) -> String {
    field.as_deref().map(|bytes| String::from_utf8_lossy(bytes).into_owned()).unwrap_or_default()
}

#[test]
fn a_pdf_says_what_the_shell_knows_of_it() {
    let authors = vec!["Ada Lovelace".to_owned(), "Grace Hopper".to_owned()];
    // The first second of 15 November 2026, in UTC.
    let pdf = export(&PdfInfo { title: "Rates".into(), authors, created: Some(1_794_700_800) });
    let about = pdf.metadata();
    assert_eq!(text(&about.title), "Rates");
    assert!(text(&about.author).contains("Ada Lovelace") && text(&about.author).contains("Grace Hopper"), "{about:?}");
    let created = about.creation_date.expect("the PDF says when it was written");
    assert_eq!((created.year, created.month, created.day), (2026, 11, 15));
}

#[test]
fn a_pdf_leaves_out_what_the_shell_does_not_know() {
    let about = export(&PdfInfo::default()).metadata().clone();
    assert_eq!((text(&about.author), about.creation_date.is_some()), (String::new(), false), "{about:?}");
}

#[test]
fn links_lead_out_of_the_document_and_references_to_their_figures() {
    let mut opening = Block::new("opening", BlockKind::Paragraph);
    let site = Run { link: Some("https://example.com/rates".into()), ..Run::plain("the rates") };
    opening.body =
        Body::Text(vec![site, Run::plain(" and "), Run::marked("Figure 1", [Mark::Ref(BlockId::new("cat"))])]);
    // Enough lines that the figure cannot be on the first sheet.
    let filler = (0..120).map(|line| Block::text(format!("line{line}").as_str(), BlockKind::Paragraph, "A line"));
    let kind =
        BlockKind::Figure { asset: None, src: None, alt: "A cat".into(), width: 80.0, height: 60.0, numbered: true };
    let blocks = [opening].into_iter().chain(filler).chain([Block::text("cat", kind, "A cat")]).collect();
    let pdf = written(&Document { blocks, ..Document::default() }, &PdfInfo::default());

    let pages = pdf.pages();
    let links = pages[0].raw().get::<Array<'_>>("Annots").expect("the first sheet has links");
    let links: Vec<Dict<'_>> = links.iter::<Dict<'_>>().collect();
    let out = links.iter().filter(|link| link.get::<Dict<'_>>("A").is_some_and(|out| out.contains_key("URI")));
    assert_eq!(out.count(), 1, "the link out of the document");
    let sheet = |link: &Dict<'_>| {
        let page = link.get::<Array<'_>>("Dest")?.iter::<Dict<'_>>().next()?.obj_id()?;
        pages.iter().position(|other| other.raw().obj_id() == Some(page))
    };
    let inner: Vec<usize> = links.iter().filter_map(sheet).collect();
    assert_eq!(inner.len(), 1, "the reference");
    assert!(inner[0] > 0, "the reference leads to the sheet its figure is on, not to its own");
}

#[test]
fn a_shell_backs_the_fonts_up_for_what_they_lack() {
    // The body's family alone, which draws no card suit. The code face does, and stands for a system's.
    let bundled = letters_fonts::bundled();
    let (body, system) = (Fonts { faces: bundled[..2].to_vec() }, Fonts { faces: bundled[2..3].to_vec() });
    let cases = [("what the fonts cover", "Rates change in January.", vec![]), ("a card suit", "♠ Spades", vec!['♠'])];
    for (name, text, lacking) in cases {
        let doc = Document { blocks: vec![Block::text("a", BlockKind::Paragraph, text)], ..Document::default() };
        let faces = |bytes: Vec<u8>| {
            let pdf = Pdf::new(bytes).expect("a reader can open the PDF");
            pdf.pages()[0].resources().fonts.keys().count()
        };
        let plain = to_pdf(&doc, &Style::default(), body.clone(), &mut |_| None, &PdfInfo::default()).unwrap();

        let mut asked = Vec::new();
        let mut backups = |lacking: &[letters_layout::Uncovered]| {
            asked.push(lacking.iter().map(|missing| missing.ch).collect::<Vec<_>>());
            system.clone()
        };
        let info = PdfInfo::default();
        let sources = PdfSources { backups: Some(&mut backups), ..PdfSources::default() };
        let backed = to_pdf_with(&doc, &Style::default(), body.clone(), &mut |_| None, &info, sources).unwrap();
        let expected: Vec<Vec<char>> = if lacking.is_empty() { vec![] } else { vec![lacking.clone()] };
        assert_eq!(asked, expected, "{name}: asked once, for what is lacking alone");
        assert_eq!(faces(backed) - faces(plain), lacking.len(), "{name}: the face that draws it is embedded");
    }
}

#[test]
fn a_shell_finds_the_word_list_of_a_script_written_without_spaces() {
    let fonts = || Fonts { faces: letters_fonts::bundled() };
    let (style, info) = (Style::default(), PdfInfo::default());
    let thai = "ภาษาไทยเขียนติดกันโดยไม่เว้นวรรคระหว่างคำ".repeat(8);
    for (name, text, lists) in [("Latin", "Rates change in January.", vec![]), ("Thai", &*thai, vec![WordList::Thai])] {
        let doc = Document { blocks: vec![Block::text("a", BlockKind::Paragraph, text)], ..Document::default() };
        let plain = to_pdf(&doc, &style, fonts(), &mut |_| None, &info).unwrap();

        let mut asked = Vec::new();
        let mut words = |list: WordList| {
            asked.push(list);
            Some(list.data())
        };
        let sources = PdfSources { words: Some(&mut words), ..PdfSources::default() };
        let listed = to_pdf_with(&doc, &style, fonts(), &mut |_| None, &info, sources);
        // A build that has the lists asks for none, and its lines end between words anyway.
        let lacking = if WordList::BUILT_IN { vec![] } else { lists };
        assert_eq!(asked, lacking, "{name}: asked once, for the list of a script it shows");
        assert_eq!(listed.unwrap() != plain, !lacking.is_empty(), "{name}: the lines end where the list says");
    }
}

/// A picture four pixels wide and two high, as a file a link led to.
const PICTURE: [u8; 73] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x04, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xf0, 0xca, 0xea, 0x34, 0x00, 0x00, 0x00, 0x10, 0x49,
    0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x38, 0x61, 0x23, 0x02, 0x47, 0x0c, 0xc8, 0x1c, 0x00, 0x77, 0x7a, 0x08, 0xc1,
    0xe9, 0x61, 0x87, 0x97, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
fn a_linked_picture_is_drawn_when_the_shell_has_loaded_it() {
    let linked = |id: &str, src: &str| {
        let src = Some(src.to_owned());
        Block::new(
            id,
            BlockKind::Figure { asset: None, src, alt: String::new(), width: 80.0, height: 60.0, numbered: false },
        )
    };
    let (near, far) = ("https://example.com/near.png", "https://example.com/far.png");
    // The same picture twice, and one the shell never loaded.
    let blocks = vec![linked("a", near), linked("b", near), linked("c", far)];
    let doc = Document { blocks, ..Document::default() };
    let (style, info) = (Style::default(), PdfInfo::default());
    let plain = to_pdf(&doc, &style, fonts(), &mut |_| None, &info).unwrap();
    assert_eq!(pictures(plain), 0, "an export fetches nothing, so a link is an empty frame");

    let mut asked = Vec::new();
    let mut loaded = |address: &str| {
        asked.push(address.to_owned());
        (address == near).then(|| PICTURE.to_vec())
    };
    let sources = PdfSources { linked: Some(&mut loaded), ..PdfSources::default() };
    let shown = to_pdf_with(&doc, &style, fonts(), &mut |_| None, &info, sources).unwrap();
    assert_eq!(asked, [near, far], "each address is asked for once");
    assert_eq!(pictures(shown), 1, "the picture the shell has is in the PDF, once for both places");
}

#[test]
fn a_link_preview_shows_its_picture() {
    let card = |id: &str| {
        let kind = BlockKind::LinkPreview {
            url: "https://example.com/rates".into(),
            title: Some("Rates".into()),
            description: None,
            image: Some("assets/card.png".into()),
        };
        Block::new(id, kind)
    };
    let doc = Document { blocks: vec![card("a"), card("b")], ..Document::default() };
    let mut asked = 0;
    let mut assets = |name: &str| {
        asked += 1;
        (name == "assets/card.png").then(|| PICTURE.to_vec())
    };
    let shown = to_pdf(&doc, &Style::default(), fonts(), &mut assets, &PdfInfo::default()).unwrap();
    assert_eq!(pictures(shown), 1, "the picture is in the PDF, once for both cards");
    assert_eq!(asked, 1, "and is read once");
}
