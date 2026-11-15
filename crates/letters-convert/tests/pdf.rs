//! PDF export: what the shell passes in reaches the file. The pages themselves are tested through the CLI.

use hayro::hayro_syntax::Pdf;
use letters_convert::{PdfInfo, to_pdf};
use letters_layout::Fonts;
use letters_model::{Block, BlockKind, Document, Style};

fn export(info: &PdfInfo) -> Pdf {
    let doc = Document { blocks: vec![Block::text("a", BlockKind::Paragraph, "Rates")], ..Document::default() };
    let fonts = Fonts { faces: letters_fonts::bundled() };
    let bytes = to_pdf(&doc, &Style::default(), fonts, &mut |_| None, info).expect("the document exports");
    Pdf::new(bytes).expect("a reader can open the PDF")
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
