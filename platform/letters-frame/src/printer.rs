//! Printing on Windows: the system's print dialog, and pages drawn on the printer it gives.
//!
//! Windows prints what a program draws, so the shell draws each page as a picture and this puts the
//! pictures on paper.

use std::ops::RangeInclusive;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDCW, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps, HDC, LOGPIXELSX,
    LOGPIXELSY, PHYSICALHEIGHT, PHYSICALOFFSETX, PHYSICALOFFSETY, PHYSICALWIDTH, RGBQUAD, SRCCOPY, StretchDIBits,
};
use windows_sys::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows_sys::Win32::UI::Controls::Dialogs::{
    PD_NOSELECTION, PD_PAGENUMS, PD_RETURNDC, PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGW, PrintDlgW,
};

/// A printer that is ready to be drawn on. It is let go of when this is.
pub struct Printer(HDC);

// SAFETY: a device context is a number the system knows the printer by, and may be used from any one
// thread at a time, which owning the `Printer` sees to.
unsafe impl Send for Printer {}

impl Drop for Printer {
    fn drop(&mut self) {
        // SAFETY: the context is this value's own, and nothing uses it after.
        unsafe { DeleteDC(self.0) };
    }
}

/// A printer's sheet of paper, in the printer's own dots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paper {
    pub width: u32,
    pub height: u32,
    /// How many dots make an inch, across and down.
    pub dots: (u32, u32),
}

/// One page as a picture, and how large it is to be on the paper.
pub struct Sheet {
    /// The picture's size in its own pixels.
    pub width: u32,
    pub height: u32,
    /// Blue, green, red and a byte that is not read for each pixel, row by row from the bottom up, as the
    /// system keeps a picture.
    pub pixels: Vec<u8>,
    /// Its size on the paper in the printer's dots. It is centred there.
    pub across: u32,
    pub down: u32,
}

/// What the person chose in the print dialog.
pub struct Chosen {
    pub printer: Printer,
    /// The pages to print, counted from 1.
    pub pages: RangeInclusive<u32>,
}

/// A string as the system reads one: UTF-16 with a zero at its end.
fn wide(text: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain(Some(0)).collect()
}

/// Shows the system's print dialog over the window `owner` (its `HWND`, or 0 for none) for a document of
/// `pages` pages, and waits for it. `Ok(None)` when the person closed it without printing, and `Err` when
/// the dialog could not be shown. The window takes no input meanwhile, so call this off its thread.
pub fn choose(owner: isize, pages: u32) -> Result<Option<Chosen>, ()> {
    let last = pages.clamp(1, u32::from(u16::MAX)) as u16;
    // SAFETY: the structure is plain numbers and pointers, for which all zeroes is the empty value.
    let mut dialog: PRINTDLGW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = size_of::<PRINTDLGW>() as u32;
    dialog.hwndOwner = owner;
    dialog.Flags = PD_RETURNDC | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE;
    (dialog.nMinPage, dialog.nMaxPage, dialog.nFromPage, dialog.nToPage) = (1, last, 1, last);
    // SAFETY: `dialog` says its own size and outlives the call, which returns once the dialog is closed.
    let printed = unsafe { PrintDlgW(&mut dialog) } != 0;
    // SAFETY: the dialog allocates these two for the caller to free, used or not. Null is allowed.
    unsafe {
        GlobalFree(dialog.hDevMode);
        GlobalFree(dialog.hDevNames);
    }
    if !printed {
        // SAFETY: the call takes no arguments. It says why the dialog ended, and 0 when it was closed.
        let failed = unsafe { windows_sys::Win32::UI::Controls::Dialogs::CommDlgExtendedError() } != 0;
        return if failed { Err(()) } else { Ok(None) };
    }
    if dialog.hDC == 0 {
        return Err(());
    }
    let printer = Printer(dialog.hDC);
    let asked = dialog.Flags & PD_PAGENUMS != 0;
    let (from, to) = if asked { (dialog.nFromPage.max(1), dialog.nToPage.min(last)) } else { (1, last) };
    Ok(Some(Chosen { printer, pages: u32::from(from)..=u32::from(to.max(from)) }))
}

impl Printer {
    /// The printer the system knows by `name`, with the settings it keeps for it. `None` when it knows
    /// none by that name.
    pub fn named(name: &str) -> Option<Self> {
        let name = wide(name);
        // SAFETY: `name` ends in a zero and outlives the call. The other pointers may be null.
        let context = unsafe { CreateDCW(std::ptr::null(), name.as_ptr(), std::ptr::null(), std::ptr::null()) };
        (context != 0).then_some(Self(context))
    }

    /// The whole sheet, also where the printer cannot print.
    pub fn paper(&self) -> Paper {
        // SAFETY: the call takes no pointers.
        let asked = |what: u32| unsafe { GetDeviceCaps(self.0, what as i32) }.max(1) as u32;
        Paper {
            width: asked(PHYSICALWIDTH),
            height: asked(PHYSICALHEIGHT),
            dots: (asked(LOGPIXELSX), asked(LOGPIXELSY)),
        }
    }

    /// Prints a job called `title`: `sheet` is asked for each page in turn until it has none. `into`
    /// names a file for a printer that writes one, which then asks nobody where to put it. `false` when
    /// the printer refused any of it, and then nothing more is sent.
    pub fn print(self, title: &str, into: Option<&Path>, mut sheet: impl FnMut() -> Option<Sheet>) -> bool {
        let (title, into) = (wide(title), into.map(wide));
        let job = DOCINFOW {
            cbSize: size_of::<DOCINFOW>() as i32,
            lpszDocName: title.as_ptr(),
            lpszOutput: into.as_ref().map_or(std::ptr::null(), |into| into.as_ptr()),
            lpszDatatype: std::ptr::null(),
            fwType: 0,
        };
        // SAFETY: `job` and the strings it points at outlive the call.
        if unsafe { StartDocW(self.0, &job) } <= 0 {
            return false;
        }
        while let Some(sheet) = sheet() {
            if !self.page(&sheet) {
                // SAFETY: the call takes no pointers. It drops what the job has so far.
                unsafe { AbortDoc(self.0) };
                return false;
            }
        }
        // SAFETY: the call takes no pointers.
        unsafe { EndDoc(self.0) > 0 }
    }

    /// Draws one page in the middle of the paper.
    fn page(&self, sheet: &Sheet) -> bool {
        // A picture shorter than it says would be read past its end.
        let rows = sheet.width as usize * 4 * sheet.height as usize;
        if rows == 0 || sheet.pixels.len() < rows {
            return false;
        }
        let paper = self.paper();
        // SAFETY: the calls take no pointers. The offsets say where the printable part of the paper starts.
        let (left, top) =
            unsafe { (GetDeviceCaps(self.0, PHYSICALOFFSETX as i32), GetDeviceCaps(self.0, PHYSICALOFFSETY as i32)) };
        // Drawing starts at the printable part's corner, so the paper's own corner is before it.
        let x = (paper.width as i32 - sheet.across as i32) / 2 - left;
        let y = (paper.height as i32 - sheet.down as i32) / 2 - top;
        let picture = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: sheet.width as i32,
                biHeight: sheet.height as i32,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbReserved: 0 }],
        };
        // SAFETY: `picture` describes `sheet.pixels`, which was checked above to hold every row it names,
        // and both outlive the call. The other calls take no pointers.
        unsafe {
            if StartPage(self.0) <= 0 {
                return false;
            }
            let drawn = StretchDIBits(
                self.0,
                x,
                y,
                sheet.across as i32,
                sheet.down as i32,
                0,
                0,
                sheet.width as i32,
                sheet.height as i32,
                sheet.pixels.as_ptr().cast(),
                &picture,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            EndPage(self.0) > 0 && drawn > 0
        }
    }
}
