//! Executing [`CommunityOp`]s: request bodies, and recording the IDs Discord
//! hands back in the lockfile.

use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;
use serde_json::{json, Map, Value};

use super::diff::{CommunityOp, SettingValue};
use super::model::*;
use crate::apply::{Applier, ApplyError};
use crate::diff::Ref;
use crate::live::api::{DiscordApi, Endpoint, FilePart, WriteRequest};
use crate::model::Emoji;
use crate::snowflake::Sf;

/// Discord's epoch, for snowflakes concord makes up for new onboarding
/// prompts and options (Discord wants every one to have an ID).
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

fn new_snowflake() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    ((ms.saturating_sub(DISCORD_EPOCH_MS)) << 22)
        | (COUNTER.fetch_add(1, Ordering::Relaxed) & 0xFFF)
}

fn data_uri(a: &Asset) -> String {
    format!(
        "data:{};base64,{}",
        a.mime,
        base64::engine::general_purpose::STANDARD.encode(&a.bytes)
    )
}

/// `{emoji_id, emoji_name}` fields, as most endpoints take them.
fn emoji_fields(e: Option<&Emoji>) -> (Value, Value) {
    match e {
        Some(Emoji::Unicode(s)) => (Value::Null, json!(s)),
        Some(Emoji::Custom(id)) => (json!(id.to_string()), Value::Null),
        None => (Value::Null, Value::Null),
    }
}

/// `{id, name}`, as onboarding responses and the Server Guide use.
fn emoji_object(e: Option<&Emoji>) -> Value {
    match e {
        None => Value::Null,
        Some(e) => {
            let (id, name) = emoji_fields(Some(e));
            json!({ "id": id, "name": name })
        }
    }
}

fn response_id(v: &Value, field: &str) -> Option<u64> {
    let id = v.get(field)?;
    id.as_str().and_then(|s| s.parse().ok()).or(id.as_u64())
}

impl Applier {
    fn channel_ids(&self, refs: &[Ref]) -> Result<Vec<String>, ApplyError> {
        refs.iter()
            .map(|r| self.channel_id(r).map(|id| id.to_string()))
            .collect()
    }

    fn role_ids(&self, refs: &[Ref]) -> Result<Vec<String>, ApplyError> {
        refs.iter()
            .map(|r| self.role_id(r).map(|id| id.to_string()))
            .collect()
    }

    pub(crate) async fn community<A: DiscordApi>(
        &mut self,
        api: &A,
        op: &CommunityOp,
    ) -> Result<(), ApplyError> {
        use CommunityOp as C;
        match op {
            C::Settings(changes) => {
                let mut b = Map::new();
                for c in changes {
                    let v = match &c.value {
                        SettingValue::Json(v) => v.clone(),
                        SettingValue::Channel(None) => Value::Null,
                        SettingValue::Channel(Some(r)) => json!(self.channel_id(r)?.to_string()),
                    };
                    b.insert(c.wire.to_owned(), v);
                }
                self.send(api, Endpoint::UpdateGuild, Value::Object(b))
                    .await?;
            }
            C::Widget {
                enabled, channel, ..
            } => {
                let mut b = json!({ "enabled": enabled });
                if let Some(c) = channel {
                    b["channel_id"] = match c {
                        Some(r) => json!(self.channel_id(r)?.to_string()),
                        None => Value::Null,
                    };
                }
                self.send(api, Endpoint::UpdateWidget, b).await?;
            }
            C::Onboarding { after, .. } => self.onboarding(api, after).await?,
            C::WelcomeScreen { after, .. } => {
                let channels = after
                    .channels
                    .iter()
                    .map(|c| {
                        let (emoji_id, emoji_name) = emoji_fields(c.emoji.as_ref());
                        Ok(json!({
                            "channel_id": self.channel_id(&c.channel)?.to_string(),
                            "description": c.description,
                            "emoji_id": emoji_id,
                            "emoji_name": emoji_name,
                        }))
                    })
                    .collect::<Result<Vec<_>, ApplyError>>()?;
                let body = json!({
                    "enabled": after.enabled,
                    "description": after.description,
                    "welcome_channels": channels,
                });
                self.send(api, Endpoint::UpdateWelcomeScreen, body).await?;
            }
            C::Guide { after, .. } => {
                let item = |i: &GuideItem<Ref>, action: bool| -> Result<Value, ApplyError> {
                    let mut o = json!({
                        "channel_id": self.channel_id(&i.channel)?.to_string(),
                        "title": i.title,
                        "description": i.description,
                    });
                    if action {
                        o["action_type"] = json!(i.kind);
                    }
                    if i.emoji.is_some() {
                        o["emoji"] = emoji_object(i.emoji.as_ref());
                    }
                    if let Some(icon) = &i.icon {
                        o["icon"] = icon.clone();
                    }
                    Ok(o)
                };
                let actions = after
                    .actions
                    .iter()
                    .map(|i| item(i, true))
                    .collect::<Result<Vec<_>, _>>()?;
                let resources = after
                    .resources
                    .iter()
                    .map(|i| item(i, false))
                    .collect::<Result<Vec<_>, _>>()?;
                let (message, authors) = after.welcome.clone().unwrap_or_default();
                let body = json!({
                    "enabled": after.enabled,
                    "welcome_message": {
                        "author_ids": authors.iter().map(u64::to_string).collect::<Vec<_>>(),
                        "message": message,
                    },
                    "new_member_actions": actions,
                    "resource_channels": resources,
                });
                self.send(api, Endpoint::PutGuide, body).await?;
            }
            C::JoinQuestions { version, after, .. } => {
                let mut fields = Vec::new();
                if !after.rules.is_empty() {
                    fields.push(json!({
                        "field_type": "TERMS",
                        "label": "Read and agree to the server rules",
                        "values": after.rules,
                        "required": true,
                    }));
                }
                for q in &after.questions {
                    let field_type = match q.kind {
                        1 => "PARAGRAPH",
                        2 => "MULTIPLE_CHOICE",
                        _ => "TEXT_INPUT",
                    };
                    let mut f = json!({
                        "field_type": field_type,
                        "label": q.label,
                        "required": q.required,
                        "description": (!q.description.is_empty()).then_some(&q.description),
                        "placeholder": (!q.placeholder.is_empty()).then_some(&q.placeholder),
                    });
                    if q.kind == 2 {
                        f["choices"] = json!(q.choices);
                    }
                    fields.push(f);
                }
                let body = json!({
                    "version": version,
                    "description": after.description,
                    "form_fields": fields,
                });
                self.send(api, Endpoint::UpdateJoinQuestions, body).await?;
            }
            C::CreateAutomod { rule, .. } => {
                let body = self.automod_body(rule, true)?;
                let resp = self.send(api, Endpoint::CreateAutomod, body).await?;
                let id =
                    response_id(&resp, "id").ok_or_else(|| ApplyError::NoId(rule.key.clone()))?;
                self.lock.automod.insert(rule.key.clone(), Sf(id));
            }
            C::UpdateAutomod { id, rule, .. } => {
                let body = self.automod_body(rule, false)?;
                self.send(api, Endpoint::UpdateAutomod(*id), body).await?;
            }
            C::DeleteAutomod { id, .. } => {
                self.delete(api, Endpoint::DeleteAutomod(*id), *id).await?
            }
            C::CreateEmoji {
                key,
                name,
                asset,
                roles,
            } => {
                let body = json!({
                    "name": name,
                    "image": data_uri(asset),
                    "roles": self.role_ids(roles)?,
                });
                let resp = self.send(api, Endpoint::CreateEmoji, body).await?;
                let id = response_id(&resp, "id").ok_or_else(|| ApplyError::NoId(key.clone()))?;
                self.lock.emoji.insert(key.clone(), Sf(id));
                self.lock
                    .assets
                    .insert(format!("emoji/{key}"), asset.sha256.clone());
            }
            C::UpdateEmoji {
                id, name, roles, ..
            } => {
                let body = json!({ "name": name, "roles": self.role_ids(roles)? });
                self.send(api, Endpoint::UpdateEmoji(*id), body).await?;
            }
            C::DeleteEmoji { id, .. } => self.delete(api, Endpoint::DeleteEmoji(*id), *id).await?,
            C::CreateSticker {
                key,
                name,
                asset,
                description,
                emoji,
            } => {
                let ext = asset.file.rsplit('.').next().unwrap_or("png");
                let req = WriteRequest {
                    endpoint: Endpoint::CreateSticker,
                    body: Some(json!({ "name": name, "description": description, "tags": emoji })),
                    file: Some(FilePart {
                        filename: format!("sticker.{ext}"),
                        mime: asset.mime,
                        bytes: asset.bytes.clone(),
                    }),
                };
                let resp = api.write(self.guild_id, &req).await?;
                let id = response_id(&resp, "id").ok_or_else(|| ApplyError::NoId(key.clone()))?;
                self.lock.stickers.insert(key.clone(), Sf(id));
                self.lock
                    .assets
                    .insert(format!("stickers/{key}"), asset.sha256.clone());
            }
            C::UpdateSticker {
                id,
                name,
                description,
                emoji,
                ..
            } => {
                let body = json!({ "name": name, "description": description, "tags": emoji });
                self.send(api, Endpoint::UpdateSticker(*id), body).await?;
            }
            C::DeleteSticker { id, .. } => {
                self.delete(api, Endpoint::DeleteSticker(*id), *id).await?
            }
            C::CreateSound {
                key,
                name,
                asset,
                volume,
                emoji,
            } => {
                let (emoji_id, emoji_name) = emoji_fields(emoji.as_ref());
                let body = json!({
                    "name": name,
                    "sound": data_uri(asset),
                    "volume": volume,
                    "emoji_id": emoji_id,
                    "emoji_name": emoji_name,
                });
                let resp = self.send(api, Endpoint::CreateSound, body).await?;
                let id = response_id(&resp, "sound_id")
                    .or_else(|| response_id(&resp, "id"))
                    .ok_or_else(|| ApplyError::NoId(key.clone()))?;
                self.lock.sounds.insert(key.clone(), Sf(id));
                self.lock
                    .assets
                    .insert(format!("sounds/{key}"), asset.sha256.clone());
            }
            C::UpdateSound {
                id,
                name,
                volume,
                emoji,
                ..
            } => {
                let (emoji_id, emoji_name) = emoji_fields(emoji.as_ref());
                let body = json!({
                    "name": name,
                    "volume": volume,
                    "emoji_id": emoji_id,
                    "emoji_name": emoji_name,
                });
                self.send(api, Endpoint::UpdateSound(*id), body).await?;
            }
            C::DeleteSound { id, .. } => self.delete(api, Endpoint::DeleteSound(*id), *id).await?,
        }
        Ok(())
    }

    async fn delete<A: DiscordApi>(
        &mut self,
        api: &A,
        endpoint: Endpoint,
        id: u64,
    ) -> Result<(), ApplyError> {
        let req = WriteRequest {
            endpoint,
            body: None,
            file: None,
        };
        api.write(self.guild_id, &req).await?;
        self.forget(id);
        Ok(())
    }

    async fn onboarding<A: DiscordApi>(
        &mut self,
        api: &A,
        o: &Onboarding<Ref, Ref>,
    ) -> Result<(), ApplyError> {
        let mut prompts = Vec::new();
        for p in &o.prompts {
            let mut options = Vec::new();
            for x in &p.options {
                let (emoji_id, emoji_name) = emoji_fields(x.emoji.as_ref());
                options.push(json!({
                    "id": x.id.unwrap_or_else(new_snowflake).to_string(),
                    "title": x.title,
                    "description": x.description,
                    "emoji_id": emoji_id,
                    "emoji_name": emoji_name,
                    "emoji_animated": false,
                    "role_ids": self.role_ids(&x.roles)?,
                    "channel_ids": self.channel_ids(&x.channels)?,
                }));
            }
            prompts.push(json!({
                "id": p.id.unwrap_or_else(new_snowflake).to_string(),
                "type": p.kind,
                "title": p.title,
                "single_select": p.single,
                "required": p.required,
                "in_onboarding": p.in_onboarding,
                "options": options,
            }));
        }
        let body = json!({
            "prompts": prompts,
            "default_channel_ids": self.channel_ids(&o.default_channels)?,
            "enabled": o.enabled,
            "mode": o.mode,
        });
        let resp = self
            .send(api, Endpoint::PutOnboarding, body.clone())
            .await?;
        // Discord keeps our order; record every prompt and option's ID.
        let sent = &body["prompts"];
        let got = resp.get("prompts").unwrap_or(sent);
        for (i, p) in o.prompts.iter().enumerate() {
            if let Some(id) = got.get(i).and_then(|v| response_id(v, "id")) {
                self.lock.onboarding.insert(p.key.clone(), Sf(id));
            }
            for (j, x) in p.options.iter().enumerate() {
                let id = got
                    .get(i)
                    .and_then(|v| v.get("options"))
                    .and_then(|v| v.get(j))
                    .and_then(|v| response_id(v, "id"));
                if let Some(id) = id {
                    self.lock
                        .onboarding
                        .insert(format!("{}/{}", p.key, x.key), Sf(id));
                }
            }
        }
        Ok(())
    }

    fn automod_body(&self, r: &Automod<Ref, Ref>, create: bool) -> Result<Value, ApplyError> {
        let mut meta = Map::new();
        match r.trigger {
            1 | 6 => {
                meta.insert("keyword_filter".into(), json!(r.keywords));
                meta.insert("regex_patterns".into(), json!(r.regex));
                meta.insert("allow_list".into(), json!(r.allow));
            }
            4 => {
                meta.insert("presets".into(), json!(r.presets));
                meta.insert("allow_list".into(), json!(r.allow));
            }
            5 => {
                meta.insert(
                    "mention_total_limit".into(),
                    json!(r.mention_limit.unwrap_or(10)),
                );
                meta.insert(
                    "mention_raid_protection_enabled".into(),
                    json!(r.raid_protection.unwrap_or(false)),
                );
            }
            _ => {}
        }
        let actions = r
            .actions
            .iter()
            .map(|a| {
                let metadata = match a {
                    AutomodAction::Block(Some(m)) => json!({ "custom_message": m }),
                    AutomodAction::Block(None) | AutomodAction::BlockInteractions => json!({}),
                    AutomodAction::Alert(c) => {
                        json!({ "channel_id": self.channel_id(c)?.to_string() })
                    }
                    AutomodAction::Timeout(t) => json!({ "duration_seconds": t }),
                };
                Ok(json!({ "type": a.wire_type(), "metadata": metadata }))
            })
            .collect::<Result<Vec<_>, ApplyError>>()?;
        let mut body = json!({
            "name": r.name,
            "event_type": r.event,
            "trigger_metadata": meta,
            "actions": actions,
            "enabled": r.enabled,
            "exempt_roles": self.role_ids(&r.exempt_roles)?,
            "exempt_channels": self.channel_ids(&r.exempt_channels)?,
        });
        if create {
            body["trigger_type"] = json!(r.trigger);
        }
        Ok(body)
    }
}

/// One line describing a community op, for progress output.
pub fn describe(op: &CommunityOp) -> String {
    use CommunityOp as C;
    match op {
        C::Settings(c) => format!(
            "update server settings ({})",
            c.iter().map(|c| c.field).collect::<Vec<_>>().join(", ")
        ),
        C::Widget { .. } => "update the server widget".to_owned(),
        C::Onboarding { .. } => "update onboarding".to_owned(),
        C::WelcomeScreen { .. } => "update the welcome screen".to_owned(),
        C::Guide { .. } => "update the Server Guide".to_owned(),
        C::JoinQuestions { .. } => "update join questions".to_owned(),
        C::CreateAutomod { rule, .. } => format!("create AutoMod rule \"{}\"", rule.name),
        C::UpdateAutomod { rule, .. } => format!("update AutoMod rule \"{}\"", rule.name),
        C::DeleteAutomod { name, .. } => format!("delete AutoMod rule \"{name}\""),
        C::CreateEmoji { name, .. } => format!("create emoji :{name}:"),
        C::UpdateEmoji { name, .. } => format!("update emoji :{name}:"),
        C::DeleteEmoji { name, .. } => format!("delete emoji :{name}:"),
        C::CreateSticker { name, .. } => format!("create sticker {name}"),
        C::UpdateSticker { name, .. } => format!("update sticker {name}"),
        C::DeleteSticker { name, .. } => format!("delete sticker {name}"),
        C::CreateSound { name, .. } => format!("create sound {name}"),
        C::UpdateSound { name, .. } => format!("update sound {name}"),
        C::DeleteSound { name, .. } => format!("delete sound {name}"),
    }
}
