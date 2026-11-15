//! Who reaches a document besides those its space lets in.
//!
//! Whoever edits the space a document is kept in shares it with someone who has signed in, or with a group
//! the identity provider names, to read, to comment on or to edit. In a person's own space that is the
//! person, and in a space of several it is everyone who edits there. Whoever it is shared with opens it by
//! its id, and is not told where it is kept.
//!
//! A share is of the file where it is: it is kept under the document's id and the folder its space is in.
//! A copy of the file in someone else's folder is shared by nobody, and whoever holds a copy cannot say who
//! reaches the original. A share holds for as long as the file is in the space it was shared from.

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use letters_api::{Entry, Grant, Grantee, Person, Role, Shares, paths};
use serde::{Deserialize, Serialize};

use crate::api::{Found, blocking};
use crate::error::{Answer, Refusal};
use crate::session::{PEOPLE, Signed};
use crate::spaces::{Place, Root};
use crate::store::Table;
use crate::{App, Shared};

/// What is kept of a document's shares.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    /// When the document was first shared. Among files of one id, a share leads to the one that was
    /// shared first, so that a copy shared later with the same people does not stand in for it.
    since: u64,
    #[serde(default)]
    people: Vec<Grant>,
}

/// By the document's id, [`BETWEEN`], and the key of the root its file is in.
const SHARES: Table<Kept> = Table::new("shares");
/// What stands between the parts of a key. No root's key holds it, and an id that does names no row of
/// another document.
pub(crate) const BETWEEN: char = '\0';
/// How long a group's name may be.
const LONGEST_GROUP: usize = 200;

fn key(id: &str, root: &Root) -> String {
    format!("{id}{BETWEEN}{}", root.key)
}

/// Whether two rows are of the same person or group.
pub(crate) fn same(one: &Grantee, other: &Grantee) -> bool {
    match (one, other) {
        (Grantee::Person { id: one, .. }, Grantee::Person { id: other, .. }) => one == other,
        (Grantee::Group { name: one }, Grantee::Group { name: other }) => one == other,
        _ => false,
    }
}

/// Whether someone the space a document is kept in lets do what `role` does says who it is shared with:
/// everyone who edits there does, and in a person's own space that is the person. Who shares is said here
/// and nowhere else.
pub(crate) fn may_share(role: Role) -> bool {
    role.edits()
}

/// What the shares of a document say to someone its space lets in.
pub(crate) struct InSpace {
    /// What they may do with it: what the space lets them, or a share when that is more.
    pub role: Role,
    /// With how many people and groups it is shared, and how many links lead to it now. Both are told to
    /// whoever says who it is shared with, and are nothing to anyone else.
    pub with: u32,
    pub links: u32,
}

impl Kept {
    /// What the shares let `who` do: the most that any row of theirs does.
    fn gives(&self, who: &Signed) -> Option<Role> {
        let theirs = |grant: &&Grant| match &grant.to {
            Grantee::Person { id, .. } => *id == who.person.id,
            Grantee::Group { name } => who.groups.contains(name),
        };
        self.people.iter().filter(theirs).map(|grant| grant.role).max()
    }
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route(paths::SHARED, get(shared))
        .route(paths::PEOPLE, get(people))
        .route("/api/v1/documents/{id}/shares", get(shares).put(share))
}

async fn shared(State(app): State<Shared>, who: Signed) -> Answer<Json<Vec<Entry>>> {
    blocking(move || Ok(Json(app.shared(&who)?.into_iter().map(|(entry, _)| entry).collect()))).await
}

async fn people(State(app): State<Shared>, who: Signed) -> Answer<Json<Vec<Grantee>>> {
    blocking(move || app.directory(&who).map(Json)).await
}

async fn shares(State(app): State<Shared>, who: Signed, Path(id): Path<String>) -> Answer<Json<Shares>> {
    blocking(move || app.shares(&who, &id).map(Json)).await
}

async fn share(
    State(app): State<Shared>,
    who: Signed,
    Path(id): Path<String>,
    Json(shares): Json<Shares>,
) -> Answer<Json<Shares>> {
    blocking(move || app.share(&who, &id, &shares.people).map(Json)).await
}

impl App {
    /// The document `id` for the one who says who else reaches it: someone its space lets in, and not
    /// someone a share led to it.
    pub(crate) fn sharing(&self, who: &Signed, id: &str) -> Answer<Found> {
        let found = self.locate(who, id)?;
        match found.space.as_ref().is_some_and(|space| may_share(space.role)) {
            true => Ok(found),
            false => Err(Refusal::forbidden("Only those who edit a document's space say who it is shared with.")),
        }
    }

    /// What the shares of the file of the document `id` in `root` say to `who`, whom its space lets do
    /// what `role` does. Shares that cannot be read give nobody more than the space does.
    pub(crate) fn in_space(&self, who: &Signed, id: &str, root: &Root, role: Role) -> InSpace {
        let kept = self.store.get(&SHARES, &key(id, root)).ok().flatten().unwrap_or_default();
        let (mut with, mut links) = (0, 0);
        if may_share(role) {
            let now = self.now();
            with = kept.people.len() as u32;
            links =
                self.links_of(id, root).map_or(0, |links| links.iter().filter(|link| link.live(now)).count() as u32);
        }
        InSpace { role: kept.gives(who).map_or(role, |given| given.max(role)), with, links }
    }

    /// Where a share leads `who` to the document `id`, and what it lets them do there.
    pub(crate) fn lent(&self, who: &Signed, id: &str) -> Answer<Option<(Role, Root, Place)>> {
        let rows = self.store.under(&SHARES, &format!("{id}{BETWEEN}"))?;
        let mut theirs = Vec::new();
        for (key, kept) in &rows {
            if let (Some((_, root)), Some(role)) = (key.split_once(BETWEEN), kept.gives(who)) {
                theirs.push((kept.since, root, role));
            }
        }
        // Most documents are shared with nobody, and no folder is opened to learn that.
        if theirs.is_empty() {
            return Ok(None);
        }
        theirs.sort();
        let folders = self.folders();
        for (_, root, role) in theirs {
            let Some(rooted) = folders.iter().find(|rooted| rooted.root.key == root) else { continue };
            if let Some((_, place)) = self.library.find(&self.store, &[&rooted.root], id, self.now())? {
                return Ok(Some((role, rooted.root.again()?, place)));
            }
        }
        Ok(None)
    }

    /// The documents shared with `who` from beyond their spaces, by name, each with whose it is: its
    /// owner's name, or its space's.
    pub fn shared(&self, who: &Signed) -> Answer<Vec<(Entry, String)>> {
        let mut theirs = Vec::new();
        for (key, kept) in self.store.under(&SHARES, "")? {
            if let (Some((id, root)), Some(role)) = (key.split_once(BETWEEN), kept.gives(who)) {
                theirs.push((kept.since, id.to_owned(), root.to_owned(), role));
            }
        }
        if theirs.is_empty() {
            return Ok(Vec::new());
        }
        theirs.sort();
        let reach: Vec<String> = self.reach(who).into_iter().map(|(_, root)| root.key).collect();
        let (folders, now) = (self.folders(), self.now());
        let (mut listed, mut entries) = (HashSet::new(), Vec::new());
        for (_, id, root, role) in theirs {
            // What is in a space the person reaches is listed there. Of several files with one id, the
            // one that opens is listed, which is the one shared first.
            if reach.contains(&root) || listed.contains(&id) {
                continue;
            }
            let Some(rooted) = folders.iter().find(|rooted| rooted.root.key == root) else { continue };
            let Some((_, place)) = self.library.find(&self.store, &[&rooted.root], &id, now)? else { continue };
            let Ok(entry) = self.lent_entry(who, role, &rooted.root, &place) else { continue };
            let whose = rooted.whose.as_ref().map_or(&rooted.space.name, |owner| &owner.name);
            entries.push((entry, whose.clone()));
            listed.insert(id);
        }
        entries.sort_by_cached_key(|(entry, _)| entry.name.to_lowercase());
        Ok(entries)
    }

    /// Who a document can be shared with: everyone else who has signed in, by name, and then every group
    /// the server has heard of, from its settings or from someone's sign-in.
    pub fn directory(&self, who: &Signed) -> Answer<Vec<Grantee>> {
        let seen = self.store.under(&PEOPLE, "")?;
        let mut people: Vec<&Person> = seen.iter().map(|(_, seen)| &seen.person).collect();
        people.retain(|person| person.id != who.person.id);
        people.sort_by_cached_key(|person| (person.name.to_lowercase(), person.id.clone()));
        let named = self.config.spaces.iter().flat_map(|space| [&space.editors, &space.commenters, &space.readers]);
        let mut groups: Vec<&String> = named.chain(seen.iter().map(|(_, seen)| &seen.groups)).flatten().collect();
        groups.sort();
        groups.dedup();
        let people =
            people.into_iter().map(|person| Grantee::Person { id: person.id.clone(), name: person.name.clone() });
        Ok(people.chain(groups.into_iter().map(|name| Grantee::Group { name: name.clone() })).collect())
    }

    /// Who the document `id` is shared with, for whoever may say so.
    pub fn shares(&self, who: &Signed, id: &str) -> Answer<Shares> {
        let found = self.sharing(who, id)?;
        let kept = self.store.get(&SHARES, &key(id, &found.root))?.unwrap_or_default();
        self.told_shares(who, id, &found, kept)
    }

    /// Shares the document `id` with `people` and nobody else. Whoever is named twice has what was said
    /// last, and whoever has it open through a share joins again as they may now.
    pub fn share(&self, who: &Signed, id: &str, people: &[Grant]) -> Answer<Shares> {
        let found = self.sharing(who, id)?;
        let mut kept: Vec<Grant> = Vec::new();
        for grant in people {
            let to = match &grant.to {
                // What they may do is theirs by the space, and is neither given nor taken away.
                Grantee::Person { id, .. } if *id == who.person.id => continue,
                Grantee::Person { id, .. } => {
                    let nobody = || Refusal::invalid("Nobody who has signed in to this server goes by that.");
                    let seen = self.store.get(&PEOPLE, id)?.ok_or_else(nobody)?;
                    Grantee::Person { id: id.clone(), name: seen.person.name }
                }
                Grantee::Group { name } if name.trim().is_empty() || name.len() > LONGEST_GROUP => {
                    return Err(Refusal::invalid("That is no group's name."));
                }
                Grantee::Group { name } => Grantee::Group { name: name.clone() },
            };
            if grant.role == Role::Owner {
                return Err(Refusal::invalid("A document is shared to read, to comment on or to edit."));
            }
            kept.retain(|other| !same(&other.to, &to));
            kept.push(Grant { to, role: grant.role });
        }
        let (key, now) = (key(id, &found.root), self.now());
        let kept = self.store.write(|batch| {
            // The row stays when nobody is left in it, so that the document keeps its place in the order.
            let since = batch.get(&SHARES, &key)?.map_or(now, |before| before.since);
            let kept = Kept { since, people: kept };
            batch.put(&SHARES, &key, &kept)?;
            Ok(kept)
        })?;
        self.rooms.reseat(&found.root, &found.place);
        self.told_shares(who, id, &found, kept)
    }

    /// The shares as whoever may change them is told them, each person by the name they go by now. In a
    /// person's own space the document is theirs, and they come first as its owner. Nobody owns what a
    /// space of several keeps.
    fn told_shares(&self, who: &Signed, id: &str, found: &Found, kept: Kept) -> Answer<Shares> {
        let mut people = Vec::new();
        if found.space.as_ref().is_some_and(|space| space.role == Role::Owner) {
            let owner = Grantee::Person { id: who.person.id.clone(), name: who.person.name.clone() };
            people.push(Grant { to: owner, role: Role::Owner });
        }
        for mut grant in kept.people {
            if let Grantee::Person { id, name } = &mut grant.to
                && let Some(seen) = self.store.get(&PEOPLE, id)?
            {
                *name = seen.person.name;
            }
            people.push(grant);
        }
        let links = self.links_of(id, &found.root)?.into_iter().map(|kept| kept.link).collect();
        Ok(Shares { people, links })
    }
}
