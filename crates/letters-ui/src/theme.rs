//! The accent: the one colour a person may choose, and the tokens that follow from it.
//!
//! `tokens::ACCENT` and its kin are worked out for yewi's own accent. Another accent has them worked out
//! again here, by the rules of yewi's `tokens.scss`, so everything that shows the accent asks [`accent`] for
//! it and never the tokens. The neutrals keep the tint they have.

use std::cell::Cell;

use crate::color::{fade, from_oklch, mix, to_oklch};
use crate::tokens::{self, Color as Ink, Rgba};

/// The accent's tokens, each named as yewi names it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Accent {
    /// The colour itself (`--yi-accent`), for marks, rings round what is chosen, and tints.
    pub base: Ink,
    /// The fill of a primary action, dark or light enough for what is written on it.
    pub solid: Ink,
    pub solid_hover: Ink,
    /// The accent as text or an icon on a surface. It is the focus ring too.
    pub text: Ink,
    /// A wash of the accent behind what is picked.
    pub soft: Ink,
    pub line: Ink,
}

impl Default for Accent {
    fn default() -> Self {
        Self::YEWI
    }
}

impl Accent {
    /// yewi's own accent, which a window has until it is given another.
    pub const YEWI: Self = Self {
        base: tokens::ACCENT,
        solid: tokens::ACCENT_SOLID,
        solid_hover: tokens::ACCENT_SOLID_HOVER,
        text: tokens::ACCENT_TEXT,
        soft: tokens::ACCENT_SOFT,
        line: tokens::ACCENT_LINE,
    };

    /// The accent of one colour, the same on light and dark.
    pub fn of([r, g, b]: [u8; 3]) -> Self {
        let color = Rgba::new(r, g, b, 255);
        Self::from(Ink { light: color, dark: color })
    }

    /// The colour of the ring round whatever has the keyboard (`--yi-focus`).
    pub(crate) fn focus(&self) -> Ink {
        self.text
    }
}

impl From<Ink> for Accent {
    /// Works the tokens out from `--yi-accent`, which may differ between light and dark.
    fn from(base: Ink) -> Self {
        // On light the colour is held dark and calm enough to carry white text or to be read as text. On
        // dark it is only ever lightened.
        let toned = |least: f32, most: (f32, f32)| {
            let [l, c, h] = to_oklch(base.light);
            let light = from_oklch([l.min(most.0), c.min(most.1), h], base.light.a);
            let [l, c, h] = to_oklch(base.dark);
            Ink { light, dark: from_oklch([l.max(least), c, h], base.dark.a) }
        };
        let solid = toned(0.74, (0.52, 0.15));
        let toward = Ink { light: Rgba::new(0, 0, 0, 255), dark: Rgba::new(255, 255, 255, 255) };
        Self {
            base,
            solid,
            solid_hover: mix(solid, toward, 0.12),
            text: toned(0.76, (0.48, 0.16)),
            soft: fade(base, 0.12),
            line: fade(base, 0.45),
        }
    }
}

thread_local! {
    /// The accent of the window now being described or painted. A window keeps its own, and lends it here
    /// for as long as its elements are built and drawn, so that no widget has to be handed it.
    static CURRENT: Cell<Accent> = const { Cell::new(Accent::YEWI) };
}

/// The accent of the window being described or painted: yewi's own outside of one.
pub(crate) fn accent() -> Accent {
    CURRENT.get()
}

/// Has [`accent`] give `accent` while `act` runs.
pub(crate) fn with<R>(accent: Accent, act: impl FnOnce() -> R) -> R {
    /// Puts the accent back, also when `act` does not return.
    struct Back(Accent);
    impl Drop for Back {
        fn drop(&mut self) {
            CURRENT.set(self.0);
        }
    }
    let _back = Back(CURRENT.replace(accent));
    act()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Ink, b: Ink) -> bool {
        let near = |a: Rgba, b: Rgba| {
            [(a.r, b.r), (a.g, b.g), (a.b, b.b), (a.a, b.a)].iter().all(|(x, y)| x.abs_diff(*y) <= 1)
        };
        near(a.light, b.light) && near(a.dark, b.dark)
    }

    /// How much lighter the lighter of two colours is, as WCAG counts contrast.
    fn contrast(a: Rgba, b: Rgba) -> f32 {
        let linear = |channel: u8| {
            let s = f32::from(channel) / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        let light = |c: Rgba| 0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b) + 0.05;
        let (a, b) = (light(a), light(b));
        a.max(b) / a.min(b)
    }

    #[test]
    fn yewis_own_accent_gives_the_tokens_yewi_generates() {
        let worked = Accent::from(tokens::ACCENT);
        let Accent { base, solid, solid_hover, text, soft, line } = Accent::YEWI;
        for (name, worked, token) in [
            ("accent", worked.base, base),
            ("solid", worked.solid, solid),
            ("solid-hover", worked.solid_hover, solid_hover),
            ("text", worked.text, text),
            ("soft", worked.soft, soft),
            ("line", worked.line, line),
        ] {
            assert!(close(worked, token), "{name}: {worked:?} is not {token:?}");
        }
        assert_eq!(Accent::YEWI.focus(), tokens::FOCUS);
    }

    #[test]
    fn any_accent_can_be_read_on_and_written_on() {
        // Pale, dark, vivid and grey: whatever is chosen, a primary button and a link stay legible.
        let chosen =
            [[0xff, 0xe0, 0x66], [0x10, 0x10, 0x40], [0xff, 0x00, 0x80], [0x80, 0x80, 0x80], [0x15, 0x6a, 0x4f]];
        for rgb in chosen {
            let accent = Accent::of(rgb);
            for (scheme, on, surface) in [
                ("light", tokens::ON_SOLID.light, tokens::SURFACE.light),
                ("dark", tokens::ON_SOLID.dark, tokens::SURFACE.dark),
            ] {
                let pick = |ink: Ink| if scheme == "light" { ink.light } else { ink.dark };
                assert!(contrast(pick(accent.solid), on) >= 4.5, "{rgb:?}: text on a {scheme} primary button");
                assert!(contrast(pick(accent.text), surface) >= 4.5, "{rgb:?}: accent text on a {scheme} surface");
            }
        }
    }

    #[test]
    fn an_accent_is_lent_for_as_long_as_it_is_asked_for() {
        let green = Accent::of([0x15, 0x6a, 0x4f]);
        assert_eq!(accent(), Accent::YEWI);
        assert_eq!(with(green, accent), green);
        assert_eq!(accent(), Accent::YEWI, "and is the window's no longer");
    }
}
