//! Compare desired and live state. Pure functions: no I/O.
//!
//! [`plan`] resolves each spec key to a live object (via the lockfile, or by
//! adopting a same-named object), then compares field by field and returns a
//! [`Plan`]: typed [`Op`]s in execution order, plus everything live that the
//! spec doesn't mention, plus diagnostics.

mod checks;
mod engine;
mod order;
mod resolve;
mod tags;

#[cfg(test)]
mod tests;

pub use engine::plan;
pub use resolve::Binding;

use crate::diag::Diagnostic;
use crate::model::{ChannelKind, Emoji, Layout, Overwrite, Sort};
use crate::perms::Perms;

#[derive(Debug, Clone, Default)]
pub struct PlanOptions {
    /// Delete live objects the spec doesn't mention.
    pub prune: bool,
}

/// A reference to a Discord object that may not exist yet.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ref {
    Id(u64),
    /// Created earlier in the same apply; the spec key.
    New(String),
}

impl Ref {
    pub fn id(&self) -> Option<u64> {
        match self {
            Ref::Id(id) => Some(*id),
            Ref::New(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change<T> {
    pub before: T,
    pub after: T,
}

impl<T: PartialEq + Clone> Change<T> {
    /// `Some` only if the value actually changes.
    pub fn of(before: &T, after: &T) -> Option<Change<T>> {
        (before != after).then(|| Change {
            before: before.clone(),
            after: after.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Role,
    Category,
    Channel,
}

impl EntityKind {
    pub fn of_channel(kind: ChannelKind) -> EntityKind {
        if kind == ChannelKind::Category {
            EntityKind::Category
        } else {
            EntityKind::Channel
        }
    }
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub guild_id: u64,
    pub guild_name: String,
    /// In execution order.
    pub ops: Vec<Op>,
    pub unmanaged: Vec<Unmanaged>,
    pub diagnostics: Vec<Diagnostic>,
    /// Every key → ID binding known after planning (locked or adopted). The
    /// lockfile is rewritten from these plus whatever apply creates.
    pub bindings: Bindings,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bindings {
    pub roles: Vec<(String, u64)>,
    pub channels: Vec<(String, u64)>,
    pub tags: Vec<(String, u64)>,
}

impl Plan {
    pub fn has_changes(&self) -> bool {
        !self.ops.is_empty()
    }
    pub fn has_errors(&self) -> bool {
        crate::diag::has_errors(&self.diagnostics)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Record an existing object in the lockfile; no API call.
    Adopt {
        kind: EntityKind,
        key: String,
        id: u64,
        name: String,
    },
    CreateRole(CreateRole),
    UpdateRole(UpdateRole),
    CreateChannel(CreateChannel),
    UpdateChannel(UpdateChannel),
    SetForumTags(SetForumTags),
    SetPermissionOverwrites(SetOverwrites),
    ReorderChannels(Vec<ReorderGroup>),
    ReorderRoles(ReorderRoles),
    Delete {
        kind: EntityKind,
        id: u64,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateRole {
    pub key: String,
    pub name: String,
    pub color: Option<u32>,
    pub hoist: Option<bool>,
    pub mentionable: Option<bool>,
    pub permissions: Option<Perms>,
    /// The lockfile pointed at this ID, which no longer exists.
    pub replaces: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateRole {
    pub key: String,
    pub id: u64,
    pub name: String,
    pub changes: Vec<RoleField>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RoleField {
    Name(Change<String>),
    Color(Change<u32>),
    Hoist(Change<bool>),
    Mentionable(Change<bool>),
    Permissions(Change<Perms>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parent {
    pub target: Ref,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateChannel {
    pub key: String,
    pub kind: ChannelKind,
    pub name: String,
    pub parent: Option<Parent>,
    pub topic: Option<String>,
    pub nsfw: Option<bool>,
    pub slowmode: Option<u32>,
    pub bitrate: Option<u32>,
    pub user_limit: Option<u32>,
    pub require_tag: Option<bool>,
    pub layout: Option<Layout>,
    pub sort: Option<Sort>,
    pub default_reaction: Option<Option<Emoji>>,
    pub auto_archive: Option<u32>,
    pub tags: Vec<TagState>,
    pub overwrites: Vec<ResolvedOverwrite>,
    /// Set when the channel's whole group is new; otherwise a reorder places it.
    pub position: Option<u64>,
    pub replaces: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateChannel {
    pub key: String,
    pub id: u64,
    pub kind: ChannelKind,
    pub name: String,
    /// Live channel flags, so `require_tag` changes keep the other bits.
    pub flags: u64,
    pub changes: Vec<ChannelField>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChannelField {
    Name(Change<String>),
    Kind(Change<ChannelKind>),
    Parent(Change<Option<Parent>>),
    Topic(Change<String>),
    Nsfw(Change<bool>),
    Slowmode(Change<u32>),
    Bitrate(Change<u32>),
    UserLimit(Change<u32>),
    RequireTag(Change<bool>),
    Layout(Change<Layout>),
    Sort(Change<Sort>),
    DefaultReaction(Change<Option<Emoji>>),
    AutoArchive(Change<u32>),
}

/// A forum tag as it will be sent: existing tags keep their ID, or Discord
/// strips them from every post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagState {
    pub id: Option<u64>,
    /// `None` for a live tag the spec doesn't mention.
    pub key: Option<String>,
    pub name: String,
    pub emoji: Option<Emoji>,
    pub moderated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetForumTags {
    pub key: String,
    pub id: u64,
    pub name: String,
    pub before: Vec<TagState>,
    pub after: Vec<TagState>,
    pub changes: Vec<TagChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagChange {
    Add(String),
    Remove(String),
    Rename(Change<String>),
    Emoji {
        name: String,
        change: Change<Option<Emoji>>,
    },
    Moderated {
        name: String,
        change: Change<bool>,
    },
    Reorder,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResolvedOverwrite {
    pub target: Ref,
    /// Role name, for display.
    pub label: String,
    pub allow: Perms,
    pub deny: Perms,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetOverwrites {
    pub key: String,
    pub id: u64,
    pub name: String,
    pub kind: ChannelKind,
    pub before: Vec<ResolvedOverwrite>,
    pub after: Vec<ResolvedOverwrite>,
    /// Member overwrites aren't managed but must be sent back unchanged.
    pub members: Vec<Overwrite>,
    /// Name of the category, if this channel simply mirrors it.
    pub synced_with: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReorderGroup {
    /// "uncategorised channels", "categories", or a category name.
    pub label: String,
    pub before: Vec<String>,
    pub after: Vec<String>,
    /// Every channel in the group with its new position.
    pub positions: Vec<(Ref, u64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReorderRoles {
    /// Highest first.
    pub before: Vec<String>,
    pub after: Vec<String>,
    /// Every role below the bot's highest role, with its new position.
    pub positions: Vec<(Ref, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unmanaged {
    pub kind: UnmanagedKind,
    pub id: u64,
    pub name: String,
    /// Why it's special (protected, unsupported type, was in the lockfile).
    pub note: Option<String>,
    /// Deleted by this plan (`--prune`).
    pub pruned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnmanagedKind {
    Role,
    Category,
    Channel {
        kind: String,
        parent: Option<String>,
    },
    Tag {
        forum: String,
    },
}
