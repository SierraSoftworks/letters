//! The PDF backend: each display list becomes a page, and its text stays text.
//!
//! Fonts are embedded as subsets, a variable font as the instance its runs use. A run that carries its
//! [`RunText`] can be selected and searched as it was written. One that does not is spelled from its font's
//! character map, which loses ligatures. An image is stored once however often it is drawn.

use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use krilla::action::{Action, LinkAction};
use krilla::annotation::{Annotation, LinkAnnotation, Target as Opens};
use krilla::color::rgb;
use krilla::destination::XyzDestination;
use krilla::geom::{PathBuilder, Transform};
use krilla::image::{BitsPerComponent, CustomImage, ImageColorspace};
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::outline::{Outline, OutlineNode};
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, LineCap, LineJoin, Stroke};
use krilla::surface::Surface;
use krilla::text::{GlyphId, KrillaGlyph, Tag};
use krilla::{Document, geom};
use skrifa::raw::TableProvider;
use skrifa::raw::types::F2Dot14;
use skrifa::{FontRef, MetadataProvider};
use vello_cpu::kurbo::{self, Shape};

use crate::cpu::{CpuRenderer, Frame};
use crate::image::Images;
use crate::paint::{EMBOLDEN, bez_path, rounded};
use crate::{Color, DisplayList, Font, Glyph, Item, Point, Rect, RunText, Size};

/// PDF points in a logical pixel: 72 to the inch against 96.
const POINTS: f32 = 0.75;
/// How far a curve may stray from its true shape, in logical pixels.
const TOLERANCE: f64 = 0.01;
/// The pixels a shadow is drawn with for each logical pixel. A blur has no detail finer than this.
const SHADOW_SCALE: f32 = 2.0;
/// The longest side a shadow's picture may have, in pixels.
const SHADOW_SIDE: f32 = 2048.0;

/// A part of a page that takes the reader somewhere.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    /// Where on the page it is, in logical pixels from the page's top left.
    pub rect: Rect,
    pub target: Target,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// An address outside the document.
    Url(String),
    /// A place on a page of this document. The page is counted from 0 and may be added later.
    Place { page: usize, at: Point },
}

/// A heading in the outline that a reader shows beside the pages.
#[derive(Clone, Debug, PartialEq)]
pub struct Heading {
    pub title: String,
    /// 1 for the top level. A heading is listed under the nearest one before it with a lower level.
    pub level: u8,
    /// The page it is on, counted from 0, and where on that page.
    pub page: usize,
    pub at: Point,
}

/// What a PDF says about itself. Whatever is empty is left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub title: String,
    pub authors: Vec<String>,
    /// The language most of the text is in, as a BCP 47 tag such as `en-GB`.
    pub language: String,
    /// The headings in reading order.
    pub outline: Vec<Heading>,
}

/// Why a PDF could not be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PdfError {
    /// A link goes to a page that was never added.
    NoSuchPage(usize),
    /// A font could not be embedded.
    Font(String),
    Other(String),
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchPage(page) => write!(f, "a link goes to page {}, which the document does not have", page + 1),
            Self::Font(reason) => write!(f, "a font cannot be embedded: {reason}"),
            Self::Other(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for PdfError {}

/// A PDF being written, a page at a time.
pub struct Pdf {
    document: Document,
    pages: usize,
    /// The furthest page a link goes to.
    furthest: Option<usize>,
    kept: Kept,
}

/// What pages share, so that each font and image goes into the file once.
#[derive(Default)]
struct Kept {
    /// By the address of the font's data, the face's index and the instance's coordinates.
    faces: HashMap<(usize, u32, Arc<[i16]>), Face>,
    /// For each font file and face in it, the character each glyph stands for.
    spellings: HashMap<(usize, u32), HashMap<u32, char>>,
    /// By [`crate::image::Image`] id.
    images: HashMap<u64, krilla::image::Image>,
    shadows: CpuRenderer,
}

struct Face {
    /// Keeps the address the face is keyed by from being reused.
    _data: Arc<[u8]>,
    /// `None` if the data is not a font.
    font: Option<krilla::text::Font>,
    glyphs: u32,
}

impl Default for Pdf {
    fn default() -> Self {
        Self::new()
    }
}

impl Pdf {
    pub fn new() -> Self {
        Self { document: Document::new(), pages: 0, furthest: None, kept: Kept::default() }
    }

    /// Adds a page of `size` logical pixels that shows `list`, with its top left at the list's origin. An
    /// image whose asset is not in `images` is left out, and so is a link with no area.
    pub fn page(&mut self, size: Size, list: &DisplayList, images: &Images, links: &[Link]) {
        let sheet = PageSettings::from_wh(size.width * POINTS, size.height * POINTS)
            .unwrap_or_else(|| PageSettings::from_wh(1.0, 1.0).expect("a point square is a valid page"));
        let mut page = self.document.start_page_with(sheet);
        for link in links {
            let Rect { x, y, width, height } = link.rect;
            if !(width > 0.0 && height > 0.0) {
                continue;
            }
            let Some(rect) = geom::Rect::from_xywh(x * POINTS, y * POINTS, width * POINTS, height * POINTS) else {
                continue;
            };
            let opens = match &link.target {
                Target::Url(url) => Opens::Action(Action::Link(LinkAction::new(url.clone()))),
                Target::Place { page, at } => {
                    self.furthest = self.furthest.max(Some(*page));
                    Opens::Destination(place(*page, *at).into())
                }
            };
            page.add_annotation(Annotation::new_link(LinkAnnotation::new(rect, opens), None));
        }

        let mut surface = page.surface();
        surface.push_transform(&Transform::from_scale(POINTS, POINTS));
        Painter { surface: &mut surface, kept: &mut self.kept, assets: images }.paint(list);
        surface.pop();
        surface.finish();
        page.finish();
        self.pages += 1;
    }

    /// Ends the document and returns the file. A heading on a page that was never added is left out.
    pub fn finish(mut self, info: &Info) -> Result<Vec<u8>, PdfError> {
        if let Some(page) = self.furthest.filter(|page| *page >= self.pages) {
            return Err(PdfError::NoSuchPage(page));
        }
        let mut metadata = Metadata::new();
        if !info.title.is_empty() {
            metadata = metadata.title(info.title.clone());
        }
        if !info.authors.is_empty() {
            metadata = metadata.authors(info.authors.clone());
        }
        if !info.language.is_empty() {
            metadata = metadata.language(info.language.clone());
        }
        self.document.set_metadata(metadata);

        let headings: Vec<&Heading> = info.outline.iter().filter(|heading| heading.page < self.pages).collect();
        if !headings.is_empty() {
            let mut outline = Outline::new();
            let mut rest = &headings[..];
            while !rest.is_empty() {
                outline.push_child(nested(&mut rest));
            }
            self.document.set_outline(outline);
        }
        self.document.finish().map_err(|error| match error {
            krilla::error::KrillaError::Font(_, reason) => PdfError::Font(reason),
            other => PdfError::Other(format!("{other:?}")),
        })
    }
}

fn place(page: usize, at: Point) -> XyzDestination {
    let points = |pixels: f32| if pixels.is_finite() { pixels * POINTS } else { 0.0 };
    XyzDestination::new(page, geom::Point::from_xy(points(at.x), points(at.y)))
}

/// Takes the first heading off `rest`, with every heading after it that lies under it.
fn nested(rest: &mut &[&Heading]) -> OutlineNode {
    let (first, after) = rest.split_first().expect("the caller checks there is a heading");
    *rest = after;
    let mut node = OutlineNode::new(first.title.clone(), place(first.page, first.at));
    while rest.first().is_some_and(|next| next.level > first.level) {
        node.push_child(nested(rest));
    }
    node
}

/// Draws display lists on one page's surface.
struct Painter<'a, 'b> {
    surface: &'a mut Surface<'b>,
    kept: &'a mut Kept,
    assets: &'a Images,
}

impl Painter<'_, '_> {
    fn paint(&mut self, list: &DisplayList) {
        // Whether each open clip is one the surface holds. A clip that lets nothing through is not, and
        // nothing is drawn until it closes.
        let mut clips: Vec<bool> = Vec::new();
        for item in &list.items {
            match item {
                Item::PushClip { rect, radius } => {
                    let path = rounded(rect, *radius).and_then(|shape| path(shape.path_elements(TOLERANCE)));
                    if let Some(path) = &path {
                        self.surface.push_clip_path(path, &FillRule::NonZero);
                    }
                    clips.push(path.is_some());
                }
                // A pop with nothing pushed is ignored.
                Item::PopClip => {
                    if clips.pop() == Some(true) {
                        self.surface.pop();
                    }
                }
                _ if clips.contains(&false) => {}
                item => self.draw(item),
            }
        }
        for _ in clips.into_iter().filter(|held| *held) {
            self.surface.pop();
        }
    }

    fn draw(&mut self, item: &Item) {
        match item {
            Item::Fill { rect, color, radius } => {
                let Some(shape) = rounded(rect, *radius) else { return };
                self.fill(shape.path_elements(TOLERANCE), *color, FillRule::NonZero);
            }
            Item::Stroke { rect, color, width, radius } => {
                let (Some(outer), true) = (rounded(rect, *radius), *width > 0.0) else { return };
                let inset =
                    Rect::new(rect.x + width, rect.y + width, rect.width - 2.0 * width, rect.height - 2.0 * width);
                // The band between the edge and the edge moved in by the width.
                let inner = rounded(&inset, radius - width);
                let ring =
                    outer.path_elements(TOLERANCE).chain(inner.iter().flat_map(|it| it.path_elements(TOLERANCE)));
                self.fill(ring, *color, FillRule::EvenOdd);
            }
            Item::Shadow { rect, color, blur, offset, radius } => self.shadow(*rect, *color, *blur, *offset, *radius),
            Item::Glyphs { origin, font, size, color, skew, embolden, glyphs, text } => {
                if !(origin.x.is_finite() && origin.y.is_finite() && size.is_finite() && *size > 0.0) {
                    return;
                }
                let run = Run { size: *size, glyphs, text: text.as_deref() };
                let Some((shaped, fill)) = self.shaped(font, &run).zip(solid(*color, FillRule::NonZero)) else {
                    return;
                };
                let start = geom::Point::from_xy(origin.x + shaped.first, origin.y);

                // A fill with an outline of the same colour thickens the glyphs and keeps them text.
                let outline = embolden.then(|| Stroke {
                    paint: fill.paint.clone(),
                    width: size * (2.0 * EMBOLDEN) as f32,
                    line_join: LineJoin::Round,
                    opacity: fill.opacity,
                    ..Stroke::default()
                });
                self.surface.set_fill(Some(fill));
                self.surface.set_stroke(outline);
                let slant = (skew.is_finite() && *skew != 0.0).then(|| skew.to_radians().tan());
                if let Some(slant) = slant {
                    // Leans about the baseline, so the foot of each glyph stays where it was.
                    self.surface.push_transform(&Transform::from_row(1.0, 0.0, -slant, 1.0, slant * origin.y, 0.0));
                }
                self.surface.draw_glyphs(start, &shaped.glyphs, shaped.font, &shaped.text, *size, false);
                if slant.is_some() {
                    self.surface.pop();
                }
                self.surface.set_stroke(None);
            }
            Item::Path { origin, scale, path: shape, color, stroke } => {
                if !(origin.x.is_finite() && origin.y.is_finite() && scale.is_finite() && *scale > 0.0) {
                    return;
                }
                let Some(shape) = bez_path(shape).and_then(|shape| path(shape.path_elements(TOLERANCE))) else {
                    return;
                };
                match stroke {
                    None => {
                        self.surface.set_fill(solid(*color, FillRule::EvenOdd));
                        self.surface.set_stroke(None);
                    }
                    Some(width) if width.is_finite() && *width > 0.0 => {
                        let Some(fill) = solid(*color, FillRule::NonZero) else { return };
                        self.surface.set_fill(None);
                        self.surface.set_stroke(Some(Stroke {
                            paint: fill.paint,
                            width: *width,
                            line_cap: LineCap::Round,
                            line_join: LineJoin::Round,
                            opacity: fill.opacity,
                            ..Stroke::default()
                        }));
                    }
                    Some(_) => return,
                }
                self.surface.push_transform(&Transform::from_row(*scale, 0.0, 0.0, *scale, origin.x, origin.y));
                self.surface.draw_path(&shape);
                self.surface.pop();
                self.surface.set_stroke(None);
            }
            Item::Image { rect, asset } => {
                let (Some(_), Some(image)) = (rounded(rect, 0.0), self.assets.get(asset)) else { return };
                let stored = self.kept.images.entry(image.id).or_insert_with(|| {
                    picture(image.pixels.data_as_u8_slice(), (image.width(), image.height()), image.id)
                });
                let stored = stored.clone();
                self.place(stored, *rect);
            }
            Item::PushClip { .. } | Item::PopClip => {}
        }
    }

    fn fill(&mut self, shape: impl Iterator<Item = kurbo::PathEl>, color: Color, rule: FillRule) {
        let (Some(shape), Some(fill)) = (path(shape), solid(color, rule)) else { return };
        self.surface.set_fill(Some(fill));
        self.surface.set_stroke(None);
        self.surface.draw_path(&shape);
    }

    /// Draws an image so that it fills `rect`.
    fn place(&mut self, image: krilla::image::Image, rect: Rect) {
        let Some(size) = geom::Size::from_wh(rect.width, rect.height) else { return };
        self.surface.push_transform(&Transform::from_translate(rect.x, rect.y));
        self.surface.draw_image(image, size);
        self.surface.pop();
    }

    /// A blur is not something a PDF can describe, so a shadow goes in as a picture of itself.
    fn shadow(&mut self, rect: Rect, color: Color, blur: f32, offset: Point, radius: f32) {
        if !(blur.is_finite() && offset.x.is_finite() && offset.y.is_finite()) || rounded(&rect, radius).is_none() {
            return;
        }
        // Beyond three standard deviations a blur has faded to nothing.
        let reach = blur.max(0.0) * 1.5 + 1.0;
        let area = Rect::new(
            (rect.x + offset.x - reach).floor(),
            (rect.y + offset.y - reach).floor(),
            (rect.width + 2.0 * reach).ceil(),
            (rect.height + 2.0 * reach).ceil(),
        );
        let scale = SHADOW_SCALE.min(SHADOW_SIDE / area.width.max(area.height));
        let (width, height) = ((area.width * scale).ceil() as u32, (area.height * scale).ceil() as u32);
        let mut list = DisplayList::default();
        let rect = Rect::new(rect.x - area.x, rect.y - area.y, rect.width, rect.height);
        list.push(Item::Shadow { rect, color: Color::rgb(0, 0, 0), blur, offset, radius });
        let mut frame = Frame::new(width, height);
        self.kept.shadows.render(&list, &Images::default(), scale, Color::rgba(0, 0, 0, 0), &mut frame);
        // The frame holds how much shadow covers each pixel. Its colour is the same everywhere.
        let cover = frame.data().as_chunks::<4>().0.iter().map(|pixel| u16::from(pixel[3]));
        let pixels: Vec<u8> =
            cover.flat_map(|cover| [color.r, color.g, color.b, (cover * u16::from(color.a) / 255) as u8]).collect();
        let size = Rect::new(area.x, area.y, frame.width() as f32 / scale, frame.height() as f32 / scale);
        let mut content = DefaultHasher::new();
        pixels.hash(&mut content);
        self.place(picture(&pixels, (frame.width(), frame.height()), content.finish()), size);
    }

    /// A run as the PDF takes it, or `None` if nothing of it can be drawn.
    fn shaped(&mut self, font: &Font, run: &Run<'_>) -> Option<Shaped> {
        let address = Arc::as_ptr(&font.data).cast::<u8>() as usize;
        let face = self.kept.faces.entry((address, font.index, font.coords.clone())).or_insert_with(|| face(font));
        let embedded = face.font.clone()?;
        let count = face.glyphs;
        let drawable = |glyph: &&Glyph| glyph.x.is_finite() && glyph.y.is_finite() && glyph.id < count;

        let written = run.text.filter(|text| text.clusters.len() == run.glyphs.len()).and_then(|text| {
            let ranges = clusters(text)?;
            Some((text.text.to_string(), ranges))
        });
        let (text, ranges) = written.unwrap_or_else(|| {
            let spelling = self.kept.spellings.entry((address, font.index)).or_insert_with(|| spelling(font));
            let mut text = String::new();
            let ranges = run.glyphs.iter().map(|glyph| {
                let start = text.len();
                text.extend(spelling.get(&glyph.id));
                start..text.len()
            });
            let ranges = ranges.collect();
            (text, ranges)
        });

        let placed: Vec<(&Glyph, Range<usize>)> =
            run.glyphs.iter().zip(ranges).filter(|(glyph, _)| drawable(glyph)).collect();
        let glyphs = placed.iter().enumerate().map(|(index, (glyph, range))| {
            // Each glyph says how far the next one is, so the last says nothing.
            let next = placed.get(index + 1).map_or(glyph.x, |(next, _)| next.x);
            KrillaGlyph {
                glyph_id: GlyphId::new(glyph.id),
                text_range: range.clone(),
                x_advance: (next - glyph.x) / run.size,
                x_offset: 0.0,
                y_offset: -glyph.y / run.size,
                y_advance: 0.0,
                location: None,
            }
        });
        let glyphs = glyphs.collect();
        Some(Shaped { font: embedded, first: placed.first()?.0.x, glyphs, text })
    }
}

/// A run in the PDF's terms.
struct Shaped {
    font: krilla::text::Font,
    /// How far the first glyph is from the run's origin.
    first: f32,
    /// Each glyph with the distance to the next and the part of `text` it stands for.
    glyphs: Vec<KrillaGlyph>,
    text: String,
}

struct Run<'a> {
    size: f32,
    glyphs: &'a [Glyph],
    text: Option<&'a RunText>,
}

/// The part of a run's text that each of its glyphs stands for, or `None` if the clusters do not fit the
/// text. Glyphs of one cluster share its part.
fn clusters(text: &RunText) -> Option<Vec<Range<usize>>> {
    let mut starts = text.clusters.to_vec();
    starts.sort_unstable();
    starts.dedup();
    // An offset past the end of the text is on no boundary either.
    if !starts.iter().all(|start| text.text.is_char_boundary(*start as usize)) {
        return None;
    }
    let part = |start: &u32| {
        let after = starts.partition_point(|other| other <= start);
        *start as usize..starts.get(after).map_or(text.text.len(), |next| *next as usize)
    };
    Some(text.clusters.iter().map(part).collect())
}

fn face(font: &Font) -> Face {
    let read = FontRef::from_index(&font.data, font.index).ok();
    let glyphs = read.as_ref().and_then(|face| face.maxp().ok()).map_or(0, |maxp| u32::from(maxp.num_glyphs()));
    let embedded = read.and_then(|face| {
        let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(font.data.clone());
        krilla::text::Font::new_variable(data.into(), font.index, &design(&face, &font.coords))
    });
    Face { _data: font.data.clone(), font: embedded, glyphs }
}

/// A variable font's instance in the units of its axes, from normalised coordinates.
fn design(face: &FontRef<'_>, coords: &[i16]) -> Vec<(Tag, f32)> {
    let maps = face.avar().ok().map(|avar| avar.axis_segment_maps());
    let along = |(index, (axis, coord)): (usize, (skrifa::Axis, &i16))| {
        let mut at = F2Dot14::from_bits(*coord).to_f32();
        // The font bends each axis before it is used. Undo that: the bend is made of straight pieces.
        let bends = maps.as_ref().and_then(|maps| maps.get(index)?.ok()).map(|map| map.axis_value_maps());
        let ends = |bend: &skrifa::raw::tables::avar::AxisValueMap| {
            (bend.from_coordinate().to_f32(), bend.to_coordinate().to_f32())
        };
        for pair in bends.unwrap_or_default().windows(2) {
            let ((from, to), (next_from, next_to)) = (ends(&pair[0]), ends(&pair[1]));
            if (to..=next_to).contains(&at) && next_to > to {
                at = from + (at - to) / (next_to - to) * (next_from - from);
                break;
            }
        }
        let (low, middle, high) = (axis.min_value(), axis.default_value(), axis.max_value());
        let value = if at < 0.0 { middle + at * (middle - low) } else { middle + at * (high - middle) };
        (Tag::new(&axis.tag().to_be_bytes()), value)
    };
    face.axes().iter().zip(coords).enumerate().map(along).collect()
}

/// The character each glyph of a face stands for: the first one in Unicode that maps to it.
fn spelling(font: &Font) -> HashMap<u32, char> {
    let mut spelling = HashMap::new();
    if let Ok(face) = FontRef::from_index(&font.data, font.index) {
        for (character, glyph) in face.charmap().mappings() {
            if let Some(character) = char::from_u32(character) {
                spelling.entry(glyph.to_u32()).or_insert(character);
            }
        }
    }
    spelling
}

/// Pixels as a PDF stores them: colour apart from cover.
#[derive(Clone)]
struct Pixels {
    colors: Arc<[u8]>,
    /// Left out when every pixel is opaque.
    cover: Option<Arc<[u8]>>,
    size: (u32, u32),
    /// Tells one picture from another, so a picture drawn twice is stored once.
    name: u64,
}

impl Hash for Pixels {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.name, self.size).hash(state);
    }
}

impl CustomImage for Pixels {
    fn color_channel(&self) -> &[u8] {
        &self.colors
    }

    fn alpha_channel(&self) -> Option<&[u8]> {
        self.cover.as_deref()
    }

    fn bits_per_component(&self) -> BitsPerComponent {
        BitsPerComponent::Eight
    }

    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn icc_profile(&self) -> Option<&[u8]> {
        None
    }

    fn color_space(&self) -> ImageColorspace {
        ImageColorspace::Rgb
    }
}

/// Straight RGBA8 pixels as a picture that a reader smooths when it is drawn larger, as the screen does.
fn picture(rgba: &[u8], size: (u32, u32), name: u64) -> krilla::image::Image {
    let pixels = rgba.as_chunks::<4>().0;
    let colors = pixels.iter().flat_map(|pixel| [pixel[0], pixel[1], pixel[2]]).collect();
    let cover = pixels.iter().any(|pixel| pixel[3] != 255).then(|| pixels.iter().map(|pixel| pixel[3]).collect());
    krilla::image::Image::from_custom(Pixels { colors, cover, size, name }, true)
        .expect("making a picture checks nothing")
}

fn solid(color: Color, rule: FillRule) -> Option<Fill> {
    let opacity = NormalizedF32::new(f32::from(color.a) / 255.0)?;
    Some(Fill { paint: rgb::Color::new(color.r, color.g, color.b).into(), opacity, rule })
}

fn path(elements: impl Iterator<Item = kurbo::PathEl>) -> Option<geom::Path> {
    let mut path = PathBuilder::new();
    for element in elements {
        match element {
            kurbo::PathEl::MoveTo(to) => path.move_to(to.x as f32, to.y as f32),
            kurbo::PathEl::LineTo(to) => path.line_to(to.x as f32, to.y as f32),
            kurbo::PathEl::QuadTo(control, to) => {
                path.quad_to(control.x as f32, control.y as f32, to.x as f32, to.y as f32);
            }
            kurbo::PathEl::CurveTo(first, second, to) => {
                path.cubic_to(
                    first.x as f32,
                    first.y as f32,
                    second.x as f32,
                    second.y as f32,
                    to.x as f32,
                    to.y as f32,
                );
            }
            kurbo::PathEl::ClosePath => path.close(),
        }
    }
    path.finish()
}
