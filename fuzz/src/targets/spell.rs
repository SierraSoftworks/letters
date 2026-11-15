//! Spelling on any text: what is marked as misspelled lies on whole characters of the text it was found in.

use std::collections::BTreeSet;

use letters_convert::from_markdown;
use letters_spell::{Proofing, Speller};

use crate::checks::ids;
use crate::input::text;

pub fn run(data: &[u8]) {
    let text = text(data);
    let mut speller = Speller::new();
    speller.set_user_words(text.split_whitespace().take(2));
    let words = BTreeSet::new();
    for language in ["en", "en-GB", "en-AU", "fr"] {
        let Some(checker) = speller.checker(language, &words) else { continue };
        let wrong = checker.misspellings(&text);
        for range in &wrong {
            assert!(text.get(range.clone()).is_some(), "{range:?} is not a stretch of the text");
        }
        // Corrections are found one word at a time, when a menu opens on it. They take a hundred times as
        // long as everything else here, so one run in eight asks for them, for one short word.
        let asked = language == "en-GB" && data.first().is_some_and(|byte| byte % 8 == 0);
        let word = wrong.first().and_then(|range| text.get(range.clone())).filter(|word| word.len() < 12);
        if let Some(word) = word.filter(|_| asked) {
            checker.suggest(word);
        }
    }

    // A document is checked a block at a time, in each block's own language.
    let (doc, _) = from_markdown(&text, &mut ids());
    let mut proofing = Proofing::new();
    while let Some(checked) = proofing.step(&speller, &doc) {
        let block = doc.block(&checked.block).expect("a block that was checked is in the document");
        let text = block.plain_text();
        for range in proofing.marks(&checked.block) {
            assert!(text.get(range.clone()).is_some(), "{range:?} is not a stretch of its block");
        }
    }
}

pub fn seeds() -> Vec<Vec<u8>> {
    let prose = concat!(
        "The colour of the neighbour's centre isn't organized: it’s a well-known co-operative, e.g. in the U.K.\n\n",
        "# Mispeled wrods and naïve café 東京 🙂 x² 3rd 1,000.5 don't 'quoted' rock'n'roll https://example.com\n\n",
        "- `code is skipped` and $x^2$ too\n- Zoë O'Brien-Smith's über-straße\n",
    );
    // The second starts with a byte that asks for corrections.
    vec![prose.into(), "Helo wrold, a mispeled centre".into()]
}
