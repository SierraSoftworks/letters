//! Printing: a document's PDF handed to the system's print dialog.
//!
//! macOS shows its print sheet on the window. Other Unix desktops print through the desktop portal, which
//! shows the dialog of the person's desktop and takes the PDF as an open file. Windows prints what a
//! program draws, so each page of the PDF is drawn as a picture and put on the printer the person chose
//! in the system's print dialog.

use winit::window::Window;

/// Hands `pdf` to the system's print dialog for `window`, as a job called `title`. Call it on the main
/// thread. `done` hears how that went, perhaps on another thread and perhaps only once the dialog is
/// closed: `Ok` when the dialog was shown, whether the person then printed or not.
pub fn print(pdf: Vec<u8>, title: String, window: &Window, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    system::print(pdf, title, window, done);
}

#[cfg(target_os = "macos")]
mod system {
    use winit::window::Window;

    pub fn print(pdf: Vec<u8>, title: String, window: &Window, done: impl FnOnce(Result<(), String>)) {
        let shown = letters_frame::print_pdf(window, &pdf, &title);
        done(shown.then_some(()).ok_or_else(|| "The print dialog could not be shown.".to_owned()));
    }
}

#[cfg(windows)]
pub use system::print_on;

#[cfg(windows)]
mod system {
    use std::ops::RangeInclusive;
    use std::path::Path;
    use std::thread;

    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::hayro_syntax::page::Page;
    use hayro::vello_cpu::color::palette::css;
    use hayro::{PixmapSettings, RenderCache, RenderSettings};
    use letters_frame::printer::{self, Paper, Printer, Sheet};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    /// The finest a page is drawn, in dots to the inch. A printer with more is sent this and fills in:
    /// text stays sharp to the eye, and a page stays tens of megabytes and not hundreds.
    const FINEST: f32 = 300.0;

    pub fn print(pdf: Vec<u8>, title: String, window: &Window, done: impl FnOnce(Result<(), String>) + Send + 'static) {
        let owner = match window.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
            _ => 0,
        };
        // The dialog is waited for, and the pages are drawn, which the window's own thread cannot do.
        thread::spawn(move || done(through_dialog(pdf, &title, owner)));
    }

    /// Shows the print dialog, then prints the pages the person chose there on the printer they chose.
    fn through_dialog(pdf: Vec<u8>, title: &str, owner: isize) -> Result<(), String> {
        let pdf = read(pdf)?;
        let count = u32::try_from(pdf.pages().len()).unwrap_or(u32::MAX);
        let chosen = printer::choose(owner, count).map_err(|()| "The print dialog could not be shown.".to_owned())?;
        // A dialog that was closed without printing is no failure.
        let Some(chosen) = chosen else { return Ok(()) };
        print_pages(chosen.printer, &pdf, title, chosen.pages, None)
    }

    fn read(pdf: Vec<u8>) -> Result<Pdf, String> {
        Pdf::new(pdf).map_err(|error| {
            eprintln!("letters: could not read what is printed: {error:?}");
            "The document could not be made ready to print.".to_owned()
        })
    }

    /// Prints the pages `pages` of `pdf`, counted from 1, on `printer` as a job called `title`. `into`
    /// names the file a printer that writes one is to write, which the print dialog otherwise asks for.
    pub fn print_on(
        printer: Printer,
        pdf: Vec<u8>,
        title: &str,
        pages: RangeInclusive<u32>,
        into: Option<&Path>,
    ) -> Result<(), String> {
        print_pages(printer, &read(pdf)?, title, pages, into)
    }

    fn print_pages(
        printer: Printer,
        pdf: &Pdf,
        title: &str,
        pages: RangeInclusive<u32>,
        into: Option<&Path>,
    ) -> Result<(), String> {
        let paper = printer.paper();
        let numbered = (1u32..).zip(pdf.pages().iter());
        let mut wanted = numbered.filter(|(number, _)| pages.contains(number)).map(|(_, page)| page);
        let sent = printer.print(title, into, || wanted.next().map(|page| sheet(page, paper)));
        sent.then_some(()).ok_or_else(|| "The printer did not take the document.".to_owned())
    }

    /// One page as the picture the printer is sent: at its own size, or smaller where the paper is, and
    /// white where the page draws nothing.
    fn sheet(page: &Page<'_>, paper: Paper) -> Sheet {
        // A page is measured in points, 72 to the inch.
        let (width, height) = page.render_dimensions();
        let (across, down) = (width * paper.dots.0 as f32 / 72.0, height * paper.dots.1 as f32 / 72.0);
        let fit = (paper.width as f32 / across).min(paper.height as f32 / down).min(1.0);
        let (across, down) = (across * fit, down * fit);
        let coarser = (FINEST / paper.dots.0.max(paper.dots.1) as f32).min(1.0);
        let scale = PixmapSettings {
            x_scale: across * coarser / width,
            y_scale: down * coarser / height,
            bg_color: css::WHITE,
        };
        let drawn = hayro::render(
            page,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &scale,
        );
        let (wide, tall) = (usize::from(drawn.width()), usize::from(drawn.height()));
        // The system keeps a picture from its bottom row up, and blue before red.
        let mut pixels = Vec::with_capacity(wide * tall * 4);
        for row in drawn.data().chunks_exact(wide.max(1)).rev() {
            pixels.extend(row.iter().flat_map(|pixel| [pixel.b, pixel.g, pixel.r, 255]));
        }
        Sheet {
            width: wide as u32,
            height: tall as u32,
            pixels,
            across: across.round() as u32,
            down: down.round() as u32,
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod system {
    use std::collections::HashMap;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::os::fd::AsFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::thread;

    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{Fd, OwnedObjectPath, OwnedValue, Value};

    use crate::platform;

    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const NO_PORTAL: &str = "This desktop offers apps no print dialog (xdg-desktop-portal). Export a PDF from Print \
                             view and print that.";

    pub fn print(pdf: Vec<u8>, title: String, window: &Window, done: impl FnOnce(Result<(), String>) + Send + 'static) {
        let parent = parent(window);
        // The dialog is waited for, which the window's own thread cannot do.
        thread::spawn(move || done(held(&pdf).and_then(|file| through_portal(&file, &title, &parent))));
    }

    /// The window as the portal is told of it, so that the dialog is shown over it. Only X11 has a name
    /// for a window that another program can use as it is.
    fn parent(window: &Window) -> String {
        match window.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Xlib(handle)) => format!("x11:{:x}", handle.window),
            Ok(RawWindowHandle::Xcb(handle)) => format!("x11:{:x}", handle.window),
            _ => String::new(),
        }
    }

    /// The PDF as an open file with no name: it is the document, so nobody else may read it, and it is
    /// gone once the portal has closed it.
    fn held(pdf: &[u8]) -> Result<File, String> {
        let path = std::env::temp_dir().join(format!("letters-print-{}.pdf", platform::block_id()));
        let written = (|| {
            OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?.write_all(pdf)?;
            File::open(&path)
        })();
        let _ = fs::remove_file(&path);
        written.map_err(|error| {
            eprintln!("letters: could not write what is printed: {error}");
            "The document could not be made ready to print.".to_owned()
        })
    }

    /// Asks the portal to show the print dialog, then to print the file as the person chose there.
    fn through_portal(pdf: &File, title: &str, parent: &str) -> Result<(), String> {
        let failed = |error: zbus::Error| match error {
            zbus::Error::MethodError(..) | zbus::Error::InterfaceNotFound => NO_PORTAL.to_owned(),
            other => {
                eprintln!("letters: the print dialog could not be reached: {other}");
                "The print dialog could not be reached.".to_owned()
            }
        };
        let connection = Connection::session().map_err(|_| NO_PORTAL.to_owned())?;
        let print = Proxy::new(&connection, PORTAL, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Print")
            .map_err(failed)?;
        let nothing = HashMap::<&str, Value<'_>>::new();
        let prepared = asked(&connection, |token| {
            let options = HashMap::from([("handle_token", Value::from(token)), ("modal", Value::from(true))]);
            print.call("PreparePrint", &(parent, title, &nothing, &nothing, options))
        });
        // A dialog that was closed without printing is no failure.
        let Some(chosen) = prepared.map_err(failed)? else { return Ok(()) };
        let token = chosen.get("token").and_then(|token| u32::try_from(token).ok()).unwrap_or_default();
        let printed = asked(&connection, |handle| {
            let options = HashMap::from([("handle_token", Value::from(handle)), ("token", Value::from(token))]);
            print.call("Print", &(parent, title, Fd::from(pdf.as_fd()), options))
        });
        printed.map(drop).map_err(failed)
    }

    /// Makes a call that the portal answers later, and waits for that answer: what was chosen, or `None`
    /// when the person closed the dialog. `call` is given the token that names where the answer comes.
    fn asked(
        connection: &Connection,
        call: impl FnOnce(&str) -> zbus::Result<OwnedObjectPath>,
    ) -> zbus::Result<Option<HashMap<String, OwnedValue>>> {
        let token = format!("letters{}", platform::block_id().replace('-', ""));
        let sender = connection.unique_name().map(|name| name.trim_start_matches(':').replace('.', "_"));
        // The answer is listened for before the call is made, at the path the portal derives from the token.
        let path = format!("/org/freedesktop/portal/desktop/request/{}/{token}", sender.unwrap_or_default());
        let request = Proxy::new(connection, PORTAL, path.as_str(), "org.freedesktop.portal.Request")?;
        let mut answers = request.receive_signal("Response")?;
        let handle = call(&token)?;
        if handle.as_str() != path {
            // A portal too old to take the token answers somewhere nothing listened in time.
            return Err(zbus::Error::InterfaceNotFound);
        }
        let answer = answers.next().ok_or(zbus::Error::InterfaceNotFound)?;
        let (response, results): (u32, HashMap<String, OwnedValue>) = answer.body().deserialize()?;
        match response {
            0 => Ok(Some(results)),
            1 => Ok(None),
            _ => Err(zbus::Error::Failure("the dialog ended without an answer".into())),
        }
    }
}
