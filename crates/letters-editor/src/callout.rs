//! Changing a callout after it is made: its tone, and the title, icon and colour chosen over the tone's.

use letters_model::{Block, BlockId, BlockKind, Op, Tone};

use crate::Editor;

/// A change to the callout the caret is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalloutEdit {
    /// Makes it a callout of another tone. What was chosen for it stays.
    Tone(Tone),
    /// The line that heads it, on one line. `None` is the tone's own, and an empty one shows no title.
    Title(Option<String>),
    /// Its icon, by a name of [`Tone::ICONS`] or `none`. `None` is the tone's own.
    Icon(Option<String>),
    /// Its colour as `#rrggbb`, as it shows on a light page. `None` is the tone's own.
    Color(Option<String>),
    /// Goes back to the tone's own title, icon and colour.
    Reset,
    /// Takes the callout away and leaves what it held.
    Remove,
}

/// `kind` as a callout of another tone, with what was chosen for it kept.
pub(crate) fn toned(kind: &BlockKind, to: Tone) -> BlockKind {
    let BlockKind::Callout { title, icon, color, .. } = kind else { return BlockKind::callout(to) };
    tidy(to, title.clone(), icon.clone(), color.clone())
}

/// A callout that stores only what differs from its tone, so that the rest follows a change of tone.
fn tidy(tone: Tone, title: Option<String>, icon: Option<String>, color: Option<String>) -> BlockKind {
    // A title is typed a key at a time, so the spaces at its ends are left as typed.
    let line = |text: String| text.split(['\n', '\r']).collect::<Vec<_>>().join(" ");
    let title = title.map(line).filter(|title| title != tone.title());
    let icon = icon.map(|icon| line(icon).trim().to_owned()).filter(|icon| !icon.is_empty() && icon != tone.icon());
    let hex = |color: &String| {
        color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|b| b.is_ascii_hexdigit())
    };
    let color = color.filter(hex).map(|color| color.to_ascii_lowercase());
    BlockKind::Callout { tone, title, icon, color }
}

impl Editor {
    /// The callout the caret is in: the innermost one around the selection's head.
    pub fn callout(&self) -> Option<&Block> {
        let mut id = self.selection()?.head.block.clone();
        loop {
            let parent = self.seat(&id)?.parent?;
            if matches!(parent.kind, BlockKind::Callout { .. }) {
                return Some(parent);
            }
            id = parent.id.clone();
        }
    }

    pub(crate) fn callout_edit(&mut self, edit: CalloutEdit) {
        let Some(block) = self.callout() else { return };
        let id: BlockId = block.id.clone();
        let BlockKind::Callout { tone, title, icon, color } = block.kind.clone() else { return };
        let kind = match edit {
            CalloutEdit::Tone(to) => tidy(to, title, icon, color),
            CalloutEdit::Title(title) => tidy(tone, title, icon, color),
            CalloutEdit::Icon(icon) => tidy(tone, title, icon, color),
            // A colour that cannot be read changes nothing.
            CalloutEdit::Color(to) => match tidy(tone, title.clone(), icon.clone(), to.clone()) {
                BlockKind::Callout { color: None, .. } if to.is_some() => return,
                kind => kind,
            },
            CalloutEdit::Reset => BlockKind::callout(tone),
            CalloutEdit::Remove => return self.unwrap(&id),
        };
        self.apply(vec![Op::SetKind { block: id, kind }]);
    }
}
