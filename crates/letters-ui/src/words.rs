//! Word lists: what the shell hands the window for the scripts that are written without spaces.
//!
//! A layout breaks a line of such a script between words only once it has the script's list, and the window
//! holds several layouts, some made long after a list came. So the lists are kept, and every layout is
//! handed each of them.

use std::rc::Rc;

use letters_layout::{DocLayout, WordList};

/// The word lists the window was handed, for the layouts made after they came. A copy shares their bytes.
#[derive(Clone, Default)]
pub(crate) struct WordLists(Vec<(WordList, Rc<[u8]>)>);

impl WordLists {
    /// Keeps a file handed as a script's list. `false` where the lists are built in, and for one that is
    /// kept already. Only a layout knows whether a file is the list, so another file for the same script
    /// is kept beside the first: a layout made later is handed both in turn, as the others were.
    pub fn keep(&mut self, list: WordList, data: &Rc<[u8]>) -> bool {
        let new = !WordList::BUILT_IN && self.iter().all(|kept| kept != (list, data));
        if new {
            self.0.push((list, data.clone()));
        }
        new
    }

    /// Hands the lists to a layout that was just made.
    pub fn hand(&self, layout: &mut DocLayout) {
        for (list, data) in self.iter() {
            layout.add_word_list(list, data);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (WordList, &Rc<[u8]>)> {
        self.0.iter().map(|(list, data)| (*list, data))
    }
}
