//! What the desktop expects of a window's frame, now that Letters draws it.
//!
//! On macOS the system keeps its traffic lights, so there is nothing to ask. On Linux the window buttons are
//! the bar's own, and they follow the desktop's `button-layout` setting: which buttons, in what order, on
//! which side.

/// A button a window's frame carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Minimise,
    Maximise,
    Close,
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

#[cfg(all(unix, not(target_os = "macos")))]
mod portal {
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedValue, Value};

    const NAMESPACE: &str = "org.gnome.desktop.wm.preferences";
    const KEY: &str = "button-layout";

    pub fn button_layout() -> Option<String> {
        let connection = Connection::session().ok()?;
        let settings = Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .ok()?;
        // `ReadOne` is the current call. Older portals only have `Read`, which wraps its answer once more.
        let value: OwnedValue =
            settings.call("ReadOne", &(NAMESPACE, KEY)).or_else(|_| settings.call("Read", &(NAMESPACE, KEY))).ok()?;
        text(&value)
    }

    /// The string inside a value, however many variants deep the portal wrapped it.
    fn text(value: &Value<'_>) -> Option<String> {
        match value {
            Value::Str(text) => Some(text.to_string()),
            Value::Value(inner) => text(inner),
            _ => None,
        }
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
mod portal {
    pub fn button_layout() -> Option<String> {
        None
    }
}
