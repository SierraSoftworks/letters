//! What a web page says of itself, read for the card of a link to it.

use html5gum::{DefaultEmitter, Token, Tokenizer};

/// The longest title and description kept, in characters. A card shows a line or two of each, and a page
/// may not make the document that links to it large.
const LONGEST: (usize, usize) = (200, 300);

/// The names a page gives its title, its description and its picture under, the one to prefer first: what
/// it says for a card such as this one, as Open Graph and then as Twitter have it, then what it says for all.
const TITLES: [&str; 2] = ["og:title", "twitter:title"];
const DESCRIPTIONS: [&str; 3] = ["og:description", "twitter:description", "description"];
const PICTURES: [&str; 5] = ["og:image:secure_url", "og:image", "og:image:url", "twitter:image", "twitter:image:src"];

/// The longest address of a picture kept, in bytes. A longer one is no address a server would take.
const LONGEST_ADDRESS: usize = 2048;

/// What a page says of itself. Each part is `None` where the page says nothing of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageSummary {
    /// The page's title, on one line.
    pub title: Option<String>,
    /// What the page holds, in a sentence or two on one line.
    pub description: Option<String>,
    /// The address of the picture the page shows itself by, as the page wrote it: it may be relative to the
    /// page's own, and it is no more checked than any other address a stranger wrote.
    pub image: Option<String>,
}

/// Reads what a fetched page says of itself in its head, for a link preview to show: its title, its
/// description and its picture, each as the page gives it for a card, or else as it gives it for all.
///
/// The page is a stranger's, so nothing in it is trusted. Its text comes back as plain text on one line,
/// cut to a length a card can show. Text that is not UTF-8 is left out, since the page's own encoding is
/// not known here. The picture's address is as written, for the caller to read against the page's own and
/// hold to the rules it holds any address to.
pub fn page_summary(html: &[u8]) -> PageSummary {
    let mut emitter = DefaultEmitter::default();
    // So that the text of a script or a style sheet is not read as tags, nor a tag in the title as one.
    emitter.naively_switch_states(true);
    let (mut titles, mut descriptions, mut pictures) = ([const { None }; 2], [const { None }; 3], [const { None }; 5]);
    // The text of the page's first `title` element, and whether it is being read.
    let (mut written, mut titling) = (None, false);
    for token in Tokenizer::new_with_emitter(html, emitter).flatten() {
        match token {
            Token::StartTag(tag) => match tag.name.as_slice() {
                b"title" if written.is_none() => (written, titling) = (Some(Vec::new()), true),
                b"meta" => {
                    let attribute = |name: &[u8]| tag.attributes.iter().find(|(key, _)| key.as_slice() == name);
                    let name = attribute(b"property").or_else(|| attribute(b"name"));
                    let name = name.and_then(|(_, name)| std::str::from_utf8(name).ok()).map_or("", str::trim);
                    let Some((_, content)) = attribute(b"content") else { continue };
                    keep(&TITLES, &mut titles, name, content);
                    keep(&DESCRIPTIONS, &mut descriptions, name, content);
                    keep(&PICTURES, &mut pictures, name, content);
                }
                // What a page says of itself, it says in its head.
                b"body" => break,
                _ => {}
            },
            Token::EndTag(tag) => match tag.name.as_slice() {
                b"title" => titling = false,
                b"head" => break,
                _ => {}
            },
            Token::String(text) if titling => written.iter_mut().for_each(|title| title.extend_from_slice(&text)),
            _ => {}
        }
    }
    let read = |texts: &[Option<Vec<u8>>], limit: usize| {
        texts.iter().flatten().find_map(|text| line(std::str::from_utf8(text).ok()?, limit))
    };
    let address = |written: &Vec<u8>| {
        let address = std::str::from_utf8(written).ok()?.trim();
        let whole = !address.is_empty() && address.len() <= LONGEST_ADDRESS && !address.contains(char::is_control);
        whole.then(|| address.to_owned())
    };
    PageSummary {
        title: read(&titles, LONGEST.0).or_else(|| read(&[written], LONGEST.0)),
        description: read(&descriptions, LONGEST.1),
        image: pictures.iter().flatten().find_map(address),
    }
}

/// Keeps what a page says under `name` in the slot of that name. The first of a name stands, as it does
/// for whatever else reads these.
fn keep(names: &[&str], slots: &mut [Option<Vec<u8>>], name: &str, content: &[u8]) {
    let at = names.iter().position(|known| known.eq_ignore_ascii_case(name));
    if let Some(slot) = at.map(|at| &mut slots[at]).filter(|slot| slot.is_none()) {
        *slot = Some(content.to_vec());
    }
}

/// Text as one line of at most `limit` characters. Longer text is cut at the end of a word and ends with
/// an ellipsis. `None` for text that shows nothing.
fn line(text: &str, limit: usize) -> Option<String> {
    let words = text.split(|c: char| c.is_whitespace() || c.is_control()).filter(|word| !word.is_empty());
    let line = words.collect::<Vec<_>>().join(" ");
    if line.chars().nth(limit).is_none() {
        return Some(line).filter(|line| !line.is_empty());
    }
    // The ellipsis takes the last place.
    let (cut, _) = line.char_indices().nth(limit.saturating_sub(1))?;
    let kept = &line[..cut];
    // A word that the cut falls in goes whole, unless it is the only one.
    let kept = if line[cut..].starts_with(' ') { kept } else { kept.rfind(' ').map_or(kept, |space| &kept[..space]) };
    Some(format!("{kept}\u{2026}"))
}
