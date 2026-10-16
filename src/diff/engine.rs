use std::collections::HashMap;

use super::order::{reorder_channels, reorder_roles};
use super::resolve::{resolve, Binding, Resolution};
use super::tags::{diff_tags, live_states};
use super::{
    checks, Bindings, Change, ChannelField, CreateChannel, CreateRole, EntityKind, Op, Parent,
    Plan, PlanOptions, Ref, ResolvedOverwrite, RoleField, SetForumTags, SetOverwrites, TagState,
    Unmanaged, UnmanagedKind, UpdateChannel, UpdateRole,
};
use crate::diag::{suggest, Diagnostic};
use crate::lock::Lockfile;
use crate::model::{
    layer_overwrites, ChannelKind, Desired, DesiredChannel, DesiredOverwrites, LiveChannel,
    LiveState, NamedOverwrite,
};
use crate::perms::Perms;
use crate::spec::expand::{limits, EVERYONE};

/// Compare the desired state with the live server.
pub fn plan(desired: &Desired, live: &LiveState, lock: &Lockfile, opts: &PlanOptions) -> Plan {
    let mut diags = Vec::new();
    if let Some(lock_guild) = lock.guild_id {
        if lock_guild.0 != live.guild.id {
            diags.push(
                Diagnostic::error(
                    "",
                    format!(
                        "the lockfile is for server {lock_guild}, but this is server {}",
                        live.guild.id
                    ),
                )
                .with_hint(
                    "use the lockfile that belongs to this server, or delete it to start over",
                ),
            );
        }
    }
    let res = resolve(desired, live, lock, &mut diags);
    let mut e = Engine {
        desired,
        live,
        lock,
        res: &res,
        opts,
        role_refs: HashMap::new(),
        effective: HashMap::new(),
        diags,
        ops: Ops::default(),
        bindings: Bindings::default(),
        unmanaged: Vec::new(),
    };
    e.build_role_refs();
    e.roles();
    e.channels();
    if let Some(r) = reorder_roles(desired, live, &res, &mut e.diags) {
        e.ops.reorder_roles.push(Op::ReorderRoles(r));
    }
    let (groups, create_positions) = reorder_channels(desired, live, &res);
    for op in e
        .ops
        .create_categories
        .iter_mut()
        .chain(&mut e.ops.create_channels)
    {
        if let Op::CreateChannel(c) = op {
            c.position = create_positions.get(&c.key).copied();
        }
    }
    if !groups.is_empty() {
        e.ops.reorder_channels.push(Op::ReorderChannels(groups));
    }
    e.unmanaged_and_prune();

    let ops = e.ops.into_vec();
    let mut diags = e.diags;
    checks::run(&ops, desired, live, &mut diags);
    Plan {
        guild_id: live.guild.id,
        guild_name: live.guild.name.clone(),
        ops,
        unmanaged: e.unmanaged,
        diagnostics: diags,
        bindings: e.bindings,
    }
}

/// Ops by kind, so they come out in dependency order.
#[derive(Default)]
struct Ops {
    adopt: Vec<Op>,
    create_roles: Vec<Op>,
    update_roles: Vec<Op>,
    create_categories: Vec<Op>,
    create_channels: Vec<Op>,
    update_channels: Vec<Op>,
    tags: Vec<Op>,
    overwrites: Vec<Op>,
    reorder_channels: Vec<Op>,
    reorder_roles: Vec<Op>,
    deletes: Vec<Op>,
}

impl Ops {
    fn into_vec(self) -> Vec<Op> {
        [
            self.adopt,
            self.create_roles,
            self.update_roles,
            self.create_categories,
            self.create_channels,
            self.update_channels,
            self.tags,
            self.overwrites,
            self.reorder_channels,
            self.reorder_roles,
            self.deletes,
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

type Tuple = (Ref, Perms, Perms);

struct Engine<'a> {
    desired: &'a Desired,
    live: &'a LiveState,
    lock: &'a Lockfile,
    res: &'a Resolution,
    opts: &'a PlanOptions,
    /// Role name → reference, for overwrites. `None` = ambiguous.
    role_refs: HashMap<String, Option<Ref>>,
    /// Channel key → its desired overwrites (`None` = unmanaged).
    effective: HashMap<String, Option<Vec<Tuple>>>,
    diags: Vec<Diagnostic>,
    ops: Ops,
    bindings: Bindings,
    unmanaged: Vec<Unmanaged>,
}

impl Engine<'_> {
    // -- roles -------------------------------------------------------------

    fn build_role_refs(&mut self) {
        let bound: Vec<u64> = self.res.roles.values().filter_map(Binding::id).collect();
        for lr in &self.live.roles {
            if lr.id == self.live.guild.id || bound.contains(&lr.id) {
                continue;
            }
            self.role_refs
                .entry(lr.name.clone())
                .and_modify(|r| *r = None)
                .or_insert(Some(Ref::Id(lr.id)));
        }
        for r in &self.desired.roles {
            if r.everyone {
                continue;
            }
            let rf = self.role_ref_of_key(&r.key);
            self.role_refs.insert(r.name.clone(), Some(rf));
        }
        self.role_refs
            .insert(EVERYONE.to_owned(), Some(Ref::Id(self.live.guild.id)));
    }

    fn role_ref_of_key(&self, key: &str) -> Ref {
        match self.res.role(key).id() {
            Some(id) => Ref::Id(id),
            None => Ref::New(key.to_owned()),
        }
    }

    fn role_label(&self, r: &Ref) -> String {
        match r {
            Ref::Id(id) if *id == self.live.guild.id => EVERYONE.to_owned(),
            Ref::Id(id) => self
                .desired
                .roles
                .iter()
                .find(|d| self.res.role(&d.key).id() == Some(*id))
                .map(|d| d.name.clone())
                .unwrap_or_else(|| self.live.role_name(*id)),
            Ref::New(key) => self
                .desired
                .roles
                .iter()
                .find(|d| d.key == *key)
                .map_or_else(|| key.clone(), |d| d.name.clone()),
        }
    }

    fn roles(&mut self) {
        for r in &self.desired.roles {
            let binding = self.res.role(&r.key).clone();
            if let Binding::Adopted(id) = binding {
                self.ops.adopt.push(Op::Adopt {
                    kind: EntityKind::Role,
                    key: r.key.clone(),
                    id,
                    name: r.name.clone(),
                });
            }
            let Some(id) = binding.id() else {
                self.ops.create_roles.push(Op::CreateRole(CreateRole {
                    key: r.key.clone(),
                    name: r.name.clone(),
                    color: r.color,
                    hoist: r.hoist,
                    mentionable: r.mentionable,
                    permissions: r.permissions,
                    replaces: match binding {
                        Binding::Recreate(old) => Some(old),
                        _ => None,
                    },
                }));
                continue;
            };
            if !r.everyone {
                self.bindings.roles.push((r.key.clone(), id));
            }
            let Some(lr) = self.live.role(id) else {
                continue;
            };
            let mut changes = Vec::new();
            if !r.everyone {
                changes.extend(Change::of(&lr.name, &r.name).map(RoleField::Name));
            }
            if let Some(c) = r.color {
                if let Some(ch) = Change::of(&lr.color, &c) {
                    if lr.gradient {
                        self.diags.push(Diagnostic::warning(
                            &r.path,
                            format!(
                                "`{}` uses a gradient colour; concord only sets the primary colour",
                                lr.name
                            ),
                        ));
                    }
                    changes.push(RoleField::Color(ch));
                }
            }
            if let Some(h) = r.hoist {
                changes.extend(Change::of(&lr.hoist, &h).map(RoleField::Hoist));
            }
            if let Some(m) = r.mentionable {
                changes.extend(Change::of(&lr.mentionable, &m).map(RoleField::Mentionable));
            }
            if let Some(p) = r.permissions {
                changes.extend(Change::of(&lr.permissions, &p).map(RoleField::Permissions));
            }
            if changes.is_empty() {
                continue;
            }
            if lr.managed {
                self.diags.push(Diagnostic::error(
                    &r.path,
                    format!(
                        "`{}` is managed by an integration (a bot or boosts) and can't be edited",
                        lr.name
                    ),
                ));
                continue;
            }
            self.ops.update_roles.push(Op::UpdateRole(UpdateRole {
                key: r.key.clone(),
                id,
                name: lr.name.clone(),
                changes,
            }));
        }
    }

    // -- channels ----------------------------------------------------------

    fn channel_ref(&self, key: &str) -> Ref {
        match self.res.channel(key).id() {
            Some(id) => Ref::Id(id),
            None => Ref::New(key.to_owned()),
        }
    }

    fn parent_of(&self, c: &DesiredChannel) -> Option<Parent> {
        c.parent.as_ref().map(|p| Parent {
            target: self.channel_ref(p),
            name: self
                .desired
                .channel(p)
                .map_or_else(|| p.clone(), |d| d.name.clone()),
        })
    }

    fn resolve_named(&mut self, list: &[NamedOverwrite]) -> Vec<Tuple> {
        let mut out = Vec::new();
        for o in list {
            match self.role_refs.get(&o.role) {
                Some(Some(r)) => out.push((r.clone(), o.allow, o.deny)),
                Some(None) => self.diags.push(
                    Diagnostic::error(
                        &o.path,
                        format!("several live roles are called `{}`", o.role),
                    )
                    .with_hint("rename one in Discord, or add the role to the spec"),
                ),
                None => {
                    let hint = suggest(&o.role, self.role_refs.keys().map(String::as_str));
                    self.diags.push(
                        Diagnostic::error(&o.path, format!("no role called `{}`", o.role))
                            .with_hint_opt(hint),
                    );
                }
            }
        }
        out
    }

    fn desired_overwrites(&mut self, c: &DesiredChannel) -> Option<Vec<Tuple>> {
        let mut tuples = match &c.overwrites {
            DesiredOverwrites::Unmanaged => return None,
            DesiredOverwrites::Explicit(list) => {
                let resolved = self.resolve_named(list);
                layer_overwrites(&[], &resolved)
            }
            DesiredOverwrites::Inherit(list) => {
                let parent = c.parent.as_deref().unwrap_or_default();
                let base = match self.effective.get(parent) {
                    Some(Some(v)) => v.clone(),
                    // Parent's permissions aren't managed: inherit what it has now.
                    _ => self
                        .res
                        .channel(parent)
                        .id()
                        .and_then(|id| self.live.channel(id))
                        .map(live_tuples)
                        .unwrap_or_default(),
                };
                let resolved = self.resolve_named(list);
                layer_overwrites(&base, &resolved)
            }
        };
        tuples.sort();
        Some(tuples)
    }

    fn to_resolved(&self, tuples: &[Tuple]) -> Vec<ResolvedOverwrite> {
        tuples
            .iter()
            .map(|(r, allow, deny)| ResolvedOverwrite {
                target: r.clone(),
                label: self.role_label(r),
                allow: *allow,
                deny: *deny,
            })
            .collect()
    }

    fn channels(&mut self) {
        for c in &self.desired.channels {
            let want_ow = self.desired_overwrites(c);
            self.effective.insert(c.key.clone(), want_ow.clone());
            let binding = self.res.channel(&c.key).clone();
            if let Binding::Adopted(id) = binding {
                self.ops.adopt.push(Op::Adopt {
                    kind: EntityKind::of_channel(c.kind),
                    key: c.key.clone(),
                    id,
                    name: c.name.clone(),
                });
            }
            match binding.id().and_then(|id| self.live.channel(id)) {
                None => self.create_channel(c, want_ow, &binding),
                Some(lc) => {
                    self.bindings.channels.push((c.key.clone(), lc.id));
                    self.update_channel(c, lc, want_ow);
                }
            }
        }
    }

    fn create_channel(&mut self, c: &DesiredChannel, want_ow: Option<Vec<Tuple>>, b: &Binding) {
        let op = Op::CreateChannel(CreateChannel {
            key: c.key.clone(),
            kind: c.kind,
            name: c.name.clone(),
            parent: self.parent_of(c),
            topic: c.topic.clone().filter(|t| !t.is_empty()),
            nsfw: c.nsfw,
            slowmode: c.slowmode,
            bitrate: c.bitrate,
            user_limit: c.user_limit,
            require_tag: c.forum.require_tag,
            layout: c.forum.layout,
            sort: c.forum.sort,
            default_reaction: c.forum.default_reaction.clone(),
            auto_archive: c.forum.auto_archive,
            tags: c
                .forum
                .tags
                .iter()
                .flatten()
                .map(|t| TagState {
                    id: None,
                    key: Some(t.key.clone()),
                    name: t.name.clone(),
                    emoji: t.emoji.clone(),
                    moderated: t.moderated,
                })
                .collect(),
            overwrites: self.to_resolved(&want_ow.unwrap_or_default()),
            position: None,
            replaces: match b {
                Binding::Recreate(old) => Some(*old),
                _ => None,
            },
        });
        if c.kind == ChannelKind::Category {
            self.ops.create_categories.push(op);
        } else {
            self.ops.create_channels.push(op);
        }
    }

    fn update_channel(
        &mut self,
        c: &DesiredChannel,
        lc: &LiveChannel,
        want_ow: Option<Vec<Tuple>>,
    ) {
        let Some(live_kind) = lc.kind.known().filter(|k| k.convertible_to(c.kind)) else {
            return; // reported during resolution
        };
        let mut changes = Vec::new();
        changes.extend(Change::of(&lc.name, &c.name).map(ChannelField::Name));
        changes.extend(Change::of(&live_kind, &c.kind).map(ChannelField::Kind));
        if c.kind != ChannelKind::Category {
            let want = self.parent_of(c);
            let have = lc.parent_id.map(|p| Parent {
                target: Ref::Id(p),
                name: self
                    .live
                    .channel(p)
                    .map(|pc| pc.name.clone())
                    .unwrap_or_default(),
            });
            if want.as_ref().map(|p| &p.target) != have.as_ref().map(|p| &p.target) {
                changes.push(ChannelField::Parent(Change {
                    before: have,
                    after: want,
                }));
            }
        }
        macro_rules! cmp {
            ($want:expr, $have:expr, $variant:ident) => {
                if let Some(w) = &$want {
                    changes.extend(Change::of(&$have, w).map(ChannelField::$variant));
                }
            };
        }
        cmp!(c.topic, lc.topic, Topic);
        cmp!(c.nsfw, lc.nsfw, Nsfw);
        cmp!(c.slowmode, lc.slowmode, Slowmode);
        cmp!(c.bitrate, lc.bitrate, Bitrate);
        cmp!(c.user_limit, lc.user_limit, UserLimit);
        cmp!(c.forum.require_tag, lc.forum.require_tag, RequireTag);
        cmp!(c.forum.layout, lc.forum.layout, Layout);
        cmp!(c.forum.sort, lc.forum.sort, Sort);
        cmp!(
            c.forum.default_reaction,
            lc.forum.default_reaction,
            DefaultReaction
        );
        cmp!(c.forum.auto_archive, lc.forum.auto_archive, AutoArchive);
        if !changes.is_empty() {
            self.ops
                .update_channels
                .push(Op::UpdateChannel(UpdateChannel {
                    key: c.key.clone(),
                    id: lc.id,
                    kind: c.kind,
                    name: lc.name.clone(),
                    flags: lc.flags,
                    changes,
                }));
        }

        if let Some(tags) = &c.forum.tags {
            let d = diff_tags(&c.key, tags, &lc.forum.tags, self.lock, self.opts.prune);
            self.bindings.tags.extend(d.bindings);
            for t in &d.unmanaged {
                self.unmanaged.push(Unmanaged {
                    kind: UnmanagedKind::Tag {
                        forum: c.name.clone(),
                    },
                    id: t.id,
                    name: t.name.clone(),
                    note: None,
                    pruned: self.opts.prune,
                });
            }
            if d.after.len() > limits::TAGS_PER_FORUM {
                self.diags.push(
                    Diagnostic::error(
                        &c.path,
                        format!(
                            "`{}` would have {} tags ({} from the spec + {} existing ones it doesn't mention); Discord allows {}",
                            c.name,
                            d.after.len(),
                            tags.len(),
                            d.unmanaged.len(),
                            limits::TAGS_PER_FORUM
                        ),
                    )
                    .with_hint("add the existing tags to the spec, or remove them with --prune"),
                );
            }
            if !d.changes.is_empty() {
                self.ops.tags.push(Op::SetForumTags(SetForumTags {
                    key: c.key.clone(),
                    id: lc.id,
                    name: c.name.clone(),
                    before: live_states(&lc.forum.tags, &d.after),
                    after: d.after,
                    changes: d.changes,
                }));
            }
        }

        if let Some(want) = want_ow {
            let have = live_tuples(lc);
            if want != have {
                let synced_with = match &c.overwrites {
                    DesiredOverwrites::Inherit(extra) if extra.is_empty() => {
                        self.parent_of(c).map(|p| p.name)
                    }
                    _ => None,
                };
                self.ops
                    .overwrites
                    .push(Op::SetPermissionOverwrites(SetOverwrites {
                        key: c.key.clone(),
                        id: lc.id,
                        name: c.name.clone(),
                        kind: c.kind,
                        before: self.to_resolved(&have),
                        after: self.to_resolved(&want),
                        members: lc.member_overwrites(),
                        synced_with,
                    }));
            }
        }
    }

    // -- things the spec doesn't mention -----------------------------------

    fn unmanaged_and_prune(&mut self) {
        let prune = self.opts.prune;
        let lock_key_of = |map: &std::collections::BTreeMap<String, crate::snowflake::Sf>,
                           id: u64| {
            map.iter().find(|(_, v)| v.0 == id).map(|(k, _)| k.clone())
        };
        let was_locked = |key: Option<String>| key.map(|k| format!("was `{k}` in the lockfile"));

        let bound_roles: Vec<u64> = self.res.roles.values().filter_map(Binding::id).collect();
        let mut role_deletes = Vec::new();
        for lr in &self.live.roles {
            if lr.id == self.live.guild.id || lr.managed || bound_roles.contains(&lr.id) {
                continue;
            }
            self.unmanaged.push(Unmanaged {
                kind: UnmanagedKind::Role,
                id: lr.id,
                name: lr.name.clone(),
                note: was_locked(lock_key_of(&self.lock.roles, lr.id)),
                pruned: prune,
            });
            if prune {
                role_deletes.push(Op::Delete {
                    kind: EntityKind::Role,
                    id: lr.id,
                    name: lr.name.clone(),
                });
            }
        }

        let bound: Vec<u64> = self.res.channels.values().filter_map(Binding::id).collect();
        let mut channel_deletes = Vec::new();
        let mut category_deletes = Vec::new();
        for lc in &self.live.channels {
            if bound.contains(&lc.id) {
                continue;
            }
            let protected = self.live.protected_reason(lc.id);
            let note = protected
                .map(|why| format!("{why}; Discord won't let it be deleted"))
                .or_else(|| match lc.kind.known() {
                    None => Some(format!(
                        "{} channels aren't supported yet",
                        lc.kind.describe()
                    )),
                    Some(_) => None,
                })
                .or_else(|| was_locked(lock_key_of(&self.lock.channels, lc.id)));
            let pruned = prune && protected.is_none();
            let kind = if lc.kind.is_category() {
                UnmanagedKind::Category
            } else {
                UnmanagedKind::Channel {
                    kind: lc.kind.describe(),
                    parent: lc
                        .parent_id
                        .and_then(|p| self.live.channel(p))
                        .map(|p| p.name.clone()),
                }
            };
            self.unmanaged.push(Unmanaged {
                kind,
                id: lc.id,
                name: lc.name.clone(),
                note,
                pruned,
            });
            if pruned {
                let op = Op::Delete {
                    kind: if lc.kind.is_category() {
                        EntityKind::Category
                    } else {
                        EntityKind::Channel
                    },
                    id: lc.id,
                    name: lc.name.clone(),
                };
                if lc.kind.is_category() {
                    category_deletes.push(op);
                } else {
                    channel_deletes.push(op);
                }
            }
        }
        self.ops.deletes.extend(channel_deletes);
        self.ops.deletes.extend(category_deletes);
        self.ops.deletes.extend(role_deletes);
    }
}

fn live_tuples(lc: &LiveChannel) -> Vec<Tuple> {
    let mut v: Vec<Tuple> = lc
        .role_overwrites()
        .into_iter()
        .map(|(id, a, d)| (Ref::Id(id), a, d))
        .collect();
    v.sort();
    v
}
