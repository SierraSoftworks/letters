//! The shadow of a window that floats, on a desktop that leaves it to the window to draw.
//!
//! Along an edge a shadow is the same everywhere once the corners are far enough away. So it is drawn
//! once, for the smallest window that has such edges, and every window's shadow is cut from that tile.

use letters_render::cpu::{CpuRenderer, Frame};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Item, Point, Rect};
use letters_ui::{Scheme, tokens};

/// How far the corners of a window are rounded where Letters draws its frame.
pub const CORNER: u32 = 10;

/// What a window casts: the shadow of anything that floats over the page.
const LAYERS: &[tokens::Shadow] = tokens::SHADOW_OVERLAY;

/// One of the four pieces of a window's shadow, which lie round it and under its corners.
pub struct Piece {
    /// Where its top left corner is from the window's.
    pub x: i32,
    pub y: i32,
    /// Its size in pixels, [`Shadow::scale`] of them to one of the window's units.
    pub width: u32,
    pub height: u32,
    /// `0xAARRGGBB` words with the colours multiplied by alpha.
    pub pixels: Vec<u32>,
}

/// The shadow of every window at one scale and in one scheme.
pub struct Shadow {
    scale: u32,
    dark: bool,
    /// How far it reaches past a window's edges.
    reach: u32,
    /// The side of the tile in pixels, and its rows.
    side: u32,
    tile: Vec<u32>,
}

impl Shadow {
    /// Draws the shadow with `scale` pixels to each of a window's units.
    pub fn new(scale: u32, dark: bool) -> Self {
        let scale = scale.clamp(1, 4);
        let most = |of: fn(&tokens::Shadow) -> f32| LAYERS.iter().map(of).fold(0.0, f32::max).ceil() as u32;
        let reach = most(|layer| layer.blur + layer.x.abs() + layer.y.abs());
        // A corner changes its edges' shadow as far as it is round and the shadow is soft.
        let body = 2 * (CORNER + most(|layer| layer.blur)) + 2;
        let side = (body + 2 * reach) * scale;

        let scheme = if dark { Scheme::Dark } else { Scheme::Light };
        let rect = Rect::new(reach as f32, reach as f32, body as f32, body as f32);
        let mut list = DisplayList::default();
        for layer in LAYERS {
            let ink = layer.color.resolve(scheme);
            let (color, offset) = (Color::rgba(ink.r, ink.g, ink.b, ink.a), Point { x: layer.x, y: layer.y });
            list.push(Item::Shadow { rect, color, blur: layer.blur, offset, radius: CORNER as f32 });
        }
        let mut frame = Frame::argb(side, side);
        CpuRenderer::new().render(&list, &Images::default(), scale as f32, Color::rgba(0, 0, 0, 0), &mut frame);
        Self { scale, dark, reach, side, tile: frame.pixels().to_vec() }
    }

    pub fn scale(&self) -> u32 {
        self.scale
    }

    pub fn dark(&self) -> bool {
        self.dark
    }

    /// The pieces for a window of `width` by `height`: above, below, left and right. Those above and
    /// below also lie under the window's corners, which are clear where they are round. `None` for a
    /// window too small to have a plain edge.
    pub fn pieces(&self, width: u32, height: u32) -> Option<[Piece; 4]> {
        let (reach, body) = (self.reach, self.side / self.scale - 2 * self.reach);
        if width < body || height < body {
            return None;
        }
        let (wide, high) = (width + 2 * reach, height + 2 * reach);
        let band = reach + CORNER;
        // Each as a rectangle of the whole shadow, which starts `reach` above and left of the window.
        let rects = [
            (0, 0, wide, band),
            (0, high - band, wide, band),
            (0, band, reach, high - 2 * band),
            (wide - reach, band, reach, high - 2 * band),
        ];
        Some(rects.map(|(x, y, width, height)| Piece {
            x: x as i32 - reach as i32,
            y: y as i32 - reach as i32,
            width: width * self.scale,
            height: height * self.scale,
            pixels: self.cut((wide, high), (x, y, width, height)),
        }))
    }

    /// The pixels of one rectangle of a shadow `wide` by `high`, all in a window's units.
    fn cut(&self, (wide, high): (u32, u32), (x, y, width, height): (u32, u32, u32, u32)) -> Vec<u32> {
        let (side, scale) = (self.side, self.scale);
        // The tile's two ends are a shadow's ends, and the line across its middle is everything between.
        let along = |at: u32, length: u32| {
            let (half, from_end) = (side / 2, length * scale - at);
            (if at < half {
                at
            } else if from_end <= half {
                side - from_end
            } else {
                half
            }) as usize
        };
        let columns: Vec<usize> = (x * scale..(x + width) * scale).map(|at| along(at, wide)).collect();
        let mut pixels = Vec::with_capacity(columns.len() * (height * scale) as usize);
        for row in (y * scale..(y + height) * scale).map(|at| along(at, high) * side as usize) {
            pixels.extend(columns.iter().map(|column| self.tile[row + column]));
        }
        pixels
    }
}
