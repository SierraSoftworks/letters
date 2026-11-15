//! Document to PDF, on the pages Print view shows.
//!
//! Nothing here sets type: layout breaks the lines and the pages, and `letters_render::pdf` writes what
//! layout paints. So a PDF and the screen cannot disagree about where a line or a page ends.

use std::collections::BTreeSet;

use letters_layout::{DocLayout, Fonts, Palette, Token, Uncovered, WordList};
use letters_model::{BlockKind, Document, Style};
use letters_render::image::{Image, Images};
use letters_render::pdf::{Heading, Info, Link, Pdf, PdfError, Target};
use letters_render::{DisplayList, Item, Rect, Size};
use yewi_highlight::tokenize;

use crate::html::{Assets, href, ink, lexer_name};

/// Logical pixels in a point: 96 to the inch against 72.
const PIXELS: f32 = 96.0 / 72.0;

/// What a PDF says of itself that the document does not hold. The shell knows these: the core has no
/// clock, and the people who wrote a document are in its file's history.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PdfInfo {
    /// Names the document, as the file's name does in the editor.
    pub title: String,
    pub authors: Vec<String>,
    /// When the PDF is written, in seconds since 1970 began in UTC.
    pub created: Option<u64>,
}

/// Finds faces of the system for characters that none of the fonts has: emoji, and scripts the fonts do not
/// cover. The core reads no files, so the shell does. It is asked once at most, with every such character
/// of the document, and a face it returns only backs the fonts up, as
/// [`DocLayout::add_fallback_fonts`] says.
pub type Backups<'a> = &'a mut dyn FnMut(&[Uncovered]) -> Fonts;

/// Writes a PDF of the document on the paper `style` sets up, in `fonts`. `assets` supplies the images the
/// pages show, and `info` what the PDF says of itself.
///
/// - Text stays text: it can be searched and copied, and its fonts are embedded as subsets.
/// - Links work, headings become the outline a reader shows beside the pages, and code is highlighted.
/// - A link that would run script is left as its text alone.
/// - An image that `assets` lacks, or that cannot be decoded, leaves its place empty.
/// - An unknown block is shown as a placeholder that names its type, as it is on screen.
/// - A character that none of the fonts has is shown as a box. [`to_pdf_with`] takes faces for those.
pub fn to_pdf(
    doc: &Document,
    style: &Style,
    fonts: Fonts,
    assets: Assets,
    info: &PdfInfo,
) -> Result<Vec<u8>, PdfError> {
    to_pdf_with(doc, style, fonts, assets, info, &mut |_| Fonts::default())
}

/// Finds the word list of a script that is written without spaces, as [`WordList::data`] gives it, so that
/// its lines end between words. It is asked once at most for each such script of the document, and only
/// where the lists are not built in, as [`DocLayout::add_word_list`] says.
pub type Words<'a> = &'a mut dyn FnMut(WordList) -> Option<Vec<u8>>;

/// Writes a PDF as [`to_pdf`] does, with the faces `backups` finds for what the fonts lack, so that a page
/// shows the emoji and scripts the window shows. A PDF carries its fonts, so these are embedded too.
pub fn to_pdf_with(
    doc: &Document,
    style: &Style,
    fonts: Fonts,
    assets: Assets,
    info: &PdfInfo,
    backups: Backups,
) -> Result<Vec<u8>, PdfError> {
    to_pdf_with_words(doc, style, fonts, assets, info, backups, &mut |_| None)
}

/// Writes a PDF as [`to_pdf_with`] does, with the lists `words` finds, so that a line ends where the
/// window ends it. A browser fetches these, and every other build has them.
pub fn to_pdf_with_words(
    doc: &Document,
    style: &Style,
    fonts: Fonts,
    assets: Assets,
    info: &PdfInfo,
    backups: Backups,
    words: Words,
) -> Result<Vec<u8>, PdfError> {
    let page = &style.page;
    let mut layout = DocLayout::new(fonts);
    layout.update(doc, style, (page.width - page.margin_left - page.margin_right) * PIXELS, None);
    for block in doc.walk() {
        if let BlockKind::Code { language } = &block.kind {
            layout.set_tokens(&block.id, tokens(language.as_deref(), &block.plain_text()));
        }
    }
    let mut pages = layout.paginate(page);
    // Paging lays out every block, so each character that no face has is known by now.
    let lacking = layout.take_uncovered();
    let mut again = !lacking.is_empty() && layout.add_fallback_fonts(&backups(&lacking));
    for list in layout.take_unlisted() {
        again |= words(list).is_some_and(|data| layout.add_word_list(list, &data));
    }
    if again {
        pages = layout.paginate(page);
    }
    let sheet = Size { width: pages.width, height: pages.height };

    // Each image is read and decoded once, when the first page that shows it is painted.
    let (mut images, mut asked) = (Images::default(), BTreeSet::new());
    let mut pdf = Pdf::new();
    for index in 0..pages.sheets.len() {
        let mut list = DisplayList::default();
        // A sheet is painted as ink alone, so the paper goes under it first.
        let paper = Rect::new(0.0, 0.0, sheet.width, sheet.height);
        list.items.push(Item::Fill { rect: paper, color: Palette::LIGHT.page, radius: 0.0 });
        layout.paint_page(&pages, index, &mut list);
        for item in &list.items {
            if let Item::Image { asset, .. } = item
                && asked.insert(asset.clone())
                && let Some(image) = assets(asset).and_then(|bytes| Image::decode(&bytes).ok())
            {
                images.insert(asset.clone(), image);
            }
        }
        let links = layout.page_links(&pages, index).into_iter().filter_map(|link| {
            // A reference leads to its figure, and any other link out of the document.
            let target = match link.place {
                Some((page, at)) => Target::Place { page, at },
                None => Target::Url(href(&link.target)?),
            };
            Some(Link { rect: link.rect, target, part: link.part })
        });
        pdf.page(sheet, &list, &images, &links.collect::<Vec<_>>());
    }

    let headings = layout.outline(&pages).into_iter();
    let outline = headings.map(|of| Heading { title: of.title, level: of.level, page: of.page, at: of.at }).collect();
    pdf.finish(&Info {
        title: info.title.clone(),
        authors: info.authors.clone(),
        language: doc.language.clone(),
        created: info.created,
        outline,
    })
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
