//! PDF export: what the shell passes in reaches the file, and its links lead somewhere. The pages themselves
//! are tested through the CLI.

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict};
use letters_convert::{PdfInfo, to_pdf, to_pdf_with};
use letters_layout::Fonts;
use letters_model::{Block, BlockId, BlockKind, Body, Document, Mark, Run, Style};

fn written(doc: &Document, info: &PdfInfo) -> Pdf {
    let fonts = Fonts { faces: letters_fonts::bundled() };
    let bytes = to_pdf(doc, &Style::default(), fonts, &mut |_| None, info).expect("the document exports");
    Pdf::new(bytes).expect("a reader can open the PDF")
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
        let backed = to_pdf_with(&doc, &Style::default(), body.clone(), &mut |_| None, &info, &mut backups).unwrap();
        let expected: Vec<Vec<char>> = if lacking.is_empty() { vec![] } else { vec![lacking.clone()] };
        assert_eq!(asked, expected, "{name}: asked once, for what is lacking alone");
        assert_eq!(faces(backed) - faces(plain), lacking.len(), "{name}: the face that draws it is embedded");
    }
}
