//! Document to PDF, on the pages Print view shows.
//!
//! Nothing here sets type: layout breaks the lines and the pages, and `letters_render::pdf` writes what
//! layout paints. So a PDF and the screen cannot disagree about where a line or a page ends.

use letters_layout::{DocLayout, Fonts, Palette, Token};
use letters_model::{BlockKind, Document, Style};
use letters_render::image::Images;
use letters_render::pdf::{Heading, Info, Link, Pdf, PdfError, Target};
use letters_render::{DisplayList, Item, Rect, Size};
use yewi_highlight::tokenize;

use crate::html::{href, ink, lexer_name};

/// Logical pixels in a point: 96 to the inch against 72.
const PIXELS: f32 = 96.0 / 72.0;

/// Writes a PDF of the document on the paper `style` sets up, in `fonts`. `title` names it, as the file's
/// name does in the editor.
///
/// - Text stays text: it can be searched and copied, and its fonts are embedded as subsets.
/// - Links work, headings become the outline a reader shows beside the pages, and code is highlighted.
/// - A link that would run script is left as its text alone.
/// - An unknown block is shown as a placeholder that names its type, as it is on screen.
pub fn to_pdf(doc: &Document, style: &Style, fonts: Fonts, title: &str) -> Result<Vec<u8>, PdfError> {
    let page = &style.page;
    let mut layout = DocLayout::new(fonts);
    layout.update(doc, style, (page.width - page.margin_left - page.margin_right) * PIXELS, None);
    for block in doc.walk() {
        if let BlockKind::Code { language } = &block.kind {
            layout.set_tokens(&block.id, tokens(language.as_deref(), &block.plain_text()));
        }
    }
    let pages = layout.paginate(page);
    let sheet = Size { width: pages.width, height: pages.height };

    // Layout draws no images yet. When it does, the document's assets are decoded into here.
    let images = Images::default();
    let mut pdf = Pdf::new();
    for index in 0..pages.sheets.len() {
        let mut list = DisplayList::default();
        // A sheet is painted as ink alone, so the paper goes under it first.
        let paper = Rect::new(0.0, 0.0, sheet.width, sheet.height);
        list.items.push(Item::Fill { rect: paper, color: Palette::LIGHT.page, radius: 0.0 });
        layout.paint_page(&pages, index, &mut list);
        let links = layout.page_links(&pages, index).into_iter().filter_map(|link| {
            let address = href(&link.target)?;
            Some(Link { rect: link.rect, target: Target::Url(address) })
        });
        pdf.page(sheet, &list, &images, &links.collect::<Vec<_>>());
    }

    let headings = layout.outline(&pages).into_iter();
    let outline = headings.map(|of| Heading { title: of.title, level: of.level, page: of.page, at: of.at }).collect();
    // The people who wrote a document are in its history, which this function is not given.
    // Nor is it given the time, since the core has no clock. Both are the shell's to pass in.
    pdf.finish(&Info { title: title.into(), language: doc.language.clone(), outline, ..Info::default() })
}

/// The coloured stretches of a code block, as layout takes them.
fn tokens(language: Option<&str>, source: &str) -> Vec<Token> {
    let mut at = 0;
    let lexed = tokenize(source, lexer_name(language)).into_iter().filter_map(|token| {
        let range = at..at + token.text.len();
        at = range.end;
        Some(Token { range, kind: ink(token.kind?) })
    });
    lexed.collect()
}
