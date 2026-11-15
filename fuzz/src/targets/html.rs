//! HTML from a clipboard or a page: it reads into a sound document, and what Letters writes of that
//! document reads back.

use letters_convert::{from_html, to_html, to_html_fragment};
use letters_model::{Block, BlockKind, Style};

use crate::checks::{ids, noted, sound};
use crate::input::text;

/// Each run of blocks that share a parent, outermost first.
fn siblings(blocks: &[Block]) -> Vec<&[Block]> {
    let inner = blocks.iter().flat_map(|block| siblings(block.body.blocks()));
    std::iter::once(blocks).chain(inner).collect()
}

pub fn run(data: &[u8]) {
    // A phrase alone keeps the spaces at its edges, so a block goes before it.
    let html = format!("<p>first</p>{}", text(data));
    let (doc, notes) = from_html(&html, &mut ids());
    sound(&doc, "imported HTML");
    noted(&doc, &notes, "imported HTML");
    for blocks in siblings(&doc.blocks) {
        let mut deepest = 0;
        for block in blocks {
            let text = block.plain_text();
            deepest = match &block.kind {
                BlockKind::ListItem { depth, .. } => {
                    assert!(*depth <= deepest, "an item sits more than one level below the one before it");
                    depth + 1
                }
                _ => 0,
            };
            let parts = block.body.blocks();
            match &block.kind {
                BlockKind::Paragraph | BlockKind::Heading { .. } => {
                    assert!(!text.trim().is_empty(), "a block of text has no text");
                    assert_eq!(text.trim_matches([' ', '\n']), text, "a block has space at its edges");
                }
                BlockKind::Quote | BlockKind::Callout { .. } => assert!(!parts.is_empty(), "a quote holds nothing"),
                BlockKind::Figure { asset, src, width, height, .. } => {
                    assert!(asset.is_some() != src.is_some(), "a figure has no one picture");
                    assert!(*width > 0.0 && *height > 0.0, "a figure has no size");
                }
                BlockKind::Gallery { .. } => {
                    let captions = parts.iter().filter(|part| part.kind == BlockKind::Caption).count();
                    assert!(captions <= 1 && parts.len() - captions > 1, "a gallery is not pictures and a caption");
                }
                _ => {}
            }
            assert!(block.body.runs().iter().all(|run| !run.text.is_empty()), "a run is empty");
        }
    }

    // What a copy puts on the clipboard, and what an export writes, are read by this and other readers.
    let (again, _) = from_html(&to_html_fragment(&doc.blocks), &mut ids());
    sound(&again, "HTML read back");
    to_html(&doc, &Style::default(), "Fuzz", &mut |_| None);
}

pub fn seeds() -> Vec<Vec<u8>> {
    let pasted = concat!(
        "<h1>Title</h1><p>Some <b>bold</b>, <i>italic</i> and <a href=\"https://example.com\">linked</a> text.</p>",
        "<ul><li>one<ol><li>two</li></ol></li><li><input type=\"checkbox\" checked> done</li></ul>",
        "<blockquote><p>quoted</p></blockquote><pre><code class=\"language-rust\">fn main() {}</code></pre><hr>",
        "<table><tr><th>a</th><th style=\"text-align:right\">b</th></tr><tr><td colspan=\"2\">c</td></tr></table>",
        "<figure><img src=\"https://example.com/a.png\" alt=\"A\" width=\"40\" height=\"30\">",
        "<img src=\"https://example.com/b.png\"><figcaption>Two</figcaption></figure>",
        "<p><span style=\"font-weight:700\">x</span><math><annotation encoding=\"application/x-tex\">x^2</annotation>",
        "</math>&nbsp;&amp;<br>y</p><script>no</script><!-- c -->",
    );
    let (doc, _) = letters_convert::from_markdown(
        include_str!("../../../crates/letters-convert/tests/corpus/notice.md"),
        &mut ids(),
    );
    vec![pasted.into(), to_html_fragment(&doc.blocks).into_bytes()]
}
