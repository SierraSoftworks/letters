//! What the tests of each format share.

use letters_model::{Block, BlockKind, Document, ListKind, Mark, Run};

/// A document as indented lines such as `h1 "Title"`, with marked runs as `<bold,italic|text>` and linked ones
/// as `<@url|text>`, so that a case can state the document it expects. A table names how its columns align, and
/// a figure its picture and `[alt text]`, with a `#` where it is numbered.
pub fn outline(doc: &Document) -> Vec<String> {
    fn runs(block: &Block) -> String {
        let run = |run: &Run| {
            let mark = |mark: &Mark| match mark {
                Mark::Ref(target) => format!("ref:{target}"),
                mark => format!("{mark:?}").to_lowercase(),
            };
            let mut format: Vec<String> = run.marks.iter().map(mark).collect();
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
                BlockKind::Callout { tone, .. } => format!("callout:{tone:?}").to_lowercase(),
                BlockKind::Divider => "divider".to_owned(),
                BlockKind::Caption => format!("caption {}", runs(block)),
                BlockKind::Figure { asset, src, alt, numbered, .. } => {
                    let picture = asset.as_deref().or(src.as_deref()).unwrap_or("-");
                    format!("figure{} {picture} [{alt}] {}", if *numbered { "#" } else { "" }, runs(block))
                }
                BlockKind::Gallery { numbered } => format!("gallery{}", if *numbered { "#" } else { "" }),
                BlockKind::Table { columns, header } => {
                    let aligns: Vec<String> = columns.iter().map(|align| format!("{align:?}").to_lowercase()).collect();
                    format!("table:{}{}", aligns.join(","), if *header { " headed" } else { "" })
                }
                BlockKind::TableRow => "row".to_owned(),
                BlockKind::TableCell => format!("cell {}", runs(block)),
                BlockKind::TableOfContents { levels } => format!("contents:{levels}"),
                BlockKind::LinkPreview { url, title, .. } => {
                    format!("preview {url} [{}]", title.as_deref().unwrap_or("-"))
                }
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
