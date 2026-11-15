//! Pictures and the shell: those it has yet to hand over, and those that are still links. Whether to fetch
//! a link is the shell's to decide, so it is told of each, and a cloud in the corner of a linked picture
//! opens the menu that loads it, trusts its host or embeds it.

use std::collections::{HashMap, HashSet};

use letters_model::{Block, BlockId, BlockKind};
use letters_render::image::Images;
use letters_render::{DisplayList, Item, Point, Rect};

use crate::color::{blend, resolve};
use crate::icon::{self, Icon};
use crate::safe::Safely;
use crate::tokens::{self, Color as Ink, Scheme};
use crate::widgets::{Menu, MenuItem, Tone};
use crate::window::Message;

/// What the menu says of a picture that safe mode holds back.
const HELD: &str = "Safe mode holds linked pictures back until this document is trusted";

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
    /// Keeps every linked picture in the document, as the command of that name asks.
    EmbedAll,
    /// Escape or a press elsewhere.
    Close,
}

/// How a linked picture that does not show is doing, as the shell that fetches it says. Its cloud tells the
/// person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PictureState {
    /// It is being fetched.
    Loading,
    /// It could not be fetched, for a short reason that reads on from "Not loaded:", such as "the server
    /// answered 404". The person is offered another try.
    Failed(String),
    /// Safe mode holds it back. Nothing fetches it until the document is trusted.
    Held,
}

/// The linked pictures that something shows on.
#[derive(Default)]
pub(crate) struct Linked {
    /// The figure whose menu is out.
    pub open: Option<BlockId>,
    /// The figure under the pointer, and whether the pointer is on its cloud.
    pointed: Option<(BlockId, bool)>,
    /// What the shell says of the links that do not show, by address.
    states: HashMap<String, PictureState>,
    /// The addresses whose source the shell says the person trusts already, and the hosts the person
    /// said so of in this window.
    vouched: (HashSet<String>, HashSet<String>),
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

    /// Whether what the person is doing puts a figure's cloud on a chip: it is selected, under the pointer
    /// or has its menu out. Comes with whether the cloud is lit, as under the pointer or over its menu.
    pub fn shows(&self, figure: &BlockId, selected: Option<&BlockId>) -> Option<bool> {
        let open = self.open.as_ref() == Some(figure);
        let pointed = self.pointed.as_ref().filter(|(at, _)| at == figure).map(|(_, on_cloud)| *on_cloud);
        (open || pointed.is_some() || selected == Some(figure)).then_some(open || pointed == Some(true))
    }

    /// What the shell says of the picture linked from `src`.
    pub fn said(&self, src: &str) -> Option<&PictureState> {
        self.states.get(src)
    }

    /// Whether the source of the picture linked from `src` is one the person trusts already, so that
    /// there is nothing left to offer.
    pub fn vouched(&self, src: &str) -> bool {
        self.vouched.0.contains(src) || self.vouched.1.contains(host(src))
    }

    /// Takes the shell's word on whether the person trusts the source of the picture linked from `src`.
    pub fn vouch(&mut self, src: &str, trusted: bool) {
        match trusted {
            true => _ = self.vouched.0.insert(src.to_owned()),
            false => _ = self.vouched.0.remove(src),
        }
    }

    /// The person has said to always load from the host of `src`.
    pub fn trust(&mut self, src: &str) {
        self.vouched.1.insert(host(src).to_owned());
    }

    /// Takes what the shell says of the picture linked from `src`: `None` when it has no more to say.
    pub fn say(&mut self, src: &str, state: Option<PictureState>) {
        match state {
            Some(state) => _ = self.states.insert(src.to_owned(), state),
            None => _ = self.states.remove(src),
        }
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
pub(crate) fn host(src: &str) -> &str {
    let rest = src.split_once("://").map_or(src, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.rsplit_once('@').map_or(host, |(_, host)| host)
}

/// The menu on the cloud of the picture linked from `src`. One that shows already needs no loading, and
/// what the shell says of one that does not show stands over what the menu offers. A source that is
/// `vouched` for is not offered to be trusted again, and in a document that is known to be `safe` a
/// picture that is held back offers to trust the document.
pub(crate) fn menu(src: &str, loaded: bool, state: Option<&PictureState>, vouched: bool, safe: bool) -> Menu<Message> {
    let host = match host(src) {
        "" => "this host",
        host => host,
    };
    let mut menu = Menu::new().label("Linked picture");
    menu = match state {
        Some(PictureState::Loading) => menu.note(format!("Loading from {host}"), Tone::Default).separator(),
        Some(PictureState::Failed(reason)) => menu.note(format!("Not loaded: {reason}"), Tone::Danger).separator(),
        Some(PictureState::Held) => menu.note(HELD, Tone::Default).separator(),
        None => menu,
    };
    let load = match state {
        Some(PictureState::Loading) => None,
        Some(PictureState::Failed(_)) => Some("Try again"),
        Some(PictureState::Held) | None => (!loaded).then_some("Load once"),
    };
    // Safe mode fetches for nobody, so what would fetch is not there to choose.
    let held = state == Some(&PictureState::Held);
    let item = |label: String, asked| MenuItem::new(label).disabled(held).on_select(Message::Linked(asked));
    if held && safe {
        let trust = MenuItem::new("Trust this document").on_select(Message::Safe(Safely::TrustDocument));
        menu = menu.item(trust).separator();
    }
    if let Some(load) = load {
        menu = menu.item(item(load.to_owned(), Linking::Load));
    }
    if !vouched {
        menu = menu.item(item(format!("Always load from {host}"), Linking::Trust));
    }
    // A rule sets apart what changes the document, when anything stands over it.
    if load.is_some() || !vouched {
        menu = menu.separator();
    }
    menu.item(item("Embed in the document".to_owned(), Linking::Embed))
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
/// in `accent`, the person's accent as it is written on a surface. The cloud is the sign of what the shell
/// says of the picture, and one that failed is in the colour of what went wrong.
pub(crate) fn cloud(
    badge: Rect,
    lit: bool,
    state: Option<&PictureState>,
    accent: Ink,
    scheme: Scheme,
    out: &mut DisplayList,
) {
    let (cloud, quiet, loud) = match state {
        Some(PictureState::Loading) => (icon::CLOUD_DOWNLOAD, tokens::FG_MUTED, accent),
        Some(PictureState::Failed(_)) => (icon::CLOUD_ALERT, tokens::DANGER, tokens::DANGER),
        Some(PictureState::Held) => (icon::CLOUD_OFF, tokens::FG_MUTED, accent),
        None => (icon::CLOUD, tokens::FG_MUTED, accent),
    };
    sign(badge, cloud, lit, (quiet, loud), scheme, out);
}

/// Draws a sign at `badge`, on a chip that sets it off from what is under it. One that is lit is on
/// `loud`, and one that is not is in `quiet` on the surface.
pub(crate) fn sign(
    badge: Rect,
    sign: Icon,
    lit: bool,
    (quiet, loud): (Ink, Ink),
    scheme: Scheme,
    out: &mut DisplayList,
) {
    let rect = chip(badge);
    let radius = rect.height / 2.0;
    let (ground, ink) = if lit { (loud, tokens::SURFACE) } else { (tokens::SURFACE, quiet) };
    out.push(Item::Fill { rect, color: resolve(ground, scheme, 1.0), radius });
    if !lit {
        out.push(Item::Stroke { rect, color: resolve(tokens::CARD_LINE, scheme, 1.0), width: 1.0, radius });
    }
    let inset = (badge.width - CLOUD) / 2.0;
    out.push(Item::Path {
        origin: Point { x: badge.x + inset, y: badge.y + inset },
        scale: CLOUD / icon::GRID,
        path: sign.outline(),
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
