//! The menu bar macOS keeps at the top of the screen: what its menus list, and how an item is done.
//!
//! The menus hold nothing of their own. An item names a command of the window by the name the palette
//! lists it under, or a key the app answers. Where a command has a key held with Cmd the item is done by
//! pressing that key: it then does in a field what the key does there. Every other command is asked of
//! the window, and its item is greyed while the window has nothing for it to act on.

use letters_editor::{Key, KeyEvent, Modifiers};
use letters_ui::Window;

/// One line of a menu.
#[derive(Debug)]
pub enum Item {
    /// A command of the window: what the menu calls it, and the name the palette lists it under.
    Command(&'static str, &'static str),
    /// Something done by a key alone, which no command of the palette's does: opening and closing
    /// windows, and the palette itself.
    Key(&'static str, KeyEvent),
    /// An item the system does itself.
    System(System),
    /// A menu inside the menu.
    Menu(&'static str, &'static [Item]),
    /// A line between two groups of items.
    Rule,
}

/// What macOS does without asking the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    /// Ends the app as the Dock's Quit does. Every document is saved on the way out.
    Quit,
    FullScreen,
    Minimize,
    Zoom,
    BringAllToFront,
}

/// One menu of the bar.
#[derive(Debug)]
pub struct Menu {
    pub title: &'static str,
    pub items: &'static [Item],
}

const fn held(key: char, shift: bool) -> KeyEvent {
    KeyEvent { key: Key::Char(key), mods: Modifiers { primary: true, shift, alt: false } }
}

use Item::{Command, Key as Pressed, Rule};

/// What Format lists under Table. Each is offered while the caret is in a table.
static TABLE: [Item; 17] = [
    Command("Add Row Above", "Add row above"),
    Command("Add Row Below", "Add row below"),
    Command("Add Column to the Left", "Add column to the left"),
    Command("Add Column to the Right", "Add column to the right"),
    Rule,
    Command("Remove Row", "Remove row"),
    Command("Remove Column", "Remove column"),
    Rule,
    Command("Move Row Up", "Move row up"),
    Command("Move Row Down", "Move row down"),
    Command("Move Column Left", "Move column left"),
    Command("Move Column Right", "Move column right"),
    Rule,
    Command("Align Column Left", "Align column left"),
    Command("Align Column Centre", "Align column centre"),
    Command("Align Column Right", "Align column right"),
    Rule,
];

/// The header row is turned on or off, and only the way it would turn is offered.
static HEADER: [Item; 2] =
    [Command("Turn Header Row On", "Turn header row on"), Command("Turn Header Row Off", "Turn header row off")];

/// The bar, from the left. The first menu is the one macOS names after the app.
pub static BAR: [Menu; 8] = [
    Menu {
        title: "Letters",
        items: &[
            Item::System(System::About),
            Rule,
            Command("Settings\u{2026}", "Settings"),
            Command("Servers\u{2026}", "Servers"),
            Rule,
            Item::System(System::Services),
            Rule,
            Item::System(System::Hide),
            Item::System(System::HideOthers),
            Item::System(System::ShowAll),
            Rule,
            Item::System(System::Quit),
        ],
    },
    Menu {
        title: "File",
        items: &[
            Pressed("New Document", held('n', false)),
            Pressed("Open\u{2026}", held('o', false)),
            Rule,
            Pressed("Close Window", held('w', false)),
            Pressed("Save", held('s', false)),
            Command("Save a Copy\u{2026}", "Save a copy"),
            Command("Rename or Move\u{2026}", "Rename or move"),
            Rule,
            Command("Export\u{2026}", "Export"),
            Command("Export Clean Copy\u{2026}", "Export clean copy"),
            Command("Share\u{2026}", "Share"),
            Rule,
            Command("Merge Conflicts", "Merge conflicts"),
            Command("Merge from File\u{2026}", "Merge from file"),
            Command("Merge into the Original", "Merge into the original"),
            Rule,
            Command("Trust This Document", "Trust this document"),
            Command("Trust This Folder", "Trust this folder"),
            Rule,
            Command("Page Setup\u{2026}", "Page setup"),
            Command("Print\u{2026}", "Print view"),
        ],
    },
    Menu {
        title: "Edit",
        items: &[
            Command("Undo", "Undo"),
            Command("Redo", "Redo"),
            Rule,
            Command("Cut", "Cut"),
            Command("Copy", "Copy"),
            Command("Paste", "Paste"),
            Command("Paste as Plain Text", "Paste as plain text"),
            Command("Select All", "Select all"),
            Rule,
            Command("Find\u{2026}", "Find"),
            Command("Find and Replace\u{2026}", "Find and replace"),
            Command("Find Next", "Find next"),
            Command("Find Previous", "Find previous"),
            Rule,
            Command("Next Spelling Mistake", "Next spelling mistake"),
            Command("Previous Spelling Mistake", "Previous spelling mistake"),
            Rule,
            Pressed("Commands\u{2026}", held('p', true)),
        ],
    },
    Menu {
        title: "View",
        items: &[
            Command("Styles Panel", "Styles panel"),
            Command("Notes Panel", "Notes panel"),
            Rule,
            Command("History", "History"),
            Command("History of Selection", "History of selection"),
            Command("Changes Since Last Visit", "Changes since last visit"),
            Rule,
            Command("Next Note", "Next note"),
            Command("Previous Note", "Previous note"),
            Command("Next Conflict", "Next conflict"),
            Command("Previous Conflict", "Previous conflict"),
            Rule,
            Item::System(System::FullScreen),
        ],
    },
    Menu {
        title: "Insert",
        items: &[
            Command("Table", "Table"),
            Command("Image\u{2026}", "Image"),
            Command("Callout", "Callout"),
            Command("Formula", "Formula"),
            Command("Divider", "Divider"),
            Command("Table of Contents", "Table of contents"),
            Command("Link Preview\u{2026}", "Link preview"),
            Rule,
            Command("Link\u{2026}", "Link"),
            Command("Comment", "Comment"),
        ],
    },
    Menu {
        title: "Format",
        items: &[
            Command("Bold", "Bold"),
            Command("Italic", "Italic"),
            Command("Underline", "Underline"),
            Command("Strikethrough", "Strikethrough"),
            Command("Highlight", "Highlight"),
            Command("Inline Code", "Inline code"),
            Rule,
            Command("Body Text", "Body text"),
            Command("Heading 1", "Heading 1"),
            Command("Heading 2", "Heading 2"),
            Command("Heading 3", "Heading 3"),
            Command("Heading 4", "Heading 4"),
            Rule,
            Command("Bulleted List", "Bulleted list"),
            Command("Numbered List", "Numbered list"),
            Command("Task List", "Task list"),
            Command("Quote", "Quote"),
            Command("Code Block", "Code block"),
            Rule,
            Command("Indent", "Indent"),
            Command("Outdent", "Outdent"),
            Command("Tick or Untick Task", "Tick or untick task"),
            Rule,
            Item::Menu("Table", &TABLE),
            Item::Menu("Header Row", &HEADER),
            Rule,
            Command("Language\u{2026}", "Language"),
        ],
    },
    Menu {
        title: WINDOW,
        items: &[
            Item::System(System::Minimize),
            Item::System(System::Zoom),
            Rule,
            Item::System(System::BringAllToFront),
        ],
    },
    // macOS puts the field that searches the menus here.
    Menu { title: HELP, items: &[] },
];

/// The menu macOS lists the open windows in.
const WINDOW: &str = "Window";
/// The menu macOS searches the others from.
const HELP: &str = "Help";

/// Every item the app does itself, in the order of the bar: a taken item is told by its place here.
pub fn items() -> Vec<&'static Item> {
    fn walk(items: &'static [Item], all: &mut Vec<&'static Item>) {
        for item in items {
            match item {
                Item::Command(..) | Item::Key(..) => all.push(item),
                Item::Menu(_, items) => walk(items, all),
                Item::System(_) | Item::Rule => {}
            }
        }
    }
    let mut all = Vec::new();
    for menu in &BAR {
        walk(menu.items, &mut all);
    }
    all
}

/// The key an item is done by, and that the menu writes beside it. Only a key held with Cmd is one: any
/// other is typing, which a menu must not take from the field it is typed into.
pub fn key(item: &Item, window: &Window) -> Option<KeyEvent> {
    let key = match item {
        Item::Key(_, key) => Some(key.clone()),
        Item::Command(_, name) => window.command_key(name),
        Item::System(_) | Item::Menu(..) | Item::Rule => None,
    };
    key.filter(|key| key.mods.primary)
}

#[cfg(target_os = "macos")]
pub use bar::Bar;

/// Only macOS has a bar of menus outside the window. Elsewhere there is none to put up.
#[cfg(not(target_os = "macos"))]
pub struct Bar;

#[cfg(not(target_os = "macos"))]
impl Bar {
    /// Nothing is put up, and `None` says so.
    pub fn install(_window: &Window, _told: impl Fn(usize) + Send + Sync + 'static) -> Option<Self> {
        None
    }

    pub fn offer(&mut self, _offers: impl Fn(&str) -> bool) {}
}

/// The bar as macOS shows it.
#[cfg(target_os = "macos")]
mod bar {
    use letters_editor::{Key, KeyEvent};
    use letters_ui::Window;
    use muda::accelerator::{Key as Typed, KeyAccelerator, Modifiers, NamedKey};
    use muda::{AboutMetadata, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

    use super::{BAR, HELP, Item, System, WINDOW};

    /// What comes before an item's place in [`super::items`] in the name macOS knows it by. The system's
    /// own items are numbered too, and none of them is the app's to do.
    const OURS: &str = "letters:";

    /// The menus, for as long as the app runs.
    pub struct Bar {
        _menu: muda::Menu,
        /// The items that are greyed while their command has nothing to act on: each with the name of
        /// its command, and whether it is offered as the bar stands.
        asked: Vec<(MenuItem, &'static str, bool)>,
    }

    impl Bar {
        /// Puts the menus in the bar, in place of those winit would. `window` says which keys its
        /// commands have. `told` hears the place in [`super::items`] of an item that was taken, on the
        /// main thread. `None` if macOS took a menu amiss: the bar is then left as it was.
        pub fn install(window: &Window, told: impl Fn(usize) + Send + Sync + 'static) -> Option<Self> {
            let (menu, mut asked, mut next) = (muda::Menu::new(), Vec::new(), 0);
            let mut roles = Vec::new();
            for listed in &BAR {
                let made = Submenu::new(listed.title, true);
                fill(&made, listed.items, window, &mut next, &mut asked).ok()?;
                menu.append(&made).ok()?;
                roles.push((listed.title, made));
            }
            menu.init_for_nsapp();
            for (title, made) in &roles {
                match *title {
                    WINDOW => made.set_as_windows_menu_for_nsapp(),
                    HELP => made.set_as_help_menu_for_nsapp(),
                    _ => {}
                }
            }
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some(place) = event.id.0.strip_prefix(OURS).and_then(|place| place.parse().ok()) {
                    told(place);
                }
            }));
            Some(Self { _menu: menu, asked })
        }

        /// Greys the items whose commands have nothing to act on now, and offers the rest.
        pub fn offer(&mut self, offers: impl Fn(&str) -> bool) {
            for (item, name, offered) in &mut self.asked {
                let now = offers(name);
                if std::mem::replace(offered, now) != now {
                    item.set_enabled(now);
                }
            }
        }
    }

    /// Lists `items` in a menu, in the order [`super::items`] counts them in.
    fn fill(
        menu: &Submenu,
        items: &'static [Item],
        window: &Window,
        next: &mut usize,
        asked: &mut Vec<(MenuItem, &'static str, bool)>,
    ) -> muda::Result<()> {
        for item in items {
            match item {
                Item::Rule => menu.append(&PredefinedMenuItem::separator())?,
                Item::System(system) => menu.append(&system_item(*system))?,
                Item::Menu(title, items) => {
                    let inner = Submenu::new(title, true);
                    fill(&inner, items, window, next, asked)?;
                    menu.append(&inner)?;
                }
                Item::Command(label, _) | Item::Key(label, _) => {
                    let key = super::key(item, window);
                    let made = MenuItem::with_id(MenuId::new(format!("{OURS}{next}")), label, true, None);
                    made.set_key_accelerator(key.as_ref().and_then(accelerator))?;
                    menu.append(&made)?;
                    // An item done by its key stays offered: what the key does depends on where the
                    // keyboard is, which the window knows only once the key is pressed.
                    if let (Item::Command(_, name), None) = (item, key) {
                        asked.push((made, name, true));
                    }
                    *next += 1;
                }
            }
        }
        Ok(())
    }

    fn system_item(system: System) -> PredefinedMenuItem {
        match system {
            System::About => {
                let version = Some(env!("CARGO_PKG_VERSION").to_owned());
                let about = AboutMetadata { name: Some("Letters".into()), version, ..AboutMetadata::default() };
                PredefinedMenuItem::about(None, Some(about))
            }
            System::Services => PredefinedMenuItem::services(None),
            System::Hide => PredefinedMenuItem::hide(None),
            System::HideOthers => PredefinedMenuItem::hide_others(None),
            System::ShowAll => PredefinedMenuItem::show_all(None),
            System::Quit => PredefinedMenuItem::quit(None),
            System::FullScreen => PredefinedMenuItem::fullscreen(None),
            System::Minimize => PredefinedMenuItem::minimize(None),
            System::Zoom => PredefinedMenuItem::zoom(None),
            System::BringAllToFront => PredefinedMenuItem::bring_all_to_front(None),
        }
    }

    /// A key as macOS matches it against what is pressed. `None` for a key no menu of Letters writes.
    fn accelerator(key: &KeyEvent) -> Option<KeyAccelerator> {
        let typed = match key.key {
            Key::Char(c) => Typed::Character(c.to_string()),
            Key::Enter => Typed::Named(NamedKey::Enter),
            Key::Backspace => Typed::Named(NamedKey::Backspace),
            Key::Delete => Typed::Named(NamedKey::Delete),
            Key::Left => Typed::Named(NamedKey::ArrowLeft),
            Key::Right => Typed::Named(NamedKey::ArrowRight),
            Key::Up => Typed::Named(NamedKey::ArrowUp),
            Key::Down => Typed::Named(NamedKey::ArrowDown),
            Key::PageUp => Typed::Named(NamedKey::PageUp),
            Key::PageDown => Typed::Named(NamedKey::PageDown),
            Key::Tab | Key::Escape | Key::Home | Key::End | Key::F(_) => return None,
        };
        let held =
            [(key.mods.primary, Modifiers::META), (key.mods.shift, Modifiers::SHIFT), (key.mods.alt, Modifiers::ALT)];
        let mods = held.into_iter().filter(|(held, _)| *held).fold(Modifiers::empty(), |mods, (_, held)| mods | held);
        Some(KeyAccelerator::new(mods, typed))
    }
}
