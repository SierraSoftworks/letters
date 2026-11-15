//! What the tests of each format share.

use letters_model::{Block, BlockKind, Document, ListKind, Run};

/// A document as indented lines such as `h1 "Title"`, with marked runs as `<bold,italic|text>` and linked ones
/// as `<@url|text>`, so that a case can state the document it expects.
pub fn outline(doc: &Document) -> Vec<String> {
    fn runs(block: &Block) -> String {
        let run = |run: &Run| {
            let mut format: Vec<String> = run.marks.iter().map(|mark| format!("{mark:?}").to_lowercase()).collect();
            format.extend(run.link.iter().map(|url| format!("@{url}")));
            if format.is_empty() { run.text.clone() } else { format!("<{}|{}>", format.join(","), run.text) }
        };
        format!("{:?}", block.body.runs().iter().map(run).collect::<String>())
    }
    fn push(blocks: &[Block], indent: usize, out: &mut Vec<String>) {
        for block in blocks {
            let line = match &block.kind {
                BlockKind::Paragraph => format!("p {}", runs(block)),
                BlockKind::Heading { level } => format!("h{level} {}", runs(block)),
                BlockKind::ListItem { list: ListKind::Bullet, depth } => format!("bullet/{depth} {}", runs(block)),
                BlockKind::ListItem { list: ListKind::Ordered, depth } => format!("ordered/{depth} {}", runs(block)),
                BlockKind::ListItem { list: ListKind::Task { checked: true }, depth } => {
                    format!("done/{depth} {}", runs(block))
                }
                BlockKind::ListItem { list: ListKind::Task { checked: false }, depth } => {
                    format!("todo/{depth} {}", runs(block))
                }
                BlockKind::Code { language } => format!("code:{} {}", language.as_deref().unwrap_or("-"), runs(block)),
                BlockKind::Formula => format!("formula {}", runs(block)),
                BlockKind::Quote => "quote".to_owned(),
                BlockKind::Callout { tone } => format!("callout:{tone:?}").to_lowercase(),
                BlockKind::Divider => "divider".to_owned(),
                BlockKind::Caption => format!("caption {}", runs(block)),
                BlockKind::Figure { .. } => format!("figure {}", runs(block)),
                BlockKind::Gallery { .. } => "gallery".to_owned(),
                BlockKind::Table { .. } => "table".to_owned(),
                BlockKind::TableRow => "row".to_owned(),
                BlockKind::TableCell => format!("cell {}", runs(block)),
                BlockKind::Unknown { type_name } => format!("unknown:{type_name}"),
            };
            out.push(format!("{:indent$}{line}", ""));
            push(block.body.blocks(), indent + 2, out);
        }
    }
    let mut out = Vec::new();
    push(&doc.blocks, 0, &mut out);
    out
}
