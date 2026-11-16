//! What the server's pages show, as yewi components rendered on the server. A browser is sent HTML, the
//! stylesheet and one small script of yewi's: no wasm, so every action is a link or a form.

use std::rc::Rc;

use letters_api::{self as api, EntryKind, Person, SpaceKind, paths};
use yew::prelude::*;
use yewi::prelude::*;

use super::{ACT_IMPORT, ACT_MERGE, ACT_MOVE, ACT_NEW_DOCUMENT, ACT_NEW_FOLDER, ACT_PUT_BACK, ACT_RENAME, ACT_TRASH};
use super::{ACT_REVOKE, ACT_SHARE, ACT_UNSHARE, RECENT_PAGE, SHARED_PAGE, TRASH_PAGE, asking, asking_folder};

/// Everything one page shows. It is made where the request is answered and drawn on another thread, so it
/// holds plain text.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// The server's name.
    pub server: String,
    /// Where the stylesheets and the script are.
    pub statics: String,
    /// Who is signed in, for a page with the frame around it.
    pub who: Option<Person>,
    pub spaces: Vec<api::Space>,
    /// What the sidebar marks as where the person is.
    pub at: At,
    /// What the last thing asked for on this page could not do.
    pub refused: Option<String>,
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum At {
    Space(String),
    Shared,
    Recent,
    Trash,
    Nowhere,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Folder(Box<Folder>),
    Recent(Vec<Row>),
    Shared(Vec<Row>),
    Trash(Vec<Trashed>),
    /// The person is signed in, and none of the server's spaces is open to them.
    NoSpaces,
    /// One thing said in the middle of the page. With nobody signed in, the page has no frame.
    Card(Card),
}

/// A row of a file table.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub entry: api::Entry,
    /// When it was made and last written, as the table says them.
    pub created: String,
    pub modified: String,
    /// Where it is from, in a table that mixes places: the name of its space, or whose it is.
    pub space: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    pub space: api::Space,
    /// The folder's place in its space. Empty for the space itself.
    pub path: String,
    pub rows: Vec<Row>,
    /// Whether the person says who the documents here are shared with.
    pub shares: bool,
    /// The question the page opens with, when a row's menu asked for one.
    pub ask: Option<Ask>,
}

/// A question about one document or one folder, asked in a dialog over the folder it is in.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    Rename(api::Entry),
    /// Where to, among the folders the person may put it in.
    Move(api::Entry, Vec<Target>),
    Trash(api::Entry),
    /// Who it is shared with, who it can be shared with besides, the links that open it, and what was
    /// done through all of them.
    Share(api::Entry, Vec<api::Grant>, Vec<api::Grantee>, Vec<Linked>, Activity),
}

/// A document's access log, as the Share dialog's Activity tab lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    /// Its rows by day, newest first, each day under what it is called.
    pub days: Vec<(String, Vec<Reached>)>,
    /// How many days the server keeps a row.
    pub kept: u32,
    /// The time zone the days and the times are told in, as a reader in another one must know it.
    pub zone: String,
    /// The address that lists the rows from before these, where there may be some.
    pub earlier: Option<String>,
    /// These are not the newest: the dialog opens on them, and offers the way back.
    pub older: bool,
}

/// One row of the access log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reached {
    /// Who did what, as a sentence without its full stop.
    pub said: String,
    pub action: api::Action,
    pub role: api::Role,
    pub refused: bool,
    /// The label of the link they came by.
    pub link: Option<String>,
    /// The address and the browser it came from.
    pub from: String,
    /// The time of day.
    pub at: String,
}

/// A link that still opens a document, as the Share dialog lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Linked {
    pub id: String,
    pub label: String,
    pub role: api::Role,
    /// Whether it asks a passphrase.
    pub passphrase: bool,
    /// The day it stops opening the document, as a person reads it.
    pub ends: Option<String>,
}

/// A folder a document can be moved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// What the form sends: the space, a line break, the folder's path.
    pub value: String,
    /// The space and the folders, as a person reads them.
    pub label: String,
}

/// A document or a folder in the trash of a space.
#[derive(Clone, Debug, PartialEq)]
pub struct Trashed {
    pub space: api::Space,
    pub folder: bool,
    /// The file's name in the trash.
    pub file: String,
    /// The name it had, and the space and folder it was in, as a person reads them.
    pub name: String,
    pub from: String,
    /// When it was thrown away, as the table says it.
    pub thrown: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Card {
    /// Nobody is signed in.
    SignIn { then: String },
    /// Something did not work. `again` is where to try once more, and what the link says.
    Failed { title: String, message: String, again: Option<(String, String)> },
    /// The desktop app asks to be signed in as the person.
    HandOver { name: String, challenge: String, port: String, state: String },
    /// A share link asks its passphrase, at the address the form goes to. `wrong` after one that was not
    /// it.
    Passphrase { action: String, wrong: bool },
}

#[derive(Properties, PartialEq)]
pub struct RootProps {
    pub page: Rc<Page>,
}

#[function_component]
pub fn Root(props: &RootProps) -> Html {
    let page = &*props.page;
    let script = format!("{}/yewi.js", page.statics);
    let inner = match (&page.who, &page.body) {
        (Some(who), body) => framed(page, who, body),
        (None, Body::Card(card)) => html! { <main class="lt-card">{ said(page, card) }</main> },
        (None, _) => Html::default(),
    };
    html! { <ServerOnly {script}>{ inner }</ServerOnly> }
}

fn brand(page: &Page) -> Html {
    html! {
        <AppBarBrand href={paths::HOME_PAGE} logo={html! { <Icon icon={icons::PEN_LINE} /> }}>
            { page.server.clone() }
        </AppBarBrand>
    }
}

fn framed(page: &Page, who: &Person, body: &Body) -> Html {
    let account = html! {
        <UserMenu name={who.name.clone()} detail={who.username.clone()} show_name=true>
            <MenuItem icon={icons::LOG_OUT} form="sign-out">{ "Sign out" }</MenuItem>
        </UserMenu>
    };
    let bar = html! { <AppBar brand={brand(page)} actions={account} /> };
    let space = |space: &api::Space| {
        let icon = if space.kind == SpaceKind::Personal { icons::HOUSE } else { icons::USERS };
        let current = page.at == At::Space(space.id.clone());
        html! {
            <SidebarItem href={paths::folder_page(&space.id, "")} {icon} {current}>{ space.name.clone() }</SidebarItem>
        }
    };
    let sidebar = html! {
        <Sidebar label="Places">
            <SidebarGroup label="Spaces">{ for page.spaces.iter().map(space) }</SidebarGroup>
            <SidebarGroup label="Library">
                <SidebarItem href={SHARED_PAGE} icon={icons::SHARE_2} current={page.at == At::Shared}>
                    { "Shared with me" }
                </SidebarItem>
                <SidebarItem href={RECENT_PAGE} icon={icons::CLOCK} current={page.at == At::Recent}>
                    { "Recent" }
                </SidebarItem>
                <SidebarItem href={TRASH_PAGE} icon={icons::TRASH_2} current={page.at == At::Trash}>
                    { "Trash" }
                </SidebarItem>
            </SidebarGroup>
        </Sidebar>
    };
    let content = match body {
        Body::Folder(folder) => self::folder(folder),
        Body::Recent(rows) => recent(rows),
        Body::Shared(rows) => shared(rows),
        Body::Trash(trashed) => trash(trashed),
        Body::NoSpaces => html! {
            <EmptyState title="No space is open to you" icon={icons::FOLDER_OPEN}>
                { "You are signed in, but none of this server's spaces lets you in. Ask whoever runs it." }
            </EmptyState>
        },
        Body::Card(card) => said(page, card),
    };
    html! {
        <>
            <AppShell {bar} {sidebar} storage_key="letters-sidebar">
                <Stack gap={Space::Lg}>
                    if let Some(refused) = &page.refused {
                        <Banner tone={Tone::Warn}>{ refused.clone() }</Banner>
                    }
                    { content }
                </Stack>
            </AppShell>
            <form id="sign-out" method="post" action={paths::SIGN_OUT_PAGE}></form>
        </>
    }
}

fn hidden(name: &'static str, value: &str) -> Html {
    html! { <input type="hidden" {name} value={value.to_owned()} /> }
}

/// The headings every file table has. `places` heads where each row is from, for a table that mixes
/// places.
fn headings(places: Option<&'static str>) -> Html {
    html! {
        <thead>
            <tr>
                <th scope="col">{ "Name" }</th>
                if let Some(places) = places {
                    <th scope="col">{ places }</th>
                }
                <th scope="col" class="lt-wide">{ "Contributors" }</th>
                <th scope="col" class="lt-wide">{ "Created" }</th>
                <th scope="col">{ "Modified" }</th>
                <th scope="col" class="lt-file__more"><VisuallyHidden>{ "More" }</VisuallyHidden></th>
            </tr>
        </thead>
    }
}

/// What a file is marked with beside its name. Neutral, but for what waits to be done.
fn tag(tag: &api::Tag) -> Html {
    let count = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    match tag {
        api::Tag::ConflictCopy { .. } => html! { <Tag icon={icons::GIT_MERGE}>{ "Conflicted copy" }</Tag> },
        api::Tag::ImportNotes { count: notes } => html! {
            <Tag icon={icons::FILE_INPUT} tone={Tone::Warn}>{ count(*notes as usize, "import note", "import notes") }</Tag>
        },
        api::Tag::Editing { people } => html! {
            <Tag icon={icons::PENCIL}>{ format!("{} editing now", people.len()) }</Tag>
        },
        api::Tag::Shared { with } => html! {
            <Tag icon={icons::EYE}>{ format!("Shared with {with}") }</Tag>
        },
        api::Tag::Links { count: links } => html! {
            <Tag icon={icons::LINK}>{ count(*links as usize, "link", "links") }</Tag>
        },
        api::Tag::Damaged => html! { <Tag icon={icons::TRIANGLE_ALERT} tone={Tone::Danger}>{ "Cannot be read" }</Tag> },
        api::Tag::Unknown => Html::default(),
    }
}

/// One row. `here` is the page the table is on, which a question about the row is asked over, and `None`
/// where a table asks none. `shares` is whether the person says who the row is shared with.
fn file(row: &Row, here: Option<&str>, places: bool, shares: bool) -> Html {
    let entry = &row.entry;
    let space = entry.space.as_deref().unwrap_or_default();
    // A conflict copy has the id of its original, so whatever is asked by id is asked of the original.
    // The copy itself is only merged.
    let copy = entry.tags.iter().any(|tag| matches!(tag, api::Tag::ConflictCopy { .. }));
    let (icon, href) = match (&entry.kind, &entry.id) {
        (EntryKind::Folder, _) => (icons::FOLDER, Some(paths::folder_page(space, &entry.path))),
        (EntryKind::Document, Some(id)) if !copy => (icons::FILE_TEXT, Some(paths::editor_page(id))),
        (EntryKind::Document, _) => (icons::FILE_TEXT, None),
    };
    let merge = entry.id.as_ref().zip(here).filter(|_| copy && entry.role.edits());
    // The button says what the tag would.
    let tags = entry.tags.iter().filter(|tag| merge.is_none() || !matches!(tag, api::Tag::ConflictCopy { .. }));
    let merge = merge.map(|(id, here)| {
        html! {
            <form method="post" action={ACT_MERGE}>
                { hidden("id", id) }
                { hidden("back", here) }
                <Button size={Size::Small} variant={ButtonVariant::Primary} icon={icons::GIT_MERGE} kind={ButtonKind::Submit}>
                    { "Merge conflicts" }
                </Button>
            </form>
        }
    });
    let name = match href {
        Some(href) => html! { <a class="lt-file__name" {href}>{ entry.name.clone() }</a> },
        None => html! { <span class="lt-file__name">{ entry.name.clone() }</span> },
    };
    let contributors = if entry.contributors.is_empty() {
        Html::default()
    } else {
        html! {
            <AvatarGroup label="Contributors" size={Size::Small} max={4}>
                { for entry.contributors.iter().map(|name| html_nested! { <Avatar name={name.clone()} /> }) }
            </AvatarGroup>
        }
    };
    let more = match &entry.id {
        Some(id) if !copy => {
            let ask = |what: &str| here.filter(|_| entry.role.edits()).map(|here| asking(here, what, id));
            let share = here.filter(|_| shares).map(|here| asking(here, "share", id));
            html! {
                <MenuButton label={format!("More for {}", entry.name)} icon_only=true variant={ButtonVariant::Subtle} size={Size::Small} placement={Placement::BottomEnd}>
                    <MenuItem icon={icons::BOOK_OPEN} href={paths::read_page(id)}>{ "Read" }</MenuItem>
                    <MenuItem icon={icons::DOWNLOAD} href={paths::file(id)}>{ "Download" }</MenuItem>
                    if let Some(share) = share {
                        <MenuItem icon={icons::SHARE_2} href={share}>{ "Share" }</MenuItem>
                    }
                    if let (Some(rename), Some(relocate), Some(bin)) = (ask("rename"), ask("move"), ask("trash")) {
                        <MenuSeparator />
                        <MenuItem icon={icons::PENCIL} href={rename}>{ "Rename" }</MenuItem>
                        <MenuItem icon={icons::FOLDER_INPUT} href={relocate}>{ "Move" }</MenuItem>
                        <MenuItem icon={icons::TRASH_2} tone={Tone::Danger} href={bin}>{ "Move to trash" }</MenuItem>
                    }
                </MenuButton>
            }
        }
        // A folder is asked about by where it is. Whoever edits its space renames and moves it, and only
        // an owner of the space throws it away.
        None if entry.kind == EntryKind::Folder => match here.filter(|_| entry.role.edits()) {
            Some(here) => {
                let ask = |what: &str| asking_folder(here, what, &entry.path);
                html! {
                    <MenuButton label={format!("More for {}", entry.name)} icon_only=true variant={ButtonVariant::Subtle} size={Size::Small} placement={Placement::BottomEnd}>
                        <MenuItem icon={icons::PENCIL} href={ask("rename")}>{ "Rename" }</MenuItem>
                        <MenuItem icon={icons::FOLDER_INPUT} href={ask("move")}>{ "Move" }</MenuItem>
                        if entry.role == api::Role::Owner {
                            <MenuItem icon={icons::TRASH_2} tone={Tone::Danger} href={ask("trash")}>{ "Move to trash" }</MenuItem>
                        }
                    </MenuButton>
                }
            }
            None => Html::default(),
        },
        _ => Html::default(),
    };
    html! {
        <tr class="yi-table__row">
            <th scope="row">
                <span class="lt-file"><Icon {icon} />{ name }{ for tags.map(tag) }{ for merge }</span>
            </th>
            if places {
                <td class="lt-dim">{ row.space.clone().unwrap_or_default() }</td>
            }
            <td class="lt-wide">{ contributors }</td>
            <td class="lt-dim lt-wide">{ row.created.clone() }</td>
            <td class="lt-dim">{ row.modified.clone() }</td>
            <td class="lt-file__more">{ more }</td>
        </tr>
    }
}

fn folder(folder: &Folder) -> Html {
    let (space, edits) = (&folder.space, folder.space.role.edits());
    let here = paths::folder_page(&space.id, &folder.path);
    let mut names: Vec<&str> = folder.path.split('/').filter(|name| !name.is_empty()).collect();
    let title = names.pop().unwrap_or(&space.name).to_owned();
    // The space, then each folder on the way down.
    let above = (0..names.len()).map(|depth| (names[depth], paths::folder_page(&space.id, &names[..=depth].join("/"))));
    let crumbs = if folder.path.is_empty() {
        Html::default()
    } else {
        html! {
            <Breadcrumbs label="Folder">
                <Crumb href={paths::folder_page(&space.id, "")}>{ space.name.clone() }</Crumb>
                { for above.map(|(name, href)| html! { <Crumb {href}>{ name.to_owned() }</Crumb> }) }
                <Crumb current=true>{ title.clone() }</Crumb>
            </Breadcrumbs>
        }
    };
    let actions = if edits {
        html! {
            <>
                <Button icon={icons::FILE_INPUT} command={Command::show_modal("import")}>{ "Import" }</Button>
                <Button icon={icons::FOLDER_PLUS} command={Command::show_modal("new-folder")}>{ "New folder" }</Button>
                <Button variant={ButtonVariant::Primary} icon={icons::PLUS} kind={ButtonKind::Submit} attrs={attrs! { "form" => "new-document" }}>
                    { "New document" }
                </Button>
            </>
        }
    } else {
        Html::default()
    };
    let table = if folder.rows.is_empty() {
        html! {
            <EmptyState title="Nothing here yet" icon={icons::FOLDER_OPEN}>
                { if edits { "Start a document, or put one in this folder." } else { "Nothing has been put in this folder." } }
            </EmptyState>
        }
    } else {
        html! {
            <SimpleTable label="Documents" variant={TableVariant::Ruled}>
                { headings(None) }
                <tbody>{ for folder.rows.iter().map(|row| file(row, Some(&here), false, folder.shares)) }</tbody>
            </SimpleTable>
        }
    };
    html! {
        <Stack gap={Space::Lg}>
            <PageHeader title={title} breadcrumbs={crumbs} {actions} />
            { table }
            if edits {
                <form id="new-document" method="post" action={ACT_NEW_DOCUMENT}>
                    { hidden("space", &space.id) }
                    { hidden("folder", &folder.path) }
                </form>
                <Dialog id="import" title="Import a file" size={Size::Small} footer={html! {
                    <>
                        <Button command={Command::close("import")}>{ "Cancel" }</Button>
                        <Button variant={ButtonVariant::Primary} kind={ButtonKind::Submit} attrs={attrs! { "form" => "import-form" }}>
                            { "Import" }
                        </Button>
                    </>
                }}>
                    <form id="import-form" method="post" action={ACT_IMPORT} enctype="multipart/form-data">
                        { hidden("space", &space.id) }
                        { hidden("folder", &folder.path) }
                        <FileDrop
                            name="file"
                            accept=".md,.markdown,.docx"
                            required=true
                            label={html! { "Drop a Markdown or Word file here, or choose one" }}
                            hint={html! { "It becomes a document in this folder, and opens." }}
                        />
                    </form>
                </Dialog>
                <Dialog id="new-folder" title="New folder" size={Size::Small} footer={html! {
                    <>
                        <Button command={Command::close("new-folder")}>{ "Cancel" }</Button>
                        <Button variant={ButtonVariant::Primary} kind={ButtonKind::Submit} attrs={attrs! { "form" => "new-folder-form" }}>
                            { "Make folder" }
                        </Button>
                    </>
                }}>
                    <form id="new-folder-form" method="post" action={ACT_NEW_FOLDER}>
                        { hidden("space", &space.id) }
                        { hidden("folder", &folder.path) }
                        <Field label="Name">
                            <TextInput name="name" required=true max_length={200} autocomplete="off" attrs={attrs! { "autofocus" => "" }} />
                        </Field>
                    </form>
                </Dialog>
            }
            if let Some(ask) = &folder.ask {
                { asked(ask, &here) }
            }
        </Stack>
    }
}

/// The dialog a row's menu asked for, open over the folder at `here`.
fn asked(ask: &Ask, here: &str) -> Html {
    let (entry, title, verb, action) = match ask {
        Ask::Rename(entry) => (entry, format!("Rename {}", entry.name), "Rename", ACT_RENAME),
        Ask::Move(entry, _) => (entry, format!("Move {}", entry.name), "Move", ACT_MOVE),
        Ask::Trash(entry) => (entry, format!("Move {} to the trash?", entry.name), "Move to trash", ACT_TRASH),
        Ask::Share(entry, people, offered, links, log) => return sharing(entry, people, offered, links, log, here),
    };
    let fields = match ask {
        Ask::Rename(entry) => html! {
            <Field label="Name">
                <TextInput name="name" value={entry.name.clone()} required=true max_length={200} autocomplete="off" attrs={attrs! { "autofocus" => "" }} />
            </Field>
        },
        Ask::Move(_, targets) => {
            let options: Rc<[(AttrValue, AttrValue)]> =
                targets.iter().map(|target| (target.value.clone().into(), target.label.clone().into())).collect();
            html! {
                <Field label="To">
                    <NativeSelect name="to" {options} placeholder="Choose a folder" required=true attrs={attrs! { "autofocus" => "" }} />
                </Field>
            }
        }
        Ask::Trash(entry) if entry.kind == EntryKind::Folder => html! {
            <p>{ "Everything in it goes with it, and all of it can be put back from the trash." }</p>
        },
        Ask::Trash(_) => html! { <p>{ "It can be put back from the trash." }</p> },
        Ask::Share(..) => Html::default(),
    };
    let variant = if matches!(ask, Ask::Trash(_)) { ButtonVariant::Danger } else { ButtonVariant::Primary };
    let footer = html! {
        <>
            <Button href={here.to_owned()}>{ "Cancel" }</Button>
            <Button {variant} kind={ButtonKind::Submit} attrs={attrs! { "form" => "asked-form" }}>{ verb }</Button>
        </>
    };
    html! {
        <Dialog id="asked" open=true {title} size={Size::Small} {footer}>
            <form id="asked-form" method="post" {action}>
                if entry.kind == EntryKind::Folder {
                    { hidden("space", entry.space.as_deref().unwrap_or_default()) }
                    { hidden("folder", &entry.path) }
                } else {
                    { hidden("id", entry.id.as_deref().unwrap_or_default()) }
                }
                { hidden("back", here) }
                { fields }
            </form>
        </Dialog>
    }
}

fn recent(rows: &[Row]) -> Html {
    html! {
        <Stack gap={Space::Lg}>
            <PageHeader title="Recent" />
            if rows.is_empty() {
                <EmptyState title="Nothing opened yet" icon={icons::CLOCK}>
                    { "The documents you open are listed here, the latest first." }
                </EmptyState>
            } else {
                <SimpleTable label="Documents opened lately" variant={TableVariant::Ruled}>
                    { headings(Some("In")) }
                    <tbody>{ for rows.iter().map(|row| file(row, None, true, false)) }</tbody>
                </SimpleTable>
            }
        </Stack>
    }
}

fn shared(rows: &[Row]) -> Html {
    html! {
        <Stack gap={Space::Lg}>
            <PageHeader title="Shared with me" />
            if rows.is_empty() {
                <EmptyState title="Nothing is shared with you" icon={icons::SHARE_2}>
                    { "Documents that people share with you from spaces you are not in are listed here." }
                </EmptyState>
            } else {
                <SimpleTable label="Documents shared with you" variant={TableVariant::Ruled}>
                    { headings(Some("From")) }
                    <tbody>{ for rows.iter().map(|row| file(row, None, true, false)) }</tbody>
                </SimpleTable>
            }
        </Stack>
    }
}

/// What a role is called, as the editor's own Share dialog calls it.
fn role(role: api::Role) -> &'static str {
    match role {
        api::Role::Reader => "Reader",
        api::Role::Commenter => "Commenter",
        api::Role::Editor => "Editor",
        api::Role::Owner => "Owner",
    }
}

/// What a form sends to name a person or a group, and what the person reads: the two kinds apart, since a
/// group may be called what a person's id is.
fn grantee(to: &api::Grantee) -> (String, String) {
    match to {
        api::Grantee::Person { id, name } => (format!("p:{id}"), name.clone()),
        api::Grantee::Group { name } => (format!("g:{name}"), format!("Everyone in {name}")),
    }
}

/// The Activity tab of the Share dialog: the document's access log by day, newest first. A row has the
/// icon of what was done in the colour of the role it was done with, and amber marks what was refused.
/// `newest` is the address that opens the dialog on the newest rows.
fn activity(log: &Activity, newest: &str) -> Html {
    let row = |reached: &Reached| {
        let icon = match reached.action {
            api::Action::Open | api::Action::Other => icons::EYE,
            api::Action::Edit => icons::PENCIL,
            api::Action::Comment => icons::MESSAGE_SQUARE,
            api::Action::Download => icons::DOWNLOAD,
            api::Action::Print => icons::PRINTER,
        };
        let tone = match reached.role {
            _ if reached.refused => Some(Tone::Warn),
            api::Role::Reader => None,
            api::Role::Commenter => Some(Tone::Info),
            api::Role::Editor | api::Role::Owner => Some(Tone::Ok),
        };
        let meta = html! {
            <>
                <span>{ role(reached.role) }</span>
                if let Some(label) = &reached.link {
                    <span class="lt-activity__link"><Icon icon={icons::LINK} />{ label.clone() }</span>
                }
                <span class="lt-activity__from" title={reached.from.clone()}>{ reached.from.clone() }</span>
            </>
        };
        html! {
            <TimelineItem title={html! { <>{ reached.said.clone() }</> }} {tone} {icon} {meta} aside={html! { <>{ reached.at.clone() }</> }} />
        }
    };
    let day = |(called, rows): &(String, Vec<Reached>)| {
        html! {
            <section>
                <h3 class="lt-shares__title">{ called.clone() }</h3>
                <Timeline label={called.clone()} size={Size::Small} run_off={TimelineRunOff::None}>
                    { for rows.iter().map(row) }
                </Timeline>
            </section>
        }
    };
    let within = match log.kept {
        0 => String::new(),
        1 => " in the last day".to_owned(),
        days => format!(" in the last {days} days"),
    };
    html! {
        <Stack gap={Space::Lg}>
            if log.days.is_empty() && log.older {
                <p class="lt-note">{ "Nothing from before that is kept." }</p>
            } else if log.days.is_empty() {
                <p class="lt-note">{ "Nobody has reached this document through a share yet." }</p>
            } else {
                <p class="lt-note">
                    { format!("Everything that reached this document through a share{within}, newest first. ") }
                    { format!("Times are in {}.", log.zone) }
                </p>
                { for log.days.iter().map(day) }
            }
            if log.older || log.earlier.is_some() {
                <div class="lt-activity__more">
                    if log.older {
                        <Button size={Size::Small} href={newest.to_owned()}>{ "Show the newest" }</Button>
                    }
                    if let Some(earlier) = &log.earlier {
                        <Button size={Size::Small} href={earlier.clone()}>{ "Show earlier" }</Button>
                    }
                </div>
            }
        </Stack>
    }
}

/// The Share dialog, open over the folder at `here`. Its first tab is who the document is shared with, each
/// with the way to take them out, a form that shares it with one more or gives one of them another role,
/// and the links that open it, each with the way to revoke it. A link is made in the editor, which shows
/// its address. Its second tab is what was done through all of them.
fn sharing(
    entry: &api::Entry,
    people: &[api::Grant],
    offered: &[api::Grantee],
    links: &[Linked],
    log: &Activity,
    here: &str,
) -> Html {
    let id = entry.id.as_deref().unwrap_or_default();
    let link = |link: &Linked| {
        let asks = link.passphrase.then(|| "with a passphrase".to_owned());
        let ends = link.ends.as_ref().map(|day| format!("until {day}"));
        let terms: Vec<String> = [Some(role(link.role).to_owned()), asks, ends].into_iter().flatten().collect();
        html! {
            <li class="lt-shares__row">
                <Icon icon={if link.passphrase { icons::KEY_ROUND } else { icons::LINK }} />
                <span class="lt-shares__name">{ link.label.clone() }</span>
                <span class="lt-dim">{ terms.join(", ") }</span>
                <form method="post" action={ACT_REVOKE}>
                    { hidden("id", id) }
                    { hidden("back", here) }
                    { hidden("link", &link.id) }
                    <Button size={Size::Small} variant={ButtonVariant::Subtle} kind={ButtonKind::Submit} attrs={attrs! { "aria-label" => format!("Revoke the link {}", link.label) }}>
                        { "Revoke" }
                    </Button>
                </form>
            </li>
        }
    };
    let row = |grant: &api::Grant| {
        let (value, label) = grantee(&grant.to);
        let face = match &grant.to {
            api::Grantee::Person { .. } => html! { <Avatar name={label.clone()} size={Size::Small} decorative=true /> },
            api::Grantee::Group { .. } => html! { <Icon icon={icons::USERS} /> },
        };
        html! {
            <li class="lt-shares__row">
                { face }
                <span class="lt-shares__name">{ label.clone() }</span>
                <span class="lt-dim">{ role(grant.role) }</span>
                if grant.role != api::Role::Owner {
                    <form method="post" action={ACT_UNSHARE}>
                        { hidden("id", id) }
                        { hidden("back", here) }
                        { hidden("to", &value) }
                        <Button size={Size::Small} variant={ButtonVariant::Subtle} kind={ButtonKind::Submit} attrs={attrs! { "aria-label" => format!("Remove {label}") }}>
                            { "Remove" }
                        </Button>
                    </form>
                }
            </li>
        }
    };
    let pairs = |(value, label): (String, String)| (AttrValue::from(value), AttrValue::from(label));
    let options: Rc<[(AttrValue, AttrValue)]> = offered.iter().map(grantee).map(pairs).collect();
    let roles = [api::Role::Reader, api::Role::Commenter, api::Role::Editor];
    let roles: Rc<[(AttrValue, AttrValue)]> =
        roles.iter().map(|given| (role(*given).to_lowercase().into(), role(*given).into())).collect();
    html! {
        <Dialog id="asked" open=true title={format!("Share {}", entry.name)} footer={html! {
            <Button href={here.to_owned()}>{ "Done" }</Button>
        }}>
            <Tabs label="Sharing" default_selected={if log.older { "activity" } else { "people" }}>
            <Tab value="people" label="People">
            <Stack gap={Space::Lg}>
                if people.is_empty() {
                    <p class="lt-note">{ "Shared with nobody yet. Whoever is in its space reaches it as before." }</p>
                } else {
                    <ul class="lt-shares" aria-label="Shared with">{ for people.iter().map(row) }</ul>
                }
                <form class="lt-shares__add" method="post" action={ACT_SHARE}>
                    { hidden("id", id) }
                    { hidden("back", here) }
                    <Field label="Share with" class="lt-shares__to">
                        <NativeSelect name="to" {options} placeholder="Choose a person or a group" required=true attrs={attrs! { "autofocus" => "" }} />
                    </Field>
                    <Field label="As">
                        <NativeSelect name="role" options={roles} value="reader" />
                    </Field>
                    <Button variant={ButtonVariant::Primary} kind={ButtonKind::Submit}>{ "Share" }</Button>
                </form>
                if !links.is_empty() {
                    <section>
                        <h3 class="lt-shares__title">{ "Links" }</h3>
                        <ul class="lt-shares" aria-label="Links">{ for links.iter().map(link) }</ul>
                    </section>
                }
            </Stack>
            </Tab>
            <Tab value="activity" label="Activity">
                { activity(log, &asking(here, "share", id)) }
            </Tab>
            </Tabs>
        </Dialog>
    }
}

fn trash(trashed: &[Trashed]) -> Html {
    let row = |(nth, trashed): (usize, &Trashed)| {
        let form = format!("put-back-{nth}");
        html! {
            <tr class="yi-table__row">
                <th scope="row">
                    <span class="lt-file"><Icon icon={if trashed.folder { icons::FOLDER } else { icons::FILE_TEXT }} /><span class="lt-file__name">{ trashed.name.clone() }</span></span>
                </th>
                <td class="lt-dim">{ trashed.from.clone() }</td>
                <td class="lt-dim">{ trashed.thrown.clone() }</td>
                <td class="lt-file__more">
                    if trashed.space.role.edits() {
                        <form id={form.clone()} method="post" action={ACT_PUT_BACK}>
                            { hidden("space", &trashed.space.id) }
                            { hidden("file", &trashed.file) }
                        </form>
                        <Button size={Size::Small} icon={icons::ROTATE_CCW} kind={ButtonKind::Submit} attrs={attrs! { "form" => form }}>
                            { "Put back" }
                        </Button>
                    }
                </td>
            </tr>
        }
    };
    html! {
        <Stack gap={Space::Lg}>
            <PageHeader title="Trash" />
            if trashed.is_empty() {
                <EmptyState title="The trash is empty" icon={icons::TRASH_2}>
                    { "Documents and folders moved to the trash are kept here until they are put back." }
                </EmptyState>
            } else {
                <SimpleTable label="Documents in the trash" variant={TableVariant::Ruled}>
                    <thead>
                        <tr>
                            <th scope="col">{ "Name" }</th>
                            <th scope="col">{ "From" }</th>
                            <th scope="col">{ "Thrown away" }</th>
                            <th scope="col" class="lt-file__more"><VisuallyHidden>{ "More" }</VisuallyHidden></th>
                        </tr>
                    </thead>
                    <tbody>{ for trashed.iter().enumerate().map(row) }</tbody>
                </SimpleTable>
            }
        </Stack>
    }
}

/// One thing said in the middle of a page.
fn said(page: &Page, card: &Card) -> Html {
    match card {
        Card::SignIn { then } => html! {
            <EmptyState title={format!("Sign in to {}", page.server)} icon={icons::PEN_LINE} actions={html! {
                <Button variant={ButtonVariant::Primary} icon={icons::LOG_IN} href={paths::sign_in_page(then)}>{ "Sign in" }</Button>
            }}>
                { "Your documents are kept here. Sign in to open them." }
            </EmptyState>
        },
        Card::Failed { title, message, again } => html! {
            <EmptyState title={title.clone()} icon={icons::TRIANGLE_ALERT} tone={Tone::Warn} actions={match again {
                Some((href, says)) => html! { <Button variant={ButtonVariant::Primary} href={href.clone()}>{ says.clone() }</Button> },
                None => Html::default(),
            }}>
                { message.clone() }
            </EmptyState>
        },
        Card::Passphrase { action, wrong } => html! {
            <>
                <EmptyState title="This link asks for a passphrase" icon={icons::KEY_ROUND}>
                    { "Whoever sent you the link has it." }
                </EmptyState>
                <form class="lt-card__ask" method="post" action={action.clone()}>
                    <Field label="Passphrase" error={if *wrong { html! { "That is not the passphrase." } } else { Html::default() }}>
                        <TextInput name="passphrase" kind={TextInputKind::Password} required=true max_length={200} autocomplete="off" attrs={attrs! { "autofocus" => "" }} />
                    </Field>
                    <Button variant={ButtonVariant::Primary} kind={ButtonKind::Submit}>{ "Open" }</Button>
                </form>
            </>
        },
        Card::HandOver { name, challenge, port, state } => html! {
            <>
                <EmptyState title="Sign in to the Letters app?" icon={icons::LAPTOP} actions={html! {
                    <Button variant={ButtonVariant::Primary} kind={ButtonKind::Submit} attrs={attrs! { "form" => "hand-over" }}>
                        { "Sign in to the app" }
                    </Button>
                }}>
                    { format!("The Letters app on this computer asks to open your documents on {} as ", page.server) }
                    <strong>{ name.clone() }</strong>
                    { ". Close this page if you did not just ask Letters to sign in." }
                </EmptyState>
                <form id="hand-over" method="post" action={paths::DESKTOP_PAGE}>
                    { hidden("challenge", challenge) }
                    { hidden("port", port) }
                    { hidden("state", state) }
                </form>
                <form class="lt-card__other" method="post" action={paths::SIGN_OUT_PAGE}>
                    { format!("Not {name}? ") }
                    <Button variant={ButtonVariant::Subtle} size={Size::Small} kind={ButtonKind::Submit}>{ "Sign out" }</Button>
                </form>
            </>
        },
    }
}
