//! Colours for code: yewi's highlighter lexes a code block, and its tokens go to the layout as stretches of
//! the block's text.

use letters_layout::{Token, TokenKind};
use yewi_highlight::TokenKind as Lexed;

/// The tokens of `source` in a language, in order. Plain text between them is left out, and a language the
/// highlighter does not know has none.
pub(crate) fn tokens(source: &str, language: &str) -> Vec<Token> {
    let mut at = 0;
    let lexed = yewi_highlight::tokenize(source, language).into_iter().filter_map(|token| {
        let range = at..at + token.text.len();
        at = range.end;
        Some(Token { range, kind: kind(token.kind?) })
    });
    lexed.collect()
}

fn kind(kind: Lexed) -> TokenKind {
    match kind {
        Lexed::Comment => TokenKind::Comment,
        Lexed::Keyword => TokenKind::Keyword,
        Lexed::Literal => TokenKind::Literal,
        Lexed::Number => TokenKind::Number,
        Lexed::String => TokenKind::String,
        Lexed::Type => TokenKind::Type,
        Lexed::Function => TokenKind::Function,
        Lexed::Property => TokenKind::Property,
        Lexed::Tag => TokenKind::Tag,
        Lexed::Variable => TokenKind::Variable,
        Lexed::Meta => TokenKind::Meta,
        Lexed::Inserted => TokenKind::Inserted,
        Lexed::Deleted => TokenKind::Deleted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_stretches_of_the_source() {
        let source = "let n = 42; // the answer";
        let tokens = tokens(source, "rust");
        let found = |kind| tokens.iter().find(|token| token.kind == kind).map(|token| &source[token.range.clone()]);
        assert_eq!(found(TokenKind::Keyword), Some("let"));
        assert_eq!(found(TokenKind::Number), Some("42"));
        assert_eq!(found(TokenKind::Comment), Some("// the answer"));
        assert!(tokens.windows(2).all(|pair| pair[0].range.end <= pair[1].range.start), "in order: {tokens:?}");
    }

    #[test]
    fn text_in_no_known_language_has_no_tokens() {
        for language in ["", "klingon"] {
            assert_eq!(tokens("let n = 42;", language), Vec::new(), "{language:?}");
        }
    }
}
