//! Reads a sound `document.json` in one pass, without building the whole file as a JSON value first.
//!
//! Opening a long document spends most of its time here, so runs and blocks are built straight from the
//! parser. This reader only ever accepts: whatever it is unsure of it declines, and the caller reads the file
//! again the slow way. The slow reader stays the one source of errors and of the rules for each block type.

use std::fmt;

use serde::de::{DeserializeSeed, Deserializer, Error, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

use super::{block_from, read_mark, shape};
use crate::{Block, Body, BodyShape, Document, Extra, Run};

/// The document, or `None` when the file needs the slow reader.
pub(super) fn document(json: &str) -> Option<Document> {
    let mut reader = serde_json::Deserializer::from_str(json);
    let document = Whole.deserialize(&mut reader).ok()?;
    reader.end().ok()?;
    Some(document)
}

/// Declines the file. The message is never shown.
fn decline<E: Error>() -> E {
    E::custom("declined")
}

/// Implements [`DeserializeSeed`] for a visitor that takes no input of its own.
macro_rules! seed {
    ($name:ident, $method:ident) => {
        impl<'de> DeserializeSeed<'de> for $name {
            type Value = <$name as Visitor<'de>>::Value;

            fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
                deserializer.$method(self)
            }
        }
    };
}

/// A key: its place in a list of known names, or the name itself. Known keys cost no allocation.
struct Key(&'static [&'static str]);

impl<'de> Visitor<'de> for Key {
    type Value = Result<usize, String>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a key")
    }

    fn visit_str<E: Error>(self, key: &str) -> Result<Self::Value, E> {
        Ok(self.0.iter().position(|known| *known == key).ok_or_else(|| key.to_owned()))
    }
}

impl<'de> DeserializeSeed<'de> for Key {
    type Value = Result<usize, String>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_str(self)
    }
}

/// Puts a field aside for the slow reader's rules. A repeated key is declined.
fn keep<E: Error>(fields: &mut Map<String, Value>, key: String, value: Value) -> Result<(), E> {
    fields.insert(key, value).map_or(Ok(()), |_| Err(decline()))
}

struct Whole;
seed!(Whole, deserialize_map);

impl<'de> Visitor<'de> for Whole {
    type Value = Document;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a document")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Document, A::Error> {
        let mut fields = Map::new();
        let mut blocks = None;
        while let Some(key) = map.next_key_seed(Key(&["blocks"]))? {
            match key {
                Ok(_) if blocks.is_none() => blocks = Some(map.next_value_seed(Blocks)?),
                Ok(_) => return Err(decline()),
                Err(key) => keep(&mut fields, key, map.next_value()?)?,
            }
        }
        super::document(fields, Some(blocks.unwrap_or_default())).map_err(|_| decline())
    }
}

struct Blocks;
seed!(Blocks, deserialize_seq);

impl<'de> Visitor<'de> for Blocks {
    type Value = Vec<Block>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a list of blocks")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<Block>, A::Error> {
        let mut blocks = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(block) = seq.next_element_seed(OneBlock)? {
            blocks.push(block);
        }
        Ok(blocks)
    }
}

struct OneBlock;
seed!(OneBlock, deserialize_map);

impl<'de> Visitor<'de> for OneBlock {
    type Value = Block;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a block")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Block, A::Error> {
        const BODIES: [(&str, BodyShape); 2] = [("content", BodyShape::Text), ("blocks", BodyShape::Blocks)];
        let mut fields = Map::new();
        let mut body = None;
        let mut seen = [false; 2];
        while let Some(key) = map.next_key_seed(Key(&["content", "blocks"]))? {
            let key = match key {
                Ok(index) if std::mem::replace(&mut seen[index], true) => return Err(decline()),
                Ok(index) => index,
                Err(key) => {
                    keep(&mut fields, key, map.next_value()?)?;
                    continue;
                }
            };
            // The body is read directly only once the type before it says what it holds. Letters writes
            // the type first, and a file in another order takes the slow reader's path for this field.
            let (name, holds) = BODIES[key];
            let expected = fields.get("type").and_then(Value::as_str).and_then(shape);
            if body.is_none() && expected == Some(holds) {
                body = Some(match holds {
                    BodyShape::Text => Body::Text(map.next_value_seed(Runs)?),
                    _ => Body::Blocks(map.next_value_seed(Blocks)?),
                });
            } else {
                keep(&mut fields, name.to_owned(), map.next_value()?)?;
            }
        }
        block_from(fields, body, "").map_err(|_| decline())
    }
}

struct Runs;
seed!(Runs, deserialize_seq);

impl<'de> Visitor<'de> for Runs {
    type Value = Vec<Run>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a list of runs")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<Run>, A::Error> {
        let mut runs = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(run) = seq.next_element_seed(OneRun)? {
            runs.push(run);
        }
        Ok(runs)
    }
}

struct OneRun;
seed!(OneRun, deserialize_map);

impl<'de> Visitor<'de> for OneRun {
    type Value = Run;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a run")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Run, A::Error> {
        let (mut text, mut marks, mut link) = (None, None, None);
        let mut extra = Extra::new();
        while let Some(key) = map.next_key_seed(Key(&["text", "marks", "link"]))? {
            let repeated = match key {
                Ok(0) => text.replace(map.next_value::<String>()?).is_some(),
                Ok(1) => marks.replace(map.next_value::<Vec<String>>()?).is_some(),
                Ok(_) => link.replace(map.next_value::<String>()?).is_some(),
                Err(key) => extra.insert(key, map.next_value()?).is_some(),
            };
            if repeated {
                return Err(decline());
            }
        }
        let text = text.ok_or_else(decline)?;
        let marks = marks.unwrap_or_default().iter().map(|name| read_mark(name)).collect();
        Ok(Run { text, marks, link, extra })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::object;

    /// Everything Letters writes must take the fast path, or opening is slow without anything failing.
    #[test]
    fn canonical_files_are_read_in_one_pass_and_alike() {
        let corpus = [
            include_str!("../../tests/corpus/design-sample.json"),
            include_str!("../../tests/corpus/every-block.json"),
        ];
        for json in corpus {
            let slow = crate::json::document(object(serde_json::from_str(json).unwrap(), "").unwrap(), None).unwrap();
            assert_eq!(document(json), Some(slow));
        }
    }
}
