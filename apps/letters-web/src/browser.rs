//! What the browser lends the editor: a clock, identifiers, and the clipboard for as long as a clipboard
//! event lasts.

use letters_editor::{Clip, Platform};
use wasm_bindgen::JsValue;
use web_sys::{Crypto, Performance, Window};

pub struct Browser {
    performance: Performance,
    crypto: Crypto,
    /// What a `paste` event brought, for the editor to read while it handles that event.
    pub pasted: Option<Clip>,
    /// What the editor copied, for the `copy` or `cut` event that asked for it to carry away.
    pub copied: Option<Clip>,
}

impl Browser {
    pub fn new(window: &Window) -> Result<Self, JsValue> {
        let performance = window.performance().ok_or("this browser has no clock")?;
        Ok(Self { performance, crypto: window.crypto()?, pasted: None, copied: None })
    }

    /// Milliseconds since the page started loading, to a fraction.
    pub fn now(&self) -> f64 {
        self.performance.now()
    }
}

impl Platform for Browser {
    fn now_ms(&self) -> u64 {
        self.now() as u64
    }

    // A page may read the clipboard only inside a paste event, so outside one there is nothing to give.
    fn clipboard_read(&mut self) -> Option<Clip> {
        self.pasted.clone()
    }

    fn clipboard_write(&mut self, clip: Clip) {
        self.copied = Some(clip);
    }

    fn new_id(&mut self) -> String {
        let mut bytes = [0; 10];
        self.crypto.get_random_values_with_u8_array(&mut bytes).expect("the browser has a source of randomness");
        block_id(bytes)
    }
}

/// A block id: 80 random bits as 16 characters, as the desktop app writes them.
pub fn block_id(random: [u8; 10]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let bits = random.iter().fold(0u128, |bits, byte| bits << 8 | u128::from(*byte));
    (0..16).map(|i| ALPHABET[(bits >> (5 * i) & 31) as usize] as char).collect()
}
