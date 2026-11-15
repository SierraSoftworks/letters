//! Where there is no Wayland: the same calls, and no window to make them on.

use std::convert::Infallible;
use std::marker::PhantomData;

use crate::Strip;

/// A Wayland window's surface. There is none here, so [`Surface::new`] never gives one.
pub struct Surface<W> {
    never: Infallible,
    _window: PhantomData<W>,
}

impl<W> Surface<W> {
    pub fn new(_window: W) -> Option<Self> {
        None
    }

    pub fn frame(&mut self, _width: u32, _height: u32) -> Option<Frame<'_>> {
        match self.never {}
    }

    pub fn release_frames(&mut self) {
        match self.never {}
    }

    pub fn set_shadow(&mut self, _strips: &[Strip<'_>]) -> bool {
        match self.never {}
    }
}

/// A frame of such a surface.
pub struct Frame<'a> {
    never: Infallible,
    _surface: PhantomData<&'a mut ()>,
}

impl Frame<'_> {
    pub fn age(&self) -> u8 {
        match self.never {}
    }

    pub fn pixels(&mut self) -> &mut [u32] {
        match self.never {}
    }

    pub fn present(self) {
        match self.never {}
    }
}
