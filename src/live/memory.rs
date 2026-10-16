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
        };
        let out = api.apply(1, &req).unwrap();
        assert_eq!(out["name"], "renamed");
        assert_eq!(out["topic"], "keep");
        assert!(out["available_tags"][0]["id"].is_string());
    }
}
