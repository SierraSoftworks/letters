//! Word lists: what says where a line may end in the scripts that are written without spaces.
//!
//! Parley breaks such a line between words only where it has the script's list. A build with the
//! `dictionaries` feature has them all. Any other is handed one when a document turns out to need it, which
//! is how a browser comes by it: as a file it fetches.

use std::marker::PhantomData;

use icu_provider::prelude::*;
use icu_segmenter::provider::{
    Baked, SegmenterDictionaryAutoV1, SegmenterDictionaryExtendedV1, UCharDictionaryBreakData,
};
use zerovec::ZeroVec;

/// A script written without spaces between its words, named for the list of its words. A line of it ends
/// between words where layout has the list, and otherwise at a space or wherever the line fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WordList {
    Thai,
    Lao,
    Khmer,
    Burmese,
}

impl WordList {
    pub const ALL: [Self; 4] = [Self::Thai, Self::Lao, Self::Khmer, Self::Burmese];

    /// Whether every list is built in, so that none is ever asked for or taken.
    pub const BUILT_IN: bool = cfg!(feature = "dictionaries");

    /// The list as a file: what a build writes out for [`crate::DocLayout::add_word_list`] to take back
    /// where the lists are not built in. They are between 220 and 780 KiB, and a binary that calls this
    /// holds the list it asks for.
    pub fn data(self) -> Vec<u8> {
        let locale = Default::default();
        let id = DataIdentifierBorrowed::for_marker_attributes_and_locale(self.name(), &locale);
        let found: Result<DataResponse<SegmenterDictionaryExtendedV1>, _> =
            Baked.load(DataRequest { id, ..Default::default() });
        found.map(|list| list.payload.get().trie_data.as_bytes().to_vec()).unwrap_or_default()
    }

    /// What to call the list's file where lists are fetched. The name changes when the list does, so a
    /// copy kept from another build is never the one asked for.
    pub fn file(self) -> String {
        format!("{self:?}-{:016x}.bin", self.seal().1).to_lowercase()
    }

    /// The list that says where the words of a character's script end, for the scripts that have one.
    pub(crate) fn of(ch: char) -> Option<Self> {
        match u32::from(ch) {
            0x0E01..=0x0E4E => Some(Self::Thai),
            0x0E81..=0x0EDF => Some(Self::Lao),
            0x1780..=0x17DD => Some(Self::Khmer),
            0x1000..=0x109F | 0xA9E0..=0xA9FE | 0xAA60..=0xAA7F => Some(Self::Burmese),
            _ => None,
        }
    }

    /// The name ICU asks for the list by.
    fn name(self) -> &'static DataMarkerAttributes {
        DataMarkerAttributes::from_str_or_panic(match self {
            Self::Thai => "thaidict",
            Self::Lao => "laodict",
            Self::Khmer => "khmerdict",
            Self::Burmese => "burmesedict",
        })
    }

    /// The length and the mark of the list as this build reads it. Only those very bytes are taken: a file
    /// cut short, or one kept from a build that read its lists another way, would be read as words it does
    /// not hold. These change when ICU's lists do, and a test says so.
    fn seal(self) -> (usize, u64) {
        match self {
            Self::Thai => (224_978, 0x5938_1467_2604_4e19),
            Self::Lao => (292_460, 0x2efd_cf35_03b9_5e09),
            Self::Khmer => (798_374, 0x2d87_2780_5e43_a91f),
            Self::Burmese => (457_264, 0xf5b8_e443_6027_4c17),
        }
    }

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A mark that tells one file from another: FNV-1a over its bytes.
fn mark(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |mark, byte| (mark ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

/// The lists a layout was handed, which Parley asks for at every text it shapes. Each shares its bytes
/// with whatever is handed it, so handing one over costs nothing.
#[derive(Default)]
pub(crate) struct Lists {
    held: [Option<DataPayload<SegmenterDictionaryExtendedV1>>; 4],
    /// The scripts met in text without their list, and those of them that were told of.
    met: u8,
    told: u8,
}

impl Lists {
    /// Whether any list was handed over, which is whether Parley is to ask here at all.
    pub fn any(&self) -> bool {
        self.held.iter().any(Option::is_some)
    }

    /// Takes a list. `false` for one that is there already, or built in, and for bytes that are not it.
    pub fn add(&mut self, list: WordList, data: &[u8]) -> bool {
        let slot = &mut self.held[list as usize];
        if WordList::BUILT_IN || slot.is_some() || (data.len(), mark(data)) != list.seal() {
            return false;
        }
        let words = DataPayload::from_owned_buffer(data.into());
        let words = words.try_map_project(|bytes, _: PhantomData<&()>| {
            ZeroVec::parse_bytes(bytes).map(|trie_data| UCharDictionaryBreakData { trie_data })
        });
        *slot = words.ok();
        self.met &= !list.bit();
        slot.is_some()
    }

    /// Whether text of the list's script was set without it.
    pub fn missed(&self, list: WordList) -> bool {
        self.met & list.bit() != 0
    }

    /// Notes a character of text being set. Returns whether its script is short of its list.
    pub fn note(&mut self, ch: char) -> bool {
        let Some(list) = WordList::of(ch).filter(|_| !WordList::BUILT_IN) else { return false };
        let short = self.held[list as usize].is_none();
        self.met |= if short { list.bit() } else { 0 };
        short
    }

    /// The scripts met without their list since this was last asked, each once ever.
    pub fn take(&mut self) -> Vec<WordList> {
        let new = self.met & !self.told;
        self.told |= new;
        WordList::ALL.into_iter().filter(|list| new & list.bit() != 0).collect()
    }
}

impl DataProvider<SegmenterDictionaryExtendedV1> for Lists {
    fn load(&self, req: DataRequest) -> Result<DataResponse<SegmenterDictionaryExtendedV1>, DataError> {
        let held = WordList::ALL.into_iter().find(|list| list.name() == req.id.marker_attributes);
        match held.and_then(|list| self.held[list as usize].clone()) {
            Some(payload) => Ok(DataResponse { metadata: Default::default(), payload }),
            None => Err(DataErrorKind::IdentifierNotFound.with_req(SegmenterDictionaryExtendedV1::INFO, req)),
        }
    }
}

/// Chinese and Japanese lines break between characters, which takes no list.
impl DataProvider<SegmenterDictionaryAutoV1> for Lists {
    fn load(&self, req: DataRequest) -> Result<DataResponse<SegmenterDictionaryAutoV1>, DataError> {
        Err(DataErrorKind::IdentifierNotFound.with_req(SegmenterDictionaryAutoV1::INFO, req))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_list_is_sealed_as_icu_has_it() {
        for list in WordList::ALL {
            let data = list.data();
            assert_eq!((data.len(), mark(&data)), list.seal(), "{list:?}: `seal` is to follow ICU's list");
        }
    }
}
