//! Pictures and the shell: those it has yet to hand over, and those that are still links. Whether to fetch
//! a link is the shell's to decide, so it is told of each, and a cloud in the corner of a linked picture
//! opens the menu that loads it, trusts its host or embeds it.

use letters_model::{Block, BlockId, BlockKind};
use letters_render::image::Images;
use letters_render::{DisplayList, Item, Point, Rect};

use crate::color::{blend, resolve};
use crate::icon;
use crate::tokens::{self, Color as Ink, Scheme};
use crate::widgets::{Menu, MenuItem};
use crate::window::Message;

/// The least a picture's place measures for the sign of a picture to show in it while it is awaited, and the
/// size of that sign.
const AWAITED: (f32, f32) = (48.0, 24.0);

/// How far the chip round a cloud reaches beyond it, and how far beyond the chip a press is still the
/// cloud's, which a finger needs.
const CHIP: (f32, f32) = (4.0, 4.0);

/// The cloud is drawn a little smaller than its chip, as an icon in a control is.
const CLOUD: f32 = 14.0;

/// What the menu on a linked picture asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linking {
    /// Shows the picture this once.
    Load,
    /// Shows the pictures of its host, now and from now on.
    Trust,
    /// Keeps the picture in the document.
    Embed,
    /// Escape or a press elsewhere.
    Close,
}

/// The linked pictures that something shows on.
#[derive(Default)]
pub(crate) struct Linked {
    /// The figure whose menu is out.
    pub open: Option<BlockId>,
    /// The figure under the pointer, and whether the pointer is on its cloud.
    pointed: Option<(BlockId, bool)>,
}

impl Linked {
    /// Takes what the pointer is over now. Returns whether what shows changes with it.
    pub fn point(&mut self, found: Option<(BlockId, bool)>) -> bool {
        let changed = self.pointed != found;
        self.pointed = found;
        changed
    }

    /// Whether the pointer is on a cloud, where a press opens the menu.
    pub fn on_cloud(&self) -> bool {
        self.pointed.as_ref().is_some_and(|(_, on_cloud)| *on_cloud)
    }

    /// The figures to draw a cloud on: the selected one, the one under the pointer and the one whose menu is
    /// out. Each comes with whether its cloud is lit, as under the pointer or over its menu.
    pub fn clouds<'a>(&'a self, selected: Option<&'a BlockId>) -> Vec<(&'a BlockId, bool)> {
        let lit = |figure: &BlockId| {
            self.open.as_ref() == Some(figure) || self.pointed.as_ref().is_some_and(|(at, on)| at == figure && *on)
        };
        let pointed = self.pointed.as_ref().map(|(figure, _)| figure);
        let mut figures: Vec<&BlockId> = self.open.iter().chain(pointed).chain(selected).collect();
        figures.sort();
        figures.dedup();
        figures.into_iter().map(|figure| (figure, lit(figure))).collect()
    }
}

/// The address of the picture a figure links to, when it does not hold the picture itself.
pub(crate) fn link(block: &Block) -> Option<&str> {
    match &block.kind {
        BlockKind::Figure { asset: None, src: Some(src), .. } => Some(src),
        _ => None,
    }
}

/// The host a link names, as the page writes it on a picture that is not loaded.
fn host(src: &str) -> &str {
    let rest = src.split_once("://").map_or(src, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.rsplit_once('@').map_or(host, |(_, host)| host)
}

/// The menu on the cloud of the picture linked from `src`. One that shows already needs no loading.
pub(crate) fn menu(src: &str, loaded: bool) -> Menu<Message> {
    let mut menu = Menu::new().label("Linked picture");
    if !loaded {
        menu = menu.item(MenuItem::new("Load once").on_select(Message::Linked(Linking::Load)));
    }
    let always = match host(src) {
        "" => "Always load from this host".to_owned(),
        host => format!("Always load from {host}"),
    };
    menu.item(MenuItem::new(always).on_select(Message::Linked(Linking::Trust)))
        .separator()
        .item(MenuItem::new("Embed in the document").on_select(Message::Linked(Linking::Embed)))
}

fn grown(rect: Rect, by: f32) -> Rect {
    Rect::new(rect.x - by, rect.y - by, rect.width + 2.0 * by, rect.height + 2.0 * by)
}

/// The chip drawn round a cloud, which the menu hangs from.
pub(crate) fn chip(badge: Rect) -> Rect {
    grown(badge, CHIP.0)
}

/// Whether a press at `at` is on the cloud at `badge`.
pub(crate) fn pressed(badge: Rect, at: Point) -> bool {
    grown(badge, CHIP.0 + CHIP.1).contains(at)
}

/// Draws the cloud that marks a linked picture, on a chip that sets it off from the picture. A lit one is
/// in `accent`, the person's accent as it is written on a surface.
pub(crate) fn cloud(badge: Rect, lit: bool, accent: Ink, scheme: Scheme, out: &mut DisplayList) {
    let rect = chip(badge);
    let radius = rect.height / 2.0;
    let (ground, ink) = if lit { (accent, tokens::SURFACE) } else { (tokens::SURFACE, tokens::FG_MUTED) };
    out.push(Item::Fill { rect, color: resolve(ground, scheme, 1.0), radius });
    if !lit {
        out.push(Item::Stroke { rect, color: resolve(tokens::CARD_LINE, scheme, 1.0), width: 1.0, radius });
    }
    let inset = (badge.width - CLOUD) / 2.0;
    out.push(Item::Path {
        origin: Point { x: badge.x + inset, y: badge.y + inset },
        scale: CLOUD / icon::GRID,
        path: icon::CLOUD.outline(),
        color: resolve(ink, scheme, 1.0),
        stroke: Some(tokens::ICON_STROKE),
    });
}

/// Keeps the place of each picture in `list` that is not among `images`: a quiet box with the sign of a
/// picture in it, under where the picture will be drawn. Returns the assets that are awaited.
pub(crate) fn awaited(images: &Images, scheme: Scheme, list: &mut DisplayList) -> Vec<String> {
    let missing = |item: &Item| matches!(item, Item::Image { asset, .. } if images.get(asset).is_none());
    if !list.items.iter().any(missing) {
        return Vec::new();
    }
    // The box is of one piece: a picture that comes with clear parts has the page behind them, not the box.
    let ground = resolve(blend(tokens::SURFACE, tokens::FG, 0.06), scheme, 1.0);
    let sign = resolve(tokens::FG_SUBTLE, scheme, 1.0);
    let mut assets = Vec::new();
    let mut items = Vec::with_capacity(list.items.len() + 2);
    for item in std::mem::take(&mut list.items) {
        if let Item::Image { rect, asset } = &item
            && images.get(asset).is_none()
        {
            items.push(Item::Fill { rect: *rect, color: ground, radius: 2.0 });
            if rect.width.min(rect.height) >= AWAITED.0 {
                let (x, y) = (rect.x + (rect.width - AWAITED.1) / 2.0, rect.y + (rect.height - AWAITED.1) / 2.0);
                items.push(Item::Path {
                    origin: Point { x, y },
                    scale: AWAITED.1 / icon::GRID,
                    path: icon::IMAGE.outline(),
                    color: sign,
                    stroke: Some(tokens::ICON_STROKE),
                });
            }
            assets.push(asset.clone());
        }
        items.push(item);
    }
    list.items = items;
    assets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_named_by_its_host_alone() {
        // The address, and the host the menu names.
        let cases = [
            ("https://photos.example.org/roof/north.png?size=2#top", "photos.example.org"),
            ("https://me@nas.tailnet.example:8443/a.png", "nas.tailnet.example:8443"),
            ("photos.example.org/a.png", "photos.example.org"),
            ("https://photos.example.org", "photos.example.org"),
            ("", ""),
        ];
        for (src, named) in cases {
            assert_eq!(host(src), named, "{src}");
        }
    }
}
