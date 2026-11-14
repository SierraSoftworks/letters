//! The PDF backend, read back the way a PDF reader would.

mod common;

use std::sync::Arc;

use common::*;
use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image as Drawn, ImageDrawProps, InterpreterCache,
    InterpreterSettings, SoftMask, interpret_page,
};
use hayro::hayro_syntax::Pdf as Read;
use hayro::hayro_syntax::object::{Array, Dict, Object, Rect as Corners, String as Bytes};
use hayro::hayro_syntax::page::Page;
use hayro::kurbo::{Affine, BezPath};
use hayro::vello_cpu::color::palette::css;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use letters_render::image::Images;
use letters_render::pdf::{Heading, Info, Link, Pdf, PdfError, Target};
use letters_render::{Color, DisplayList, Item, Point, Rect, RunText, Size};

/// The scenes' size in logical pixels, and in the points a PDF measures in.
const SHEET: Size = Size { width: 240.0, height: 160.0 };
const POINTS: (f32, f32) = (180.0, 120.0);

/// A PDF with a page of [`SHEET`]'s size for each list.
fn written(pages: &[(DisplayList, Vec<Link>)], images: &Images, info: &Info) -> Result<Vec<u8>, PdfError> {
    let mut pdf = Pdf::new();
    for (list, links) in pages {
        pdf.page(SHEET, list, images, links);
    }
    pdf.finish(info)
}

fn read(bytes: Vec<u8>) -> Read {
    Read::new(bytes).expect("a reader can open the PDF")
}

/// A page as a reader shows it, a pixel to a logical pixel.
fn shown(page: &Page<'_>) -> Vec<[u8; 4]> {
    let (width, height) = page.render_dimensions();
    let scale = PixmapSettings { x_scale: SHEET.width / width, y_scale: SHEET.height / height, bg_color: css::WHITE };
    let settings = RenderSettings { hinting: false, ..RenderSettings::default() };
    let pixmap = hayro::render(page, &RenderCache::new(), &InterpreterSettings::default(), &settings, &scale);
    pixmap.data().iter().map(|pixel| [pixel.r, pixel.g, pixel.b, pixel.a]).collect()
}

/// Collects the text of a page in the order it is drawn.
#[derive(Default)]
struct Reader(String);

impl<'a> Device<'a> for Reader {
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {
        for glyph in run.glyphs() {
            match glyph.as_unicode() {
                Some(BfString::Char(character)) => self.0.push(character),
                Some(BfString::String(text)) => self.0.push_str(&text),
                None => {}
            }
        }
    }

    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Drawn<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// The text a reader finds on a page when it is searched or copied.
fn text_of(page: &Page<'_>) -> String {
    let cache = InterpreterCache::default();
    let bounds = hayro::kurbo::Rect::new(0.0, 0.0, 1e4, 1e4);
    let mut context = Context::new(Affine::IDENTITY, bounds, &cache, page.xref(), InterpreterSettings::default());
    let mut reader = Reader::default();
    interpret_page(page, &mut context, &mut reader);
    reader.0
}

fn string(dict: &Dict<'_>, key: &str) -> Option<String> {
    let bytes = dict.get::<Bytes<'_>>(key)?;
    let bytes = bytes.as_bytes();
    // A PDF writes text that is not plain ASCII as UTF-16 after a byte order mark.
    Some(match bytes.strip_prefix(&[0xFE, 0xFF]) {
        Some(wide) => {
            let units: Vec<u16> = wide.as_chunks::<2>().0.iter().map(|pair| u16::from_be_bytes(*pair)).collect();
            String::from_utf16_lossy(&units)
        }
        None => String::from_utf8_lossy(bytes).into_owned(),
    })
}

/// The index of the page a destination leads to, and the point on it from the page's bottom left.
fn destination(read: &Read, dict: &Dict<'_>, key: &str) -> Option<(usize, f32, f32)> {
    let place = dict.get::<Array<'_>>(key)?;
    let mut parts = place.flex_iter();
    let page = parts.next::<Dict<'_>>()?.obj_id()?;
    let index = read.pages().iter().position(|other| other.raw().obj_id() == Some(page))?;
    let _kind = parts.next::<Object<'_>>()?;
    Some((index, parts.next::<f32>()?, parts.next::<f32>()?))
}

/// The outline's titles with the page each leads to, indented by a space for each level below the first.
fn outline_of(read: &Read) -> Vec<(String, usize)> {
    fn walk(read: &Read, parent: &Dict<'_>, depth: usize, out: &mut Vec<(String, usize)>) {
        let mut next = parent.get::<Dict<'_>>("First");
        while let Some(entry) = next {
            let title = string(&entry, "Title").unwrap_or_default();
            let page = destination(read, &entry, "Dest").map_or(usize::MAX, |(page, ..)| page);
            out.push((format!("{}{title}", " ".repeat(depth)), page));
            walk(read, &entry, depth + 1, out);
            next = entry.get::<Dict<'_>>("Next");
        }
    }
    let mut out = Vec::new();
    let root = read.xref().get::<Dict<'_>>(read.xref().root_id()).expect("a PDF has a catalogue");
    if let Some(outline) = root.get::<Dict<'_>>("Outlines") {
        walk(read, &outline, 0, &mut out);
    }
    out
}

fn on_white(items: impl IntoIterator<Item = Item>) -> DisplayList {
    let mut all = vec![fill(0.0, 0.0, SHEET.width, SHEET.height, WHITE)];
    all.extend(items);
    list(all)
}

/// A line set as [`text`] sets it, a glyph to a character, said to spell `said` in the clusters given.
fn spelling(set: &str, said: &str, clusters: &[u32]) -> Item {
    let Item::Glyphs { origin, font, size, color, skew, embolden, glyphs, .. } =
        text(&display(400.0), 16.0, BLACK, Point { x: 10.0, y: 40.0 }, set)
    else {
        unreachable!()
    };
    let text = Some(Arc::new(RunText { text: said.into(), clusters: clusters.into() }));
    Item::Glyphs { origin, font, size, color, skew, embolden, glyphs, text }
}

/// One page's worth of a PDF, read back.
fn page_of(items: impl IntoIterator<Item = Item>, images: &Images) -> Read {
    read(written(&[(on_white(items), vec![])], images, &Info::default()).expect("the page can be written"))
}

#[test]
fn a_page_is_a_sheet_of_its_size_that_shows_what_the_screen_shows() {
    let images = scene_images();
    let scenes: Vec<(DisplayList, Vec<Link>)> = SCENES.iter().map(|(_, scene, _)| (list(scene()), vec![])).collect();
    let read = read(written(&scenes, &images, &Info::default()).expect("the scenes can be written"));

    assert_eq!(read.pages().len(), SCENES.len(), "a page for each list");
    for (((name, ..), (scene, _)), page) in SCENES.iter().zip(&scenes).zip(read.pages().iter()) {
        assert_eq!(page.render_dimensions(), POINTS, "{name} is the size of its sheet");

        let screen = render_with(scene, &images, SHEET.width as u32, SHEET.height as u32, 1.0);
        let page = shown(page);
        let pixels = screen.data().as_chunks::<4>().0.iter().zip(&page);
        let apart = |(a, b): (&[u8; 4], &[u8; 4])| a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        let apart: Vec<u8> = pixels.map(apart).collect();
        assert_eq!(apart.len(), page.len(), "{name} is drawn at the screen's size");
        // Two rasterisers never agree on an edge, so look at how much of the page is far off.
        let far = apart.iter().filter(|apart| **apart > 96).count() as f64 / apart.len() as f64;
        let mean = apart.iter().map(|apart| f64::from(*apart)).sum::<f64>() / apart.len() as f64;
        assert!(far < 0.005 && mean < 2.0, "{name} looks as it does on screen: {far:.4} far off, {mean:.2} mean");
    }
}

#[test]
fn text_can_be_searched_and_copied_as_it_was_written() {
    let line = |set: &str| text(&display(400.0), 16.0, BLACK, Point { x: 10.0, y: 40.0 }, set);
    let cases = [
        ("a glyph to a character", spelling("rates", "rates", &[0, 1, 2, 3, 4]), "rates"),
        ("a ligature spells all of its letters", spelling("\u{FB01}gures", "figures", &[0, 2, 3, 4, 5, 6]), "figures"),
        (
            "an accent drawn apart belongs to its letter",
            spelling("re\u{301}sume\u{301}", "résumé", &[0, 1, 1, 3, 4, 5, 6, 6]),
            "résumé",
        ),
        ("glyphs set right to left", spelling("cba", "abc", &[2, 1, 0]), "cba"),
        ("no text is spelled from the font", line("rates"), "rates"),
        ("which cannot undo a ligature", line("\u{FB01}gures"), "\u{FB01}gures"),
        ("clusters that are too few", spelling("rates", "xy", &[0, 1]), "rates"),
        ("clusters past the end of the text", spelling("rates", "xyz", &[0, 1, 2, 3, 9]), "rates"),
        ("clusters inside a character", spelling("r\u{E9}s", "r\u{E9}s", &[0, 2, 3]), "r\u{E9}s"),
    ];
    for (name, item, found) in cases {
        let read = page_of([item], &Images::default());

        assert_eq!(text_of(&read.pages()[0]), found, "{name}");
    }
}

#[test]
fn a_font_is_embedded_as_the_part_of_it_that_is_used() {
    let lines = [(sans(400.0), 30.0), (sans(700.0), 60.0), (mono(), 90.0)];
    let fonts: usize = lines.iter().map(|(font, _)| font.data.len()).sum();
    let items = lines.iter().map(|(font, y)| text(font, 14.0, BLACK, Point { x: 10.0, y: *y }, "The quick brown fox"));

    let bytes = written(&[(on_white(items), vec![])], &Images::default(), &Info::default()).expect("it can be written");

    assert!(bytes.len() < fonts / 20, "{} bytes of PDF from {fonts} bytes of fonts", bytes.len());
}

#[test]
fn an_image_is_stored_once_however_often_it_is_drawn() {
    // Noise, so that nothing but storing it once can make it smaller.
    let photo = image(128, 128, |x, y| {
        let [r, g, b, _] =
            (x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B)).wrapping_mul(0xC2B2_AE35).to_be_bytes();
        Color::rgb(r, g, b)
    });
    let stored = 128 * 128 * 3;
    let mut images = Images::default();
    images.insert("photo", photo);
    let draw = |x| Item::Image { rect: Rect::new(x, 10.0, 100.0, 100.0), asset: "photo".into() };
    let size = |pages: &[(DisplayList, Vec<Link>)]| written(pages, &images, &Info::default()).expect("written").len();

    let once = size(&[(on_white([draw(10.0)]), vec![])]);
    let often = size(&[(on_white([draw(10.0), draw(120.0)]), vec![]), (on_white([draw(60.0)]), vec![])]);

    assert!(once > stored / 2, "the image is in the file: {once} bytes");
    assert!(often < once + stored / 4, "three draws take {often} bytes and one takes {once}");
}

#[test]
fn links_and_the_outline_lead_where_they_say() {
    let link = |target| Link { rect: Rect::new(10.0, 20.0, 100.0, 12.0), target };
    let heading =
        |title: &str, level, page| Heading { title: title.into(), level, page, at: Point { x: 0.0, y: 40.0 } };
    let links = vec![
        link(Target::Url("https://example.com/rates?year=2027".into())),
        link(Target::Place { page: 2, at: Point { x: 0.0, y: 40.0 } }),
        Link { rect: Rect::new(10.0, 20.0, 0.0, 0.0), target: Target::Url("https://example.com/nowhere".into()) },
    ];
    let info = Info {
        title: "Household rates".into(),
        authors: vec!["Ada Lovelace".into()],
        language: "en-GB".into(),
        outline: vec![
            heading("Rates", 1, 0),
            heading("Bands and r\u{E9}sum\u{E9}s", 2, 1),
            heading("Appeals", 3, 2),
            heading("Notes", 1, 2),
            heading("On a page that is not there", 1, 9),
        ],
    };
    let pages = [(on_white([]), links), (on_white([]), vec![]), (on_white([]), vec![])];

    let read = read(written(&pages, &Images::default(), &info).expect("it can be written"));

    let found = |index: usize| {
        let page = &read.pages()[index];
        let links = page.raw().get::<Array<'_>>("Annots").map(|links| links.iter::<Dict<'_>>().collect::<Vec<_>>());
        links.unwrap_or_default()
    };
    let (first, rest) = (found(0), [found(1), found(2)]);
    assert_eq!(first.len(), 2, "a link with no area is left out");
    assert!(rest.iter().all(Vec::is_empty), "links stay on their page");
    // A PDF measures in points from the bottom left of the page.
    for link in &first {
        let area = link.get::<Corners>("Rect").expect("a link has an area");
        let corners = [area.x0, area.y0, area.x1, area.y1];
        let expected = [7.5, 96.0, 82.5, 105.0];
        assert!(corners.iter().zip(expected).all(|(a, b)| (a - b).abs() < 0.01), "the link is at {corners:?}");
    }
    let address = first.iter().find_map(|link| string(&link.get::<Dict<'_>>("A")?, "URI"));
    assert_eq!(address.as_deref(), Some("https://example.com/rates?year=2027"));
    let place = first.iter().find_map(|link| destination(&read, link, "Dest"));
    assert_eq!(place, Some((2, 0.0, 90.0)), "the page and the point a link jumps to");

    let outline = outline_of(&read);
    let expected = [("Rates", 0), (" Bands and r\u{E9}sum\u{E9}s", 1), ("  Appeals", 2), ("Notes", 2)];
    assert_eq!(outline, expected.map(|(title, page)| (title.to_owned(), page)));
    let said = |text: &Option<Box<[u8]>>| text.as_deref().map(|text| String::from_utf8_lossy(text).into_owned());
    assert_eq!(said(&read.metadata().title).as_deref(), Some("Household rates"));
    assert_eq!(said(&read.metadata().author).as_deref(), Some("Ada Lovelace"));
}

#[test]
fn a_link_to_a_page_that_is_never_added_fails() {
    let link = Link { rect: Rect::new(0.0, 0.0, 10.0, 10.0), target: Target::Place { page: 7, at: Point::default() } };

    let outcome = written(&[(on_white([]), vec![link])], &Images::default(), &Info::default());

    assert_eq!(outcome, Err(PdfError::NoSuchPage(7)));
}

#[test]
fn what_cannot_be_drawn_is_left_out() {
    let mut images = Images::default();
    images.insert("pixel", image(1, 1, |_, _| BLACK));
    let kept = fill(50.0, 50.0, 10.0, 10.0, RED);
    let expected = shown(&page_of([kept.clone()], &images).pages()[0]);
    let unbalanced = |inside| {
        vec![
            Item::PopClip,
            Item::PushClip { rect: Rect::new(0.0, 0.0, 0.0, f32::NAN), radius: 0.0 },
            inside,
            Item::PopClip,
            kept.clone(),
            Item::PushClip { rect: Rect::new(0.0, 0.0, 200.0, 100.0), radius: 8.0 },
        ]
    };
    let mut cases: Vec<(&str, Vec<Item>)> =
        undrawable().into_iter().map(|(name, item)| (name, vec![item, kept.clone()])).collect();
    cases.push(("an empty clip, and clips that do not pair up", unbalanced(fill(0.0, 0.0, 240.0, 160.0, BLUE))));

    for (name, items) in cases {
        let read = page_of(items, &images);

        assert!(shown(&read.pages()[0]) == expected, "{name} draws nothing and stops nothing");
    }
}
