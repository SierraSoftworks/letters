//! One input, read whole or as the choices of a script.

use std::borrow::Cow;

/// The most of an input that a target reads. A longer one is cut, which keeps every run short.
pub const MAX_INPUT: usize = 64 * 1024;

pub fn bounded(data: &[u8]) -> &[u8] {
    &data[..data.len().min(MAX_INPUT)]
}

/// The input as text. Bytes that are not UTF-8 become the replacement character, so that a mutation which
/// breaks one character does not waste the rest.
pub fn text(data: &[u8]) -> Cow<'_, str> {
    String::from_utf8_lossy(bounded(data))
}

/// An input read a choice at a time. Once it runs out every choice is the first, so a script ends quietly
/// wherever its input does.
pub struct Script<'a> {
    data: &'a [u8],
}

impl<'a> Script<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data: bounded(data) }
    }

    pub fn done(&self) -> bool {
        self.data.is_empty()
    }

    pub fn byte(&mut self) -> u8 {
        let Some((first, rest)) = self.data.split_first() else { return 0 };
        self.data = rest;
        *first
    }

    /// One of `n` choices, counted from 0.
    pub fn pick(&mut self, n: usize) -> usize {
        self.byte() as usize % n.max(1)
    }

    pub fn flag(&mut self) -> bool {
        self.byte() & 1 == 1
    }

    /// One of `from`.
    pub fn one<T: Clone>(&mut self, from: &[T]) -> T {
        from[self.pick(from.len())].clone()
    }

    /// Up to `max` bytes: a length, then as many bytes as the input still holds.
    pub fn bytes(&mut self, max: usize) -> &'a [u8] {
        let wanted =
            if max > 255 { usize::from(self.byte()) << 8 | usize::from(self.byte()) } else { self.byte().into() };
        let (bytes, rest) = self.data.split_at(wanted.min(max).min(self.data.len()));
        self.data = rest;
        bytes
    }

    /// Text of up to `max` bytes, read as [`text`] reads it.
    pub fn text(&mut self, max: usize) -> String {
        String::from_utf8_lossy(self.bytes(max)).into_owned()
    }

    /// Whatever the script has not read.
    pub fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.data)
    }
}

/// Writes what [`Script::bytes`] reads, for the seeds a target makes of real files.
pub fn put(out: &mut Vec<u8>, max: usize, bytes: &[u8]) {
    let bytes = &bytes[..bytes.len().min(max)];
    if max > 255 {
        out.push((bytes.len() >> 8) as u8);
    }
    out.push(bytes.len() as u8);
    out.extend_from_slice(bytes);
}
