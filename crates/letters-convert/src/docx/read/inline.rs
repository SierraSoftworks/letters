//! What a paragraph holds: runs and their marks, links, fields, notes, and the drawings that sit in the text.

use std::collections::BTreeSet;

use letters_model::{Block, BlockKind, Mark};
use roxmltree::Node;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{DIRECT, Field, Inline, MAX_DEPTH, Para, Reader, Spot, math, text_of};
use crate::docx::monospace;
use crate::docx::package::Element;
use crate::docx::styles::{Role, RunProps, code_family};

/// Fields left open beyond this many are not followed, so a file cannot make every run cost a long search.
const MAX_FIELDS: usize = 32;
/// A picture larger than this is left out.
const PICTURE_LIMIT: usize = 64 << 20;

const TRACKED: &str = "Tracked changes were accepted";
const FOOTNOTE: &str = "Footnote kept as text";
const EQUATION: &str = "An equation holds something Letters cannot typeset. Its source is kept as Word had it";
const SHIFTED: &str = "Superscript and subscript became plain text";
const SYMBOL: &str = "A symbol from a symbol font could not be converted";
const INTERNAL_LINK: &str = "A link to a place in the document was kept as text";
const TEXT_BOX: &str = "Text box flattened into the text here";
const DRAWING: &str = "A drawing or chart was left out";
const BREAK_MOVED: &str = "A page break inside the paragraph now follows it";

impl Reader<'_, '_> {
    /// Reads the runs inside a paragraph, a link or anything else that wraps them.
    pub(super) fn inline(&mut self, parent: Node, around: &Inline, para: &mut Para) {
        if around.depth > MAX_DEPTH {
            return;
        }
        for node in parent.children() {
            match node.word() {
                Some("r") => self.run(node, around, para),
                Some("hyperlink") => {
                    let address = node.attr("id").and_then(|id| around.rels.get(id)).filter(|rel| rel.external);
                    let link = address.map(|rel| match node.attr("anchor") {
                        Some(anchor) => format!("{}#{anchor}", rel.target),
                        None => rel.target.clone(),
                    });
                    if link.is_none() && node.attr("anchor").is_some() {
                        para.note(INTERNAL_LINK);
                    }
                    self.inline(node, &around.deeper(link), para);
                }
                Some("ins" | "moveTo") => {
                    para.note(TRACKED);
                    self.inline(node, &around.deeper(None), para);
                }
                Some("del" | "moveFrom") => para.note(TRACKED),
                Some("smartTag" | "customXml" | "dir" | "bdo") => self.inline(node, &around.deeper(None), para),
                Some("sdt") => {
                    let ticked = node.child("sdtPr").and_then(|props| {
                        let checkbox = props.children().find(|child| child.tag_name().name() == "checkbox")?;
                        let state = checkbox.children().find(|child| child.tag_name().name() == "checked");
                        Some(state.is_some_and(|state| matches!(state.attr("val"), Some("1" | "true"))))
                    });
                    match ticked {
                        // A checkbox that opens a paragraph makes it a task, and is not text itself.
                        Some(ticked) if para.len == 0 => para.task = Some(ticked),
                        _ => {
                            if let Some(content) = node.child("sdtContent") {
                                self.inline(content, &around.deeper(None), para);
                            }
                        }
                    }
                }
                Some("fldSimple") => {
                    let code = node.attr("instr").unwrap_or_default();
                    para.contents |= is_contents(code);
                    self.inline(node, &around.deeper(hyperlink(code)), para);
                }
                Some("commentRangeStart") => self.span(node).0 = Some(Spot { para: para.index, at: para.len }),
                Some("commentRangeEnd") => self.span(node).1 = Some(Spot { para: para.index, at: para.len }),
                _ if matches!(node.tag_name().name(), "oMath" | "oMathPara") && node.word().is_none() => {
                    // A group of equations is one formula with a line for each.
                    let lines = node.children().filter(|line| line.tag_name().name() == "oMath");
                    let lines = if node.tag_name().name() == "oMath" { vec![node] } else { lines.collect() };
                    let (mut read, mut whole) = (Vec::new(), true);
                    for (tex, followed) in lines.into_iter().map(math::tex) {
                        whole &= followed;
                        read.extend(Some(tex).filter(|tex| !tex.is_empty()));
                    }
                    let tex = match read.as_slice() {
                        [] => continue,
                        [line] => line.clone(),
                        lines => format!(r"\begin{{gathered}} {} \end{{gathered}}", lines.join(r" \\ ")),
                    };
                    if node.descendants().any(|part| matches!(part.word(), Some("ins" | "del" | "moveFrom" | "moveTo")))
                    {
                        para.note(TRACKED);
                    }
                    if !whole || letters_math::mathml(&tex, false).is_err() {
                        para.note(EQUATION);
                    }
                    self.text(&tex, &BTreeSet::from([Mark::Math]), around, para);
                }
                _ => {}
            }
        }
    }

    fn run(&mut self, node: Node, around: &Inline, para: &mut Para) {
        let direct = node.child("rPr").map(|props| RunProps::read(props, &self.theme)).unwrap_or_default();
        // A fill set on the run itself is a highlight. One that comes with a style is part of that style's look.
        let shaded = direct.shaded == Some(true);
        let named = direct.style.clone();
        let character = named.as_deref().map(|id| self.styles.character(id)).unwrap_or_default();
        // What the run asks for itself, as Word's buttons would have it, and what it comes to in its paragraph.
        let own = direct.over(&character);
        let look = own.clone().over(&para.styled);
        let linked = around.link.is_some() || self.fields.iter().any(|field| field.link.is_some());
        // The words of a comment are in no language of the document's.
        para.lang = look.lang.clone().filter(|_| !self.detached);

        let on = |value: Option<bool>| value.unwrap_or(false);
        let base = &para.baseline;
        let code = para.role != Role::Code
            && (named.as_deref().is_some_and(|id| self.styles.code(id)) || code_family(&look) && !code_family(base));
        let mut marks = BTreeSet::new();
        let wanted = [
            // Emphasis the run asks for is a mark even where the style is as heavy, so it survives a change
            // of style. What a paragraph style adds to body text is one too.
            (Mark::Bold, on(own.bold) || on(look.bold) && !on(base.bold)),
            (Mark::Italic, on(own.italic) || on(look.italic) && !on(base.italic)),
            // Word underlines a link through its style, and the page draws links its own way.
            (Mark::Underline, on(look.underline) && !linked),
            (Mark::Strike, on(look.strike)),
            (Mark::Highlight, on(look.highlight) || shaded),
            (Mark::Code, code),
        ];
        marks.extend(wanted.into_iter().filter(|(_, wanted)| *wanted).map(|(mark, _)| mark));
        // Code and links take their look from the page, so what Word's styles give them is no loss.
        let restyled = !code
            && !self.plain
            && para.role != Role::Code
            && (look.size != base.size
                || look.color != base.color && !linked
                || look.family != base.family && !look.family.as_deref().is_some_and(monospace));

        for child in node.children() {
            let before = para.len;
            match child.word() {
                Some("t") => self.text(&text_of(child), &marks, around, para),
                Some("tab" | "ptab") => self.text("\t", &marks, around, para),
                // A page break is not text: it says where the next page starts. Nor is a column break.
                Some("br") if child.attr("type") == Some("page") => {
                    *(if para.len == 0 { &mut para.breaks } else { &mut para.broken }) = true;
                }
                Some("br") if child.attr("type") == Some("column") => {}
                Some("br" | "cr") => self.text("\n", &marks, around, para),
                Some("noBreakHyphen") => self.text("\u{2011}", &marks, around, para),
                Some("softHyphen") => self.text("\u{ad}", &marks, around, para),
                Some("sym") => {
                    let code = child.attr("char").and_then(|code| u32::from_str_radix(code, 16).ok());
                    // Symbol fonts keep their glyphs in a private range, where a code says nothing by itself.
                    match code.and_then(char::from_u32).filter(|c| !('\u{e000}'..='\u{f8ff}').contains(c)) {
                        Some(symbol) => self.text(symbol.encode_utf8(&mut [0; 4]), &marks, around, para),
                        None => {
                            self.text("\u{fffd}", &marks, around, para);
                            para.note(SYMBOL);
                        }
                    }
                }
                Some("fldChar") => self.field(child, para),
                Some("instrText") => {
                    if let Some(field) = self.fields.last_mut().filter(|field| !field.shown) {
                        field.code.push_str(&text_of(child));
                    }
                }
                Some("footnoteReference") => self.cite(child, false, para),
                Some("endnoteReference") => self.cite(child, true, para),
                Some("commentReference") => {
                    // A comment with no passage of its own sits where its mark does.
                    let here = Spot { para: para.index, at: para.len };
                    let span = self.span(child);
                    *span = (span.0.or(Some(here)), span.1.or(Some(here)));
                }
                Some("drawing" | "pict" | "object") => self.drawing(child, around, para),
                // Word offers a drawing twice: as it draws one now, and as older versions did.
                _ if child.tag_name().name() == "AlternateContent" => {
                    let choice = child.children().find(|option| option.tag_name().name() == "Choice");
                    for drawing in choice.iter().flat_map(|choice| choice.children()) {
                        if drawing.is("drawing") || drawing.is("pict") {
                            self.drawing(drawing, around, para);
                        }
                    }
                }
                _ => {}
            }
            if para.len > before && child.is("t") {
                if restyled {
                    self.whole(DIRECT);
                }
                if on(look.shifted) {
                    para.note(SHIFTED);
                }
                if para.broken {
                    para.note(BREAK_MOVED);
                }
            }
        }
        para.lang = None;
    }

    /// Adds text to the paragraph, unless it is a field's instructions.
    fn text(&mut self, text: &str, marks: &BTreeSet<Mark>, around: &Inline, para: &mut Para) {
        if self.fields.iter().any(|field| !field.shown) {
            return;
        }
        let link = self.fields.iter().rev().find_map(|field| field.link.clone()).or_else(|| around.link.clone());
        para.push(text, marks, link);
    }

    fn field(&mut self, mark: Node, para: &mut Para) {
        match mark.attr("fldCharType") {
            Some("begin") if self.fields.len() < MAX_FIELDS => self.fields.push(Field::default()),
            Some("separate") => {
                if let Some(field) = self.fields.last_mut() {
                    field.shown = true;
                    field.link = hyperlink(&field.code);
                    field.contents = is_contents(&field.code);
                    para.contents |= field.contents;
                }
            }
            Some("end") => _ = self.fields.pop(),
            _ => {}
        }
    }

    /// Writes a note's mark as text, since the document has no notes at the foot of the page.
    fn cite(&mut self, reference: Node, endnote: bool, para: &mut Para) {
        let Some(id) = reference.attr("id") else { return };
        let key = (endnote, id.to_owned());
        let number = match self.numbers.get(&key) {
            Some(number) => *number,
            None => {
                self.cited.push(key.clone());
                self.numbers.insert(key, self.cited.len());
                self.cited.len()
            }
        };
        para.push(&format!("[^{number}]"), &BTreeSet::new(), None);
        para.note(FOOTNOTE);
    }

    /// Reads what a drawing holds that the document can: the text of its text boxes, and its pictures.
    fn drawing(&mut self, drawing: Node, around: &Inline, para: &mut Para) {
        // What sits inside a text box is read with the box, and an older form of the same drawing is not read.
        let own = |node: &Node| {
            let outer = node.ancestors().skip(1).take_while(|outer| *outer != drawing);
            outer.into_iter().all(|outer| !outer.is("txbxContent") && outer.tag_name().name() != "Fallback")
        };
        let mut found = false;
        for node in drawing.descendants().filter(own) {
            if node.is("txbxContent") {
                found = true;
                let mut blocks = self.blocks(node, around.rels);
                if let Some(first) = blocks.first().map(|block| block.id.clone()) {
                    self.note(&first, TEXT_BOX);
                }
                para.after.append(&mut blocks);
                continue;
            }
            let rule = node.attr("hr").is_some_and(|rule| matches!(rule, "t" | "true"));
            let source = match node.tag_name().name() {
                "blip" => node.attr("embed").or_else(|| node.attr("link")),
                "imagedata" => node.attr("id"),
                _ if rule => {
                    found = true;
                    para.ruled = true;
                    None
                }
                _ => None,
            };
            let Some(rel) = source.and_then(|id| around.rels.get(id)) else { continue };
            found = true;
            let frame = node.ancestors().take_while(|outer| *outer != drawing);
            let named = |name: &'static str| move |node: &Node| node.tag_name().name() == name;
            let size = frame.clone().find(named("pic")).and_then(|pic| extent(pic.descendants().find(named("ext"))?));
            let size = size
                .or_else(|| extent(drawing.descendants().find(named("extent"))?))
                .or_else(|| styled_size(frame.clone().find(named("shape"))?.attribute("style")?));
            let alt = drawing.descendants().find(named("docPr")).and_then(|props| props.attribute("descr"));
            let alt = alt.or_else(|| frame.clone().find(named("shape"))?.attribute("alt")).unwrap_or_default();
            match self.picture(&rel.target, rel.external, size, alt) {
                Ok(block) => {
                    para.pictures.push(block.id.clone());
                    para.after.push(block);
                }
                Err(message) => para.note(&message),
            }
        }
        if !found {
            // A shape that is wide and has next to no height is a line drawn across the page.
            let size = drawing.descendants().find(|node| node.tag_name().name() == "extent");
            let points = |name: &str| {
                let emu = size?.attribute(name)?.trim().parse::<f32>().ok()?;
                Some(emu / 12700.0)
            };
            match (points("cx"), points("cy")) {
                (Some(width), Some(height)) if width >= 72.0 && height <= 4.0 => para.ruled = true,
                _ => para.note(DRAWING),
            }
        }
    }

    /// Makes the block for a picture, keeping its bytes as an asset. Says why when the picture cannot be kept.
    fn picture(&mut self, target: &str, remote: bool, size: Option<(f32, f32)>, alt: &str) -> Result<Block, String> {
        // A picture Word did not record the size of is shown at a size that suits most.
        let (width, height) = size.unwrap_or((320.0, 240.0));
        let mut block = Block::new(self.id(), BlockKind::Unknown { type_name: "figure".into() });
        if remote {
            if !target.starts_with("https://") {
                return Err("A picture linked from outside the file was left out".into());
            }
            block.extra.insert("src".into(), target.into());
        } else {
            let name = match self.pictures.get(target) {
                Some(known) => known.clone(),
                None => {
                    let asset = self.asset(target);
                    self.pictures.insert(target.to_owned(), asset.clone());
                    asset
                }
            };
            block.extra.insert("asset".into(), name?.into());
        }
        block.extra.insert("alt".into(), alt.into());
        block.extra.insert("width".into(), number(width));
        block.extra.insert("height".into(), number(height));
        block.extra.insert("fallback".into(), if alt.is_empty() { "Picture" } else { alt }.into());
        Ok(block)
    }

    /// Keeps a picture part as an asset named after its content, and returns the name.
    fn asset(&mut self, part: &str) -> Result<String, String> {
        let bytes = self.package.bytes(part).ok().flatten().filter(|bytes| bytes.len() <= PICTURE_LIMIT);
        let Some(bytes) = bytes else { return Err("A picture could not be read and was left out".into()) };
        let extension = match bytes.as_slice() {
            [0x89, b'P', b'N', b'G', ..] => "png",
            [0xff, 0xd8, 0xff, ..] => "jpg",
            [b'G', b'I', b'F', b'8', ..] => "gif",
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
            _ => {
                let kind = part.rsplit_once('.').map_or("unknown", |(_, extension)| extension).to_ascii_uppercase();
                return Err(format!("A picture in a format Letters cannot show ({kind}) was left out"));
            }
        };
        let hash: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
        let name = format!("assets/{hash}.{extension}");
        self.assets.insert(name.clone(), bytes);
        Ok(name)
    }
}

fn number(value: f32) -> Value {
    let rounded = (f64::from(value) * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 { Value::from(rounded as i64) } else { Value::from(rounded) }
}

/// A size in points from an element that gives one in English Metric Units, as drawings do.
fn extent(node: Node) -> Option<(f32, f32)> {
    let points = |name: &str| Some(node.attribute(name)?.trim().parse::<f32>().ok()? / 12700.0);
    Some((points("cx")?, points("cy")?)).filter(|(width, height)| *width > 0.0 && *height > 0.0)
}

/// A size in points from the CSS-like style of an older drawing.
fn styled_size(style: &str) -> Option<(f32, f32)> {
    let length = |name: &str| {
        let value = style.split(';').find_map(|rule| rule.trim().strip_prefix(name)?.trim_start().strip_prefix(':'))?;
        let value = value.trim();
        let (number, per_unit) = [("pt", 1.0), ("in", 72.0), ("px", 0.75), ("cm", 28.35), ("mm", 2.835)]
            .iter()
            .find_map(|(unit, scale)| Some((value.strip_suffix(unit)?, *scale)))?;
        Some(number.trim().parse::<f32>().ok()? * per_unit).filter(|points| *points > 0.0)
    };
    Some((length("width")?, length("height")?))
}

/// Whether a field's instructions ask for a table of contents.
fn is_contents(code: &str) -> bool {
    code.trim_start().get(..3).is_some_and(|name| name.eq_ignore_ascii_case("TOC"))
}

/// The address a field leads to, when its instructions make it a link out of the document.
fn hyperlink(code: &str) -> Option<String> {
    let rest = code.trim_start();
    let rest = rest.get(..9).filter(|name| name.eq_ignore_ascii_case("HYPERLINK")).map(|_| &rest[9..])?;
    // Arguments are bare words or quoted, and a switch such as `\l` takes the one after it.
    let mut words = Vec::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            '"' => {
                let mut word = String::new();
                while let Some(c) = chars.next().filter(|c| *c != '"') {
                    // A backslash in quotes is doubled.
                    word.push(if c == '\\' { chars.next().unwrap_or(c) } else { c });
                }
                words.push((word, true));
            }
            c => {
                let mut word = String::from(c);
                while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                    word.push(c);
                }
                words.push((word, false));
            }
        }
    }
    let (mut address, mut place) = (None, None);
    let mut words = words.into_iter();
    while let Some((word, quoted)) = words.next() {
        match word.as_str() {
            "\\l" if !quoted => place = words.next().map(|(place, _)| place),
            "\\o" | "\\t" | "\\m" if !quoted => _ = words.next(),
            switch if !quoted && switch.starts_with('\\') => {}
            _ => address = address.or(Some(word)),
        }
    }
    let address = address.filter(|address| !address.is_empty())?;
    Some(place.map_or(address.clone(), |place| format!("{address}#{place}")))
}
