//! The empty window that launches are measured against, for `--baseline`.
//!
//! It starts the process, the event loop and one window of the usual size, and shows one white frame. No
//! document, fonts, layout, accessibility or GPU: whatever a launch of the app takes beyond this is ours.

use std::error::Error;
use std::num::NonZeroU32;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::app::{event_loop, window_attributes};
use crate::measure::{FIRST_FRAME, Launch};

const WHITE: u32 = 0x00ff_ffff;

type Surface = softbuffer::Surface<Arc<Window>, Arc<Window>>;

/// Shows the empty window, prints the report a launch of the app would, and exits.
pub fn run(headless: bool, mut launch: Launch) -> Result<(), Box<dyn Error>> {
    if headless {
        // With no window there is nothing to open: what remains is starting the process.
        finish(launch);
        return Ok(());
    }
    let event_loop = event_loop::<()>(true)?;
    launch.mark("event loop built");
    let mut app = Baseline { launch: Some(launch), surface: None, failure: None };
    event_loop.run_app(&mut app)?;
    app.failure.map_or(Ok(()), |failure| Err(failure.into()))
}

fn finish(mut launch: Launch) {
    launch.mark(FIRST_FRAME);
    println!("{FIRST_FRAME}\n{}", launch.report());
}

struct Baseline {
    /// The stopwatch, until the frame is on screen.
    launch: Option<Launch>,
    surface: Option<Surface>,
    failure: Option<String>,
}

impl Baseline {
    fn mark(&mut self, stage: &'static str) {
        if let Some(launch) = &mut self.launch {
            launch.mark(stage);
        }
    }

    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let window = Arc::new(event_loop.create_window(window_attributes(event_loop))?);
        self.surface = Some(softbuffer::Surface::new(&softbuffer::Context::new(window.clone())?, window)?);
        self.mark("window shown");
        Ok(())
    }

    fn paint(&mut self) -> Result<(), softbuffer::SoftBufferError> {
        let Some(surface) = &mut self.surface else { return Ok(()) };
        let size = surface.window().inner_size();
        let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(());
        };
        surface.resize(width, height)?;
        let mut buffer = surface.buffer_mut()?;
        buffer.fill(WHITE);
        buffer.present()
    }
}

impl ApplicationHandler for Baseline {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        self.mark("app launched");
        if let Err(error) = self.open(event_loop) {
            self.failure = Some(format!("no window could be opened: {error}"));
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if event != WindowEvent::RedrawRequested {
            return;
        }
        match self.paint() {
            Ok(()) => {
                if let Some(launch) = self.launch.take() {
                    finish(launch);
                }
            }
            Err(error) => self.failure = Some(format!("the window could not be painted: {error}")),
        }
        event_loop.exit();
    }
}
