//! The system's print sheet for a PDF, which neither winit nor AppKit's own views give: a view prints one
//! page, and a document of many needs PDFKit.

use objc2::msg_send_id;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject};
use objc2_app_kit::{NSPrintInfo, NSPrintOperation, NSView};
use objc2_foundation::{MainThreadMarker, NSData, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

// `PDFDocument` is looked up by name, so the framework that holds it has to be loaded.
#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}

/// `kPDFPrintPageScaleDownToFit`: a page larger than the paper is made to fit, and no page is enlarged.
const DOWN_TO_FIT: isize = 2;

/// Shows the print sheet on a window for the PDF in `pdf`, named `title` in the print queue. The person
/// prints or cancels from the sheet, after this returns. `false` off the main thread, when the window is
/// not AppKit's, or when the bytes are no PDF: nothing was shown then.
pub fn print_pdf(window: &impl HasWindowHandle, pdf: &[u8], title: &str) -> bool {
    shown(window, pdf, title).is_some()
}

fn shown(window: &impl HasWindowHandle, pdf: &[u8], title: &str) -> Option<()> {
    MainThreadMarker::new()?;
    let RawWindowHandle::AppKit(handle) = window.window_handle().ok()?.as_raw() else { return None };
    // SAFETY: the handle holds the window's content view, an `NSView` that lives while `window` is
    // borrowed, and views are only used on the main thread, which the marker says this is.
    let view: Retained<NSView> = unsafe { Retained::retain(handle.ns_view.as_ptr().cast()) }?;
    let window = view.window()?;
    let data = NSData::with_bytes(pdf);
    let class = AnyClass::get("PDFDocument")?;
    // SAFETY: `+alloc` then `-initWithData:` of `PDFDocument`, which takes an `NSData` and gives the
    // document or nil when the data is no PDF.
    let document: Option<Retained<AnyObject>> = unsafe {
        let empty: Allocated<AnyObject> = msg_send_id![class, alloc];
        msg_send_id![empty, initWithData: &*data]
    };
    let document = document?;
    // SAFETY: a class property of `NSPrintInfo`, read on the main thread.
    let info = unsafe { NSPrintInfo::sharedPrintInfo() };
    // SAFETY: `-[PDFDocument printOperationForPrintInfo:scalingMode:autoRotate:]` takes an `NSPrintInfo`,
    // an `NSInteger` enum and a `BOOL`, and gives an `NSPrintOperation` or nil.
    let operation: Option<Retained<NSPrintOperation>> = unsafe {
        msg_send_id![&*document, printOperationForPrintInfo: &*info, scalingMode: DOWN_TO_FIT, autoRotate: true]
    };
    let operation = operation?;
    // SAFETY: a property setter, and the call that shows the sheet, on the main thread with objects
    // that are alive. No delegate is told how it ended, so there is no selector or context to keep
    // alive. The sheet retains the operation while it is up.
    unsafe {
        operation.setJobTitle(Some(&NSString::from_str(title)));
        operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
            &window,
            None,
            None,
            std::ptr::null_mut(),
        );
    }
    Some(())
}
