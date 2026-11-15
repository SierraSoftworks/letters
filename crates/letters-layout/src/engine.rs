//! The Parley setup everything is shaped with: the registered faces and the scratch space layouts are built in.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use letters_render::{Color, Font, Glyph, Item, Point};
use parley::fontique::{Blob, Collection, CollectionOptions, FamilyId, SourceCache};
use parley::setting::Tag;
use parley::{
    FontContext, FontFamily, FontFamilyName, FontStyle, FontVariation, FontVariations, FontWeight, GenericFamily,
    GlyphRun, InlineBox, Language, Layout, LayoutContext, LineHeight, OverflowWrap, Run, TextStyle,
};

use crate::Fonts;
use crate::ink::Brush;

/// How one stretch of text is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spec<'a> {
    pub family: &'a str,
    /// Falls back to a monospace face, not a proportional one, when the family is missing.
    pub mono: bool,
    pub size: f32,
    pub weight: f32,
    pub italic: bool,
    pub line: LineHeight,
    pub brush: Brush,
    pub underline: bool,
    pub strike: bool,
}

pub(crate) struct Engine {
    /// The formulas typeset so far. They are set far more often than they change.
    pub maths: letters_math::Cache,
    /// Whether a formula may link. Only a document that is trusted may.
    pub links: bool,
    fcx: FontContext,
    lcx: LayoutContext<Brush>,
    /// The bytes behind each registered blob, by blob id.
    sources: Vec<(u64, Arc<[u8]>)>,
    /// Font instances already handed out, so that painting clones one and allocates nothing.
    instances: Vec<(u64, Font)>,
    /// The families registered, in the order they came.
    families: Vec<FamilyId>,
    /// Each face registered: its family's name, and whether it is italic.
    faces: Vec<(String, bool)>,
    /// Whether text shaped since this was last cleared asked for a face that is not registered, and so was
    /// set in another. Such text is set again when more fonts arrive.
    pub short: bool,
}

const OPSZ: Tag = Tag::new(b"opsz");

/// The slant, in degrees, of text set in italics at paint time, which leaves its layout alone.
const SLANT: f32 = 14.0;

impl Engine {
    /// Registers exactly `fonts`. System fonts are never consulted, so text looks the same everywhere.
    pub fn new(fonts: &Fonts) -> Self {
        let collection = Collection::new(CollectionOptions { shared: false, system_fonts: false });
        let mut engine = Self {
            maths: letters_math::Cache::default(),
            links: false,
            fcx: FontContext { collection, source_cache: SourceCache::default() },
            lcx: LayoutContext::new(),
            sources: Vec::new(),
            instances: Vec::new(),
            families: Vec::new(),
            faces: Vec::new(),
            short: false,
        };
        engine.add(fonts);
        engine
    }

    /// Registers the faces of `fonts` that are not registered yet, and says whether there were any.
    pub fn add(&mut self, fonts: &Fonts) -> bool {
        let known = self.sources.len();
        for font in &fonts.faces {
            if self.sources.iter().any(|(_, data)| Arc::ptr_eq(data, &font.data) || data == &font.data) {
                continue;
            }
            let blob = Blob::new(Arc::new(font.data.clone()));
            self.sources.push((blob.id(), font.data.clone()));
            for (family, faces) in self.fcx.collection.register_fonts(blob, None) {
                if !self.families.contains(&family) {
                    self.families.push(family);
                }
                let name = self.fcx.collection.family_name(family).unwrap_or_default().to_owned();
                self.faces.extend(faces.iter().map(|face| (name.clone(), face.style() != FontStyle::Normal)));
            }
        }
        if self.sources.len() == known {
            return false;
        }
        // Every family backs up every other, so a glyph one face lacks comes from the next that has it.
        let families = self.families.clone();
        let (mono, sans): (Vec<_>, Vec<_>) = families.iter().partition(|family| self.is_monospace(**family));
        let generic = |first: &[FamilyId], rest: &[FamilyId]| first.iter().chain(rest).copied().collect::<Vec<_>>();
        self.fcx.collection.set_generic_families(GenericFamily::SansSerif, generic(&sans, &mono).into_iter());
        self.fcx.collection.set_generic_families(GenericFamily::Monospace, generic(&mono, &sans).into_iter());
        self.short = false;
        true
    }

    fn is_monospace(&mut self, family: FamilyId) -> bool {
        let Some(name) = self.fcx.collection.family_name(family).map(str::to_owned) else { return false };
        let mut width = |text: &str| {
            let spec = Spec {
                family: &name,
                mono: false,
                size: 16.0,
                weight: 400.0,
                italic: false,
                line: LineHeight::NORMAL,
                brush: Brush::default(),
                underline: false,
                strike: false,
            };
            let mut layout = Layout::new();
            self.build(text, &[spec], &[(0, 0..text.len())], None, &mut layout);
            layout.break_all_lines(None);
            layout.full_width()
        };
        width("i") == width("m")
    }

    /// Shapes `text` into `layout`. `runs` name a spec for each stretch of the text, in order and without gaps.
    /// The caller still breaks the lines.
    pub fn build(
        &mut self,
        text: &str,
        specs: &[Spec],
        runs: &[(u16, Range<usize>)],
        locale: Option<Language>,
        layout: &mut Layout<Brush>,
    ) {
        self.build_with(text, specs, runs, &[], false, locale, layout);
    }

    /// As [`Engine::build`], with `boxes` set in the line, each at a U+FFFC of the text. `by_words` keeps a
    /// word whole when the layout is asked how narrow it can go, which is what sizes a table's columns.
    #[allow(clippy::too_many_arguments)]
    pub fn build_with(
        &mut self,
        text: &str,
        specs: &[Spec],
        runs: &[(u16, Range<usize>)],
        boxes: &[InlineBox],
        by_words: bool,
        locale: Option<Language>,
        layout: &mut Layout<Brush>,
    ) {
        let has = |spec: &Spec| {
            self.faces.iter().any(|(family, italic)| *italic == spec.italic && family.eq_ignore_ascii_case(spec.family))
        };
        self.short |= !specs.iter().all(has);
        let mut builder = self.lcx.style_run_builder(&mut self.fcx, text, 1.0, true);
        for spec in specs {
            let generic = if spec.mono { GenericFamily::Monospace } else { GenericFamily::SansSerif };
            let stack = [FontFamilyName::named(spec.family), FontFamilyName::Generic(generic)];
            // Optical size follows the font size, as it does on the web. A face with no such axis ignores it.
            let optical = [FontVariation::new(OPSZ, spec.size)];
            builder.push_style(TextStyle {
                font_family: FontFamily::List(Cow::Borrowed(&stack)),
                font_size: spec.size,
                font_weight: FontWeight::new(spec.weight),
                font_style: if spec.italic { FontStyle::Italic } else { FontStyle::Normal },
                font_variations: FontVariations::List(Cow::Borrowed(&optical)),
                locale,
                brush: spec.brush,
                has_underline: spec.underline,
                has_strikethrough: spec.strike,
                line_height: spec.line,
                overflow_wrap: if by_words { OverflowWrap::BreakWord } else { OverflowWrap::Anywhere },
                ..TextStyle::default()
            });
        }
        for (spec, range) in runs {
            builder.push_style_run(*spec, range.clone());
        }
        for inline in boxes {
            builder.push_inline_box(inline.clone());
        }
        builder.build_into(layout, text);
    }

    /// The glyphs of one run as a display item, with the run's layout placed at `at`.
    pub fn glyphs(&mut self, run: &GlyphRun<'_, Brush>, at: Point, color: Color) -> Option<Item> {
        let mut pen = 0.0;
        let glyphs = run
            .glyphs()
            .map(|glyph| {
                let placed = Glyph { id: glyph.id, x: pen + glyph.x, y: glyph.y };
                pen += glyph.advance;
                placed
            })
            .collect();
        let origin = Point { x: at.x + run.offset(), y: at.y + run.baseline() };
        self.item(run.run(), origin, color, false, glyphs)
    }

    /// Glyphs of `run` as a display item, with the first on the baseline at `origin`. `slant` sets them in
    /// italics without another face.
    pub fn item(
        &mut self,
        run: &Run<'_, Brush>,
        origin: Point,
        color: Color,
        slant: bool,
        glyphs: Vec<Glyph>,
    ) -> Option<Item> {
        let font = self.font(run)?;
        let synthesis = run.synthesis();
        let skew = synthesis.skew().unwrap_or(if slant { SLANT } else { 0.0 });
        Some(Item::Glyphs {
            origin,
            font,
            size: run.font_size(),
            color,
            skew,
            embolden: synthesis.embolden(),
            glyphs,
            text: None,
        })
    }

    fn font(&mut self, run: &Run<'_, Brush>) -> Option<Font> {
        let face = run.font();
        let (id, coords) = (face.data.id(), run.normalized_coords());
        let known = self.instances.iter().find(|(source, font)| {
            *source == id
                && font.index == face.index
                && font.coords.len() == coords.len()
                && font.coords.iter().zip(coords).all(|(a, b)| *a == b.to_bits())
        });
        if let Some((_, font)) = known {
            return Some(font.clone());
        }
        let data = self.sources.iter().find(|(source, _)| *source == id)?.1.clone();
        let font = Font::new(data, face.index).with_coords(coords.iter().map(|c| c.to_bits()).collect::<Vec<_>>());
        // Sizes are few, so this stays small. The cap only guards against a caller that animates them.
        if self.instances.len() >= 256 {
            self.instances.clear();
        }
        self.instances.push((id, font.clone()));
        Some(font)
    }
}
