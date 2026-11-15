//! The list of figures: what the word for a figure, typed on the page, offers to refer to.
//!
//! The list hangs under the typed words while the caret stays on the page. A number typed after the word
//! narrows it to the figures whose numbers begin so. No figure is picked until a number or an arrow says
//! which, so that Enter after a sentence that ends in "figure" still starts a new paragraph.

use letters_editor::{Editor, Numbered, Referring};
use letters_layout::figure_label;
use letters_model::BlockId;

use crate::element::Element;
use crate::style::Edges;
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Choice, Options, Popover};
use crate::window::Message;

/// How wide the list is.
const WIDTH: f32 = 320.0;
/// The most characters of a caption a figure's row shows: what fits beside its label.
const CAPTION: usize = 34;

/// What the pointer and the keyboard ask of the list.
#[derive(Clone, Debug, PartialEq)]
pub enum Picking {
    /// A figure was pressed.
    Take(BlockId),
    /// Escape or a press elsewhere, which leaves the words as they were typed.
    Close,
}

/// The figures a typed number may yet name: those whose numbers begin with it, in their order. All of them
/// when no number is typed yet.
pub(crate) fn matching(figures: Vec<Numbered>, typed: Option<u32>) -> Vec<Numbered> {
    let Some(typed) = typed.map(|typed| typed.to_string()) else { return figures };
    figures.into_iter().filter(|figure| figure.number.to_string().starts_with(&typed)).collect()
}

/// A caption cut to what a row has room for.
fn brief(caption: &str) -> String {
    let caption = caption.split_whitespace().collect::<Vec<_>>().join(" ");
    if caption.chars().count() <= CAPTION {
        return caption;
    }
    let cut: String = caption.chars().take(CAPTION - 1).collect();
    format!("{}…", cut.trim_end())
}

/// The list's own state: how far the arrows have walked it.
#[derive(Default)]
pub(crate) struct Refer {
    /// The words a walk began under and the figure it has reached, counted from the first that shows. A
    /// walk ends when the words change.
    walked: Option<(Referring, usize)>,
}

impl Refer {
    /// The words being typed and the figures the list shows for them. `None` while no reference is being
    /// typed or no figure matches.
    pub fn offers(editor: &Editor) -> Option<(Referring, Vec<Numbered>)> {
        let typed = editor.referring()?;
        let figures = matching(editor.numbered_figures(), typed.number);
        (!figures.is_empty()).then_some((typed, figures))
    }

    /// The figure that is picked among `shown`: where the arrows have walked to, else the first once a
    /// number is typed, else none.
    pub fn picked(&self, typed: &Referring, shown: usize) -> Option<usize> {
        let walked = self.walked.as_ref().filter(|(under, _)| under == typed).map(|(_, picked)| *picked);
        walked.or(typed.number.map(|_| 0)).map(|picked| picked.min(shown.saturating_sub(1)))
    }

    /// Moves the pick one figure down or up a list of `shown`, round its ends. The first arrow picks the
    /// figure at the end it sets out from.
    pub fn pick(&mut self, typed: &Referring, down: bool, shown: usize) {
        let picked = match self.picked(typed, shown) {
            Some(picked) => Options::<Message>::step(picked, down, shown),
            None if down => 0,
            None => shown.saturating_sub(1),
        };
        self.walked = Some((typed.clone(), picked));
    }

    /// The list over `figures`, to hang under the typed words.
    pub fn list(&self, editor: &Editor, typed: &Referring, figures: &[Numbered]) -> Popover<Message> {
        let style = editor.model().style();
        // A pick past the last choice shows none as picked.
        let picked = self.picked(typed, figures.len()).unwrap_or(usize::MAX);
        let mut options = Options::new("Figures").picked(picked);
        for figure in figures {
            let label = figure_label(style, figure.number);
            let choice = Choice::new(label.trim_end_matches(':'))
                .aside(brief(&figure.caption), TextStyle::control().size(tokens::TEXT_SM))
                .on_take(Message::Refer(Picking::Take(figure.id.clone())));
            options = options.choice(choice);
        }
        let list = Element::from(options).width(WIDTH).padding(Edges::all(tokens::SPACE_XS));
        Popover::new(list).flush().beside().open(true).on_close(Message::Refer(Picking::Close))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_number_narrows_the_figures_to_those_it_may_yet_name() {
        let figures = || (1..=12).map(|number| Numbered { id: BlockId::new("f"), number, caption: String::new() });
        // The number typed, and the numbers of the figures left.
        let cases: [(Option<u32>, &[u32]); 5] = [
            (None, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]),
            (Some(1), &[1, 10, 11, 12]),
            (Some(12), &[12]),
            (Some(13), &[]),
            (Some(0), &[]),
        ];
        for (typed, left) in cases {
            let numbers: Vec<u32> = matching(figures().collect(), typed).iter().map(|figure| figure.number).collect();
            assert_eq!(numbers, left, "{typed:?}");
        }
    }

    #[test]
    fn a_long_caption_is_cut_to_fit_its_row() {
        assert_eq!(brief("The roof"), "The roof");
        let cut = brief(&"slate ".repeat(20));
        assert!(cut.chars().count() <= CAPTION && cut.ends_with('…'), "{cut}");
    }
}
