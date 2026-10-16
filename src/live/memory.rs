//! An in-memory Discord: [`DiscordApi`] over a [`Snapshot`] that writes
//! mutate. Used by tests and by `apply --simulate` to check that applying a
//! plan really converges (a second plan shows no changes).
//!
//! It mimics what matters for concord: new IDs, forum tag IDs, new roles
//! appearing just above `@everyone`, partial updates, and positions. It does
//! not enforce permissions or Discord's validation rules.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use super::api::{ApiError, DiscordApi, Endpoint, WriteRequest};
use super::wire::{Snapshot, WireChannel, WireRole};
use crate::model::FLAG_REQUIRE_TAG;
use crate::snowflake::Sf;

pub struct MemoryApi {
    state: Mutex<Snapshot>,
    next_id: AtomicU64,
    log: Mutex<Vec<WriteRequest>>,
}

impl MemoryApi {
    pub fn new(snapshot: Snapshot) -> Self {
        let max = snapshot
            .roles
            .iter()
            .map(|r| r.id.0)
            .chain(snapshot.channels.iter().map(|c| c.id.0))
            .chain(
                snapshot
                    .channels
                    .iter()
                    .flat_map(|c| c.available_tags.iter().flatten())
                    .filter_map(|t| t.id.map(|i| i.0)),
            )
            .chain(snapshot.community.iter().flat_map(community_ids))
            .max()
            .unwrap_or(0);
        MemoryApi {
            state: Mutex::new(snapshot),
            next_id: AtomicU64::new(max + 1),
            log: Mutex::new(Vec::new()),
        }
    }

    pub fn state(&self) -> Snapshot {
        self.state.lock().unwrap().clone()
    }

    /// Every write received, in order.
    pub fn log(&self) -> Vec<WriteRequest> {
        self.log.lock().unwrap().clone()
    }

    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn apply(&self, guild_id: u64, req: &WriteRequest) -> Result<Value, ApiError> {
        let mut s = self.state.lock().unwrap();
        let body = req.body.clone().unwrap_or(Value::Null);
        let not_found = |what: &str, id: u64| ApiError::Status {
            status: 404,
            method: "?",
            path: format!("{what}/{id}"),
            message: format!("Unknown {what}"),
        };
        match req.endpoint {
            Endpoint::CreateRole => {
                let everyone = s
                    .roles
                    .iter()
                    .find(|r| r.id.0 == guild_id)
                    .map(|r| r.permissions.clone())
                    .unwrap_or_else(|| "0".to_owned());
                // Like Discord, the new role lands at position 1 *without*
                // shifting the others, so it ties with the lowest role.
                let mut role = WireRole {
                    id: Sf(self.id()),
                    permissions: everyone,
                    position: 1,
                    ..Default::default()
                };
                update_role(&mut role, &body);
                s.roles.push(role.clone());
                Ok(to_value(&role))
            }
            Endpoint::UpdateRole(id) => {
                let role = s
                    .roles
                    .iter_mut()
                    .find(|r| r.id.0 == id)
                    .ok_or_else(|| not_found("role", id))?;
                update_role(role, &body);
                Ok(to_value(role))
            }
            Endpoint::RolePositions => {
                // Discord moves each role to its position, shifting the
                // others, then numbers every role 1..n from the bottom. A bot
                // can't touch positions at or above its own highest role.
                let bot_top = s.bot.as_ref().and_then(|b| {
                    s.roles
                        .iter()
                        .filter(|r| b.member.roles.contains(&r.id))
                        .map(|r| r.position)
                        .max()
                });
                let mut order: Vec<WireRole> = s
                    .roles
                    .iter()
                    .filter(|r| r.id.0 != guild_id)
                    .cloned()
                    .collect();
                // Bottom first; ties put the newer (larger) ID lower.
                order.sort_by_key(|r| (r.position, std::cmp::Reverse(r.id.0)));
                for item in body.as_array().into_iter().flatten() {
                    let (id, pos) = id_position(item);
                    let Some(i) = order.iter().position(|r| r.id.0 == id) else {
                        continue;
                    };
                    if bot_top.is_some_and(|top| pos >= top || order[i].position >= top) {
                        return Err(ApiError::Status {
                            status: 403,
                            method: "PATCH",
                            path: format!("guilds/{guild_id}/roles"),
                            message: "Missing Permissions".to_owned(),
                        });
                    }
                    let r = order.remove(i);
                    let at = usize::try_from(pos - 1).unwrap_or(0).min(order.len());
                    order.insert(at, r);
                }
                for (i, r) in order.iter_mut().enumerate() {
                    r.position = i as i64 + 1;
                }
                for r in s.roles.iter_mut() {
                    if let Some(o) = order.iter().find(|o| o.id == r.id) {
                        r.position = o.position;
                    }
                }
                Ok(Value::Array(s.roles.iter().map(to_value).collect()))
            }
            Endpoint::DeleteRole(id) => {
                s.roles.retain(|r| r.id.0 != id);
                for c in &mut s.channels {
                    if let Some(ows) = &mut c.permission_overwrites {
                        ows.retain(|o| o.id.0 != id);
                    }
                }
                Ok(Value::Null)
            }
            Endpoint::CreateChannel => {
                let mut c: WireChannel = merge(
                    &WireChannel {
                        id: Sf(self.id()),
                        ..Default::default()
                    },
                    &body,
                );
                if c.position.is_none() {
                    let last = s
                        .channels
                        .iter()
                        .filter(|o| o.parent_id == c.parent_id)
                        .filter_map(|o| o.position)
                        .max();
                    c.position = Some(last.map_or(0, |p| p + 1));
                }
                self.tag_ids(&mut c);
                s.channels.push(c.clone());
                Ok(to_value(&c))
            }
            Endpoint::UpdateChannel(id) => {
                let i = s
                    .channels
                    .iter()
                    .position(|c| c.id.0 == id)
                    .ok_or_else(|| not_found("channel", id))?;
                let mut c: WireChannel = merge(&s.channels[i], &body);
                self.tag_ids(&mut c);
                // Discord: a forum can only require tags it has.
                let requires = c.flags.unwrap_or(0) & FLAG_REQUIRE_TAG != 0;
                let open_tags = c.available_tags.iter().flatten().any(|t| !t.moderated);
                if requires && !open_tags {
                    return Err(ApiError::Status {
                        status: 400,
                        method: "PATCH",
                        path: format!("channels/{id}"),
                        message: "There are no tags available that can be set by non-moderators"
                            .to_owned(),
                    });
                }
                s.channels[i] = c.clone();
                Ok(to_value(&c))
            }
            Endpoint::ChannelPositions => {
                for item in body.as_array().into_iter().flatten() {
                    let (id, pos) = id_position(item);
                    if let Some(c) = s.channels.iter_mut().find(|c| c.id.0 == id) {
                        c.position = Some(pos);
                    }
                }
                Ok(Value::Null)
            }
            Endpoint::DeleteChannel(id) => {
                let i = s
                    .channels
                    .iter()
                    .position(|c| c.id.0 == id)
                    .ok_or_else(|| not_found("channel", id))?;
                let gone = s.channels.remove(i);
                for c in &mut s.channels {
                    if c.parent_id == Some(Sf(id)) {
                        c.parent_id = None;
                    }
                }
                Ok(to_value(&gone))
            }
            other => self.community(&mut s, guild_id, other, &body, req.file.as_ref()),
        }
    }

    /// Server-wide endpoints: settings, onboarding, AutoMod, expressions.
    fn community(
        &self,
        s: &mut Snapshot,
        guild_id: u64,
        endpoint: Endpoint,
        body: &Value,
        file: Option<&crate::live::api::FilePart>,
    ) -> Result<Value, ApiError> {
        use crate::community::wire::*;
        let (method, path) = endpoint.method_path(guild_id);
        let bad = |message: &str| ApiError::Status {
            status: 400,
            method,
            path: path.clone(),
            message: message.to_owned(),
        };
        let not_found = || ApiError::Status {
            status: 404,
            method,
            path: path.clone(),
            message: "Unknown object".to_owned(),
        };
        let str_id = |v: &Value| -> Option<Sf> {
            v.as_str()
                .and_then(|s| s.parse().ok())
                .or(v.as_u64())
                .map(Sf)
        };
        let emoji_ref = |v: &Value| -> Option<WireEmojiRef> {
            let id = v.get("emoji_id").and_then(str_id);
            let name = v
                .get("emoji_name")
                .and_then(Value::as_str)
                .map(str::to_owned);
            (id.is_some() || name.is_some()).then_some(WireEmojiRef {
                id,
                name,
                animated: None,
            })
        };
        let channels = s.channels.clone();
        let c = s.community.get_or_insert_with(WireCommunity::default);
        match endpoint {
            Endpoint::UpdateGuild => {
                s.guild = merge(&s.guild, body);
                Ok(to_value(&s.guild))
            }
            Endpoint::UpdateWidget => {
                let w: WireWidget = merge(&c.widget.clone().unwrap_or_default(), body);
                c.widget = Some(w.clone());
                Ok(to_value(&w))
            }
            Endpoint::PutOnboarding => {
                let mut o: WireOnboarding =
                    serde_json::from_value(body.clone()).map_err(|e| bad(&e.to_string()))?;
                // The body uses emoji_id/emoji_name; responses use `emoji`.
                for (pi, p) in o.prompts.iter_mut().enumerate() {
                    for (oi, x) in p.options.iter_mut().enumerate() {
                        x.emoji = emoji_ref(&body["prompts"][pi]["options"][oi]);
                        if x.role_ids.is_empty() && x.channel_ids.is_empty() {
                            return Err(bad("Onboarding options need a role or a channel"));
                        }
                    }
                }
                // Discord: at least 7 default channels (a category counts
                // its channels) for enabled onboarding.
                let count: usize = o
                    .default_channel_ids
                    .iter()
                    .map(|id| match channels.iter().find(|ch| ch.id == *id) {
                        Some(ch) if ch.kind == 4 => {
                            channels.iter().filter(|x| x.parent_id == Some(*id)).count()
                        }
                        _ => 1,
                    })
                    .sum();
                if o.enabled && count < 7 {
                    return Err(bad(
                        "Onboarding requirements not met: too few default channels",
                    ));
                }
                c.onboarding = Some(o.clone());
                Ok(to_value(&o))
            }
            Endpoint::UpdateWelcomeScreen => {
                let mut w: WireWelcomeScreen =
                    merge(&c.welcome_screen.clone().unwrap_or_default(), body);
                if let Some(enabled) = body.get("enabled").and_then(Value::as_bool) {
                    w.enabled = enabled;
                }
                c.welcome_screen = Some(w.clone());
                Ok(to_value(&w))
            }
            Endpoint::PutGuide => {
                let g: WireGuide =
                    serde_json::from_value(body.clone()).map_err(|e| bad(&e.to_string()))?;
                if g.enabled && g.new_member_actions.len() < 3 {
                    return Err(bad("The Server Guide needs at least 3 new member actions"));
                }
                c.guide = Some(g.clone());
                Ok(to_value(&g))
            }
            Endpoint::UpdateJoinQuestions => {
                let current = c.verification.clone().unwrap_or_default();
                let sent = body
                    .get("version")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if current.version.is_some() && sent != current.version {
                    return Err(bad("The form was changed elsewhere (version mismatch)"));
                }
                let mut v: WireVerification = merge(&current, body);
                v.version = Some(format!("v{}", self.id()));
                c.verification = Some(v.clone());
                Ok(to_value(&v))
            }
            Endpoint::CreateAutomod => {
                let mut r: WireAutomod = merge(
                    &WireAutomod {
                        id: Sf(self.id()),
                        ..Default::default()
                    },
                    body,
                );
                r.id = Sf(self.id());
                c.automod.get_or_insert_with(Vec::new).push(r.clone());
                Ok(to_value(&r))
            }
            Endpoint::UpdateAutomod(id) => {
                let rules = c.automod.get_or_insert_with(Vec::new);
                let r = rules
                    .iter_mut()
                    .find(|r| r.id.0 == id)
                    .ok_or_else(not_found)?;
                if body.get("trigger_type").is_some() {
                    return Err(bad("trigger_type can't be changed"));
                }
                *r = merge(&*r, body);
                Ok(to_value(&*r))
            }
            Endpoint::CreateEmoji => {
                if body
                    .get("image")
                    .and_then(Value::as_str)
                    .is_none_or(|i| !i.starts_with("data:"))
                {
                    return Err(bad("image must be a data URI"));
                }
                let e = WireEmoji {
                    id: Some(Sf(self.id())),
                    name: body.get("name").and_then(Value::as_str).map(str::to_owned),
                    roles: body
                        .get("roles")
                        .and_then(Value::as_array)
                        .map(|v| v.iter().filter_map(str_id).collect())
                        .unwrap_or_default(),
                    managed: false,
                };
                c.emojis.get_or_insert_with(Vec::new).push(e.clone());
                Ok(to_value(&e))
            }
            Endpoint::UpdateEmoji(id) => {
                let list = c.emojis.get_or_insert_with(Vec::new);
                let e = list
                    .iter_mut()
                    .find(|e| e.id == Some(Sf(id)))
                    .ok_or_else(not_found)?;
                *e = merge(&*e, body);
                Ok(to_value(&*e))
            }
            Endpoint::CreateSticker => {
                if file.is_none() {
                    return Err(bad("stickers need a file"));
                }
                let st = WireSticker {
                    id: Sf(self.id()),
                    ..merge(
                        &WireSticker {
                            id: Sf(0),
                            ..Default::default()
                        },
                        body,
                    )
                };
                c.stickers.get_or_insert_with(Vec::new).push(st.clone());
                Ok(to_value(&st))
            }
            Endpoint::UpdateSticker(id) => {
                let list = c.stickers.get_or_insert_with(Vec::new);
                let st = list
                    .iter_mut()
                    .find(|x| x.id.0 == id)
                    .ok_or_else(not_found)?;
                *st = merge(&*st, body);
                Ok(to_value(&*st))
            }
            Endpoint::CreateSound => {
                let snd = WireSound {
                    sound_id: Sf(self.id()),
                    ..merge(
                        &WireSound {
                            sound_id: Sf(0),
                            volume: 1.0,
                            ..Default::default()
                        },
                        body,
                    )
                };
                c.sounds.get_or_insert_with(Vec::new).push(snd.clone());
                Ok(to_value(&snd))
            }
            Endpoint::UpdateSound(id) => {
                let list = c.sounds.get_or_insert_with(Vec::new);
                let snd = list
                    .iter_mut()
                    .find(|x| x.sound_id.0 == id)
                    .ok_or_else(not_found)?;
                *snd = merge(&*snd, body);
                Ok(to_value(&*snd))
            }
            Endpoint::DeleteAutomod(id) => {
                let list = c.automod.get_or_insert_with(Vec::new);
                list.retain(|x| x.id.0 != id);
                Ok(Value::Null)
            }
            Endpoint::DeleteEmoji(id) => {
                c.emojis
                    .get_or_insert_with(Vec::new)
                    .retain(|x| x.id != Some(Sf(id)));
                Ok(Value::Null)
            }
            Endpoint::DeleteSticker(id) => {
                c.stickers
                    .get_or_insert_with(Vec::new)
                    .retain(|x| x.id.0 != id);
                Ok(Value::Null)
            }
            Endpoint::DeleteSound(id) => {
                c.sounds
                    .get_or_insert_with(Vec::new)
                    .retain(|x| x.sound_id.0 != id);
                Ok(Value::Null)
            }
            _ => unreachable!("role and channel endpoints are handled above"),
        }
    }

    /// Discord gives new forum tags an ID.
    fn tag_ids(&self, c: &mut WireChannel) {
        for t in c.available_tags.iter_mut().flatten() {
            if t.id.is_none() {
                t.id = Some(Sf(self.id()));
            }
        }
    }
}

impl DiscordApi for MemoryApi {
    async fn snapshot(&self, _guild_id: u64) -> Result<Snapshot, ApiError> {
        Ok(self.state())
    }

    async fn write(&self, guild_id: u64, request: &WriteRequest) -> Result<Value, ApiError> {
        self.log.lock().unwrap().push(request.clone());
        self.apply(guild_id, request)
    }
}

/// IDs of server-wide objects, so new ones don't reuse them.
fn community_ids(c: &crate::community::wire::WireCommunity) -> Vec<u64> {
    let mut ids = Vec::new();
    for p in c.onboarding.iter().flat_map(|o| &o.prompts) {
        ids.push(p.id.0);
        ids.extend(p.options.iter().map(|x| x.id.0));
    }
    ids.extend(c.automod.iter().flatten().map(|r| r.id.0));
    ids.extend(c.emojis.iter().flatten().filter_map(|e| e.id.map(|i| i.0)));
    ids.extend(c.stickers.iter().flatten().map(|s| s.id.0));
    ids.extend(c.sounds.iter().flatten().map(|s| s.sound_id.0));
    ids
}

fn to_value<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("wire types serialise")
}

/// `base` with the body's fields written over it, like a PATCH.
fn merge<T: Serialize + DeserializeOwned>(base: &T, body: &Value) -> T {
    let mut v = to_value(base);
    if let (Some(obj), Some(patch)) = (v.as_object_mut(), body.as_object()) {
        for (k, val) in patch {
            let val = match (k.as_str(), val) {
                // Discord trims topics.
                ("topic", Value::String(s)) => Value::String(s.trim().to_owned()),
                _ => val.clone(),
            };
            obj.insert(k.clone(), val);
        }
    }
    serde_json::from_value(v).expect("patched object still matches the wire type")
}

fn update_role(role: &mut WireRole, body: &Value) {
    let mut r = merge(role, body);
    // Clients read `colors` when it's there; keep it in step with `color`.
    if body.get("color").is_some() {
        if let Some(c) = &mut r.colors {
            c.primary_color = r.color;
        }
    }
    *role = r;
}

fn id_position(item: &Value) -> (u64, i64) {
    let id = item
        .get("id")
        .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or(v.as_u64()))
        .unwrap_or(0);
    let pos = item.get("position").and_then(Value::as_i64).unwrap_or(0);
    (id, pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patch_keeps_unmentioned_fields_and_assigns_tag_ids() {
        let api = MemoryApi::new(Snapshot {
            channels: vec![WireChannel {
                id: Sf(5),
                kind: 15,
                name: Some("forum".into()),
                topic: Some("keep".into()),
                ..Default::default()
            }],
            ..Default::default()
        });
        let req = WriteRequest {
            endpoint: Endpoint::UpdateChannel(5),
            body: Some(json!({ "name": "renamed", "available_tags": [{ "name": "Open" }] })),
            file: None,
        };
        let out = api.apply(1, &req).unwrap();
        assert_eq!(out["name"], "renamed");
        assert_eq!(out["topic"], "keep");
        assert!(out["available_tags"][0]["id"].is_string());
    }
}
