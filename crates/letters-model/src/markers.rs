//! The Markdown markers revealed around the caret. The editor decides them and layout draws them.

use crate::BlockId;

/// The Markdown markers to show for the block and the spans under the caret.
///
/// They are a view of the formatting, not text in the document: offsets count the text without them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Markers {
    /// A block's own marker and the block it belongs to: `##` on a heading, `[!INFO]` on a callout.
    pub block: Option<(BlockId, String)>,
    /// The markers of the spans the caret is inside or touching, in drawing order. All are in the caret's
    /// block.
    pub inline: Vec<Marker>,
    /// How many of `inline` are drawn before the caret.
    pub before_caret: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    /// Where in the caret's block the marker is drawn, in bytes.
    pub offset: usize,
    pub text: &'static str,
}
