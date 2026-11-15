//! The parts of a formula's TeX source, for a sheet that colours them.
//!
//! This reads only as far as colour needs: commands, words, comments, and which bracket pairs with which.
//! Whether the source is a formula at all is the maths engine's to say.

use std::ops::Range;

/// What a stretch of source is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// `\frac`, `\,`, `\\`.
    Command,
    /// What `\text` and its like set as words, and the name of an environment in its braces.
    Text,
    /// `%` and the rest of its line.
    Comment,
    /// One of a pair, and how many pairs it is inside: a bracket, `\left` and `\right`, `\begin` and `\end`.
    Paired(usize),
    /// What opens a pair that nothing closes, as deep.
    Open(usize),
    /// What closes a pair that nothing opened.
    Stray,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    /// Where it is in the source, in bytes.
    pub range: Range<usize>,
    pub part: Part,
}

/// What a pair is made of.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pair {
    Brace,
    Round,
    Square,
    /// `\left` and `\right`.
    Sized,
    /// `\begin` and `\end`.
    Environment,
}

impl Pair {
    fn of(bracket: u8) -> Self {
        match bracket {
            b'{' | b'}' => Pair::Brace,
            b'(' | b')' => Pair::Round,
            _ => Pair::Square,
        }
    }

    /// Whether one without the other is a mistake. `(0, 1]` is maths, `{0, 1` is not TeX.
    fn strict(self) -> bool {
        !matches!(self, Pair::Round | Pair::Square)
    }
}

/// What a command makes of the bracket after it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum After {
    Nothing,
    /// The delimiter `\left` or `\right` sizes, which pairs with nothing itself.
    Sized,
    /// Braces that hold words.
    Worded,
    /// Braces that hold an environment's name.
    Named,
}

/// The commands whose argument is words, not maths.
const WORDED: [&str; 10] =
    ["text", "textrm", "textbf", "textit", "textsf", "texttt", "textnormal", "mbox", "operatorname", "tag"];

/// The parts being read: each stretch found, and what is still open.
#[derive(Default)]
struct Reading {
    /// A round or square bracket is a part only once its partner is found.
    tokens: Vec<(Range<usize>, Option<Part>)>,
    /// What is open: its kind, and its token.
    open: Vec<(Pair, usize)>,
}

impl Reading {
    fn part(&mut self, range: Range<usize>, part: Part) {
        self.tokens.push((range, Some(part)));
    }

    fn opens(&mut self, pair: Pair, range: Range<usize>) {
        let part = pair.strict().then_some(Part::Open(self.open.len()));
        self.open.push((pair, self.tokens.len()));
        self.tokens.push((range, part));
    }

    fn closes(&mut self, pair: Pair, range: Range<usize>) {
        // A pair closes over round and square brackets left open inside it, which is plain maths, and never
        // over a brace, a `\left` or a `\begin`.
        let inside = self.open.iter().rposition(|(other, _)| other.strict()).map_or(0, |strict| strict + 1);
        let found = match pair.strict() {
            true => inside.checked_sub(1).filter(|&depth| self.open[depth].0 == pair),
            false => self.open[inside..].iter().rposition(|(other, _)| *other == pair).map(|depth| inside + depth),
        };
        let part = match found {
            Some(depth) => {
                let (_, opener) = self.open[depth];
                self.open.truncate(depth);
                self.tokens[opener].1 = Some(Part::Paired(depth));
                Some(Part::Paired(depth))
            }
            None => pair.strict().then_some(Part::Stray),
        };
        self.tokens.push((range, part));
    }
}

/// The parts of `source` that are more than plain maths, in order.
pub(crate) fn parts(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut reading = Reading::default();
    let mut after = After::Nothing;
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        let before = std::mem::replace(&mut after, After::Nothing);
        match bytes[at] {
            b'\\' => {
                let letters = bytes[at + 1..].iter().take_while(|byte| byte.is_ascii_alphabetic()).count();
                // Without letters the command is the one character after the backslash.
                let other = source[at + 1..].chars().next().map_or(0, char::len_utf8);
                at += 1 + if letters > 0 { letters } else { other };
                let name = &source[start + 1..at];
                match name {
                    "left" => reading.opens(Pair::Sized, start..at),
                    "right" => reading.closes(Pair::Sized, start..at),
                    "begin" => reading.opens(Pair::Environment, start..at),
                    "end" => reading.closes(Pair::Environment, start..at),
                    _ => reading.part(start..at, Part::Command),
                }
                after = match name {
                    "left" | "right" | "middle" => After::Sized,
                    "begin" | "end" => After::Named,
                    _ if WORDED.contains(&name) => After::Worded,
                    _ => After::Nothing,
                };
            }
            b'%' => {
                at += source[at..].find('\n').unwrap_or(source.len() - at);
                reading.part(start..at, Part::Comment);
            }
            b'{' if before == After::Named => {
                // The name is read with its braces, which are no pair of the formula's.
                let name = words(&source[at + 1..]);
                at = (at + 1 + name + 1).min(source.len());
                reading.part(start..at, Part::Text);
            }
            b'{' | b'(' | b'[' if before != After::Sized => {
                at += 1;
                reading.opens(Pair::of(bytes[start]), start..at);
                if before == After::Worded && bytes[start] == b'{' {
                    let words = words(&source[at..]);
                    reading.tokens.extend((words > 0).then(|| (at..at + words, Some(Part::Text))));
                    at += words;
                }
            }
            b'}' | b')' | b']' if before != After::Sized => {
                at += 1;
                reading.closes(Pair::of(bytes[start]), start..at);
            }
            byte if byte.is_ascii_whitespace() => {
                // `\left (` and `\text {a}` mean what they do without the space.
                after = before;
                at += 1;
            }
            _ => at += source[at..].chars().next().map_or(1, char::len_utf8),
        }
    }
    reading.tokens.into_iter().filter_map(|(range, part)| Some(Token { range, part: part? })).collect()
}

/// How far the words in braces run: to the brace that closes them, or the end of the source.
fn words(after: &str) -> usize {
    let mut depth = 0;
    let mut escaped = false;
    for (at, byte) in after.bytes().enumerate() {
        match byte {
            _ if escaped => {}
            b'{' => depth += 1,
            b'}' if depth == 0 => return at,
            b'}' => depth -= 1,
            _ => {}
        }
        escaped = byte == b'\\' && !escaped;
    }
    after.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parts of `source`, each with the text it covers.
    fn read(source: &str) -> Vec<(&str, Part)> {
        parts(source).into_iter().map(|token| (&source[token.range], token.part)).collect()
    }

    #[test]
    fn commands_words_and_comments_are_told_from_plain_maths() {
        use Part::{Command, Comment, Paired, Text};
        // A source, and its parts.
        let cases: [(&str, &[(&str, Part)]); 7] = [
            ("x + 12", &[]),
            ("\\alpha\\,\\beta", &[("\\alpha", Command), ("\\,", Command), ("\\beta", Command)]),
            ("a \\\\ b \\{ \\%", &[("\\\\", Command), ("\\{", Command), ("\\%", Command)]),
            ("\\text{if } x", &[("\\text", Command), ("{", Paired(0)), ("if ", Text), ("}", Paired(0))]),
            ("\\text{a {b} \\} c}", &[("\\text", Command), ("{", Paired(0)), ("a {b} \\} c", Text), ("}", Paired(0))]),
            ("x % half\ny", &[("% half", Comment)]),
            ("\\", &[("\\", Command)]),
        ];
        for (source, parts) in cases {
            assert_eq!(read(source), parts, "{source}");
        }
    }

    #[test]
    fn brackets_pair_by_how_deep_they_are_nested() {
        use Part::{Command, Open, Paired, Stray, Text};
        // A source, and its parts.
        let cases: [(&str, &[(&str, Part)]); 10] = [
            ("{a{b}}", &[("{", Paired(0)), ("{", Paired(1)), ("}", Paired(1)), ("}", Paired(0))]),
            ("f(x[i])", &[("(", Paired(0)), ("[", Paired(1)), ("]", Paired(1)), (")", Paired(0))]),
            // A half-open interval is maths, and its brackets are no pair.
            ("(0, 1]", &[]),
            ("{(0, 1]}", &[("{", Paired(0)), ("}", Paired(0))]),
            ("{a{b}", &[("{", Open(0)), ("{", Paired(1)), ("}", Paired(1))]),
            ("a}", &[("}", Stray)]),
            // The delimiters `\left` and `\right` size need not match, and a brace does not close over them.
            ("\\left( a } \\right]", &[("\\left", Paired(0)), ("}", Stray), ("\\right", Paired(0))]),
            ("\\left ( a", &[("\\left", Open(0))]),
            (
                "\\begin{matrix} a \\end{matrix}",
                &[("\\begin", Paired(0)), ("{matrix}", Text), ("\\end", Paired(0)), ("{matrix}", Text)],
            ),
            (
                "\\sqrt[3]{x}",
                &[("\\sqrt", Command), ("[", Paired(0)), ("]", Paired(0)), ("{", Paired(0)), ("}", Paired(0))],
            ),
        ];
        for (source, parts) in cases {
            assert_eq!(read(source), parts, "{source}");
        }
    }
}
