use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use serde_json::Value;

/// The `document.json` format version this build reads and writes.
pub const FORMAT: u32 = 1;

/// Fields this build does not know, kept so that a newer client's content survives a save.
pub type Extra = BTreeMap<String, Value>;

/// A block's stable identity, unique within its document.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(Arc<str>);

impl BlockId {
    pub fn new(id: impl AsRef<str>) -> Self {
        Self(id.as_ref().into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for BlockId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

/// The tree stored in `document.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    /// A BCP 47 tag such as `en-GB`.
    pub language: String,
    /// Words every copy of this document accepts as spelled correctly.
    pub dictionary: BTreeSet<String>,
    /// The front matter of the file the document came from, if it had any.
    pub properties: Option<Properties>,
    pub blocks: Vec<Block>,
    pub extra: Extra,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            language: "en".into(),
            dictionary: BTreeSet::new(),
            properties: None,
            blocks: Vec::new(),
            extra: Extra::new(),
        }
    }
}

/// A document's properties as their source, so that they go back into a file exactly as they came out
/// of one. Nothing in the model reads inside them.
#[derive(Clone, Debug, PartialEq)]
pub struct Properties {
    pub format: PropertiesFormat,
    /// The text between the fences, with `\n` line endings.
    pub source: String,
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertiesFormat {
    Yaml,
    Toml,
}

impl Document {
    /// Finds a block anywhere in the tree.
    pub fn block(&self, id: &BlockId) -> Option<&Block> {
        fn find<'a>(blocks: &'a [Block], id: &BlockId) -> Option<&'a Block> {
            blocks.iter().find_map(|b| if &b.id == id { Some(b) } else { find(b.body.blocks(), id) })
        }
        find(&self.blocks, id)
    }

    /// Every block in document order, parents before their children.
    pub fn walk(&self) -> impl Iterator<Item = &Block> {
        fn push<'a>(blocks: &'a [Block], out: &mut Vec<&'a Block>) {
            for b in blocks {
                out.push(b);
                push(b.body.blocks(), out);
            }
        }
        let mut out = Vec::new();
        push(&self.blocks, &mut out);
        out.into_iter()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub id: BlockId,
    pub kind: BlockKind,
    pub body: Body,
    /// A BCP 47 tag, for a block in another language than the document's. Its children take it too.
    pub lang: Option<String>,
    /// Whether the block starts a new page.
    pub page_break: bool,
    pub extra: Extra,
}

impl Block {
    /// A block of `kind` with the empty body that kind takes.
    pub fn new(id: impl Into<BlockId>, kind: BlockKind) -> Self {
        let body = match kind.shape() {
            BodyShape::Text => Body::Text(Vec::new()),
            BodyShape::Blocks => Body::Blocks(Vec::new()),
            BodyShape::None => Body::None,
        };
        Self { id: id.into(), kind, body, lang: None, page_break: false, extra: Extra::new() }
    }

    /// A text block holding one unmarked run.
    pub fn text(id: impl Into<BlockId>, kind: BlockKind, text: impl Into<String>) -> Self {
        let mut block = Self::new(id, kind);
        debug_assert_eq!(block.kind.shape(), BodyShape::Text);
        block.body = Body::Text(vec![Run::plain(text)]);
        block
    }

    /// The block's text with marks dropped. Empty for a block that holds no text.
    pub fn plain_text(&self) -> String {
        self.body.runs().iter().map(|r| r.text.as_str()).collect()
    }
}

/// A block's type and the fields that belong to that type.
#[derive(Clone, Debug, PartialEq)]
pub enum BlockKind {
    Paragraph,
    /// `level` runs from 1 to 4.
    Heading {
        level: u8,
    },
    /// One item of a list. Consecutive items form the list, and `depth` nests them.
    ListItem {
        list: ListKind,
        depth: u8,
    },
    /// `language` is kept as written, so an unknown one survives.
    Code {
        language: Option<String>,
    },
    Quote,
    Callout {
        tone: Tone,
    },
    Divider,
    /// A type this build does not know. Its fields sit in [`Block::extra`] and are written back verbatim.
    Unknown {
        type_name: String,
    },
}

impl BlockKind {
    pub fn shape(&self) -> BodyShape {
        match self {
            Self::Paragraph | Self::Heading { .. } | Self::ListItem { .. } | Self::Code { .. } => BodyShape::Text,
            Self::Quote | Self::Callout { .. } => BodyShape::Blocks,
            Self::Divider | Self::Unknown { .. } => BodyShape::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Bullet,
    Ordered,
    Task { checked: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Tip,
    Info,
    Warn,
    Danger,
}

/// What a block holds. The block's kind fixes which variant it takes.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Text(Vec<Run>),
    Blocks(Vec<Block>),
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyShape {
    Text,
    Blocks,
    None,
}

impl Body {
    pub fn runs(&self) -> &[Run] {
        if let Self::Text(runs) = self { runs } else { &[] }
    }

    pub fn blocks(&self) -> &[Block] {
        if let Self::Blocks(blocks) = self { blocks } else { &[] }
    }
}

/// A stretch of text with one set of marks.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Run {
    pub text: String,
    pub marks: BTreeSet<Mark>,
    pub link: Option<String>,
    /// A BCP 47 tag, for text in another language than its block's.
    pub lang: Option<String>,
    pub extra: Extra,
}

impl Run {
    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), ..Self::default() }
    }

    pub fn marked(text: impl Into<String>, marks: impl IntoIterator<Item = Mark>) -> Self {
        Self { text: text.into(), marks: marks.into_iter().collect(), ..Self::default() }
    }

    /// Whether two runs differ only in their text, so they can be stored as one.
    pub(crate) fn same_format(&self, other: &Self) -> bool {
        self.marks == other.marks && self.link == other.link && self.lang == other.lang && self.extra == other.extra
    }
}

/// The direct formatting a run can carry. Everything else comes from the style of its block.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mark {
    Bold,
    Italic,
    Underline,
    Strike,
    Highlight,
    Code,
    /// A mark this build does not know, kept as written.
    Other(String),
}
