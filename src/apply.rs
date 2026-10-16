//! Execute a [`Plan`] against Discord, one op at a time.
//!
//! Ops arrive in dependency order (roles, categories, channels, then
//! settings, overwrites and positions), referring to objects created earlier
//! in the same run by spec key ([`Ref::New`]). The [`Applier`] turns each op
//! into requests, swaps keys for the IDs Discord hands back, and keeps the
//! lockfile current after every op, so a run that stops halfway can simply be
//! planned and applied again.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use crate::diff::{
    ChannelField, CreateChannel, CreateRole, EntityKind, Op, Plan, Ref, ResolvedOverwrite,
    RoleField, SetForumTags, SetOverwrites, TagState, UpdateChannel, UpdateRole,
};
use crate::live::api::{ApiError, DiscordApi, Endpoint, WriteRequest};
use crate::lock::{tag_key, Lockfile};
use crate::model::{ChannelKind, Emoji, Overwrite, OverwriteKind, FLAG_REQUIRE_TAG};
use crate::snowflake::Sf;

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error("`{0}` should have been created earlier in this run, but wasn't")]
    Unresolved(String),
    #[error("Discord's response to creating `{0}` had no ID")]
    NoId(String),
}

/// Runs a plan's ops and tracks the IDs they create.
pub struct Applier {
    guild_id: u64,
    lock: Lockfile,
    roles: HashMap<String, u64>,
    channels: HashMap<String, u64>,
    /// Forums whose tags this plan sets later on.
    tags_pending: HashSet<u64>,
    /// `flags` to send once a forum's tags are set: Discord refuses to require
    /// a tag on a forum that has none yet.
    deferred_flags: HashMap<u64, u64>,
}

impl Applier {
    /// Start from the plan's bindings: every key it resolved to a live object.
    pub fn new(plan: &Plan) -> Self {
        let mut lock = Lockfile::new(plan.guild_id);
        let b = &plan.bindings;
        lock.roles = b.roles.iter().map(|(k, id)| (k.clone(), Sf(*id))).collect();
        lock.channels = b
            .channels
            .iter()
            .map(|(k, id)| (k.clone(), Sf(*id)))
            .collect();
        lock.tags = b.tags.iter().map(|(k, id)| (k.clone(), Sf(*id))).collect();
        Applier {
            guild_id: plan.guild_id,
            lock,
            roles: HashMap::new(),
            channels: HashMap::new(),
            tags_pending: plan
                .ops
                .iter()
                .filter_map(|op| match op {
                    Op::SetForumTags(t) => Some(t.id),
                    _ => None,
                })
                .collect(),
            deferred_flags: HashMap::new(),
        }
    }

    /// The lockfile as of the last completed op.
    pub fn lock(&self) -> &Lockfile {
        &self.lock
    }

    /// Run every op in order, calling `after` once each one is done. Stops at
    /// the first failure, returning its index.
    pub async fn run_all<A: DiscordApi>(
        &mut self,
        api: &A,
        ops: &[Op],
        mut after: impl FnMut(usize, &Op, &Lockfile),
    ) -> Result<(), (usize, ApplyError)> {
        for (i, op) in ops.iter().enumerate() {
            self.run(api, op).await.map_err(|e| (i, e))?;
            after(i, op, &self.lock);
        }
        Ok(())
    }

    pub async fn run<A: DiscordApi>(&mut self, api: &A, op: &Op) -> Result<(), ApplyError> {
        match op {
            // Already in the lockfile, from the plan's bindings.
            Op::Adopt { .. } => Ok(()),
            Op::CreateRole(c) => self.create_role(api, c).await,
            Op::UpdateRole(u) => {
                self.send(api, Endpoint::UpdateRole(u.id), role_patch(u))
                    .await?;
                Ok(())
            }
            Op::CreateChannel(c) => self.create_channel(api, c).await,
            Op::UpdateChannel(u) => {
                let mut body = self.channel_patch(u)?;
                let requires_tag = u
                    .changes
                    .iter()
                    .any(|f| matches!(f, ChannelField::RequireTag(c) if c.after));
                if requires_tag && self.tags_pending.contains(&u.id) {
                    if let Some(flags) = body.as_object_mut().and_then(|b| b.remove("flags")) {
                        self.deferred_flags
                            .insert(u.id, flags.as_u64().unwrap_or_default());
                    }
                }
                if body.as_object().is_some_and(|b| !b.is_empty()) {
                    self.send(api, Endpoint::UpdateChannel(u.id), body).await?;
                }
                Ok(())
            }
            Op::SetForumTags(t) => self.set_tags(api, t).await,
            Op::SetPermissionOverwrites(o) => {
                let body =
                    json!({ "permission_overwrites": self.overwrites(&o.after, &o.members)? });
                self.send(api, Endpoint::UpdateChannel(o.id), body).await?;
                Ok(())
            }
            Op::ReorderChannels(groups) => {
                let mut items = Vec::new();
                let mut seen = Vec::new();
                for g in groups {
                    for (r, pos) in &g.positions {
                        let id = self.channel_id(r)?;
                        if !seen.contains(&id) {
                            seen.push(id);
                            items.push(json!({ "id": id.to_string(), "position": pos }));
                        }
                    }
                }
                self.send(api, Endpoint::ChannelPositions, Value::Array(items))
                    .await?;
                Ok(())
            }
            Op::ReorderRoles(r) => {
                let items = r
                    .positions
                    .iter()
                    .map(|(r, pos)| {
                        Ok(json!({ "id": self.role_id(r)?.to_string(), "position": pos }))
                    })
                    .collect::<Result<Vec<_>, ApplyError>>()?;
                // Positions are counted 1..n below the bot's role, which only
                // fits if Discord's numbering has no gaps or ties, and a new
                // role ties with the lowest one. Any position change makes
                // Discord renumber every role, so first move the role that
                // belongs at the bottom there.
                if let Some(last) = items.last() {
                    let settle = json!([{ "id": last["id"], "position": 1 }]);
                    self.send(api, Endpoint::RolePositions, settle).await?;
                }
                self.send(api, Endpoint::RolePositions, Value::Array(items))
                    .await?;
                Ok(())
            }
            Op::Delete { kind, id, .. } => {
                let endpoint = match kind {
                    EntityKind::Role => Endpoint::DeleteRole(*id),
                    EntityKind::Category | EntityKind::Channel => Endpoint::DeleteChannel(*id),
                };
                let req = WriteRequest {
                    endpoint,
                    body: None,
                };
                api.write(self.guild_id, &req).await?;
                self.forget(*id);
                Ok(())
            }
        }
    }

    async fn send<A: DiscordApi>(
        &self,
        api: &A,
        endpoint: Endpoint,
        body: Value,
    ) -> Result<Value, ApplyError> {
        let req = WriteRequest {
            endpoint,
            body: Some(body),
        };
        Ok(api.write(self.guild_id, &req).await?)
    }

    // -- roles -----------------------------------------------------------

    async fn create_role<A: DiscordApi>(
        &mut self,
        api: &A,
        c: &CreateRole,
    ) -> Result<(), ApplyError> {
        let mut b = Map::new();
        b.insert("name".into(), json!(c.name));
        if let Some(v) = c.color {
            b.insert("color".into(), json!(v));
        }
        if let Some(v) = c.hoist {
            b.insert("hoist".into(), json!(v));
        }
        if let Some(v) = c.mentionable {
            b.insert("mentionable".into(), json!(v));
        }
        if let Some(p) = c.permissions {
            b.insert("permissions".into(), json!(p.bits().to_string()));
        }
        let resp = self
            .send(api, Endpoint::CreateRole, Value::Object(b))
            .await?;
        let id = response_id(&resp).ok_or_else(|| ApplyError::NoId(c.key.clone()))?;
        self.roles.insert(c.key.clone(), id);
        self.lock.roles.insert(c.key.clone(), Sf(id));
        Ok(())
    }

    fn role_id(&self, r: &Ref) -> Result<u64, ApplyError> {
        match r {
            Ref::Id(id) => Ok(*id),
            Ref::New(key) => self
                .roles
                .get(key)
                .copied()
                .ok_or_else(|| ApplyError::Unresolved(key.clone())),
        }
    }

    // -- channels --------------------------------------------------------

    fn channel_id(&self, r: &Ref) -> Result<u64, ApplyError> {
        match r {
            Ref::Id(id) => Ok(*id),
            Ref::New(key) => self
                .channels
                .get(key)
                .copied()
                .ok_or_else(|| ApplyError::Unresolved(key.clone())),
        }
    }

    async fn create_channel<A: DiscordApi>(
        &mut self,
        api: &A,
        c: &CreateChannel,
    ) -> Result<(), ApplyError> {
        let mut b = Map::new();
        b.insert("name".into(), json!(c.name));
        b.insert("type".into(), json!(c.kind.wire()));
        if let Some(p) = &c.parent {
            b.insert(
                "parent_id".into(),
                json!(self.channel_id(&p.target)?.to_string()),
            );
        }
        if let Some(pos) = c.position {
            b.insert("position".into(), json!(pos));
        }
        let opt = |b: &mut Map<String, Value>, k: &str, v: Option<Value>| {
            if let Some(v) = v {
                b.insert(k.into(), v);
            }
        };
        opt(&mut b, "topic", c.topic.as_ref().map(|t| json!(t)));
        opt(&mut b, "nsfw", c.nsfw.map(|v| json!(v)));
        opt(&mut b, "rate_limit_per_user", c.slowmode.map(|v| json!(v)));
        opt(&mut b, "bitrate", c.bitrate.map(|v| json!(v)));
        opt(&mut b, "user_limit", c.user_limit.map(|v| json!(v)));
        opt(
            &mut b,
            "default_auto_archive_duration",
            c.auto_archive.map(|v| json!(v)),
        );
        if c.kind == ChannelKind::Forum {
            opt(
                &mut b,
                "default_forum_layout",
                c.layout.map(|l| json!(l.wire())),
            );
            opt(
                &mut b,
                "default_sort_order",
                c.sort.map(|s| json!(s.wire())),
            );
            opt(
                &mut b,
                "default_reaction_emoji",
                c.default_reaction.as_ref().map(reaction_json),
            );
            if !c.tags.is_empty() {
                b.insert("available_tags".into(), tags_json(&c.tags));
            }
        }
        b.insert(
            "permission_overwrites".into(),
            self.overwrites(&c.overwrites, &[])?,
        );
        let resp = self
            .send(api, Endpoint::CreateChannel, Value::Object(b))
            .await?;
        let id = response_id(&resp).ok_or_else(|| ApplyError::NoId(c.key.clone()))?;
        self.channels.insert(c.key.clone(), id);
        self.lock.channels.insert(c.key.clone(), Sf(id));
        self.bind_new_tags(&c.key, &c.tags, &resp);

        // Discord's create endpoint doesn't take channel flags.
        if c.require_tag == Some(true) {
            let flags = resp.get("flags").and_then(Value::as_u64).unwrap_or(0);
            self.send(
                api,
                Endpoint::UpdateChannel(id),
                json!({ "flags": flags | FLAG_REQUIRE_TAG }),
            )
            .await?;
        }
        Ok(())
    }

    fn channel_patch(&self, u: &UpdateChannel) -> Result<Value, ApplyError> {
        let mut b = Map::new();
        for f in &u.changes {
            let (k, v) = match f {
                ChannelField::Name(c) => ("name", json!(c.after)),
                ChannelField::Kind(c) => ("type", json!(c.after.wire())),
                ChannelField::Parent(c) => (
                    "parent_id",
                    match &c.after {
                        Some(p) => json!(self.channel_id(&p.target)?.to_string()),
                        None => Value::Null,
                    },
                ),
                ChannelField::Topic(c) => ("topic", json!(c.after)),
                ChannelField::Nsfw(c) => ("nsfw", json!(c.after)),
                ChannelField::Slowmode(c) => ("rate_limit_per_user", json!(c.after)),
                ChannelField::Bitrate(c) => ("bitrate", json!(c.after)),
                ChannelField::UserLimit(c) => ("user_limit", json!(c.after)),
                ChannelField::RequireTag(c) => {
                    let flags = if c.after {
                        u.flags | FLAG_REQUIRE_TAG
                    } else {
                        u.flags & !FLAG_REQUIRE_TAG
                    };
                    ("flags", json!(flags))
                }
                ChannelField::Layout(c) => ("default_forum_layout", json!(c.after.wire())),
                ChannelField::Sort(c) => ("default_sort_order", json!(c.after.wire())),
                ChannelField::DefaultReaction(c) => {
                    ("default_reaction_emoji", reaction_json(&c.after))
                }
                ChannelField::AutoArchive(c) => ("default_auto_archive_duration", json!(c.after)),
            };
            b.insert(k.into(), v);
        }
        Ok(Value::Object(b))
    }

    async fn set_tags<A: DiscordApi>(
        &mut self,
        api: &A,
        t: &SetForumTags,
    ) -> Result<(), ApplyError> {
        let resp = self
            .send(
                api,
                Endpoint::UpdateChannel(t.id),
                json!({ "available_tags": tags_json(&t.after) }),
            )
            .await?;
        self.bind_new_tags(&t.key, &t.after, &resp);
        self.tags_pending.remove(&t.id);
        if let Some(flags) = self.deferred_flags.remove(&t.id) {
            self.send(
                api,
                Endpoint::UpdateChannel(t.id),
                json!({ "flags": flags }),
            )
            .await?;
        }
        Ok(())
    }

    /// Record the IDs Discord gave tags that were sent without one.
    fn bind_new_tags(&mut self, channel_key: &str, sent: &[TagState], resp: &Value) {
        let live = resp
            .get("available_tags")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for t in sent {
            let Some(key) = &t.key else { continue };
            let id = t.id.or_else(|| {
                live.iter()
                    .find(|l| l.get("name").and_then(Value::as_str) == Some(t.name.as_str()))
                    .and_then(response_id)
            });
            if let Some(id) = id {
                self.lock.tags.insert(tag_key(channel_key, key), Sf(id));
            }
        }
    }

    fn overwrites(
        &self,
        roles: &[ResolvedOverwrite],
        members: &[Overwrite],
    ) -> Result<Value, ApplyError> {
        let mut out = Vec::new();
        for o in roles {
            out.push(json!({
                "id": self.role_id(&o.target)?.to_string(),
                "type": 0,
                "allow": o.allow.bits().to_string(),
                "deny": o.deny.bits().to_string(),
            }));
        }
        for m in members {
            out.push(json!({
                "id": m.id.to_string(),
                "type": if m.kind == OverwriteKind::Member { 1 } else { 0 },
                "allow": m.allow.bits().to_string(),
                "deny": m.deny.bits().to_string(),
            }));
        }
        Ok(Value::Array(out))
    }

    /// Drop lockfile entries pointing at a deleted object.
    fn forget(&mut self, id: u64) {
        let keys: Vec<String> = self
            .lock
            .channels
            .iter()
            .filter(|(_, v)| v.0 == id)
            .map(|(k, _)| k.clone())
            .collect();
        for k in &keys {
            let prefix = format!("{k}/");
            self.lock.tags.retain(|t, _| !t.starts_with(&prefix));
        }
        self.lock.roles.retain(|_, v| v.0 != id);
        self.lock.channels.retain(|_, v| v.0 != id);
    }
}

fn role_patch(u: &UpdateRole) -> Value {
    let mut b = Map::new();
    for f in &u.changes {
        let (k, v) = match f {
            RoleField::Name(c) => ("name", json!(c.after)),
            RoleField::Color(c) => ("color", json!(c.after)),
            RoleField::Hoist(c) => ("hoist", json!(c.after)),
            RoleField::Mentionable(c) => ("mentionable", json!(c.after)),
            RoleField::Permissions(c) => ("permissions", json!(c.after.bits().to_string())),
        };
        b.insert(k.into(), v);
    }
    Value::Object(b)
}

fn emoji_fields(e: Option<&Emoji>) -> (Value, Value) {
    match e {
        Some(Emoji::Unicode(s)) => (Value::Null, json!(s)),
        Some(Emoji::Custom(id)) => (json!(id.to_string()), Value::Null),
        None => (Value::Null, Value::Null),
    }
}

fn reaction_json(e: &Option<Emoji>) -> Value {
    match e {
        None => Value::Null,
        Some(e) => {
            let (id, name) = emoji_fields(Some(e));
            json!({ "emoji_id": id, "emoji_name": name })
        }
    }
}

/// The full tag list. Existing tags keep their ID, or Discord would delete
/// them and strip them from every post.
fn tags_json(tags: &[TagState]) -> Value {
    Value::Array(
        tags.iter()
            .map(|t| {
                let (emoji_id, emoji_name) = emoji_fields(t.emoji.as_ref());
                let mut o = json!({
                    "name": t.name,
                    "moderated": t.moderated,
                    "emoji_id": emoji_id,
                    "emoji_name": emoji_name,
                });
                if let Some(id) = t.id {
                    o["id"] = json!(id.to_string());
                }
                o
            })
            .collect(),
    )
}

fn response_id(v: &Value) -> Option<u64> {
    let id = v.get("id")?;
    id.as_str().and_then(|s| s.parse().ok()).or(id.as_u64())
}

/// One line describing an op, for progress output.
pub fn describe(op: &Op) -> String {
    match op {
        Op::Adopt { name, .. } => format!("adopt {name}"),
        Op::CreateRole(c) => format!("create role @{}", c.name),
        Op::UpdateRole(u) => format!("update role @{}", u.name.trim_start_matches('@')),
        Op::CreateChannel(c) => match c.kind {
            ChannelKind::Category => format!("create category {}", c.name),
            ChannelKind::Voice => format!("create voice channel {}", c.name),
            k => format!("create {} #{}", k.as_str(), c.name),
        },
        Op::UpdateChannel(u) => match u.kind {
            ChannelKind::Category => format!("update category {}", u.name),
            ChannelKind::Voice => format!("update voice channel {}", u.name),
            _ => format!("update #{}", u.name),
        },
        Op::SetForumTags(t) => format!("set tags on #{}", t.name),
        Op::SetPermissionOverwrites(SetOverwrites { name, kind, .. }) => match kind {
            ChannelKind::Category => format!("set permissions on category {name}"),
            ChannelKind::Voice => format!("set permissions on voice channel {name}"),
            _ => format!("set permissions on #{name}"),
        },
        Op::ReorderChannels(g) => format!("reorder channels ({} group(s))", g.len()),
        Op::ReorderRoles(_) => "reorder roles".to_owned(),
        Op::Delete { kind, name, .. } => match kind {
            EntityKind::Role => format!("delete role @{name}"),
            EntityKind::Category => format!("delete category {name}"),
            EntityKind::Channel => format!("delete {name}"),
        },
    }
}
