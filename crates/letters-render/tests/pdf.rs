//! The PDF backend, read back the way a PDF reader would.
#![cfg(feature = "pdf")]

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use common::*;
use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image as Drawn, ImageDrawProps, InterpreterCache,
    InterpreterSettings, SoftMask, interpret_page,
};
use hayro::hayro_syntax::Pdf as Read;
use hayro::hayro_syntax::object::{Array, Dict, Name, Object, Rect as Corners, String as Bytes};
use hayro::hayro_syntax::page::Page;
use hayro::kurbo::{Affine, BezPath};
use hayro::vello_cpu::color::palette::css;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use letters_render::image::{Image, Images};
use letters_render::pdf::{Heading, Info, Link, Pdf, PdfError, Target};
use letters_render::{Color, DisplayList, Item, Point, Rect, Role, RunText, Size};

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

/// Collects the text of a page in the order it is drawn, and what each stretch of it is marked as.
#[derive(Default)]
struct Reader {
    all: String,
    /// The marks that are open, innermost last.
    marks: Vec<Mark>,
    /// The text of each stretch that belongs to the structure, by its number on the page.
    content: HashMap<i32, String>,
    /// The text marked as no part of the document.
    decoration: String,
}

#[derive(Clone, Copy)]
enum Mark {
    None,
    Decoration,
    Content(i32),
}

impl<'a> Device<'a> for Reader {
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {
        let mut text = String::new();
        for glyph in run.glyphs() {
            match glyph.as_unicode() {
                Some(BfString::Char(character)) => text.push(character),
                Some(BfString::String(more)) => text.push_str(&more),
                None => {}
            }
        }
        self.all.push_str(&text);
        match self.marks.last() {
            Some(Mark::Content(number)) => self.content.entry(*number).or_default().push_str(&text),
            Some(Mark::Decoration) => self.decoration.push_str(&text),
            Some(Mark::None) | None => {}
        }
    }

    fn begin_marked_content(&mut self, tag: &[u8], properties: Option<&Dict<'_>>) {
        let around = self.marks.last().copied().unwrap_or(Mark::None);
        let number = properties.and_then(|properties| properties.get::<i32>("MCID"));
        if let Some(number) = number {
            self.content.entry(number).or_default();
        }
        self.marks.push(match number {
            Some(number) => Mark::Content(number),
            None if tag == b"Artifact" => Mark::Decoration,
            None => around,
        });
    }

    fn end_marked_content(&mut self) {
        self.marks.pop();
    }

    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Drawn<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn read_through(page: &Page<'_>) -> Reader {
    let cache = InterpreterCache::default();
    let bounds = hayro::kurbo::Rect::new(0.0, 0.0, 1e4, 1e4);
    let mut context = Context::new(Affine::IDENTITY, bounds, &cache, page.xref(), InterpreterSettings::default());
    let mut reader = Reader::default();
    interpret_page(page, &mut context, &mut reader);
    reader
}

/// The text a reader finds on a page when it is searched or copied.
fn text_of(page: &Page<'_>) -> String {
    read_through(page).all
}

/// The index of the page that `dict` names under `key`.
fn page_at(read: &Read, dict: &Dict<'_>, key: &str) -> Option<usize> {
    let page = dict.get::<Dict<'_>>(key)?.obj_id()?;
    read.pages().iter().position(|other| other.raw().obj_id() == Some(page))
}

/// The element at the top of the PDF's structure, if it has one.
fn document_of(read: &Read) -> Option<Dict<'_>> {
    let root = read.xref().get::<Dict<'_>>(read.xref().root_id()).expect("a PDF has a catalogue");
    root.get::<Dict<'_>>("StructTreeRoot")?.get::<Array<'_>>("K")?.iter::<Dict<'_>>().next()
}

/// The document as a reader that cannot see the pages gets it: each element as its PDF type, then what
/// kind of list it is, then `[the words that stand for it]`, around `{what it holds}` in reading order. A
/// stretch of a page is the text it draws, or `*` if it draws none, and a link is `@` and its address. Empty
/// if the PDF says nothing.
fn structure_of(read: &Read) -> String {
    fn element(read: &Read, pages: &[Reader], dict: &Dict<'_>) -> String {
        let kind = dict.get::<Name<'_>>("S").map(|kind| kind.as_str().to_owned()).unwrap_or_default();
        let page = page_at(read, dict, "Pg");
        let stretch = |page: Option<usize>, number: Option<i32>| {
            let text = page.zip(number).and_then(|(page, number)| pages.get(page)?.content.get(&number));
            match text.map(|text| text.trim()) {
                Some("") => "*".to_owned(),
                Some(text) => text.to_owned(),
                None => "nothing on the page".to_owned(),
            }
        };
        let attributes: Vec<Dict<'_>> = match dict.get::<Object<'_>>("A") {
            Some(Object::Dict(one)) => vec![one],
            Some(Object::Array(many)) => many.iter::<Dict<'_>>().collect(),
            _ => vec![],
        };
        let numbering = attributes.iter().find_map(|attribute| attribute.get::<Name<'_>>("ListNumbering"));
        let numbering = numbering.map(|numbering| format!(":{}", numbering.as_str())).unwrap_or_default();
        let stands = string(dict, "Alt").or_else(|| string(dict, "ActualText"));
        let stands = stands.map(|stands| format!("[{stands}]")).unwrap_or_default();
        let holds: Vec<Object<'_>> = match dict.get::<Object<'_>>("K") {
            Some(Object::Array(many)) => many.iter::<Object<'_>>().collect(),
            Some(one) => vec![one],
            None => vec![],
        };
        let holds: Vec<String> = holds
            .iter()
            .map(|held| match held {
                Object::Number(number) => stretch(page, Some(number.as_i64() as i32)),
                Object::Dict(inner) if inner.contains_key("S") => element(read, pages, inner),
                Object::Dict(link) if link.contains_key("Obj") => {
                    let address =
                        link.get::<Dict<'_>>("Obj").and_then(|link| string(&link.get::<Dict<'_>>("A")?, "URI"));
                    format!("@{}", address.unwrap_or_default())
                }
                Object::Dict(marked) => stretch(page_at(read, marked, "Pg").or(page), marked.get::<i32>("MCID")),
                _ => "something else".to_owned(),
            })
            .collect();
        format!("{kind}{numbering}{stands}{{{}}}", holds.join(" "))
    }
    let Some(document) = document_of(read) else { return String::new() };
    let pages: Vec<Reader> = read.pages().iter().map(read_through).collect();
    let all = element(read, &pages, &document);
    all.strip_prefix("Document{").and_then(|inside| inside.strip_suffix('}')).unwrap_or(&all).to_owned()
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

/// A line of plain text, on the row given.
fn line(said: &str, row: u8) -> Item {
    text(&sans(400.0), 12.0, BLACK, Point { x: 10.0, y: 20.0 + 14.0 * f32::from(row) }, said)
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
fn a_photo_goes_in_as_the_file_it_came_from() {
    // Soft shapes under grain, as a camera gives: a JPEG is a fraction of its pixels.
    let grainy = |x: u32, y: u32| {
        let grain = (x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B)).wrapping_mul(0xC2B2_AE35) >> 28;
        Color::rgb((x + grain) as u8, (y + grain) as u8, (255 - x / 2 + grain) as u8)
    };
    let jpeg = encoded(160, 120, image::ImageFormat::Jpeg, grainy);
    let cases = [("a photo", jpeg.clone(), true), ("a photo taken on its side", oriented(&jpeg, 6), false)];
    for (name, file, kept) in cases {
        let photo = Image::decode(&file).expect("the photo decodes");
        let frame = Rect::new(20.0, 20.0, photo.width() as f32, photo.height() as f32);
        let mut images = Images::default();
        images.insert("photo", photo);
        let scene = on_white([Item::Image { rect: frame, asset: "photo".into() }]);

        let bytes = written(&[(scene.clone(), vec![])], &images, &Info::default()).expect("it can be written");

        let pixels = 160 * 120 * 3;
        assert_eq!(
            bytes.len() < file.len() + pixels / 20,
            kept,
            "{name}: {} bytes from a file of {}",
            bytes.len(),
            file.len()
        );
        let screen = render_with(&scene, &images, SHEET.width as u32, SHEET.height as u32, 1.0);
        let page = shown(&read(bytes).pages()[0]);
        let apart = |(a, b): (&[u8; 4], &[u8; 4])| a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        let far = screen.data().as_chunks::<4>().0.iter().zip(&page).filter(|pair| apart(*pair) > 48).count();
        assert!(far < page.len() / 100, "{name} looks as it does on screen: {far} pixels are far off");
    }
}

#[test]
fn links_and_the_outline_lead_where_they_say() {
    let link = |target| Link { rect: Rect::new(10.0, 20.0, 100.0, 12.0), target, part: None };
    let heading =
        |title: &str, level, page| Heading { title: title.into(), level, page, at: Point { x: 0.0, y: 40.0 } };
    let links = vec![
        link(Target::Url("https://example.com/rates?year=2027".into())),
        link(Target::Place { page: 2, at: Point { x: 0.0, y: 40.0 } }),
        Link {
            rect: Rect::new(10.0, 20.0, 0.0, 0.0),
            target: Target::Url("https://example.com/nowhere".into()),
            part: None,
        },
    ];
    let info = Info {
        title: "Household rates".into(),
        authors: vec!["Ada Lovelace".into()],
        language: "en-GB".into(),
        created: None,
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
fn the_file_says_when_it_was_written_if_the_shell_says() {
    let cases = [
        ("no date", None, None),
        ("the first second", Some(0), Some((1970, 1, 1, 0, 0, 0))),
        ("a leap day", Some(951_868_799), Some((2000, 2, 29, 23, 59, 59))),
        ("the day after a leap day", Some(1_709_251_200), Some((2024, 3, 1, 0, 0, 0))),
        ("the end of a year", Some(1_798_761_599), Some((2026, 12, 31, 23, 59, 59))),
        ("an afternoon", Some(1_794_670_245), Some((2026, 11, 14, 15, 30, 45))),
    ];
    for (name, created, expected) in cases {
        let info = Info { created, ..Info::default() };

        let read = read(written(&[(on_white([]), vec![])], &Images::default(), &info).expect("it can be written"));

        let date = read.metadata().creation_date.map(|date| {
            assert_eq!((date.utc_offset_hour, date.utc_offset_minute), (0, 0), "{name} is in UTC");
            (date.year, date.month, date.day, date.hour, date.minute, date.second)
        });
        assert_eq!(date, expected, "{name}");
    }
}

#[test]
fn a_link_to_a_page_that_is_never_added_fails() {
    let link = Link {
        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        target: Target::Place { page: 7, at: Point::default() },
        part: None,
    };

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

#[test]
fn a_reader_that_cannot_see_the_page_gets_its_parts_in_reading_order() {
    let item = |depth, numbered, checked| Role::ListItem { depth, numbered, checked };
    let item_of = |role: Role, id: u64, label: Option<&str>, said: &str| {
        let label = label.map(|label| part(Role::Label, id + 100, [line(label, 0)])).unwrap_or_default();
        part(role, id, label.into_iter().chain([line(said, 1)]))
    };
    let cell = |header, id, said| part(Role::Cell { header }, id, [line(said, 0)]);
    let cases: Vec<(&str, Vec<Vec<Item>>, &str)> =
        vec![
            (
                "headings and paragraphs",
                vec![
                    part(Role::Heading(1), 1, [line("Title", 0)]),
                    part(Role::Paragraph, 2, [line("Body goes on", 1)]),
                    part(Role::Heading(2), 3, [line("More", 3)]),
                ],
                "H1{Title} P{Body goes on} H2{More}",
            ),
            (
                "a part inside another",
                vec![
                    part(Role::Quote, 1, part(Role::Paragraph, 2, [line("Said", 0)])),
                    part(
                        Role::Paragraph,
                        3,
                        [line("Area", 1)]
                            .into_iter()
                            .chain(part(Role::Formula { alt: "a^2".into() }, 4, [line("a2", 1)]))
                            .chain([line("here", 1)]),
                    ),
                    part(Role::Note, 5, part(Role::Paragraph, 6, [line("Mind", 2)])),
                    part(Role::Code, 7, [line("let", 3)]),
                ],
                "BlockQuote{P{Said}} P{Area Formula[a^2]{a2} here} Div{P{Mind}} Code{let}",
            ),
            (
                "list items that follow one another, and deeper ones",
                vec![
                    item_of(item(0, true, None), 1, Some("1."), "One"),
                    item_of(item(1, false, None), 2, None, "Inner"),
                    item_of(item(1, false, None), 3, None, "Other"),
                    item_of(item(0, true, None), 4, Some("2."), "Two"),
                    item_of(item(0, false, Some(true)), 5, Some("x"), "Done"),
                    item_of(item(0, false, Some(false)), 6, Some("o"), "Open"),
                    part(Role::Paragraph, 7, [line("After", 2)]),
                    item_of(item(0, false, None), 8, None, "Apart"),
                ],
                "L:Decimal{LI{Lbl{1.} LBody{One L:Disc{LI{LBody{Inner}} LI{LBody{Other}}}}} LI{Lbl{2.} LBody{Two}}} \
             L:None{LI{Lbl[\u{2611}]{x} LBody{Done}} LI{Lbl[\u{2610}]{o} LBody{Open}}} P{After} \
             L:Disc{LI{LBody{Apart}}}",
            ),
            (
                "a figure and a table",
                vec![
                    part(
                        Role::Figure { alt: "A red square".into() },
                        1,
                        [fill(10.0, 10.0, 20.0, 20.0, RED)].into_iter().chain(part(Role::Caption, 2, [line("Fig", 2)])),
                    ),
                    part(Role::Figure { alt: "".into() }, 3, [fill(40.0, 10.0, 20.0, 20.0, BLUE)]),
                    part(
                        Role::Table,
                        4,
                        part(Role::Row, 5, [cell(true, 6, "Name"), cell(true, 7, "Age")].concat())
                            .into_iter()
                            .chain(part(Role::Row, 8, [cell(false, 9, "Ada"), cell(false, 10, "36")].concat())),
                    ),
                ],
                "Figure[A red square]{* Caption{Fig}} Figure{*} Table{TR{TH{Name} TH{Age}} TR{TD{Ada} TD{36}}}",
            ),
        ];

    for (name, parts, expected) in cases {
        let read = page_of(parts.concat(), &Images::default());

        assert_eq!(structure_of(&read), expected, "{name}");
    }
}

#[test]
fn a_part_that_runs_on_to_the_next_page_is_one_part() {
    let first = part(Role::Heading(1), 1, [line("Title", 0)]).into_iter().chain(part(
        Role::Quote,
        2,
        part(Role::Paragraph, 3, [line("Starts", 1)]),
    ));
    let second = part(
        Role::Quote,
        2,
        [part(Role::Paragraph, 3, [line("ends", 0)]), part(Role::Paragraph, 4, [line("Next", 1)])].concat(),
    );
    let pages = [(on_white(first), vec![]), (on_white(second), vec![])];
    let info = Info { language: "en-GB".into(), ..Info::default() };

    let read = read(written(&pages, &Images::default(), &info).expect("the pages can be written"));

    assert_eq!(structure_of(&read), "H1{Title} BlockQuote{P{Starts ends} P{Next}}");
    let document = document_of(&read);
    assert_eq!(document.and_then(|document| string(&document, "Lang")).as_deref(), Some("en-GB"), "the language");
}

#[test]
fn what_is_drawn_inside_no_part_is_decoration() {
    let number = line("Page 1", 5);
    let parts = part(Role::Paragraph, 1, [line("Body", 0)]);
    let with_parts = page_of(parts.into_iter().chain([number.clone()]), &Images::default());
    let without = page_of([line("Body", 0), number], &Images::default());

    assert_eq!(structure_of(&with_parts), "P{Body}", "the page number is no part of the document");
    assert_eq!(read_through(&with_parts.pages()[0]).decoration, "Page 1", "and is marked as decoration");
    assert_eq!(structure_of(&without), "", "a list that names no parts says nothing of structure");
    assert_eq!(read_through(&without.pages()[0]).decoration, "", "and none of it is called decoration");
    assert_eq!(text_of(&with_parts.pages()[0]), text_of(&without.pages()[0]), "the text is found either way");
}

#[test]
fn parts_that_do_not_pair_up_change_nothing_on_the_page() {
    let words = [line("One", 0), line("Two", 1), line("Three", 2)];
    let expected = page_of(words.clone(), &Images::default());
    let open = |id| Item::Open { role: Role::Paragraph, id };
    let hidden = Item::PushClip { rect: Rect::new(0.0, 0.0, 0.0, 0.0), radius: 0.0 };
    let [one, two, three] = words.clone();
    let cases: Vec<(&str, Vec<Item>)> = vec![
        ("a close with nothing open", vec![Item::Close, one.clone(), two.clone(), three.clone()]),
        ("a part that never closes", vec![one.clone(), open(1), two.clone(), three.clone()]),
        ("a part with nothing in it", vec![open(1), Item::Close, one.clone(), two.clone(), three.clone()]),
        ("a part inside itself", vec![open(1), one.clone(), open(1), two.clone(), Item::Close, three.clone()]),
        (
            "a part around a clip's end",
            vec![
                Item::PushClip { rect: Rect::new(0.0, 0.0, 240.0, 160.0), radius: 0.0 },
                open(1),
                one.clone(),
                Item::PopClip,
                two.clone(),
                Item::Close,
                three.clone(),
            ],
        ),
        (
            "a part opened where nothing shows",
            vec![
                hidden,
                open(1),
                fill(0.0, 0.0, 240.0, 160.0, BLUE),
                Item::PopClip,
                one.clone(),
                two.clone(),
                three.clone(),
            ],
        ),
        (
            "parts nested very deep",
            (0..10_000).map(open).chain(words.clone()).chain((0..10_000).map(|_| Item::Close)).collect(),
        ),
    ];

    for (name, items) in cases {
        let read = page_of(items, &Images::default());

        assert!(shown(&read.pages()[0]) == shown(&expected.pages()[0]), "{name} shows what the plain page shows");
        assert_eq!(text_of(&read.pages()[0]), "OneTwoThree", "{name} keeps the text");
    }
}

#[test]
fn a_link_is_read_with_the_text_it_covers() {
    let link = |address: &str, part| Link {
        rect: Rect::new(10.0, 20.0, 60.0, 14.0),
        target: Target::Url(address.into()),
        part,
    };
    let text = [line("See", 0)].into_iter().chain(part(Role::Link, 2, [line("the rates", 0)])).chain([line("now", 0)]);
    let list = on_white(part(Role::Paragraph, 1, text));
    let links = vec![
        link("https://example.com/rates", Some(2)),
        link("https://example.com/paragraph", Some(1)),
        link("https://example.com/nothing", Some(9)),
        link("https://example.com/plain", None),
    ];

    let read = read(written(&[(list, links)], &Images::default(), &Info::default()).expect("it can be written"));

    assert_eq!(structure_of(&read), "P{See Link{the rates @https://example.com/rates} now}");
    let links = read.pages()[0].raw().get::<Array<'_>>("Annots").map(|links| links.iter::<Dict<'_>>().count());
    assert_eq!(links, Some(4), "a link that names no link's text can still be clicked");
}

#[test]
fn a_colour_glyph_is_written_in_its_own_colours_where_the_screen_shows_it() {
    /// How many pixels are plainly coloured, and the middle of them.
    fn coloured(pixels: impl Iterator<Item = [u8; 4]>) -> (usize, f32, f32) {
        let spread = |[r, g, b, _]: [u8; 4]| r.max(g).max(b) - r.min(g).min(b);
        let at: Vec<usize> = pixels.enumerate().filter(|(_, pixel)| spread(*pixel) > 60).map(|(at, _)| at).collect();
        let (width, count) = (SHEET.width as usize, at.len().max(1) as f32);
        let middle = |of: fn(usize, usize) -> usize| at.iter().map(|at| of(*at, width) as f32).sum::<f32>() / count;
        (at.len(), middle(|at, width| at % width), middle(|at, width| at / width))
    }
    let size = 40.0;
    for (format, font, id) in colour_faces() {
        let Item::Glyphs { origin, font, size, color, skew, embolden, glyphs, .. } =
            glyph(&font, id, size, Point { x: 80.0, y: 100.0 })
        else {
            unreachable!()
        };
        let text = Some(Arc::new(RunText { text: "\u{1F600}".into(), clusters: [0].into() }));
        let glyph = Item::Glyphs { origin, font, size, color, skew, embolden, glyphs, text };
        let screen = render(&on_white([glyph.clone()]), SHEET.width as u32, SHEET.height as u32, 1.0);

        let read = page_of([glyph], &Images::default());

        let (on_screen, x, y) = coloured(screen.data().as_chunks::<4>().0.iter().copied());
        let (on_page, page_x, page_y) = coloured(shown(&read.pages()[0]).into_iter());
        assert!(on_screen > 100, "{format} is in colour on screen");
        let ratio = on_page as f32 / on_screen as f32;
        assert!((0.7..1.4).contains(&ratio), "{format} is as large on the page: {on_page} against {on_screen}");
        let apart = (page_x - x).abs().max((page_y - y).abs());
        assert!(apart < 0.2 * size, "{format} is where the screen shows it: {apart} pixels away");
        assert_eq!(text_of(&read.pages()[0]), "\u{1F600}", "{format} is found as the character it stands for");
    }
}
