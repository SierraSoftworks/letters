//! A window of another process as Windows has it, for the tests that start the app and look at its
//! window from outside: where it is, how it is framed, and a click on it. Nothing the app itself calls.

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_STYLE, GetClientRect, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    PostMessageW, SW_MAXIMIZE, SW_RESTORE, SetCursorPos, ShowWindow, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WS_CAPTION, WS_MAXIMIZEBOX, WS_SIZEBOX,
};

/// A rectangle of the screen, in its pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl From<RECT> for Area {
    fn from(rect: RECT) -> Self {
        Self { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom }
    }
}

/// A window on the screen that some process shows.
#[derive(Clone, Copy, Debug)]
pub struct Probe(HWND);

/// What [`EnumWindows`] is to find, and where it puts it.
struct Search {
    process: u32,
    found: HWND,
}

/// Called by the system with each window that has no parent. It stops at the first one `Search` asks for.
unsafe extern "system" fn visit(window: HWND, search: LPARAM) -> BOOL {
    // SAFETY: `search` is the pointer `window_of` handed to `EnumWindows`, which calls this before it
    // returns, so the `Search` is alive and nothing else has it.
    let search = unsafe { &mut *(search as *mut Search) };
    let mut process = 0;
    // SAFETY: `window` is a window the system is listing, and `process` is where its owner's id goes.
    let visible = unsafe {
        GetWindowThreadProcessId(window, &mut process);
        IsWindowVisible(window) != 0
    };
    if process == search.process && visible {
        search.found = window;
        return 0;
    }
    1
}

/// The window the process `process` shows, if it shows one yet.
pub fn window_of(process: u32) -> Option<Probe> {
    let mut search = Search { process, found: 0 };
    // SAFETY: `visit` is called only during this call, with the pointer to `search`, which outlives it.
    unsafe { EnumWindows(Some(visit), &raw mut search as LPARAM) };
    (search.found != 0).then_some(Probe(search.found))
}

impl Probe {
    /// The whole window, with whatever frame the system gives it.
    pub fn outer(&self) -> Area {
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: `rect` is where the call writes one rectangle. A window that has gone leaves it empty.
        unsafe { GetWindowRect(self.0, &mut rect) };
        rect.into()
    }

    /// The part of the window the app draws.
    pub fn inner(&self) -> Area {
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let mut corner = POINT { x: 0, y: 0 };
        // SAFETY: each call writes the one value it is given a pointer to.
        unsafe {
            GetClientRect(self.0, &mut rect);
            ClientToScreen(self.0, &mut corner);
        }
        Area { left: corner.x, top: corner.y, right: corner.x + rect.right, bottom: corner.y + rect.bottom }
    }

    /// The part of the window's display that the taskbar leaves free.
    pub fn work_area(&self) -> Area {
        let empty = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let mut info =
            MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, rcMonitor: empty, rcWork: empty, dwFlags: 0 };
        // SAFETY: `info` says its own size, which is how the call knows what it may write.
        unsafe { GetMonitorInfoW(MonitorFromWindow(self.0, MONITOR_DEFAULTTONEAREST), &mut info) };
        info.rcWork.into()
    }

    /// How many pixels make one of the window's units.
    pub fn scale(&self) -> f64 {
        // SAFETY: the call takes no pointers, and answers 0 for a window that has gone.
        let dpi = unsafe { GetDpiForWindow(self.0) };
        if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 }
    }

    /// Whether the system still treats the window as one with a frame of its own to resize, snap and
    /// maximise, whatever is drawn of that frame.
    pub fn framed(&self) -> bool {
        // SAFETY: the call takes no pointers.
        let style = unsafe { GetWindowLongW(self.0, GWL_STYLE) } as u32;
        let asked = WS_CAPTION | WS_SIZEBOX | WS_MAXIMIZEBOX;
        style & asked == asked
    }

    /// Maximises the window, or puts it back, as the system's own menu does.
    pub fn maximise(&self, maximised: bool) {
        // SAFETY: the call takes no pointers.
        unsafe { ShowWindow(self.0, if maximised { SW_MAXIMIZE } else { SW_RESTORE }) };
    }

    /// Clicks the window at a point of [`Probe::inner`], counted in pixels from its top left corner.
    pub fn click(&self, x: i32, y: i32) {
        let inner = self.inner();
        let at = ((y as u16 as isize) << 16) | (x as u16 as isize);
        // The left button is down in the message for the press, as the system would say.
        const LEFT: usize = 1;
        // SAFETY: none of the calls takes a pointer. A window that has gone takes no message.
        unsafe {
            SetCursorPos(inner.left + x, inner.top + y);
            PostMessageW(self.0, WM_MOUSEMOVE, 0, at);
            PostMessageW(self.0, WM_LBUTTONDOWN, LEFT, at);
            PostMessageW(self.0, WM_LBUTTONUP, 0, at);
        }
    }
}
