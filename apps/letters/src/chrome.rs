//! What the desktop expects of a window, now that Letters draws all of it.
//!
//! On macOS the system keeps its traffic lights, so there is nothing to ask. On Linux the window buttons are
//! the bar's own, and they follow the desktop's `button-layout` setting: which buttons, in what order, on
//! which side. Whether things may move is the desktop's to say too.

use letters_ui::{Chrome, Frame, WindowButton};

/// Room the bar leaves for the three lights macOS draws over its start.
const LIGHTS: f32 = 78.0;

/// Whose rules a window's frame follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    /// The system keeps its lights and its frame, and the bar is the title bar under them.
    Mac,
    /// Wayland and X11, where Letters draws the frame and the window's buttons.
    Linux,
    /// Anywhere else the system frames and titles the window as it does any other.
    Other,
}

impl Host {
    /// The desktop this build runs on.
    pub const HERE: Self = if cfg!(target_os = "macos") {
        Self::Mac
    } else if cfg!(all(unix, not(target_os = "android"), not(target_os = "ios"))) {
        Self::Linux
    } else {
        Self::Other
    };

    /// What the desktop expects of a window's frame and bar. A window that is maximised or full screen
    /// has no edge of its own to show or to be resized by.
    pub fn chrome(self, buttons: &Buttons, maximized: bool, full_screen: bool) -> Chrome {
        match self {
            Self::Other => Chrome::default(),
            // A full screen window has no lights to leave room for.
            Self::Mac => {
                let inset_start = if full_screen { 0.0 } else { LIGHTS };
                Chrome { frame: Frame::Unified, inset_start, maximized, ..Chrome::default() }
            }
            Self::Linux => Chrome {
                buttons: buttons.order.iter().map(|button| button.shown()).collect(),
                buttons_at_start: buttons.at_start,
                frame: if maximized || full_screen { Frame::Filled } else { Frame::Floating },
                maximized,
                ..Chrome::default()
            },
        }
    }
}

/// A button a window's frame carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Minimise,
    Maximise,
    Close,
}

impl Button {
    fn shown(self) -> WindowButton {
        match self {
            Self::Minimise => WindowButton::Minimise,
            Self::Maximise => WindowButton::Maximise,
            Self::Close => WindowButton::Close,
        }
    }
}

/// The window buttons a desktop shows, in its order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Buttons {
    pub order: Vec<Button>,
    /// Whether they sit at the start of the bar, where macOS and some Linux desktops keep them.
    pub at_start: bool,
}

impl Buttons {
    /// Reads a `button-layout` value such as `appmenu:minimize,maximize,close`: what comes before the colon
    /// sits at the start of the title bar and what comes after at the end. Names that are not window
    /// buttons, such as `appmenu`, are passed over.
    ///
    /// A layout with buttons on both sides keeps the side that holds Close, since the bar has one group.
    pub fn parse(layout: &str) -> Self {
        let side = |names: &str| -> Vec<Button> {
            let button = |name: &str| match name.trim() {
                "minimize" => Some(Button::Minimise),
                "maximize" => Some(Button::Maximise),
                "close" => Some(Button::Close),
                _ => None,
            };
            names.split(',').filter_map(button).collect()
        };
        let (start, end) = layout.split_once(':').unwrap_or(("", layout));
        let (start, end) = (side(start), side(end));
        let at_start = match (start.contains(&Button::Close), end.contains(&Button::Close)) {
            (true, false) => true,
            (false, true) => false,
            _ => end.is_empty() && !start.is_empty(),
        };
        Self { order: if at_start { start } else { end }, at_start }
    }

    /// What a desktop shows when it cannot be asked: Close alone on GNOME, all three elsewhere.
    ///
    /// `desktop` is `XDG_CURRENT_DESKTOP`, a colon-separated list such as `ubuntu:GNOME`.
    pub fn default_for(desktop: &str) -> Self {
        let names: Vec<String> = desktop.split(':').map(str::to_ascii_lowercase).collect();
        let is = |name: &str| names.iter().any(|desktop| desktop == name);
        let order = if is("gnome") && !is("ubuntu") {
            vec![Button::Close]
        } else {
            vec![Button::Minimise, Button::Maximise, Button::Close]
        };
        Self { order, at_start: false }
    }

    /// The buttons to draw before the desktop has answered.
    pub fn guess() -> Self {
        Self::default_for(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default())
    }

    /// Asks the desktop for its layout through the settings portal. `None` when there is no portal, or it
    /// does not know the setting. This waits for the answer, so call it off the main thread.
    pub fn ask() -> Option<Self> {
        portal::button_layout().map(|layout| Self::parse(&layout))
    }
}

/// Whether the desktop asks for less motion. `None` when there is no portal, or it does not say. This
/// waits for the answer, so call it off the main thread.
pub fn reduced_motion() -> Option<bool> {
    // The setting every desktop is meant to share, then the one GNOME has always had.
    let asked = portal::number("org.freedesktop.appearance", "reduced-motion").map(|reduced| reduced == 1);
    asked.or_else(|| portal::flag("org.gnome.desktop.interface", "enable-animations").map(|animated| !animated))
}

#[cfg(all(unix, not(target_os = "macos")))]
mod portal {
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedValue, Value};

    pub fn button_layout() -> Option<String> {
        let value = read("org.gnome.desktop.wm.preferences", "button-layout")?;
        match plain(&value) {
            Value::Str(text) => Some(text.to_string()),
            _ => None,
        }
    }

    pub fn flag(namespace: &str, key: &str) -> Option<bool> {
        let value = read(namespace, key)?;
        match plain(&value) {
            Value::Bool(flag) => Some(*flag),
            _ => None,
        }
    }

    pub fn number(namespace: &str, key: &str) -> Option<u32> {
        let value = read(namespace, key)?;
        match plain(&value) {
            Value::U32(number) => Some(*number),
            _ => None,
        }
    }

    fn read(namespace: &str, key: &str) -> Option<OwnedValue> {
        let connection = Connection::session().ok()?;
        let settings = Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .ok()?;
        // `ReadOne` is the current call. Older portals only have `Read`, which wraps its answer once more.
        settings.call("ReadOne", &(namespace, key)).or_else(|_| settings.call("Read", &(namespace, key))).ok()
    }

    /// The value inside, however many variants deep the portal wrapped it.
    fn plain<'a>(value: &'a Value<'a>) -> &'a Value<'a> {
        match value {
            Value::Value(inner) => plain(inner),
            plain => plain,
        }
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
mod portal {
    pub fn button_layout() -> Option<String> {
        None
    }

    pub fn flag(_namespace: &str, _key: &str) -> Option<bool> {
        None
    }

    pub fn number(_namespace: &str, _key: &str) -> Option<u32> {
        None
    }
}
