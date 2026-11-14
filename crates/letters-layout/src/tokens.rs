//! The tokens of highlighted code. Whoever lexes maps its kinds onto [`TokenKind`], so layout needs no lexer.

use std::ops::Range;

/// What a token is, which decides its colour. The kinds are those of yewi's highlighter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// Set in italics as well as in its colour.
    Comment,
    Keyword,
    Literal,
    Number,
    String,
    Type,
    Function,
    Property,
    Tag,
    Variable,
    Meta,
    Inserted,
    Deleted,
}

/// A stretch of a code block's text, in bytes, and what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

/// The kind of the token that holds a byte offset. `tokens` are in order.
pub(crate) fn at(tokens: &[Token], offset: usize) -> Option<TokenKind> {
    let next = tokens.partition_point(|token| token.range.end <= offset);
    tokens.get(next).filter(|token| token.range.start <= offset).map(|token| token.kind)
}

/// Carries tokens across an edit of their text, so that colours hold still until the block is lexed again.
/// Tokens after the edit move with it, and typing at the end of a token grows it.
pub(crate) fn follow(tokens: &mut Vec<Token>, old: &str, new: &str) {
    let moved = carry(old, new);
    for token in tokens.iter_mut() {
        token.range = moved(token.range.start)..moved(token.range.end);
    }
    tokens.retain(|token| !token.range.is_empty());
}

/// Carries plain ranges across an edit of their text, as [`follow`] does tokens.
pub(crate) fn follow_ranges(ranges: &mut Vec<Range<usize>>, old: &str, new: &str) {
    let moved = carry(old, new);
    for range in ranges.iter_mut() {
        *range = moved(range.start)..moved(range.end);
    }
    ranges.retain(|range| !range.is_empty());
}

/// Where each byte offset of `old` lands in `new`, taking the two to differ by one edit.
fn carry(old: &str, new: &str) -> impl Fn(usize) -> usize {
    let (old, new) = (old.as_bytes(), new.as_bytes());
    let shorter = old.len().min(new.len());
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old.iter().rev().zip(new.iter().rev()).take(shorter - prefix).take_while(|(a, b)| a == b).count();
    // The edit replaced `old[prefix..cut]` with `new[prefix..pasted]`.
    let (cut, pasted) = (old.len() - suffix, new.len() - suffix);
    move |offset: usize| match offset {
        before if before < prefix => before,
        after if after >= cut => after - cut + pasted,
        _ => pasted,
    }
}
