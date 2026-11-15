//! The Parley setup everything is shaped with: the registered faces and the scratch space layouts are built in.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use icu_properties::props::BidiClass;
use letters_render::{Color, Font, FontBytes, Glyph, Item, Point};
use parley::fontique::{Blob, CharmapIndex, Collection, CollectionOptions, FamilyId, SourceCache};
use parley::setting::Tag;
use parley::{
    BaseDirection, FontContext, FontFamily, FontFamilyName, FontStyle, FontVariation, FontVariations, FontWeight,
    GenericFamily, GlyphRun, InlineBox, Language, Layout, LayoutContext, LineHeight, OverflowWrap, Run, TextStyle,
};
use parley_data::Properties;

use crate::Fonts;
use crate::ink::Brush;

/// A character that is shown without a glyph, since no face has one. See [`crate::DocLayout::take_uncovered`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uncovered {
    pub ch: char,
    /// Whether it is written to show as an emoji, which takes a colour emoji face and not one for text.
    pub emoji: bool,
    /// The language of its text, as a BCP 47 tag: the one named for its stretch, else for its block or the
    /// nearest block around it, else the document's. Han is drawn another way in each of Chinese, Japanese
    /// and Korean, so this says which face to fetch for it.
    pub lang: Option<String>,
}

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
    /// The language of the stretch, where it is another than its text's.
    pub locale: Option<Language>,
}

pub(crate) struct Engine {
    /// The formulas typeset so far. They are set far more often than they change.
    pub maths: letters_math::Cache,
    /// Whether a formula may link. Only a document that is trusted may.
    pub links: bool,
    fcx: FontContext,
    lcx: LayoutContext<Brush>,
    /// The bytes behind each registered blob, by blob id.
    sources: Vec<(u64, FontBytes)>,
    /// Font instances already handed out, so that painting clones one and allocates nothing.
    instances: Vec<(u64, Font)>,
    /// The families of the document's fonts, in the order they came.
    families: Vec<FamilyId>,
    /// The families that only back those up, in the order they came.
    backups: Vec<FamilyId>,
    /// Each face of the document's fonts: its family's name, and whether it is italic.
    faces: Vec<(String, bool)>,
    /// The character map of every face registered, with its blob, its family and its place in its file.
    maps: Vec<(u64, FamilyId, u32, CharmapIndex)>,
    /// Whether a face has a glyph for a character, for each one met beyond ASCII.
    covered: HashMap<char, bool>,
    /// The characters met without a glyph since they were last taken: whether each is an emoji, and the
    /// language of its text.
    missed: Vec<(char, bool, Option<Language>)>,
    /// Whether text shaped since this was last cleared asked for a face that is not registered, and so was
    /// set in another, or held a character no face has. Such text is set again when more fonts arrive.
    pub short: bool,
}

const OPSZ: Tag = Tag::new(b"opsz");

/// The slant, in degrees, of text set in italics at paint time, which leaves its layout alone.
const SLANT: f32 = 14.0;

impl Engine {
    /// Registers exactly `fonts`. The system's fonts are never looked up, so text looks the same everywhere:
    /// its faces come only as the backups a shell hands to [`Engine::back`].
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
            backups: Vec::new(),
            faces: Vec::new(),
            maps: Vec::new(),
            covered: HashMap::new(),
            missed: Vec::new(),
            short: false,
        };
        engine.add(fonts);
        engine
    }

    /// Registers the faces of `fonts` that are not registered yet, and says whether there were any.
    pub fn add(&mut self, fonts: &Fonts) -> bool {
        let known = self.sources.len();
        for font in &fonts.faces {
            if self.source(&font.data).is_some() {
                continue;
            }
            let blob = Blob::new(font.data.shared());
            self.sources.push((blob.id(), font.data.clone()));
            for (family, faces) in self.fcx.collection.register_fonts(blob.clone(), None) {
                if !self.families.contains(&family) {
                    self.families.push(family);
                }
                let name = self.fcx.collection.family_name(family).unwrap_or_default().to_owned();
                self.faces.extend(faces.iter().map(|face| (name.clone(), face.style() != FontStyle::Normal)));
                self.maps.extend(faces.iter().map(|face| (blob.id(), family, face.index(), face.charmap_index())));
            }
        }
        if self.sources.len() == known {
            return false;
        }
        // Every family backs up every other, so a glyph one face lacks comes from the next that has it.
        // The backups come after them all: what the document's fonts cover is set as if there were none.
        let families = self.families.clone();
        let (mono, sans): (Vec<_>, Vec<_>) = families.iter().partition(|family| self.is_monospace(**family));
        let generic = |first: &[FamilyId], rest: &[FamilyId]| {
            first.iter().chain(rest).chain(&self.backups).copied().collect::<Vec<_>>()
        };
        let (proportional, fixed) = (generic(&sans, &mono), generic(&mono, &sans));
        self.fcx.collection.set_generic_families(GenericFamily::SansSerif, proportional.into_iter());
        self.fcx.collection.set_generic_families(GenericFamily::Monospace, fixed.into_iter());
        self.recover();
        self.short = false;
        true
    }

    /// Takes the faces of `fonts` as backups: text is set in one only where no face given to
    /// [`Engine::add`] has its characters, and never because a style names it, so a document does not
    /// depend on which of them a machine has. Says whether a character that had no glyph has one now.
    pub fn back(&mut self, fonts: &Fonts) -> bool {
        let mut new = Vec::new();
        for font in &fonts.faces {
            let source = self.source(&font.data).or_else(|| {
                let blob = Blob::new(font.data.shared());
                // A file that holds a family of the document's is left out. The two would be one family,
                // and the document's own text could come out in the other's faces.
                if self.clashes(&blob) {
                    return None;
                }
                self.sources.push((blob.id(), font.data.clone()));
                for (family, faces) in self.fcx.collection.register_fonts(blob.clone(), None) {
                    self.maps.extend(faces.iter().map(|face| (blob.id(), family, face.index(), face.charmap_index())));
                }
                Some(blob.id())
            });
            // A file may hold several families. Only that of the face asked for backs anything up.
            let face = self.maps.iter().find(|(blob, _, index, _)| Some(*blob) == source && *index == font.index);
            let family = face.map(|(_, family, ..)| *family).filter(|family| !self.usable(*family));
            new.extend(family);
            self.backups.extend(family);
        }
        for generic in [GenericFamily::SansSerif, GenericFamily::Monospace] {
            self.fcx.collection.append_generic_families(generic, new.iter().copied());
        }
        self.recover()
    }

    /// The characters met without a glyph since this was last asked, each once.
    pub fn take_uncovered(&mut self) -> Vec<Uncovered> {
        let told = |(ch, emoji, lang): (char, bool, Option<Language>)| Uncovered {
            ch,
            emoji,
            lang: lang.map(|lang| lang.to_string()),
        };
        self.missed.drain(..).map(told).collect()
    }

    /// The blob registered with these bytes.
    fn source(&self, data: &FontBytes) -> Option<u64> {
        self.sources.iter().find(|(_, known)| known.same(data) || **known == **data).map(|(id, _)| *id)
    }

    /// Whether a font file holds a family the document's fonts have.
    fn clashes(&self, blob: &Blob<u8>) -> bool {
        let mut probe = Collection::new(CollectionOptions { shared: false, system_fonts: false });
        let families = probe.register_fonts(blob.clone(), None);
        families.iter().any(|(family, _)| {
            let name = probe.family_name(*family).unwrap_or_default();
            self.faces.iter().any(|(own, _)| own.eq_ignore_ascii_case(name))
        })
    }

    /// Whether text may be set in a family.
    fn usable(&self, family: FamilyId) -> bool {
        self.families.contains(&family) || self.backups.contains(&family)
    }

    /// Whether a face text may be set in has a glyph for `ch`.
    fn covers(&self, ch: char) -> bool {
        self.maps.iter().filter(|(_, family, ..)| self.usable(*family)).any(|(blob, .., map)| {
            let data = self.sources.iter().find(|(id, _)| id == blob).map(|(_, data)| data);
            data.and_then(|data| map.charmap(data)).and_then(|map| map.map(ch)).is_some_and(|glyph| glyph != 0)
        })
    }

    /// Looks again for the characters that had no glyph, and says whether any has one now.
    fn recover(&mut self) -> bool {
        let lacking = self.covered.iter().filter(|(_, has)| !**has).map(|(ch, _)| *ch);
        let found: Vec<char> = lacking.filter(|ch| self.covers(*ch)).collect();
        self.covered.extend(found.iter().map(|ch| (*ch, true)));
        let covered = &self.covered;
        self.missed.retain(|(ch, ..)| !covered[ch]);
        !found.is_empty()
    }

    /// Notes which characters of `text` no face has a glyph for.
    fn note(&mut self, text: &str, locale: Option<Language>) {
        if text.is_ascii() {
            return;
        }
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch.is_ascii() {
                continue;
            }
            let has = self.covered.get(&ch).copied().unwrap_or_else(|| {
                let has = !drawn(ch) || self.covers(ch);
                self.covered.insert(ch, has);
                self.missed.extend((!has).then(|| (ch, emoji(ch, chars.peek().copied()), locale)));
                has
            });
            self.short |= !has;
        }
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
                locale: None,
            };
            let mut layout = Layout::new();
            self.build(text, &[spec], &[(0, 0..text.len())], None, &mut layout);
            layout.break_all_lines(None);
            layout.full_width()
        };
        width("i") == width("m")
    }

    /// Shapes `text` into `layout`. `runs` name a spec for each stretch of the text, in order and without gaps.
    /// `locale` is the language of the text, which a spec may name another for. The caller still breaks the
    /// lines.
    pub fn build(
        &mut self,
        text: &str,
        specs: &[Spec],
        runs: &[(u16, Range<usize>)],
        locale: Option<Language>,
        layout: &mut Layout<Brush>,
    ) {
        self.build_with(text, specs, runs, &[], false, locale, BaseDirection::Auto, layout);
    }

    /// As [`Engine::build`], with `boxes` set in the line, each at a U+FFFC of the text. `by_words` keeps a
    /// word whole when the layout is asked how narrow it can go, which is what sizes a table's columns.
    /// `reads` is the side the text reads from, where the caller has decided it.
    #[allow(clippy::too_many_arguments)]
    pub fn build_with(
        &mut self,
        text: &str,
        specs: &[Spec],
        runs: &[(u16, Range<usize>)],
        boxes: &[InlineBox],
        by_words: bool,
        locale: Option<Language>,
        reads: BaseDirection,
        layout: &mut Layout<Brush>,
    ) {
        let has = |spec: &Spec| {
            self.faces.iter().any(|(family, italic)| *italic == spec.italic && family.eq_ignore_ascii_case(spec.family))
        };
        self.short |= !specs.iter().all(has);
        for (spec, range) in runs {
            let locale = specs.get(usize::from(*spec)).and_then(|spec| spec.locale).or(locale);
            self.note(text.get(range.clone()).unwrap_or_default(), locale);
        }
        let mut builder = self.lcx.style_run_builder(&mut self.fcx, text, 1.0, true);
        builder.set_base_direction(reads);
        for spec in specs {
            let generic = if spec.mono { GenericFamily::Monospace } else { GenericFamily::SansSerif };
            let stack = [FontFamilyName::named(spec.family), FontFamilyName::Generic(generic)];
            // A backup is never had by its name: the same style would then look another way elsewhere.
            let own = self.faces.iter().any(|(family, _)| family.eq_ignore_ascii_case(spec.family));
            let stack = &stack[usize::from(!own)..];
            // Optical size follows the font size, as it does on the web. A face with no such axis ignores it.
            let optical = [FontVariation::new(OPSZ, spec.size)];
            builder.push_style(TextStyle {
                font_family: FontFamily::List(Cow::Borrowed(stack)),
                font_size: spec.size,
                font_weight: FontWeight::new(spec.weight),
                font_style: if spec.italic { FontStyle::Italic } else { FontStyle::Normal },
                font_variations: FontVariations::List(Cow::Borrowed(&optical)),
                locale: spec.locale.or(locale),
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
        let font = Font::of(data, face.index).with_coords(coords.iter().map(|c| c.to_bits()).collect::<Vec<_>>());
        // Sizes are few, so this stays small. The cap only guards against a caller that animates them.
        if self.instances.len() >= 256 {
            self.instances.clear();
        }
        self.instances.push((id, font.clone()));
        Some(font)
    }
}

/// The side a letter reads from, `true` for the right. Digits, spaces and signs have none of their own.
pub(crate) fn reads(ch: char) -> Option<bool> {
    if ch.is_ascii() {
        return ch.is_ascii_alphabetic().then_some(false);
    }
    match Properties::get(ch).bidi_class() {
        BidiClass::LeftToRight => Some(false),
        BidiClass::RightToLeft | BidiClass::ArabicLetter => Some(true),
        _ => None,
    }
}

/// Whether a character shows as a glyph at all. One that does not is never missed. U+FFFC is where a box
/// stands in a line.
fn drawn(ch: char) -> bool {
    !(ch.is_control() || ch.is_whitespace() || ch == '\u{fffc}' || Properties::get(ch).is_default_ignorable())
}

/// Whether a character is written to show as an emoji: one that is an emoji by itself, the keycap that
/// makes one of a digit, or a symbol with the selector after it that asks for its emoji form.
fn emoji(ch: char, next: Option<char>) -> bool {
    let of = Properties::get(ch);
    of.is_emoji_presentation() || ch == '\u{20e3}' || (of.is_emoji_or_pictograph() && next == Some('\u{fe0f}'))
}
