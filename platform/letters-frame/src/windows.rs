//! What Windows is asked that winit and the standard library do not ask.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{BOOL, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authentication::Identity::{GetUserNameExW, NameDisplay};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, FOLDERID_Profile, SHGetKnownFolderPath};
use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};
use windows_sys::core::GUID;

/// Lets a program built without a console write to the one it was started from, as `letters export` and
/// `letters --version` do. Output that was sent to a file or a pipe is left where it was sent. `false`
/// when there is no console to write to, as when the app is started from the Start menu.
pub fn attach_console() -> bool {
    // SAFETY: both calls take no pointers. `GetStdHandle` only reads the process's own handle table.
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        if out != 0 && out != INVALID_HANDLE_VALUE {
            return true;
        }
        AttachConsole(ATTACH_PARENT_PROCESS) != 0
    }
}

/// The full name of the account, as the sign-in screen shows it. `None` for an account that has only the
/// name it signs in with.
pub fn account_name() -> Option<String> {
    let mut name = [0u16; 512];
    let mut length = name.len() as u32;
    // SAFETY: `name` is writable for `length` units, and the call writes no more than it is told there are.
    let found = unsafe { GetUserNameExW(NameDisplay, name.as_mut_ptr(), &mut length) } != 0;
    let name = name.get(..length as usize).filter(|_| found)?;
    Some(String::from_utf16_lossy(name)).filter(|name| !name.trim().is_empty())
}

/// The Documents folder of the person whose home folder is `home`, wherever it is kept: a sync app such
/// as OneDrive moves it out of the home folder. `None` when `home` is not the home folder the system
/// knows the person by, as in a test, since the system's answer is then about someone else.
pub fn documents_folder(home: &Path) -> Option<PathBuf> {
    let same = |a: &Path, b: &Path| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy());
    known_folder(&FOLDERID_Profile).filter(|profile| same(profile, home))?;
    known_folder(&FOLDERID_Documents)
}

/// Where the system keeps one of the folders it knows by an id.
fn known_folder(id: &GUID) -> Option<PathBuf> {
    let mut path = std::ptr::null_mut();
    // SAFETY: `id` is a constant of the system's, and `path` is where the call puts a string it
    // allocated, or null. It is freed below whether the call worked or not, as the call asks.
    let found = unsafe { SHGetKnownFolderPath(id, 0, 0, &mut path) } >= 0;
    let folder = (found && !path.is_null()).then(|| {
        // SAFETY: on success `path` is a string that ends in a zero, which is as far as this reads.
        let units = unsafe {
            let length = (0..).take_while(|&at| *path.add(at) != 0).count();
            std::slice::from_raw_parts(path, length)
        };
        PathBuf::from(OsString::from_wide(units))
    });
    // SAFETY: `path` came from the call above and is not used again. Null is allowed.
    unsafe { CoTaskMemFree(path.cast()) };
    folder.filter(|folder| folder.is_absolute())
}

/// Whether the person has left animations on in the system's settings. `None` when it does not say.
pub fn animations() -> Option<bool> {
    let mut animated: BOOL = 0;
    // SAFETY: for this question the call writes one BOOL through the pointer, which `animated` is.
    let said = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut animated).cast(), 0) } != 0;
    said.then_some(animated != 0)
}
