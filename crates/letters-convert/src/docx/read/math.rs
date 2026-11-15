//! Word's equations as TeX.
//!
//! Each of Word's structures has a command that sets the same thing, so an equation reads as the source a
//! person would have typed. A sign takes the name the editor offers for it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use roxmltree::Node;

use crate::docx::math::{ACCENTS, BIG, beside};

/// How deep an equation is followed. Word nests a few levels, and a file may nest without end.
const MAX_DEPTH: usize = 64;

/// How many columns of a matrix are told how to stand. A file can claim any number.
const MAX_COLUMNS: usize = 64;

/// The delimiters TeX stretches, by the character Word names each with.
const DELIMITERS: &[(&str, &str)] = &[
    ("", "."),
    ("(", "("),
    (")", ")"),
    ("[", "["),
    ("]", "]"),
    ("{", r"\{"),
    ("}", r"\}"),
    ("|", "|"),
    ("\u{2223}", "|"),
    ("\u{2016}", r"\|"),
    ("\u{2225}", r"\|"),
    ("\u{27e8}", r"\langle"),
    ("\u{27e9}", r"\rangle"),
    ("\u{2329}", r"\langle"),
    ("\u{232a}", r"\rangle"),
    ("\u{230a}", r"\lfloor"),
    ("\u{230b}", r"\rfloor"),
    ("\u{2308}", r"\lceil"),
    ("\u{2309}", r"\rceil"),
    ("/", "/"),
    ("\\", r"\backslash"),
    ("\u{2191}", r"\uparrow"),
    ("\u{2193}", r"\downarrow"),
];

/// The environment for a matrix between each pair of delimiters.
const MATRICES: &[(&str, &str, &str)] = &[
    ("(", ")", "pmatrix"),
    ("[", "]", "bmatrix"),
    (r"\{", r"\}", "Bmatrix"),
    ("|", "|", "vmatrix"),
    (r"\|", r"\|", "Vmatrix"),
];

/// The command for each arrow Word stretches along a label.
const ARROWS: &[(char, &str)] = &[
    ('\u{2192}', r"\xrightarrow"),
    ('\u{2190}', r"\xleftarrow"),
    ('\u{2194}', r"\xleftrightarrow"),
    ('\u{21d2}', r"\xRightarrow"),
    ('\u{21d0}', r"\xLeftarrow"),
    ('\u{21d4}', r"\xLeftrightarrow"),
    ('\u{21a6}', r"\xmapsto"),
    ('\u{21aa}', r"\xhookrightarrow"),
    ('\u{21a9}', r"\xhookleftarrow"),
    ('\u{21cc}', r"\xrightleftharpoons"),
    ('\u{21cb}', r"\xleftrightharpoons"),
    ('=', r"\xlongequal"),
];

/// The commands for each arrow that is also set as a mark, over and under what it spans.
const SPANS: &[(char, &str, &str)] = &[
    ('\u{2192}', r"\overrightarrow", r"\underrightarrow"),
    ('\u{2190}', r"\overleftarrow", r"\underleftarrow"),
    ('\u{2194}', r"\overleftrightarrow", r"\underleftrightarrow"),
];

/// The command for each brace or bracket Word stretches over or under what it groups.
const BRACES: &[(char, &str)] = &[
    ('\u{23de}', r"\overbrace"),
    ('\u{23df}', r"\underbrace"),
    ('\u{23b4}', r"\overbracket"),
    ('\u{23b5}', r"\underbracket"),
];

/// The command for each face Word sets letters in.
const FACES: &[(&str, &str)] = &[
    ("double-struck", r"\mathbb"),
    ("script", r"\mathcal"),
    ("fraktur", r"\mathfrak"),
    ("sans-serif", r"\mathsf"),
    ("monospace", r"\mathtt"),
];

/// What the editor calls things, the other way round.
struct Names {
    /// The command for each sign.
    signs: BTreeMap<char, &'static str>,
    /// The functions with a command of their own, such as `sin`.
    functions: BTreeSet<&'static str>,
}

/// Lists every command by what the engine writes it as. The first to give a sign names it, which is the
/// one the editor offers first.
fn names() -> &'static Names {
    static NAMES: OnceLock<Names> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names = Names { signs: BTreeMap::new(), functions: BTreeSet::new() };
        for row in letters_math::commands().iter().filter(|row| row.arguments == 0) {
            let Ok(written) = letters_math::mathml(row.command, false) else { continue };
            let Ok(tree) = roxmltree::Document::parse(&written) else { continue };
            let mut parts = tree.root_element().children().filter(Node::is_element);
            let (Some(first), second, None) = (parts.next(), parts.next(), parts.next()) else { continue };
            let text = first.text().unwrap_or_default();
            let mut characters = text.chars();
            match (characters.next(), characters.next(), second) {
                (Some(sign), None, None)
                    if !sign.is_ascii() && first.attribute("mathvariant") != Some("double-struck") =>
                {
                    names.signs.entry(sign).or_insert(row.command);
                }
                // A function is written as its name, then the mark that it is applied.
                (Some(_), Some(_), Some(_)) if row.command.strip_prefix('\\') == Some(text) => {
                    names.functions.insert(&row.command[1..]);
                }
                _ => {}
            }
        }
        names
    })
}

/// Reads an `m:oMath` as TeX, and says whether all of its structure was followed.
pub(super) fn tex(math: Node) -> (String, bool) {
    let mut writer = Writer { out: String::new(), depth: 0, cut: false, aligned: false, array: false, naming: false };
    writer.items(math);
    (writer.out.trim_matches(' ').to_owned(), !writer.cut)
}

struct Writer {
    out: String,
    depth: usize,
    /// Whether the equation went deeper than it is followed.
    cut: bool,
    /// Whether an alignment mark was written into the array being read.
    aligned: bool,
    /// Whether what is being read is a row of an array, where `&` aligns.
    array: bool,
    /// Whether what is being read is the name of a function.
    naming: bool,
}

fn named(node: Node, name: &str) -> bool {
    node.tag_name().name() == name
}

/// The child of this name, whichever namespace prefix the file gives it.
fn child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children().find(|child| named(*child, name))
}

/// The one character a part holds, where it holds nothing else.
fn lone(node: Node, name: &str) -> Option<char> {
    let mut parts =
        child(node, name)?.children().filter(|part| part.is_element() && !part.tag_name().name().ends_with("Pr"));
    let run = parts.next().filter(|run| named(*run, "r") && parts.next().is_none())?;
    let text: String = run.children().filter(|text| named(*text, "t")).filter_map(|text| text.text()).collect();
    let mut characters = text.chars();
    characters.next().filter(|_| characters.next().is_none())
}

/// A value from an element's properties: `chr` of `accPr`, say. A property that is there with no value is
/// the empty one.
fn property<'a>(node: Node<'a, 'a>, name: &str) -> Option<&'a str> {
    let props = node.children().find(|child| child.tag_name().name().ends_with("Pr"))?;
    Some(val(child(props, name)?).unwrap_or_default())
}

/// The value an element is given.
fn val<'a>(node: Node<'a, 'a>) -> Option<&'a str> {
    node.attributes().find(|attribute| attribute.name() == "val").map(|value| value.value())
}

/// Whether an element that switches something on is there and does.
fn set(switch: Option<Node>) -> bool {
    switch.is_some_and(|switch| !matches!(val(switch), Some("0" | "false" | "off")))
}

/// Whether a property that switches something is on.
fn on(node: Node, name: &str) -> bool {
    set(node.children().find(|child| child.tag_name().name().ends_with("Pr")).and_then(|props| child(props, name)))
}

/// How the columns of a matrix are set where any is not centred: `lcr`, as an array is told.
fn columns(matrix: Node) -> Option<String> {
    let width = matrix
        .children()
        .filter(|row| named(*row, "mr"))
        .map(|row| row.children().filter(|cell| named(*cell, "e")).count());
    let width = width.max()?.min(MAX_COLUMNS);
    let mut sides = String::new();
    for column in child(child(matrix, "mPr")?, "mcs")?.children().filter(|column| named(*column, "mc")) {
        let count = property(column, "count").and_then(|count| count.parse().ok()).unwrap_or(1);
        let side = match property(column, "mcJc") {
            Some("left") => 'l',
            Some("right") => 'r',
            _ => 'c',
        };
        sides.extend(std::iter::repeat_n(side, count.min(width - sides.len().min(width))));
    }
    sides.extend(std::iter::repeat_n('c', width - sides.len()));
    sides.contains(['l', 'r']).then_some(sides)
}

/// Whether TeX takes this as one thing, so that it needs no braces to be an argument.
fn atomic(tex: &str) -> bool {
    let mut characters = tex.chars();
    match (characters.next(), characters.next()) {
        (Some(_), None) => true,
        (Some('\\'), Some(_)) => tex[1..].bytes().all(|byte| byte.is_ascii_alphabetic()) || tex.len() == 2,
        _ => false,
    }
}

impl Writer {
    /// Adds TeX. A command is kept apart from a letter that follows it, and so is a script of one character
    /// from what it would run into.
    fn push(&mut self, tex: &str) {
        let letters = self.out.trim_end_matches(|character: char| character.is_ascii_alphabetic());
        let command = letters.len() < self.out.len() && letters.ends_with('\\');
        let mut last = self.out.chars().rev().skip(1);
        let script = matches!(last.next(), Some('_' | '^')) && last.next() != Some('\\');
        let (letter, digit) = match tex.chars().next() {
            Some(first) => (first.is_ascii_alphabetic(), first.is_ascii_digit()),
            None => (false, false),
        };
        if command && letter || script && (letter || digit) {
            self.out.push(' ');
        }
        self.out.push_str(tex);
    }

    /// Reads what an element holds, apart from the output, and hands it back.
    fn held(&mut self, read: impl FnOnce(&mut Self)) -> String {
        let outer = std::mem::take(&mut self.out);
        read(self);
        std::mem::replace(&mut self.out, outer)
    }

    /// Reads an argument of a structure, such as `m:e` or `m:sup`.
    fn part(&mut self, node: Node, name: &str) -> String {
        match child(node, name) {
            Some(part) => self.held(|writer| writer.items(part)),
            None => String::new(),
        }
    }

    /// Adds something as an argument: in braces unless TeX takes it as one thing anyway.
    fn argument(&mut self, tex: &str) {
        if atomic(tex) {
            self.push(tex);
        } else {
            self.out.push('{');
            self.out.push_str(tex);
            self.out.push('}');
        }
    }

    /// Adds a command with one argument, always in braces.
    fn command(&mut self, command: &str, argument: &str) {
        self.push(command);
        self.out.push('{');
        self.out.push_str(argument);
        self.out.push('}');
    }

    fn scripts(&mut self, sub: &str, sup: &str) {
        // Primes are typed after the base, and TeX raises them itself.
        let raised = sup.trim_start_matches('\'');
        self.out.push_str(&sup[..sup.len() - raised.len()]);
        for (mark, script) in [('_', sub), ('^', raised)] {
            if !script.is_empty() {
                self.out.push(mark);
                self.argument(script);
            }
        }
    }

    /// Reads the elements inside one. Properties are not content.
    fn items(&mut self, parent: Node) {
        self.depth += 1;
        if self.depth <= MAX_DEPTH {
            for node in parent.children().filter(|node| node.is_element() && !node.tag_name().name().ends_with("Pr")) {
                self.item(node);
            }
        } else {
            // Deeper than any equation goes, the characters are kept and what they stood in is not.
            self.cut = true;
            parent.descendants().filter(|node| named(*node, "r")).for_each(|run| self.run(run));
        }
        self.depth -= 1;
    }

    fn item(&mut self, node: Node) {
        if named(node, "r") {
            return self.run(node);
        }
        // Only what stands in a row itself can align it.
        let array = std::mem::take(&mut self.array);
        match node.tag_name().name() {
            // What a tracked change took out is gone.
            "del" | "moveFrom" => {}
            "f" => {
                let (over, under) = (self.part(node, "num"), self.part(node, "den"));
                match property(node, "type") {
                    Some("noBar") => self.push(&format!(r"{{{over} \atop {under}}}")),
                    Some("lin" | "skw") => {
                        self.argument(&over);
                        self.out.push('/');
                        self.argument(&under);
                    }
                    _ => {
                        self.command(r"\frac", &over);
                        self.out.push('{');
                        self.out.push_str(&under);
                        self.out.push('}');
                    }
                }
            }
            "sSup" | "sSub" | "sSubSup" => {
                let (base, sub, sup) = (self.part(node, "e"), self.part(node, "sub"), self.part(node, "sup"));
                // A base of nothing still has to be there for the scripts to hang on.
                if base.is_empty() {
                    self.out.push_str("{}")
                } else {
                    self.argument(&base)
                }
                self.scripts(&sub, &sup);
            }
            "sPre" => {
                let (base, sub, sup) = (self.part(node, "e"), self.part(node, "sub"), self.part(node, "sup"));
                self.out.push_str("{}");
                self.scripts(&sub, &sup);
                self.argument(&base);
            }
            "rad" => {
                let (degree, inside) = (self.part(node, "deg"), self.part(node, "e"));
                self.push(r"\sqrt");
                if !degree.is_empty() && !on(node, "degHide") {
                    // A bracket in the degree would end it early.
                    match degree.contains(['[', ']']) {
                        true => self.out.push_str(&format!("[{{{degree}}}]")),
                        false => self.out.push_str(&format!("[{degree}]")),
                    }
                }
                self.out.push_str(&format!("{{{inside}}}"));
            }
            "d" => self.delimited(node),
            "nary" => {
                let sign = property(node, "chr").and_then(|sign| sign.chars().next()).unwrap_or('\u{222b}');
                match BIG.iter().find(|(big, _)| *big == sign) {
                    Some((_, command)) => self.push(command),
                    None => self.sign(sign),
                }
                // Said only where Word sets the limits another way than TeX would.
                let stacked = property(node, "limLoc").map(|place| place == "undOvr");
                match stacked {
                    Some(true) if beside(sign) => self.push(r"\limits"),
                    Some(false) if !beside(sign) => self.push(r"\nolimits"),
                    _ => {}
                }
                let hidden = |name: &str| on(node, name);
                let sub = if hidden("subHide") { String::new() } else { self.part(node, "sub") };
                let sup = if hidden("supHide") { String::new() } else { self.part(node, "sup") };
                self.scripts(&sub, &sup);
                let over = self.part(node, "e");
                if !over.is_empty() {
                    self.out.push(' ');
                    self.push(&over);
                }
            }
            "func" => {
                let outer = std::mem::replace(&mut self.naming, true);
                let name = self.part(node, "fName");
                self.naming = outer;
                let over = self.part(node, "e");
                self.push(&name);
                if !over.is_empty() {
                    self.out.push(' ');
                    self.push(&over);
                }
            }
            "acc" => {
                let mark = property(node, "chr").and_then(|mark| mark.chars().next()).unwrap_or('\u{302}');
                let base = self.part(node, "e");
                let known = ACCENTS.iter().find(|(spacing, combining, _)| *spacing == mark || *combining == mark);
                let wide = base.chars().count() > 1;
                let command = match known.map(|(.., command)| *command) {
                    _ if mark == '\u{305}' || mark == '\u{203e}' && wide => r"\overline",
                    _ if mark == '\u{203e}' => r"\bar",
                    Some(r"\bar") if wide => r"\overline",
                    Some(r"\hat") if wide => r"\widehat",
                    Some(r"\tilde") if wide => r"\widetilde",
                    Some(r"\vec") if wide => r"\overrightarrow",
                    Some(command) => command,
                    None => r"\hat",
                };
                self.command(command, &base);
            }
            "bar" => {
                let base = self.part(node, "e");
                let command = if property(node, "pos") == Some("top") { r"\overline" } else { r"\underline" };
                self.command(command, &base);
            }
            "borderBox" => {
                let inside = self.part(node, "e");
                let hidden = ["hideTop", "hideBot", "hideLeft", "hideRight"].iter().all(|side| on(node, side));
                let command = match (on(node, "strikeBLTR"), on(node, "strikeTLBR"), on(node, "strikeH")) {
                    (true, true, _) => Some(r"\xcancel"),
                    (true, ..) => Some(r"\cancel"),
                    (_, true, _) => Some(r"\bcancel"),
                    (.., true) => Some(r"\sout"),
                    _ if hidden => None,
                    _ => Some(r"\boxed"),
                };
                match command {
                    Some(command) => self.command(command, &inside),
                    None => self.push(&inside),
                }
            }
            "groupChr" => {
                let base = self.part(node, "e");
                let mark = property(node, "chr").unwrap_or("\u{23df}");
                let top = property(node, "pos") == Some("top");
                // Whether the base sits on the line, and the mark over or under it.
                let hangs = (property(node, "vertJc") == Some("bot")) == top;
                let mark = {
                    let mut characters = mark.chars();
                    characters.next().filter(|_| characters.next().is_none())
                };
                let brace = BRACES.iter().find(|(brace, _)| Some(*brace) == mark);
                let span = SPANS.iter().find(|(arrow, ..)| Some(*arrow) == mark && hangs);
                let arrow = ARROWS.iter().find(|(arrow, _)| Some(*arrow) == mark);
                match (brace, span, arrow) {
                    (Some((_, command)), ..) => self.command(command, &base),
                    (_, Some((_, over, under)), _) => self.command(if top { over } else { under }, &base),
                    // The arrow sits on the line, and what it spans is its label.
                    (.., Some((_, labelled))) if top => {
                        self.push(labelled);
                        self.out.push_str(&format!("[{base}]{{}}"));
                    }
                    (.., Some((_, labelled))) => self.command(labelled, &base),
                    _ => self.push(&base),
                }
            }
            "limLow" | "limUpp" => {
                let (base, limit) = (self.part(node, "e"), self.part(node, "lim"));
                let under = named(node, "limLow");
                let side = if under { r"\under" } else { r"\over" };
                let (sign, mark) = (lone(node, "e"), lone(node, "lim"));
                // A brace set as a limit is the brace of what it is under or over.
                let brace = BRACES.iter().find(|(brace, command)| Some(*brace) == mark && command.starts_with(side));
                let arrow = ARROWS.iter().find(|(arrow, _)| Some(*arrow) == sign);
                // An arrow that spans a label over it takes the one under it too.
                let labelled = ARROWS.iter().find_map(|(_, command)| {
                    let label = base.strip_prefix(command)?.strip_prefix('{')?;
                    (under && label.ends_with('}')).then_some((command, label))
                });
                let big = sign.filter(|sign| BIG.iter().any(|(big, _)| big == sign));
                // TeX sets a limit under these by itself. Anything else has to be stacked by hand.
                let function = base.strip_prefix('\\').is_some_and(|name| names().functions.contains(name));
                let braced = base.starts_with(if under { r"\underbrace{" } else { r"\overbrace{" });
                if let Some((_, command)) = brace {
                    self.command(command, &base);
                } else if let Some((_, command)) = arrow {
                    self.push(command);
                    self.out.push_str(&if under { format!("[{limit}]{{}}") } else { format!("{{{limit}}}") });
                } else if let Some((command, label)) = labelled {
                    self.push(&format!("{command}[{limit}]{{{label}"));
                } else if big.is_some() || function && under || braced {
                    self.push(&base);
                    if big.is_some_and(beside) {
                        self.push(r"\limits");
                    }
                    let (sub, sup) = if under { (limit.as_str(), "") } else { ("", limit.as_str()) };
                    self.scripts(sub, sup);
                } else {
                    self.command(&format!("{side}set"), &limit);
                    self.out.push_str(&format!("{{{base}}}"));
                }
            }
            "m" => {
                let rows = self.rows(node, "mr", false);
                match columns(node) {
                    // Columns that meet in pairs are equations set out to line up, as tools other than Word
                    // write them.
                    Some(sides) if sides.as_bytes().chunks(2).all(|pair| pair == b"rl") => {
                        self.environment("aligned", "aligned", &rows);
                    }
                    Some(sides) => self.environment(&format!("array}}{{{sides}"), "array", &rows),
                    None => self.environment("matrix", "matrix", &rows),
                }
            }
            "eqArr" => {
                let outer = std::mem::take(&mut self.aligned);
                let rows = self.rows(node, "e", true);
                let aligned = std::mem::replace(&mut self.aligned, outer);
                let name = if aligned { "aligned" } else { "gathered" };
                self.environment(name, name, &rows);
            }
            "phant" => {
                let inside = self.part(node, "e");
                // A phantom that shows is only spacing rules, which TeX has its own.
                if property(node, "show").is_some() && !on(node, "show") {
                    self.command(r"\phantom", &inside);
                } else {
                    self.push(&inside);
                }
            }
            // A box, a nested equation, and whatever Word adds later: what it holds is read.
            _ => {
                self.array = array;
                self.items(node);
            }
        }
        self.array = array;
    }

    /// Reads the rows of a matrix or of an array of equations, each as its cells with `&` between them.
    fn rows(&mut self, node: Node, row: &str, array: bool) -> Vec<String> {
        let outer = std::mem::replace(&mut self.array, array);
        let rows = node.children().filter(|child| named(*child, row));
        let rows = rows
            .map(|row| match array {
                true => self.held(|writer| writer.items(row)),
                false => {
                    let cells = row.children().filter(|cell| named(*cell, "e"));
                    let cells: Vec<String> = cells.map(|cell| self.held(|writer| writer.items(cell))).collect();
                    cells.join(" & ")
                }
            })
            .collect();
        self.array = outer;
        rows
    }

    /// Adds rows between how an environment begins, which may say more than its name, and its end.
    fn environment(&mut self, begin: &str, end: &str, rows: &[String]) {
        self.push(&format!(r"\begin{{{begin}}} {} \end{{{end}}}", rows.join(r" \\ ")));
    }

    /// Reads what sits between delimiters.
    fn delimited(&mut self, node: Node) {
        let delimiter = |name: &str, default: &'static str| {
            let sign = property(node, name).unwrap_or(default);
            DELIMITERS.iter().find(|(known, _)| *known == sign).map(|(_, tex)| *tex).ok_or(sign)
        };
        let (open, close) = (delimiter("begChr", "("), delimiter("endChr", ")"));
        let parts: Vec<Node> = node.children().filter(|child| named(*child, "e")).collect();
        // What the one thing inside is, when there is one.
        let only = match parts.as_slice() {
            [part] => {
                let mut inside =
                    part.children().filter(|child| child.is_element() && !child.tag_name().name().ends_with("Pr"));
                inside.next().filter(|_| inside.next().is_none())
            }
            _ => None,
        };
        let table = only.filter(|only| named(*only, "m") || named(*only, "eqArr"));
        match (open, close, only, table) {
            // A brace that opens rows and is never closed sets out cases.
            (Ok(r"\{"), Ok("."), _, Some(table)) => {
                let outer = std::mem::take(&mut self.aligned);
                let rows = if named(table, "m") { self.rows(table, "mr", false) } else { self.rows(table, "e", true) };
                self.aligned = outer;
                self.environment("cases", "cases", &rows);
            }
            (Ok(open), Ok(close), _, Some(table))
                if named(table, "m")
                    && columns(table).is_none()
                    && MATRICES.iter().any(|(left, right, _)| (*left, *right) == (open, close)) =>
            {
                let name = MATRICES
                    .iter()
                    .find(|(left, right, _)| (*left, *right) == (open, close))
                    .map_or("matrix", |(.., name)| *name);
                let rows = self.rows(table, "mr", false);
                self.environment(name, name, &rows);
            }
            (Ok("("), Ok(")"), Some(fraction), _)
                if named(fraction, "f") && property(fraction, "type") == Some("noBar") =>
            {
                let (over, under) = (self.part(fraction, "num"), self.part(fraction, "den"));
                self.command(r"\binom", &over);
                self.out.push_str(&format!("{{{under}}}"));
            }
            _ => {
                let divider = delimiter("sepChr", "|");
                // A delimiter TeX cannot stretch stands as a sign beside the place it would have been.
                let side = |writer: &mut Self, command: &str, delimiter: Result<&str, &str>, before: bool| {
                    let sign = delimiter.err().unwrap_or_default();
                    if before {
                        sign.chars().for_each(|sign| writer.sign(sign));
                    }
                    writer.push(command);
                    writer.push(delimiter.unwrap_or("."));
                    if !before {
                        sign.chars().for_each(|sign| writer.sign(sign));
                    }
                };
                side(self, r"\left", open, true);
                for (index, part) in parts.iter().enumerate() {
                    if index > 0 {
                        side(self, r"\middle", divider, false);
                    }
                    let inside = self.held(|writer| writer.items(*part));
                    self.push(&inside);
                }
                side(self, r"\right", close, false);
            }
        }
    }

    /// Reads a run: its characters, in the face its properties ask for.
    fn run(&mut self, node: Node) {
        let texts = node.children().filter(|child| named(*child, "t"));
        let text: String = texts.flat_map(|text| text.children()).filter_map(|text| text.text()).collect();
        if text.is_empty() {
            return;
        }
        // Properties of maths, as against those of text, which a run may carry too.
        let maths = |child: &Node| child.tag_name().namespace().is_some_and(|space| space.ends_with("/math"));
        let props = node.children().find(|child| named(*child, "rPr") && maths(child));
        let has = |name: &str| props.and_then(|props| child(props, name));
        let struck = |props: Node| named(props, "rPr") && !maths(&props) && child(props, "del").is_some();
        if node.children().any(struck) {
            return;
        }
        let value = |name: &str| has(name).and_then(val);
        if set(has("nor")) {
            let words = self.held(|writer| text.chars().for_each(|character| writer.letter(character)));
            // Words are text, and take their weight as text does.
            let look = node.children().find(|child| named(*child, "rPr") && !maths(child));
            let weight = |name: &str| set(look.and_then(|look| child(look, name)));
            return match (weight("b"), weight("i")) {
                (true, true) => self.command(r"\textbf", &format!(r"\textit{{{words}}}")),
                (true, false) => self.command(r"\textbf", &words),
                (false, true) => self.command(r"\textit", &words),
                (false, false) => self.command(r"\text", &words),
            };
        }
        if has("aln").is_some() && self.array {
            self.out.push('&');
            self.aligned = true;
        }
        let signs = self.held(|writer| text.chars().for_each(|character| writer.sign(character)));
        let word = text.len() > 1 && text.bytes().all(|byte| byte.is_ascii_alphabetic());
        let style = value("sty");
        let upright = style == Some("p") || self.naming;
        let face = value("scr").and_then(|face| FACES.iter().find(|(known, _)| *known == face));
        match (face, style) {
            (Some((_, command)), _) => self.command(command, &signs),
            // Upright letters are a function TeX knows, one it does not, or just upright.
            _ if word && upright && names().functions.contains(text.as_str()) => self.push(&format!(r"\{text}")),
            _ if word && self.naming => self.command(r"\operatorname", &signs),
            // Only Latin letters slant in TeX to begin with.
            (None, Some("p")) if text.bytes().any(|byte| byte.is_ascii_alphabetic()) => {
                self.command(r"\mathrm", &signs);
            }
            (None, Some("b")) if text.bytes().any(|byte| byte.is_ascii_alphanumeric()) => {
                self.command(r"\mathbf", &signs);
            }
            (None, Some("b" | "bi")) => self.command(r"\boldsymbol", &signs),
            _ => self.push(&signs),
        }
    }

    /// Adds one character of maths as TeX writes it.
    fn sign(&mut self, sign: char) {
        match sign {
            '&' if self.array => {
                self.out.push('&');
                self.aligned = true;
            }
            '{' | '}' | '%' | '&' | '#' | '_' | '$' => {
                self.out.push('\\');
                self.out.push(sign);
            }
            '\\' => self.push(r"\backslash"),
            '~' => self.push(r"\sim"),
            '^' => self.push(r"\wedge"),
            ' ' => self.push(r"\ "),
            '\u{a0}' => self.out.push('~'),
            '\u{2009}' | '\u{2006}' => self.push(r"\,"),
            '\u{205f}' | '\u{2005}' => self.push(r"\:"),
            '\u{2004}' => self.push(r"\;"),
            '\u{2002}' | '\u{2000}' => self.push(r"\enspace"),
            '\u{2003}' | '\u{2001}' => self.push(r"\quad"),
            // Marks that say how to read the maths aloud, and spaces of no width.
            '\u{2061}'..='\u{2064}' | '\u{200a}'..='\u{200d}' => {}
            '\u{2212}' => self.out.push('-'),
            '\u{2217}' => self.out.push('*'),
            '\u{2032}' => self.out.push('\''),
            '\u{2033}' => self.out.push_str("''"),
            '\u{22ee}' => self.push(r"\vdots"),
            '\u{2223}' => self.push(r"\mid"),
            '\u{2016}' => self.push(r"\|"),
            sign if sign.is_ascii() => self.push(sign.encode_utf8(&mut [0; 4])),
            sign => match names().signs.get(&sign) {
                Some(command) => self.push(command),
                None => self.out.push(sign),
            },
        }
    }

    /// Adds one character of words as TeX writes it inside `\text`.
    fn letter(&mut self, letter: char) {
        match letter {
            '{' | '}' | '%' | '&' | '#' | '_' | '$' => {
                self.out.push('\\');
                self.out.push(letter);
            }
            '\\' => self.out.push_str(r"\textbackslash{}"),
            '~' => self.out.push_str(r"\textasciitilde{}"),
            '^' => self.out.push_str(r"\textasciicircum{}"),
            '\u{a0}' => self.out.push('~'),
            letter => self.out.push(letter),
        }
    }
}
