//! Which bytes of its source each part of a formula is set from.
//!
//! The engine's drawing keeps no positions. So each part of its tree is put in a colour of its own, which
//! is its number, and the formula is laid out a second time: whatever comes out in a part's colour is that
//! part. A part is as small as can be marked without moving anything: a symbol, or a whole that the engine
//! sets as one, such as a run in a font or a line of text. What a command draws of its own, such as the bar
//! of a fraction, is in no part.
//!
//! The tree keeps a position on its symbols and its brace groups alone. An operator such as `\sum` and
//! what a macro of the engine's makes, such as `\ne`, have none, so each is looked for by its command, in
//! the source between the positions on either side of it.

use std::collections::HashMap;
use std::mem;
use std::ops::Range;

use ratex_parser::macro_expander::{MacroDefinition, MacroExpander};
use ratex_parser::parse_node::{Mode, ParseNode};
use ratex_types::display_item::DisplayItem;

use crate::engine::children;

/// The most parts a formula is cut into: as many as there are colours to tell them apart by.
const MOST: usize = 0xff_ffff;

/// Where the parts of a marked formula are from: for the part numbered `n`, at `n - 1`.
pub(crate) struct Parts(Vec<Option<Range<usize>>>);

impl Parts {
    /// Where in the source something is from, given how it is drawn with the marks and without them.
    pub(crate) fn source(&self, marked: &DisplayItem, real: &DisplayItem) -> Option<Range<usize>> {
        let (marked, real) = (color(marked), color(real));
        // In the colour it has anyway, it is in no part.
        if marked == real {
            return None;
        }
        let number = usize::from(marked[0]) << 16 | usize::from(marked[1]) << 8 | usize::from(marked[2]);
        self.0.get(number.checked_sub(1)?)?.clone()
    }
}

fn color(item: &DisplayItem) -> [u8; 4] {
    use DisplayItem as D;
    let (D::GlyphPath { color, .. } | D::Line { color, .. } | D::Rect { color, .. } | D::Path { color, .. }) = item;
    [color.r, color.g, color.b, color.a].map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Marks the parts of a formula in `moved`, the tree of its source with one byte put before it, and says
/// where each is from. `plain` is the tree of the source as it is. `None` when the two are not one tree.
pub(crate) fn mark(source: &str, plain: &mut [ParseNode], moved: &mut [ParseNode]) -> Option<Parts> {
    if !settle(source, plain, moved) {
        return None;
    }
    let macros = MacroExpander::new("", Mode::Math);
    let mut marker = Marker { source, macros, sources: Vec::new(), waiting: Vec::new(), passed: 0, unordered: false };
    marker.walk(moved.iter_mut().collect());
    marker.pass(source.len()..source.len());
    Some(Parts(marker.sources))
}

/// Rewrites each position in `moved` as what it is to the source: a place in it, or [`MADE`]. The engine
/// gives what a macro of its own makes a position in that macro, which looks like a place in the source and
/// is not. Only a real place moves when the source does. `false` when the trees differ.
fn settle(source: &str, plain: &mut [ParseNode], moved: &mut [ParseNode]) -> bool {
    if plain.len() != moved.len() {
        return false;
    }
    let mut pending: Vec<(&mut ParseNode, &mut ParseNode)> = plain.iter_mut().zip(moved).collect();
    while let Some((plain, moved)) = pending.pop() {
        if mem::discriminant(plain) != mem::discriminant(moved) {
            return false;
        }
        let was = position(plain).map(|(start, end)| *start..*end);
        if let Some((start, end)) = position(moved) {
            let real =
                was.filter(|was| (was.start + 1, was.end + 1) == (*start, *end) && source.get(was.clone()).is_some());
            (*start, *end) = real.map_or(MADE, |real| (real.start, real.end));
        }
        let (mut inside, mut inside_moved) = (Vec::new(), Vec::new());
        children(plain, &mut inside);
        children(moved, &mut inside_moved);
        if inside.len() != inside_moved.len() {
            return false;
        }
        pending.extend(inside.into_iter().zip(inside_moved));
    }
    true
}

/// The position of what is not written in the source as such, but made by a macro: one that ends before it
/// starts, which no place in a source does.
const MADE: (usize, usize) = (usize::MAX, 0);

/// What a node is to the source.
enum Place {
    /// Written at these bytes.
    Written(Range<usize>),
    /// Made by a macro.
    Made,
    /// The engine keeps no position for it.
    Unknown,
}

fn place(node: &mut ParseNode) -> Place {
    match position(node) {
        Some((start, end)) if *start <= *end => Place::Written(*start..*end),
        Some(_) => Place::Made,
        None => Place::Unknown,
    }
}

/// The bytes of the source a node and all inside it are written at, from the first to the last, and whether
/// a macro made any of it.
fn inside(node: &mut ParseNode) -> (Option<Range<usize>>, bool) {
    let (mut span, mut made): (Option<Range<usize>>, bool) = match place(node) {
        // What is inside a group is inside its braces.
        Place::Written(range) => return (Some(range), false),
        Place::Made => (None, true),
        Place::Unknown => (None, false),
    };
    let mut nodes = Vec::new();
    children(node, &mut nodes);
    for child in nodes {
        let (within, by_macro) = inside(child);
        made |= by_macro;
        span = match (span, within) {
            (Some(a), Some(b)) => Some(a.start.min(b.start)..a.end.max(b.end)),
            (a, b) => a.or(b),
        };
    }
    (span, made)
}

/// How a node is marked.
enum Kind {
    /// A part, written at these bytes.
    Part(Range<usize>),
    /// A part that the command of an operator draws, such as `\\sum`. The engine keeps its name and not its
    /// place.
    Operator(String),
    /// A part that a macro made, so the tree has no place for it.
    Made,
    /// Not a part itself: the parts are inside it.
    Inner,
}

fn kind(source: &str, node: &mut ParseNode) -> Kind {
    use ParseNode as N;
    match (inside(node), &*node) {
        ((Some(span), _), node) if symbol(node) => Kind::Part(span),
        ((Some(span), _), node) if whole(node) => Kind::Part(reach(source, span)),
        ((None, _), N::Op { name: Some(name), body: None, .. }) => Kind::Operator(name.clone()),
        // A command written in the source may hold what a macro made, and is no part for that. These are
        // what a macro makes as one thing.
        ((None, true), node) if symbol(node) || whole(node) || matches!(node, N::HtmlMathMl { .. }) => Kind::Made,
        _ => Kind::Inner,
    }
}

fn symbol(node: &ParseNode) -> bool {
    use ParseNode as N;
    matches!(
        node,
        N::Atom { .. } | N::MathOrd { .. } | N::TextOrd { .. } | N::OpToken { .. } | N::Op { body: None, .. }
    )
}

/// Whether a node is one part, with nothing marked inside it. A mark is a node around another, and the
/// engine does not see through one everywhere: a font does not reach what is marked inside it, and a
/// stroke is fitted to a lone character only when it sees that character.
fn whole(node: &ParseNode) -> bool {
    use ParseNode as N;
    match node {
        N::Font { .. } | N::Text { .. } | N::HBox { .. } | N::OperatorName { .. } | N::ProofTree { .. } => true,
        N::Enclose { body, .. } => lone(body),
        _ => false,
    }
}

/// The bytes of a whole, with the command it is an argument of and the braces around it where those are
/// written right at it: all of `\mathbb{R}` for its `R`.
fn reach(source: &str, inner: Range<usize>) -> Range<usize> {
    let bytes = source.as_bytes();
    let blank = |bytes: &[u8]| bytes.iter().rev().take_while(|byte| byte.is_ascii_whitespace()).count();
    let (mut start, mut end) = (inner.start - blank(&bytes[..inner.start]), inner.end);
    let closing = end + bytes[end..].iter().take_while(|byte| byte.is_ascii_whitespace()).count();
    if start > 0 && bytes[start - 1] == b'{' && bytes.get(closing) == Some(&b'}') {
        (start, end) = (start - 1, closing + 1);
        start -= blank(&bytes[..start]);
    }
    let letters = bytes[..start].iter().rev().take_while(|byte| byte.is_ascii_alphabetic()).count();
    let word = start - letters;
    // Two backslashes are a line break, and no command.
    let slashes = bytes[..word].iter().rev().take_while(|byte| **byte == b'\\').count();
    if letters > 0 && slashes % 2 == 1 { word - 1..end } else { inner }
}

/// Whether a node is a lone character, as the engine reads one.
fn lone(node: &ParseNode) -> bool {
    use ParseNode as N;
    match node {
        N::OrdGroup { body, .. } | N::Styling { body, .. } => matches!(&body[..], [only] if lone(only)),
        N::Atom { .. } | N::MathOrd { .. } | N::TextOrd { .. } => true,
        _ => false,
    }
}

struct Marker<'a> {
    source: &'a str,
    /// The macros the engine defines.
    macros: MacroExpander<'static>,
    /// Where each part is from, in the order they are numbered. `None` until that is known.
    sources: Vec<Option<Range<usize>>>,
    /// The parts met since `passed` that the tree has no place for, by number, with an operator's name.
    waiting: Vec<(usize, Option<String>)>,
    /// How far through the source the walk has come.
    passed: usize,
    /// Set while what is walked may not be in the order it is written in.
    unordered: bool,
}

impl Marker<'_> {
    fn walk(&mut self, nodes: Vec<&mut ParseNode>) {
        for node in nodes {
            self.mark(node);
        }
    }

    /// Marks a node, or the parts inside it.
    fn mark(&mut self, node: &mut ParseNode) {
        use ParseNode as N;
        if let N::SupSub { base, sup, sub, .. } = node {
            let (sup, sub) = (sup.as_deref_mut(), sub.as_deref_mut());
            match base.as_deref_mut().map(|base| (kind(self.source, base), base)) {
                // `\overset` and its like are written with the script first.
                Some((Kind::Inner, base @ N::Op { always_handle_sup_sub: Some(true), body: Some(_), .. })) => {
                    self.scripts(sup, sub, false);
                    self.mark(base);
                }
                Some((Kind::Inner, base)) => {
                    self.mark(base);
                    self.scripts(sup, sub, false);
                }
                // With scripts on it, a part is marked around them: the engine sets scripts by what they
                // are on, and does not see that through a mark. The scripts are parts of their own.
                Some((kind, _)) => {
                    let part = self.part(kind);
                    self.scripts(sup, sub, true);
                    wrap(node, part);
                }
                None => self.scripts(sup, sub, false),
            }
            return;
        }
        match kind(self.source, node) {
            Kind::Inner => {
                // A group bounds what is inside it.
                let bounds = match place(node) {
                    Place::Written(range) => Some(range),
                    _ => None,
                };
                self.pass_at(bounds.as_ref().map(|range| range.start));
                let mut inside = Vec::new();
                children(node, &mut inside);
                self.walk(inside);
                self.pass_at(bounds.as_ref().map(|range| range.end));
            }
            kind => {
                let part = self.part(kind);
                wrap(node, part);
            }
        }
    }

    /// Marks scripts. `around` says that what they are on is marked around them.
    fn scripts(&mut self, sup: Option<&mut ParseNode>, sub: Option<&mut ParseNode>, around: bool) {
        let starts = |script: &mut ParseNode| inside(script).0.map(|span| span.start);
        let mut scripts: Vec<_> = [sup, sub].into_iter().flatten().map(|script| (starts(script), script)).collect();
        // The tree holds the upper script first, whichever was written first.
        let ordered = scripts.len() < 2 || scripts.iter().all(|(start, _)| start.is_some());
        scripts.sort_by_key(|(start, _)| *start);
        let unordered = self.unordered;
        self.unordered |= !ordered;
        for (_, script) in scripts {
            self.mark(script);
            // What is in no part of the script is not in the part the script is on.
            if around {
                wrap(script, 0);
            }
        }
        self.unordered = unordered;
    }

    /// Numbers a part. 0 when it cannot be one.
    fn part(&mut self, kind: Kind) -> usize {
        let (source, name) = match kind {
            Kind::Part(source) => (Some(source), None),
            Kind::Operator(name) => (None, Some(name)),
            _ => (None, None),
        };
        // Its place is found from the order it is met in, so without an order it has none.
        if self.sources.len() >= MOST || (source.is_none() && self.unordered) {
            return 0;
        }
        self.sources.push(source.clone());
        let part = self.sources.len();
        match source {
            Some(source) => self.pass(source),
            None => self.waiting.push((part, name)),
        }
        part
    }

    fn pass_at(&mut self, at: Option<usize>) {
        if let Some(at) = at {
            self.pass(at..at);
        }
    }

    /// Moves on past bytes of the source that the tree has a place for. The parts met since the last such
    /// are written between the two, so that is where they are looked for.
    fn pass(&mut self, bytes: Range<usize>) {
        let gap = mem::replace(&mut self.passed, bytes.end)..bytes.start;
        let waiting = mem::take(&mut self.waiting);
        if waiting.is_empty() {
            return;
        }
        let mut words = words(self.source, gap);
        let spelt = |word: &Range<usize>| &self.source[word.clone()];
        // An operator is the command of its name.
        for (part, name) in &waiting {
            if let Some(name) = name
                && let Some(at) = words.iter().position(|word| spelt(word) == name)
            {
                self.sources[part - 1] = Some(words.remove(at));
            }
        }
        // What macros made is the macros written there, when each made one thing. A macro that takes
        // arguments draws around them, and is not looked for.
        let plain = |word: &Range<usize>| match self.macros.get_macro(spelt(word)) {
            Some(MacroDefinition::Text(body)) => !body.contains('#'),
            Some(MacroDefinition::Tokens { num_args, .. }) => *num_args == 0,
            Some(MacroDefinition::Function(_)) => true,
            None => false,
        };
        words.retain(plain);
        let made = waiting.iter().filter(|(_, name)| name.is_none());
        if made.clone().count() == words.len() {
            for ((part, _), word) in made.zip(words) {
                self.sources[part - 1] = Some(word);
            }
        }
    }
}

/// Puts a node in the colour of a part, or with 0 in that of no part.
fn wrap(node: &mut ParseNode, part: usize) {
    let mode = node.mode();
    let inner = mem::replace(node, ParseNode::OrdGroup { mode, body: Vec::new(), semisimple: None, loc: None });
    let attributes = HashMap::from([("style".to_owned(), format!("color:#{part:06x}"))]);
    *node = ParseNode::Html { mode, attributes, body: vec![inner], loc: None };
}

/// The commands spelt with letters in a stretch of the source, such as `\sum`. Empty for a stretch that is
/// not one.
fn words(source: &str, within: Range<usize>) -> Vec<Range<usize>> {
    let Some(text) = source.get(within.clone()) else { return Vec::new() };
    let (bytes, mut at, mut found) = (text.as_bytes(), 0, Vec::new());
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => {
                let letters = bytes[at + 1..].iter().take_while(|byte| byte.is_ascii_alphabetic()).count();
                if letters > 0 {
                    found.push(within.start + at..within.start + at + 1 + letters);
                }
                // A backslash takes the character after it, so that `\\` and `\%` are each read as one.
                at += 1 + letters.max(1);
            }
            // A comment runs to the end of its line.
            b'%' => at += bytes[at..].iter().position(|byte| *byte == b'\n').unwrap_or(bytes.len()),
            _ => at += 1,
        }
    }
    found
}

/// The position the engine keeps on a node: where it starts and where it ends. Every kind of node is
/// named, so that a kind new to the engine does not compile until it has a place here.
fn position(node: &mut ParseNode) -> Option<(&mut usize, &mut usize)> {
    use ParseNode as N;
    let (N::Atom { loc, .. }
    | N::MathOrd { loc, .. }
    | N::TextOrd { loc, .. }
    | N::OpToken { loc, .. }
    | N::AccentToken { loc, .. }
    | N::SpacingNode { loc, .. }
    | N::OrdGroup { loc, .. }
    | N::SupSub { loc, .. }
    | N::GenFrac { loc, .. }
    | N::Sqrt { loc, .. }
    | N::Accent { loc, .. }
    | N::AccentUnder { loc, .. }
    | N::Op { loc, .. }
    | N::OperatorName { loc, .. }
    | N::Font { loc, .. }
    | N::Text { loc, .. }
    | N::Color { loc, .. }
    | N::ColorToken { loc, .. }
    | N::Size { loc, .. }
    | N::Styling { loc, .. }
    | N::Sizing { loc, .. }
    | N::DelimSizing { loc, .. }
    | N::LeftRight { loc, .. }
    | N::LeftRightRight { loc, .. }
    | N::Middle { loc, .. }
    | N::Overline { loc, .. }
    | N::Underline { loc, .. }
    | N::Rule { loc, .. }
    | N::Kern { loc, .. }
    | N::Phantom { loc, .. }
    | N::VPhantom { loc, .. }
    | N::Smash { loc, .. }
    | N::MClass { loc, .. }
    | N::Array { loc, .. }
    | N::Environment { loc, .. }
    | N::Cr { loc, .. }
    | N::Infix { loc, .. }
    | N::Internal { loc, .. }
    | N::Verb { loc, .. }
    | N::Href { loc, .. }
    | N::Url { loc, .. }
    | N::Raw { loc, .. }
    | N::HBox { loc, .. }
    | N::HorizBrace { loc, .. }
    | N::Enclose { loc, .. }
    | N::Lap { loc, .. }
    | N::MathChoice { loc, .. }
    | N::RaiseBox { loc, .. }
    | N::VCenter { loc, .. }
    | N::XArrow { loc, .. }
    | N::Pmb { loc, .. }
    | N::Tag { loc, .. }
    | N::NoNumber { loc, .. }
    | N::Html { loc, .. }
    | N::HtmlMathMl { loc, .. }
    | N::IncludeGraphics { loc, .. }
    | N::CdLabel { loc, .. }
    | N::CdLabelParent { loc, .. }
    | N::CdArrow { loc, .. }
    | N::ProofTree { loc, .. }) = node;
    loc.as_mut().map(|loc| (&mut loc.start, &mut loc.end))
}
