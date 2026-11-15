//! A Wayland window's pixels with alpha, and a shadow outside its edges.
//!
//! winit gives a window one surface, whose size is the window's. So the corners are rounded by leaving
//! them clear, which needs frames with alpha, and the shadow lies on surfaces of its own under the window.
//! Both are made of shared memory here, on the connection and surface winit made.

use std::fs::File;
use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use memmap2::MmapMut;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use rustix::fs::{MemfdFlags, SealFlags};
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_subcompositor::WlSubcompositor;
use wayland_client::protocol::wl_subsurface::WlSubsurface;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};

use crate::Strip;

/// Takes the events of what is made here. Only a buffer's release says anything.
struct Events;

delegate_noop!(Events: ignore WlShm);
delegate_noop!(Events: ignore WlSurface);
delegate_noop!(Events: WlShmPool);
delegate_noop!(Events: WlCompositor);
delegate_noop!(Events: WlSubcompositor);
delegate_noop!(Events: WlSubsurface);
delegate_noop!(Events: WlRegion);

impl Dispatch<WlRegistry, GlobalListContents> for Events {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlBuffer, Arc<AtomicBool>> for Events {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        event: wl_buffer::Event,
        free: &Arc<AtomicBool>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            free.store(true, Ordering::Relaxed);
        }
    }
}

/// Shared memory the compositor shows as a picture: one `0xAARRGGBB` word per pixel, colours multiplied
/// by alpha.
struct Picture {
    buffer: WlBuffer,
    map: MmapMut,
    width: u32,
    height: u32,
    /// Whether the compositor has let go of it since it was last shown.
    free: Arc<AtomicBool>,
    /// How many presents ago its pixels were on screen, or 0 if they never were.
    age: u8,
}

impl Picture {
    fn new(shm: &WlShm, queue: &QueueHandle<Events>, width: u32, height: u32) -> Option<Self> {
        let (wide, high) = (i32::try_from(width).ok()?, i32::try_from(height).ok()?);
        let bytes = wide.checked_mul(high)?.checked_mul(4).filter(|bytes| *bytes > 0)?;
        let memory = rustix::fs::memfd_create("letters-frame", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING).ok()?;
        let file = File::from(memory);
        file.set_len(bytes as u64).ok()?;
        rustix::fs::fcntl_add_seals(&file, SealFlags::SHRINK | SealFlags::SEAL).ok()?;
        // SAFETY: the file is memory made just above and sealed against shrinking, so every byte of the
        // map stays backed. Its only other holder is the compositor, which reads it and is told through
        // `free` and the surface's commits when the pixels change.
        let map = unsafe { MmapMut::map_mut(&file) }.ok()?;
        let pool = shm.create_pool(file.as_fd(), bytes, queue, ());
        let free = Arc::new(AtomicBool::new(true));
        let buffer = pool.create_buffer(0, wide, high, wide * 4, wl_shm::Format::Argb8888, queue, free.clone());
        // The buffer keeps the pool's memory, and the pool has the file.
        pool.destroy();
        Some(Self { buffer, map, width, height, free, age: 0 })
    }

    fn pixels(&mut self) -> &mut [u32] {
        // A map starts on a page, so its bytes are whole words.
        bytemuck::cast_slice_mut(&mut self.map)
    }

    /// Makes it what `surface` shows from its next commit.
    fn show(&self, surface: &WlSurface) {
        self.free.store(false, Ordering::Relaxed);
        surface.attach(Some(&self.buffer), 0, 0);
        if surface.version() >= 4 {
            surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        } else {
            surface.damage(0, 0, i32::MAX, i32::MAX);
        }
    }
}

impl Drop for Picture {
    fn drop(&mut self) {
        // The memory goes with the map, and is never written again: what is still shown stays as it is.
        self.buffer.destroy();
    }
}

/// A surface under the window's own, which takes no input.
struct Shade {
    surface: WlSurface,
    role: WlSubsurface,
    picture: Option<Picture>,
}

impl Drop for Shade {
    fn drop(&mut self) {
        self.role.destroy();
        self.surface.destroy();
    }
}

/// A Wayland window's surface, painted from here instead of through a software surface without alpha.
///
/// It keeps `window`, whose surface and connection it uses, for as long as it lives.
pub struct Surface<W> {
    shadow: Vec<Shade>,
    /// The picture on screen, and the one the next frame is painted into.
    frames: [Option<Picture>; 2],
    surface: WlSurface,
    shm: WlShm,
    compositor: WlCompositor,
    subcompositor: WlSubcompositor,
    events: EventQueue<Events>,
    connection: Connection,
    _window: W,
}

impl<W: HasDisplayHandle + HasWindowHandle> Surface<W> {
    /// `None` when the window is not a Wayland one, or its compositor lacks what this needs.
    pub fn new(window: W) -> Option<Self> {
        let RawDisplayHandle::Wayland(display) = window.display_handle().ok()?.as_raw() else { return None };
        let RawWindowHandle::Wayland(handle) = window.window_handle().ok()?.as_raw() else { return None };
        // SAFETY: the pointer is the `wl_display` of the connection `window` was made on, which is open
        // while the window is. This keeps the window, and drops the connection before it.
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, events) = registry_queue_init::<Events>(&connection).ok()?;
        let queue = events.handle();
        let shm = globals.bind(&queue, 1..=1, ()).ok()?;
        let compositor = globals.bind(&queue, 3..=4, ()).ok()?;
        let subcompositor = globals.bind(&queue, 1..=1, ()).ok()?;
        // SAFETY: the pointer is the window's `wl_surface`, a live object of that same connection, for
        // the same reason and as long.
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), handle.surface.as_ptr().cast()) }.ok()?;
        let surface = WlSurface::from_id(&connection, id).ok()?;
        let shadow = Vec::new();
        Some(Self {
            shadow,
            frames: [None, None],
            surface,
            shm,
            compositor,
            subcompositor,
            events,
            connection,
            _window: window,
        })
    }

    /// The next frame of `width` by `height` pixels, to paint and then present. `None` when no memory
    /// could be had for it.
    pub fn frame(&mut self, width: u32, height: u32) -> Option<Frame<'_>> {
        let _ = self.events.dispatch_pending(&mut Events);
        let [_, back] = &mut self.frames;
        let usable = |picture: &Picture| {
            (picture.width, picture.height) == (width, height) && picture.free.load(Ordering::Relaxed)
        };
        // A picture the compositor still holds is not waited for: the frame gets a new one.
        if !back.as_ref().is_some_and(usable) {
            *back = Some(Picture::new(&self.shm, &self.events.handle(), width, height)?);
        }
        Some(Frame { frames: &mut self.frames, surface: &self.surface, connection: &self.connection })
    }

    /// Gives up the frames' memory, when something else paints the window from now on.
    pub fn release_frames(&mut self) {
        self.frames = [None, None];
        let _ = self.connection.flush();
    }

    /// Lays `strips` under the window in place of any before them, from the window's next frame on: they
    /// change size together with it. Says whether they will show. No strips, no shadow.
    pub fn set_shadow(&mut self, strips: &[Strip<'_>]) -> bool {
        let laid = self.lay(strips).is_some();
        if !laid {
            self.shadow.clear();
        }
        let _ = self.connection.flush();
        laid && !strips.is_empty()
    }

    fn lay(&mut self, strips: &[Strip<'_>]) -> Option<()> {
        let queue = self.events.handle();
        self.shadow.truncate(strips.len());
        for (index, strip) in strips.iter().enumerate() {
            if self.shadow.len() == index {
                let surface = self.compositor.create_surface(&queue, ());
                let role = self.subcompositor.get_subsurface(&surface, &self.surface, &queue, ());
                role.place_below(&self.surface);
                // A click on the shadow belongs to whatever lies under it.
                let nowhere = self.compositor.create_region(&queue, ());
                surface.set_input_region(Some(&nowhere));
                nowhere.destroy();
                self.shadow.push(Shade { surface, role, picture: None });
            }
            let shade = &mut self.shadow[index];
            let mut picture = Picture::new(&self.shm, &queue, strip.width, strip.height)?;
            let pixels = picture.pixels().get_mut(..strip.pixels.len()).filter(|_| !strip.pixels.is_empty())?;
            pixels.copy_from_slice(strip.pixels);
            shade.surface.set_buffer_scale(i32::try_from(strip.scale.max(1)).ok()?);
            picture.show(&shade.surface);
            shade.role.set_position(strip.x, strip.y);
            // Held back by the compositor until the window's own next commit.
            shade.surface.commit();
            shade.picture = Some(picture);
        }
        Some(())
    }
}

impl<W> Drop for Surface<W> {
    fn drop(&mut self) {
        self.shadow.clear();
        self.frames = [None, None];
        let _ = self.connection.flush();
    }
}

/// A frame being painted. Dropping it without presenting leaves the window as it was.
pub struct Frame<'a> {
    frames: &'a mut [Option<Picture>; 2],
    surface: &'a WlSurface,
    connection: &'a Connection,
}

impl Frame<'_> {
    /// What the pixels hold now: the frame presented last (1), the one before it (2), or nothing to rely
    /// on (0).
    pub fn age(&self) -> u8 {
        self.frames[1].as_ref().map_or(0, |picture| picture.age)
    }

    /// The frame's pixels: `0xAARRGGBB` words with the colours multiplied by alpha, row by row from the
    /// top left.
    pub fn pixels(&mut self) -> &mut [u32] {
        self.frames[1].as_mut().map_or(&mut [], Picture::pixels)
    }

    /// Puts the frame on screen.
    pub fn present(self) {
        let [front, back] = self.frames;
        let Some(shown) = back else { return };
        shown.show(self.surface);
        self.surface.commit();
        shown.age = 1;
        if let Some(before) = front {
            before.age = if before.age == 1 { 2 } else { 0 };
        }
        std::mem::swap(front, back);
        let _ = self.connection.flush();
    }
}
