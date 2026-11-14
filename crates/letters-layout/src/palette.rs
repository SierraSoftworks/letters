//! The colours of a page that the document's style does not set, and how the style's own are shown on a dark one.

use letters_model::Tone;
use letters_render::Color;

use crate::tokens::TokenKind;

/// Whether the page is light or dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scheme {
    /// The style's colours are drawn as written.
    #[default]
    Light,
    /// The style's colours are recoloured to read on a dark page. See [`Palette::recolor`].
    Dark,
}

/// The colours layout paints with besides those in the document's style.
///
/// Layout has no theme of its own: the shell builds a palette from its design tokens and sets it with
/// [`crate::DocLayout::set_palette`]. The default is [`Palette::LIGHT`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub scheme: Scheme,
    /// The page behind the text. Layout does not paint it, but a dark scheme recolours text to read on it.
    pub page: Color,
    /// The theme's own text colour. A style colour that cannot be read falls back to it, and a dark scheme
    /// turns the style's ink into it.
    pub text: Color,
    /// Layout says where the selection and the caret go, and whoever paints them takes their colours from here.
    pub selection: Color,
    pub caret: Color,
    /// List markers, and the label of a block that cannot be shown.
    pub marker: Color,
    pub link: Color,
    /// A ticked task's box.
    pub accent: Color,
    /// The tick inside it.
    pub on_accent: Color,
    /// Dividers.
    pub line: Color,
    /// The bar beside a quote, an unticked task's box and the outline of a block that cannot be shown.
    pub line_strong: Color,
    /// The fill behind a code block.
    pub code: Color,
    /// The fill behind code inside a line.
    pub inline_code: Color,
    /// The fill behind highlighted text.
    pub highlight: Color,
    /// The wavy line under a misspelled word.
    pub spelling: Color,
    pub tip: Callout,
    pub info: Callout,
    pub warn: Callout,
    pub danger: Callout,
    /// The colours of a code block's tokens. See [`crate::DocLayout::set_tokens`].
    pub syntax: Syntax,
}

/// The colours of a callout in one tone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Callout {
    /// The bar down its side.
    pub accent: Color,
    /// The fill behind its text.
    pub fill: Color,
}

/// The colours of highlighted code: six hues and a grey, as yewi's `--yi-syntax-*` tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Syntax {
    pub comment: Color,
    pub keyword: Color,
    pub string: Color,
    pub number: Color,
    pub type_name: Color,
    pub function: Color,
    pub property: Color,
}

impl Syntax {
    /// The colour of a kind of token. The thirteen kinds share the seven colours as they do in yewi.
    pub fn color(&self, kind: TokenKind) -> Color {
        match kind {
            TokenKind::Comment => self.comment,
            TokenKind::Keyword => self.keyword,
            TokenKind::String | TokenKind::Inserted => self.string,
            TokenKind::Number | TokenKind::Literal | TokenKind::Meta => self.number,
            TokenKind::Type => self.type_name,
            TokenKind::Function => self.function,
            TokenKind::Property | TokenKind::Tag | TokenKind::Variable | TokenKind::Deleted => self.property,
        }
    }
}

const fn callout(r: u8, g: u8, b: u8, fill: u8) -> Callout {
    Callout { accent: Color::rgb(r, g, b), fill: Color::rgba(r, g, b, fill) }
}

impl Palette {
    /// yewi's light theme on a white page. Documents are styled for this page, and exports are painted with it.
    pub const LIGHT: Self = Self {
        scheme: Scheme::Light,
        page: Color::rgb(0xff, 0xff, 0xff),
        text: Color::rgb(0x1f, 0x23, 0x28),
        selection: Color::rgba(0x09, 0x69, 0xda, 64),
        caret: Color::rgb(0x1f, 0x23, 0x28),
        marker: Color::rgb(0x6b, 0x6b, 0x70),
        link: Color::rgb(0x1a, 0x63, 0xc6),
        accent: Color::rgb(0x1f, 0x75, 0xd9),
        on_accent: Color::rgb(0xff, 0xff, 0xff),
        line: Color::rgb(0xe3, 0xe4, 0xe8),
        line_strong: Color::rgb(0xca, 0xcc, 0xd1),
        // Translucent, so they also read on a callout's fill.
        code: Color::rgba(0x15, 0x17, 0x1a, 10),
        inline_code: Color::rgba(0x15, 0x17, 0x1a, 16),
        highlight: Color::rgba(0xff, 0xd6, 0x0a, 96),
        spelling: Color::rgb(0xb8, 0x2c, 0x24),
        tip: callout(0x14, 0x6f, 0x43, 31),
        info: callout(0x2a, 0x63, 0xb5, 31),
        warn: callout(0x85, 0x59, 0x00, 33),
        danger: callout(0xb8, 0x2c, 0x24, 31),
        syntax: Syntax {
            comment: Color::rgb(0x6b, 0x6b, 0x70),
            keyword: Color::rgb(0x8f, 0x3a, 0x80),
            string: Color::rgb(0x14, 0x6f, 0x43),
            number: Color::rgb(0x85, 0x59, 0x00),
            type_name: Color::rgb(0x0b, 0x6a, 0x75),
            function: Color::rgb(0x2a, 0x63, 0xb5),
            property: Color::rgb(0xa8, 0x38, 0x2c),
        },
    };

    /// The colour that a colour from the document's style is drawn in on this page.
    ///
    /// A light scheme draws it as written. A dark scheme keeps its hue and turns its lightness over, so that
    /// what was ink on white paper reads on `page`: the light page's text colour becomes `text`, white becomes
    /// `page`, and the colours between keep their order.
    pub fn recolor(&self, color: Color) -> Color {
        if self.scheme == Scheme::Light {
            return color;
        }
        let [lightness, a, b] = oklab(color);
        let (ink, paper) = (oklab(Self::LIGHT.text), oklab(Self::LIGHT.page));
        let (text, page) = (oklab(self.text), oklab(self.page));
        // How far the colour sits from ink towards paper, which is how far it goes from this page's text
        // towards the page. What set it apart from plain ink on paper, its hue, is carried over.
        let along = ((lightness - ink[0]) / (paper[0] - ink[0])).clamp(0.0, 1.0);
        let mix = |from: [f32; 3], to: [f32; 3], axis: usize| from[axis] + along * (to[axis] - from[axis]);
        let lightness = mix(text, page, 0);
        let (a, b) = (a - mix(ink, paper, 1) + mix(text, page, 1), b - mix(ink, paper, 2) + mix(text, page, 2));
        // A screen shows less colour near white and black, so the hue gives up strength until it fits.
        let fits = |keep: f32| linear([lightness, a * keep, b * keep]).iter().all(|c| (-0.001..=1.001).contains(c));
        let (mut keep, mut over) = (0.0, 1.0);
        if fits(1.0) {
            keep = 1.0;
        } else {
            for _ in 0..8 {
                let middle = (keep + over) / 2.0;
                if fits(middle) { keep = middle } else { over = middle }
            }
        }
        let [red, green, blue] = linear([lightness, a * keep, b * keep]).map(encode);
        Color::rgba(red, green, blue, color.a)
    }

    pub(crate) fn callout(&self, tone: Tone) -> Callout {
        match tone {
            Tone::Tip => self.tip,
            Tone::Info => self.info,
            Tone::Warn => self.warn,
            Tone::Danger => self.danger,
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::LIGHT
    }
}

/// A colour in Oklab, where equal steps of lightness look equal: lightness from 0 to 1, then the two hue axes.
fn oklab(color: Color) -> [f32; 3] {
    let decode = |channel: u8| {
        let c = f32::from(channel) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (decode(color.r), decode(color.g), decode(color.b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// Oklab to linear-light sRGB. A channel outside 0 to 1 is a colour a screen cannot show.
fn linear([lightness, a, b]: [f32; 3]) -> [f32; 3] {
    let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

fn encode(channel: f32) -> u8 {
    let c = channel.clamp(0.0, 1.0);
    let c = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (c * 255.0).round() as u8
}
