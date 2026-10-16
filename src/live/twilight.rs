//! [`DiscordApi`] over twilight-http. twilight handles rate limits (429 and
//! `retry_after`); requests are built from raw JSON because twilight's typed
//! builders can't express everything we send (new forum tags have no ID).
//! Endpoints twilight has no route for (the Server Guide, join questions,
//! the soundboard) are sent as raw requests.

use http::header::{HeaderName, HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::Value;
use twilight_http::error::ErrorType;
use twilight_http::request::{Form, Method, Request, RequestBuilder};
use twilight_http::routing::Route;
use twilight_http::Client;

use super::api::{ApiError, DiscordApi, Endpoint, WriteRequest};
use super::wire::{Snapshot, WireBot, WireChannel, WireGuild, WireMember, WireRole, WireUser};
use crate::community::wire::*;

pub struct TwilightApi {
    client: Client,
    verbose: bool,
}

impl TwilightApi {
    /// `token` is the raw bot token; it is never logged (twilight's `Debug`
    /// redacts it too).
    pub fn new(token: String, verbose: bool) -> Self {
        TwilightApi {
            client: Client::new(token),
            verbose,
        }
    }

    async fn send<T: DeserializeOwned>(&self, request: Request) -> Result<T, ApiError> {
        let method = request.method().name();
        let path = request.path().to_owned();
        if self.verbose {
            eprintln!("  → {method} /{path}");
        }
        let response = self
            .client
            .request::<()>(request)
            .await
            .map_err(|e| map_error(e, method, &path))?;
        let bytes = response.bytes().await.map_err(|e| ApiError::Transport {
            method,
            path: path.clone(),
            message: e.to_string(),
        })?;
        // 204 No Content (positions, role deletes) has an empty body.
        let bytes: &[u8] = if bytes.is_empty() { b"null" } else { &bytes };
        serde_json::from_slice(bytes).map_err(|e| ApiError::Decode {
            path,
            message: e.to_string(),
        })
    }

    async fn get<T: DeserializeOwned>(&self, route: Route<'_>) -> Result<T, ApiError> {
        self.send(Request::from_route(&route)).await
    }

    async fn get_raw<T: DeserializeOwned>(&self, path: String) -> Result<T, ApiError> {
        let request = RequestBuilder::raw(Method::Get, path)
            .build()
            .map_err(|e| ApiError::Transport {
                method: "GET",
                path: String::new(),
                message: e.to_string(),
            })?;
        self.send(request).await
    }

    /// Read everything beyond roles and channels. Parts that fail are
    /// recorded in `errors` instead of failing the snapshot, so a server
    /// without, say, a Server Guide can still be planned.
    async fn community(&self, guild_id: u64, guild: &WireGuild) -> WireCommunity {
        let mut out = WireCommunity::default();
        macro_rules! read {
            ($field:ident, $what:expr, $fut:expr) => {
                match $fut.await {
                    Ok(v) => out.$field = Some(v),
                    Err(e) => out.errors.push(($what.to_owned(), short(&e))),
                }
            };
        }
        let g = format!("guilds/{guild_id}");
        read!(
            onboarding,
            "onboarding",
            self.get(Route::GetGuildOnboarding { guild_id })
        );
        match self
            .get::<WireWelcomeScreen>(Route::GetGuildWelcomeScreen { guild_id })
            .await
        {
            Ok(mut w) => {
                w.enabled = guild.features.iter().any(|f| f == "WELCOME_SCREEN_ENABLED");
                out.welcome_screen = Some(w);
            }
            // Never set up: an empty, disabled welcome screen.
            Err(ApiError::Status { status: 404, .. }) => {
                out.welcome_screen = Some(WireWelcomeScreen::default())
            }
            Err(e) => out.errors.push(("welcome screen".to_owned(), short(&e))),
        }
        read!(
            guide,
            "Server Guide",
            self.get_raw(format!("{g}/new-member-welcome"))
        );
        read!(
            verification,
            "join questions",
            self.get_raw(format!("{g}/member-verification?with_guild=false"))
        );
        read!(
            automod,
            "AutoMod rules",
            self.get(Route::GetGuildAutoModerationRules { guild_id })
        );
        read!(emojis, "emoji", self.get(Route::GetEmojis { guild_id }));
        read!(
            stickers,
            "stickers",
            self.get(Route::GetGuildStickers { guild_id })
        );
        match self
            .get_raw::<Value>(format!("{g}/soundboard-sounds"))
            .await
        {
            Ok(v) => {
                let items = v.get("items").cloned().unwrap_or(Value::Array(Vec::new()));
                match serde_json::from_value(items) {
                    Ok(s) => out.sounds = Some(s),
                    Err(e) => out.errors.push(("soundboard".to_owned(), e.to_string())),
                }
            }
            Err(e) => out.errors.push(("soundboard".to_owned(), short(&e))),
        }
        read!(
            widget,
            "widget",
            self.get(Route::GetGuildWidgetSettings { guild_id })
        );
        out
    }
}

fn short(e: &ApiError) -> String {
    match e {
        ApiError::Status {
            status, message, ..
        } => format!("{status}: {message}"),
        e => e.to_string(),
    }
}

/// twilight's route for an endpoint, where it has one.
fn route(guild_id: u64, endpoint: Endpoint) -> Option<Route<'static>> {
    use Endpoint as E;
    Some(match endpoint {
        E::CreateRole => Route::CreateRole { guild_id },
        E::UpdateRole(role_id) => Route::UpdateRole { guild_id, role_id },
        E::RolePositions => Route::UpdateRolePositions { guild_id },
        E::DeleteRole(role_id) => Route::DeleteRole { guild_id, role_id },
        E::CreateChannel => Route::CreateChannel { guild_id },
        E::UpdateChannel(channel_id) => Route::UpdateChannel { channel_id },
        E::ChannelPositions => Route::UpdateGuildChannels { guild_id },
        E::DeleteChannel(channel_id) => Route::DeleteChannel { channel_id },
        E::UpdateGuild => Route::UpdateGuild { guild_id },
        E::UpdateWidget => Route::UpdateGuildWidgetSettings { guild_id },
        E::PutOnboarding => Route::UpdateGuildOnboarding { guild_id },
        E::UpdateWelcomeScreen => Route::UpdateGuildWelcomeScreen { guild_id },
        E::CreateAutomod => Route::CreateAutoModerationRule { guild_id },
        E::UpdateAutomod(id) => Route::UpdateAutoModerationRule {
            guild_id,
            auto_moderation_rule_id: id,
        },
        E::DeleteAutomod(id) => Route::DeleteAutoModerationRule {
            guild_id,
            auto_moderation_rule_id: id,
        },
        E::CreateEmoji => Route::CreateEmoji { guild_id },
        E::UpdateEmoji(emoji_id) => Route::UpdateEmoji { guild_id, emoji_id },
        E::DeleteEmoji(emoji_id) => Route::DeleteEmoji { guild_id, emoji_id },
        E::CreateSticker => Route::CreateGuildSticker { guild_id },
        E::UpdateSticker(sticker_id) => Route::UpdateGuildSticker {
            guild_id,
            sticker_id,
        },
        E::DeleteSticker(sticker_id) => Route::DeleteGuildSticker {
            guild_id,
            sticker_id,
        },
        E::UpdateJoinQuestions | E::CreateSound | E::UpdateSound(_) | E::DeleteSound(_) => {
            return None
        }
    })
}

fn method(name: &str) -> Method {
    match name {
        "POST" => Method::Post,
        "PUT" => Method::Put,
        "PATCH" => Method::Patch,
        "DELETE" => Method::Delete,
        _ => Method::Get,
    }
}

fn map_error(err: twilight_http::Error, method: &'static str, path: &str) -> ApiError {
    match err.kind() {
        ErrorType::Unauthorized => ApiError::Unauthorized,
        ErrorType::Response { status, body, .. } => {
            let status = status.get();
            if status == 401 {
                return ApiError::Unauthorized;
            }
            let json = serde_json::from_slice::<serde_json::Value>(body).ok();
            let mut detail = json
                .as_ref()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_owned))
                .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());
            // Validation errors say which field was wrong under `errors`.
            if let Some(errors) = json.as_ref().and_then(|v| v.get("errors")) {
                detail.push_str(&format!(" {errors}"));
            }
            let message = match status {
                403 => format!("{detail} (does the bot have the permissions it needs, and is it in the server?)"),
                404 => format!("{detail} (wrong guild ID, or the bot isn't in that server?)"),
                _ => detail,
            };
            ApiError::Status {
                status,
                method,
                path: path.to_owned(),
                message,
            }
        }
        _ => ApiError::Transport {
            method,
            path: path.to_owned(),
            message: err.to_string(),
        },
    }
}

impl DiscordApi for TwilightApi {
    async fn snapshot(&self, guild_id: u64) -> Result<Snapshot, ApiError> {
        let guild: WireGuild = self
            .get(Route::GetGuild {
                guild_id,
                with_counts: false,
            })
            .await?;
        let roles: Vec<WireRole> = self.get(Route::GetGuildRoles { guild_id }).await?;
        let channels: Vec<WireChannel> = self.get(Route::GetChannels { guild_id }).await?;
        let user: WireUser = self.get(Route::GetCurrentUser).await?;
        let member: WireMember = self
            .get(Route::GetMember {
                guild_id,
                user_id: user.id.0,
            })
            .await?;
        let community = self.community(guild_id, &guild).await;
        Ok(Snapshot {
            guild,
            roles,
            channels,
            bot: Some(WireBot { user, member }),
            community: Some(community),
        })
    }

    async fn write(&self, guild_id: u64, request: &WriteRequest) -> Result<Value, ApiError> {
        let (method_name, path) = request.endpoint.method_path(guild_id);
        let builder = match route(guild_id, request.endpoint) {
            Some(r) => Request::builder(&r),
            None => RequestBuilder::raw(method(method_name), path.clone()),
        };
        // Shows up in the server's audit log next to every change.
        let reason = (
            HeaderName::from_static("x-audit-log-reason"),
            HeaderValue::from_static("concord apply"),
        );
        let mut builder = builder.headers(std::iter::once(reason));
        match (&request.file, &request.body) {
            (Some(file), body) => {
                let mut form = Form::new();
                for (k, v) in body
                    .as_ref()
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                {
                    let text = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
                    form = form.part(k.as_bytes(), text.as_bytes());
                }
                form = form.file_part(b"file", file.filename.as_bytes(), &file.bytes);
                builder = builder.form(form);
            }
            (None, Some(body)) => builder = builder.json(body),
            (None, None) => {}
        }
        let built = builder.build().map_err(|e| ApiError::Transport {
            method: method_name,
            path,
            message: e.to_string(),
        })?;
        self.send(built).await
    }
}
