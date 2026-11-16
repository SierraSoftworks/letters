//! Dictionaries by the names of their files, for whoever picks among those a system or a server holds.
//!
//! A Hunspell dictionary is an `.aff` and a `.dic` file of one name, such as `de_DE` or `pt_BR`. Every shell
//! and the server choose by the one rule here, so that a language is checked alike wherever it is written.

use crate::speller::Tag;

/// A name as it chooses: its tag, and whether it says more than a language and a region, as `de_DE_frami`
/// and `sr-Latn-RS` do.
fn read(name: &str) -> Option<(Tag, bool)> {
    let tag = Tag::parse(name)?;
    let after = name.trim().split(['-', '_']).count() - 1;
    let more = after > usize::from(tag.region.is_some());
    Some((tag, more))
}

/// The language the dictionary named `name` is for, as the tag `de` or `de-CH`: `de_DE_frami` is `de-DE`.
/// A BCP 47 tag is read alike, so a document's language and a dictionary's name can be compared. `None`
/// for a name that says no language.
pub fn dictionary_language(name: &str) -> Option<String> {
    let (tag, _) = read(name)?;
    Some(match tag.region {
        Some(region) => format!("{}-{region}", tag.language),
        None => tag.language,
    })
}

/// The place in `names` of the dictionary that best serves `language`, a tag such as `de` or `de-CH`: one
/// for its region, else one for the language alone, else that of the country the language is named after,
/// else any. A plain name is preferred to a variant such as `de_DE_frami`, and of two that serve alike the
/// earlier in `names`. `None` when none is for the language.
pub fn best_dictionary<'a>(names: impl IntoIterator<Item = &'a str>, language: &str) -> Option<usize> {
    let wanted = Tag::parse(language)?;
    let rank = |tag: &Tag| match &tag.region {
        region if *region == wanted.region => 0,
        None => 1,
        Some(region) if region.eq_ignore_ascii_case(&wanted.language) => 2,
        Some(_) => 3,
    };
    let serves = names.into_iter().enumerate().filter_map(|(place, name)| Some((place, read(name)?)));
    let serves = serves.filter(|(_, (tag, _))| tag.language == wanted.language);
    serves.min_by_key(|(place, (tag, more))| (rank(tag), *more, *place)).map(|(place, _)| place)
}
