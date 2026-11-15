//! Formulas as Word equations.
//!
//! The document holds a formula as TeX and Word as a tree of its own, Office MathML. Writing goes through
//! the structure the maths engine gives for a source. Reading, in `read/math.rs`, writes TeX for Word's tree.

use std::fmt::Write as _;

use roxmltree::Node;

/// The operators Word sets large with their limits, and the command for each.
pub(super) const BIG: &[(char, &str)] = &[
    ('\u{2211}', r"\sum"),
    ('\u{220f}', r"\prod"),
    ('\u{2210}', r"\coprod"),
    ('\u{222b}', r"\int"),
    ('\u{222c}', r"\iint"),
    ('\u{222d}', r"\iiint"),
    ('\u{222e}', r"\oint"),
    ('\u{22c3}', r"\bigcup"),
    ('\u{22c2}', r"\bigcap"),
    ('\u{22c1}', r"\bigvee"),
    ('\u{22c0}', r"\bigwedge"),
    ('\u{2a01}', r"\bigoplus"),
    ('\u{2a02}', r"\bigotimes"),
    ('\u{2a00}', r"\bigodot"),
    ('\u{2a04}', r"\biguplus"),
    ('\u{2a06}', r"\bigsqcup"),
];

/// Whether TeX sets a big operator's limits beside it, as it does an integral's.
pub(super) fn beside(operator: char) -> bool {
    ('\u{222b}'..='\u{2233}').contains(&operator)
}

/// The accents Word sets over a base: the mark MathML names each by, the combining character Word does, and
/// the command.
pub(super) const ACCENTS: &[(char, char, &str)] = &[
    ('^', '\u{302}', r"\hat"),
    ('~', '\u{303}', r"\tilde"),
    ('\u{af}', '\u{304}', r"\bar"),
    ('\u{2d9}', '\u{307}', r"\dot"),
    ('\u{a8}', '\u{308}', r"\ddot"),
    ('\u{b4}', '\u{301}', r"\acute"),
    ('`', '\u{300}', r"\grave"),
    ('\u{2c7}', '\u{30c}', r"\check"),
    ('\u{2d8}', '\u{306}', r"\breve"),
    ('\u{2da}', '\u{30a}', r"\mathring"),
    ('\u{2192}', '\u{20d7}', r"\vec"),
    ('\u{2190}', '\u{20d6}', r"\overleftarrow"),
    ('\u{2194}', '\u{20e1}', r"\overleftrightarrow"),
    ('\u{21c0}', '\u{20d1}', r"\overrightharpoon"),
    ('\u{21bc}', '\u{20d0}', r"\overleftharpoon"),
];

const OVERLINE: &str = "\u{203e}";
/// The mark MathML puts between a function's name and what it is applied to.
const APPLY: &str = "\u{2061}";

/// How a run with no properties starts and how any run ends, so that one can run on from the last.
const PLAIN: &str = "<m:r><m:t xml:space=\"preserve\">";
const END: &str = "</m:t></m:r>";

/// Writes a formula as a Word equation, an `m:oMath`. `display` says that it stands as a block of its own.
/// `None` for a source the engine cannot give the structure of.
pub(super) fn equation(tex: &str, display: bool) -> Option<String> {
    let mathml = letters_math::mathml(tex, display).ok()?;
    let tree = roxmltree::Document::parse(&mathml).ok()?;
    let mut out = String::from("<m:oMath>");
    row(&elements(tree.root_element()), display, &mut out);
    out.push_str("</m:oMath>");
    Some(out)
}

fn elements<'a>(parent: Node<'a, 'a>) -> Vec<Node<'a, 'a>> {
    parent.children().filter(Node::is_element).collect()
}

fn named(node: Node, name: &str) -> bool {
    node.tag_name().name() == name
}

fn is(node: Node, attribute: &str, value: &str) -> bool {
    node.attribute(attribute) == Some(value)
}

fn text<'a>(node: Node<'a, 'a>) -> &'a str {
    node.text().unwrap_or_default()
}

/// The one character of an operator that is one.
fn sign(node: Node) -> Option<char> {
    let mut characters = text(node).chars();
    characters.next().filter(|_| named(node, "mo") && characters.next().is_none())
}

fn escape(text: &str, out: &mut String) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            character => out.push(character),
        }
    }
}

/// Writes a property whose value is one character, or none.
fn character(name: &str, value: &str, out: &mut String) {
    let _ = write!(out, "<m:{name} m:val=\"");
    escape(value, out);
    out.push_str("\"/>");
}

/// Writes one argument of a structure, such as `m:e` or `m:sup`.
fn argument(name: &str, node: Option<Node>, display: bool, out: &mut String) {
    let _ = write!(out, "<m:{name}>");
    match node {
        // A plain row only groups, which the argument does by itself.
        Some(node) if named(node, "mrow") && fences(node).is_none() => row(&elements(node), display, out),
        Some(node) => row(&[node], display, out),
        None => {}
    }
    let _ = write!(out, "</m:{name}>");
}

/// Writes elements that follow one another.
fn row(nodes: &[Node], display: bool, out: &mut String) {
    let applied = |node: &Node| named(*node, "mo") && text(*node) == APPLY;
    // What a function or a big operator works on: whatever follows it, unless that is a sign.
    let operand = |at: usize| nodes.get(at).copied().filter(|next| !named(*next, "mo"));
    let mut at = 0;
    while let Some(node) = nodes.get(at).copied() {
        at += 1;
        // A function's name and what it is applied to are one structure in Word.
        if let Some(operand) = operand(at + 1).filter(|_| nodes.get(at).is_some_and(applied)) {
            at += 2;
            out.push_str("<m:func><m:fName>");
            element(node, display, out);
            out.push_str("</m:fName>");
            argument("e", Some(operand), display, out);
            out.push_str("</m:func>");
            continue;
        }
        // So are a big operator and what it sums. Alone, it is a sign like any other.
        match big(node).zip(operand(at)) {
            Some(((operator, sub, sup, stacked), operand)) => {
                at += 1;
                // Word sets an equation in a line of text tightly by itself, as TeX does.
                let stacked = if display { stacked } else { !beside(operator) };
                out.push_str("<m:nary><m:naryPr>");
                character("chr", operator.encode_utf8(&mut [0; 4]), out);
                let _ = write!(out, "<m:limLoc m:val=\"{}\"/>", if stacked { "undOvr" } else { "subSup" });
                for (name, script) in [("subHide", sub), ("supHide", sup)] {
                    if script.is_none() {
                        let _ = write!(out, "<m:{name} m:val=\"1\"/>");
                    }
                }
                out.push_str("</m:naryPr>");
                argument("sub", sub, false, out);
                argument("sup", sup, false, out);
                argument("e", Some(operand), display, out);
                out.push_str("</m:nary>");
            }
            None => element(node, display, out),
        }
    }
}

/// A big operator with the limits it has: its sign, what is under and over it, and whether they are set
/// there or beside it.
fn big<'a>(node: Node<'a, 'a>) -> Option<(char, Option<Node<'a, 'a>>, Option<Node<'a, 'a>>, bool)> {
    let known = |node: Node| sign(node).filter(|sign| BIG.iter().any(|(big, _)| big == sign));
    if let Some(operator) = known(node) {
        return Some((operator, None, None, !beside(operator)));
    }
    let parts = elements(node);
    let operator = known(*parts.first()?)?;
    let (second, third) = (parts.get(1).copied(), parts.get(2).copied());
    Some(match node.tag_name().name() {
        "msub" => (operator, second, None, false),
        "msup" => (operator, None, second, false),
        "msubsup" => (operator, second, third, false),
        "munder" => (operator, second, None, true),
        "mover" => (operator, None, second, true),
        "munderover" => (operator, second, third, true),
        _ => return None,
    })
}

/// The delimiters of a row that `\left` and `\right` made.
fn fences<'a>(node: Node<'a, 'a>) -> Option<(Node<'a, 'a>, Node<'a, 'a>)> {
    let fence = |node: Node, form: &str| named(node, "mo") && is(node, "fence", "true") && is(node, "form", form);
    let (first, last) = (node.first_element_child()?, node.last_element_child()?);
    (first != last && fence(first, "prefix") && fence(last, "postfix")).then_some((first, last))
}

fn element(node: Node, display: bool, out: &mut String) {
    let parts = elements(node);
    let part = |index: usize| parts.get(index).copied();
    match node.tag_name().name() {
        "mrow" => match fences(node) {
            Some((open, close)) => {
                let inner = &parts[1..parts.len() - 1];
                let divides = |node: &Node| named(*node, "mo") && is(*node, "fence", "true");
                out.push_str("<m:d><m:dPr>");
                character("begChr", text(open), out);
                if let Some(divider) = inner.iter().copied().find(|node| divides(node)) {
                    character("sepChr", text(divider), out);
                }
                character("endChr", text(close), out);
                out.push_str("</m:dPr>");
                for held in inner.split(divides) {
                    out.push_str("<m:e>");
                    row(held, display, out);
                    out.push_str("</m:e>");
                }
                out.push_str("</m:d>");
            }
            None => row(&parts, display, out),
        },
        "mi" | "mn" | "mo" | "mtext" => run(node, out),
        "msup" | "msub" | "msubsup" => {
            let (name, scripts): (_, &[&str]) = match node.tag_name().name() {
                "msup" => ("sSup", &["sup"]),
                "msub" => ("sSub", &["sub"]),
                _ => ("sSubSup", &["sub", "sup"]),
            };
            let _ = write!(out, "<m:{name}>");
            argument("e", part(0), display, out);
            for (index, script) in scripts.iter().enumerate() {
                argument(script, part(index + 1), false, out);
            }
            let _ = write!(out, "</m:{name}>");
        }
        "mfrac" => {
            out.push_str("<m:f>");
            if is(node, "linethickness", "0") {
                out.push_str("<m:fPr><m:type m:val=\"noBar\"/></m:fPr>");
            }
            argument("num", part(0), false, out);
            argument("den", part(1), false, out);
            out.push_str("</m:f>");
        }
        "msqrt" => {
            out.push_str("<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e>");
            row(&parts, display, out);
            out.push_str("</m:e></m:rad>");
        }
        "mroot" => {
            out.push_str("<m:rad>");
            argument("deg", part(1), false, out);
            argument("e", part(0), display, out);
            out.push_str("</m:rad>");
        }
        "mover" | "munder" => stack(node, part(0), part(1), display, out),
        "munderover" => match part(0).and_then(stretched) {
            // The arrow spans the label over it, and the one under is set beneath both.
            Some(sign) => {
                out.push_str("<m:limLow><m:e>");
                arrow(sign, part(2), true, out);
                out.push_str("</m:e>");
                argument("lim", part(1), false, out);
                out.push_str("</m:limLow>");
            }
            // Word has no structure with both, so the one under goes inside the one over.
            None => {
                out.push_str("<m:limUpp><m:e><m:limLow>");
                argument("e", part(0), display, out);
                argument("lim", part(1), false, out);
                out.push_str("</m:limLow></m:e>");
                argument("lim", part(2), false, out);
                out.push_str("</m:limUpp>");
            }
        },
        "mtable" => table(node, &parts, out),
        "mspace" => {
            let width = node.attribute("width").and_then(|width| width.strip_suffix("em")?.parse::<f32>().ok());
            // Word spaces maths with characters. A rule drawn with a space has none.
            let gap = match width.filter(|_| node.attribute("mathbackground").is_none()) {
                Some(width) if width >= 0.75 => "\u{2003}".repeat((width.round() as usize).clamp(1, 8)),
                Some(width) if width >= 0.4 => "\u{2002}".into(),
                Some(width) if width >= 0.25 => "\u{2004}".into(),
                Some(width) if width >= 0.2 => "\u{205f}".into(),
                Some(width) if width > 0.0 => "\u{2009}".into(),
                _ => String::new(),
            };
            if !gap.is_empty() {
                plain(&gap, out);
            }
        }
        "mphantom" => {
            out.push_str("<m:phant><m:phantPr><m:show m:val=\"0\"/></m:phantPr><m:e>");
            row(&parts, display, out);
            out.push_str("</m:e></m:phant>");
        }
        "menclose" => {
            let notation = node.attribute("notation").unwrap_or_default();
            let drawn = |name: &str| notation.split(' ').any(|drawn| drawn == name);
            let strikes = [
                ("strikeBLTR", drawn("updiagonalstrike")),
                ("strikeTLBR", drawn("downdiagonalstrike")),
                ("strikeH", drawn("horizontalstrike")),
            ];
            if !drawn("box") && strikes.iter().all(|(_, struck)| !struck) {
                return row(&parts, display, out);
            }
            out.push_str("<m:borderBox>");
            if !drawn("box") {
                out.push_str("<m:borderBoxPr>");
                for side in ["hideTop", "hideBot", "hideLeft", "hideRight"] {
                    let _ = write!(out, "<m:{side} m:val=\"1\"/>");
                }
                for (strike, _) in strikes.iter().filter(|(_, struck)| *struck) {
                    let _ = write!(out, "<m:{strike} m:val=\"1\"/>");
                }
                out.push_str("</m:borderBoxPr>");
            }
            out.push_str("<m:e>");
            row(&parts, display, out);
            out.push_str("</m:e></m:borderBox>");
        }
        // What only styles or nudges what it holds.
        _ => row(&parts, display, out),
    }
}

/// Writes a run of characters in the face its token asks for.
fn run(node: Node, out: &mut String) {
    let text = text(node);
    // A mark that a function is applied, with nothing to apply it to, says nothing.
    if text.is_empty() || text == APPLY {
        return;
    }
    let mut props = String::new();
    let variant = node.attribute("mathvariant").unwrap_or_default();
    if named(node, "mtext") && text.trim().is_empty() {
        // Words that are only space are a space.
    } else if named(node, "mtext") {
        props.push_str("<m:rPr><m:nor/></m:rPr>");
        let weight = [("bold", "<w:b/>"), ("italic", "<w:i/>")];
        let weight: String = weight.iter().filter(|(name, _)| variant.contains(name)).map(|(_, on)| *on).collect();
        if !weight.is_empty() {
            let _ = write!(props, "<w:rPr>{weight}</w:rPr>");
        }
    } else if text.chars().any(char::is_alphanumeric) {
        // Word slants a letter unless it is told otherwise, and MathML only a letter that stands alone.
        let alone = named(node, "mi") && text.chars().count() == 1;
        let (script, style) = match variant {
            "normal" => ("", "p"),
            "bold" => ("", "b"),
            "italic" => ("", "i"),
            "bold-italic" => ("", "bi"),
            "double-struck" => ("double-struck", "p"),
            "script" => ("script", "p"),
            "fraktur" => ("fraktur", "p"),
            "sans-serif" => ("sans-serif", "p"),
            "sans-serif-italic" => ("sans-serif", "i"),
            "monospace" => ("monospace", "p"),
            _ if named(node, "mi") && !alone => ("", "p"),
            _ => ("", ""),
        };
        if !script.is_empty() || !style.is_empty() {
            props.push_str("<m:rPr>");
            if !script.is_empty() {
                let _ = write!(props, "<m:scr m:val=\"{script}\"/>");
            }
            if !style.is_empty() {
                let _ = write!(props, "<m:sty m:val=\"{style}\"/>");
            }
            props.push_str("</m:rPr>");
        }
    }
    if props.is_empty() {
        return plain(text, out);
    }
    let _ = write!(out, "<m:r>{props}<m:t xml:space=\"preserve\">");
    // The engine holds the space at either end of words with one that does not break. Word keeps a plain one.
    let inner = text.trim_matches('\u{a0}');
    let start = text.len() - text.trim_start_matches('\u{a0}').len();
    let spaces = |held: &str| " ".repeat(held.chars().count());
    escape(&format!("{}{inner}{}", spaces(&text[..start]), spaces(&text[start + inner.len()..])), out);
    out.push_str(END);
}

/// Writes characters with no properties, as more of the run before them where that has none either.
fn plain(text: &str, out: &mut String) {
    let last = out.rfind("<m:r>").filter(|_| out.ends_with(END));
    match last.filter(|last| out[*last..].starts_with(PLAIN)) {
        Some(_) => out.truncate(out.len() - END.len()),
        None => out.push_str(PLAIN),
    }
    escape(text, out);
    out.push_str(END);
}

/// Writes a base with something over or under it: an accent, a bar, a brace, a labelled arrow, or a limit.
fn stack(node: Node, base: Option<Node>, mark: Option<Node>, display: bool, out: &mut String) {
    let over = named(node, "mover");
    let accent = is(node, "accent", "true") || is(node, "accentunder", "true");
    let group = |sign: &str, side: &str, base: Option<Node>, out: &mut String| {
        out.push_str("<m:groupChr><m:groupChrPr>");
        character("chr", sign, out);
        let _ = write!(out, "<m:pos m:val=\"{side}\"/>");
        // The base sits on the line, and the label of an arrow over it.
        let _ = write!(out, "<m:vertJc m:val=\"{}\"/></m:groupChrPr>", if side == "top" { "bot" } else { "top" });
        argument("e", base, display, out);
        out.push_str("</m:groupChr>");
    };
    let side = if over { "top" } else { "bot" };
    match (accent, mark.map(text)) {
        (true, Some(OVERLINE)) => {
            let _ = write!(out, "<m:bar><m:barPr><m:pos m:val=\"{side}\"/></m:barPr>");
            argument("e", base, display, out);
            out.push_str("</m:bar>");
        }
        (true, Some(sign)) => {
            let combining = sign.chars().next().and_then(|sign| ACCENTS.iter().find(|(mark, ..)| *mark == sign));
            match combining.filter(|_| over) {
                Some((_, combining, _)) => {
                    out.push_str("<m:acc><m:accPr>");
                    character("chr", combining.encode_utf8(&mut [0; 4]), out);
                    out.push_str("</m:accPr>");
                    argument("e", base, display, out);
                    out.push_str("</m:acc>");
                }
                // A brace, or a mark Word sets as it does one.
                None => group(sign, side, base, out),
            }
        }
        (false, _) if base.and_then(stretched).is_some() => {
            arrow(base.and_then(stretched).unwrap_or_default(), mark, over, out);
        }
        _ => {
            let name = if over { "limUpp" } else { "limLow" };
            let _ = write!(out, "<m:{name}>");
            argument("e", base, display, out);
            argument("lim", mark, false, out);
            let _ = write!(out, "</m:{name}>");
        }
    }
}

/// The sign of an operator that stretches to span what is set on it, as a labelled arrow does.
fn stretched<'a>(node: Node<'a, 'a>) -> Option<&'a str> {
    (named(node, "mo") && is(node, "stretchy", "true")).then(|| text(node))
}

/// Writes a stretching arrow with its label over or under it: the arrow is the mark on the line, and the
/// label what it spans.
fn arrow(sign: &str, label: Option<Node>, over: bool, out: &mut String) {
    let side = if over { "bot" } else { "top" };
    out.push_str("<m:groupChr><m:groupChrPr>");
    character("chr", sign, out);
    let _ = write!(out, "<m:pos m:val=\"{side}\"/><m:vertJc m:val=\"{side}\"/></m:groupChrPr>");
    argument("e", label, false, out);
    out.push_str("</m:groupChr>");
}

/// Writes a table as a matrix, or as Word's aligned equations where its columns meet as theirs do.
fn table(node: Node, rows: &[Node], out: &mut String) {
    let cells: Vec<Vec<Node>> = rows.iter().map(|row| elements(*row)).collect();
    let display = is(node, "displaystyle", "true");
    if node.attribute("columnspacing").is_some() {
        out.push_str("<m:eqArr>");
        for row in &cells {
            out.push_str("<m:e>");
            for (index, cell) in row.iter().enumerate() {
                if index > 0 {
                    out.push_str("<m:r><m:t>&amp;</m:t></m:r>");
                }
                self::row(&elements(*cell), display, out);
            }
            out.push_str("</m:e>");
        }
        out.push_str("</m:eqArr>");
        return;
    }
    let width = cells.iter().map(Vec::len).max().unwrap_or(0);
    let sides: Vec<&str> = node.attribute("columnalign").unwrap_or_default().split_whitespace().collect();
    out.push_str("<m:m><m:mPr><m:mcs>");
    for column in 0..width {
        // MathML's last word on alignment holds for the columns after it.
        let side = sides.get(column).or(sides.last()).copied().unwrap_or("center");
        let _ = write!(out, "<m:mc><m:mcPr><m:count m:val=\"1\"/><m:mcJc m:val=\"{side}\"/></m:mcPr></m:mc>");
    }
    out.push_str("</m:mcs></m:mPr>");
    for row in &cells {
        out.push_str("<m:mr>");
        for column in 0..width {
            out.push_str("<m:e>");
            if let Some(cell) = row.get(column) {
                self::row(&elements(*cell), display, out);
            }
            out.push_str("</m:e>");
        }
        out.push_str("</m:mr>");
    }
    out.push_str("</m:m>");
}
