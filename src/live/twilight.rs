//! [`DiscordApi`] over twilight-http. twilight handles rate limits (429 and
//! `retry_after`); requests are built from raw JSON because twilight's typed
//! builders can't express everything we send (new forum tags have no ID).

use http::header::{HeaderName, HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::Value;
use twilight_http::error::ErrorType;
use twilight_http::request::Request;
use twilight_http::routing::Route;
use twilight_http::Client;

use super::api::{ApiError, DiscordApi, Endpoint, WriteRequest};
use super::wire::{Snapshot, WireBot, WireChannel, WireGuild, WireMember, WireRole, WireUser};

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
}

fn route(guild_id: u64, endpoint: Endpoint) -> Route<'static> {
    match endpoint {
        Endpoint::CreateRole => Route::CreateRole { guild_id },
        Endpoint::UpdateRole(role_id) => Route::UpdateRole { guild_id, role_id },
        Endpoint::RolePositions => Route::UpdateRolePositions { guild_id },
        Endpoint::DeleteRole(role_id) => Route::DeleteRole { guild_id, role_id },
        Endpoint::CreateChannel => Route::CreateChannel { guild_id },
        Endpoint::UpdateChannel(channel_id) => Route::UpdateChannel { channel_id },
        Endpoint::ChannelPositions => Route::UpdateGuildChannels { guild_id },
        Endpoint::DeleteChannel(channel_id) => Route::DeleteChannel { channel_id },
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
            let detail = serde_json::from_slice::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_owned))
                .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());
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
        Ok(Snapshot {
            guild,
            roles,
            channels,
            bot: Some(WireBot { user, member }),
        })
    }

    async fn write(&self, guild_id: u64, request: &WriteRequest) -> Result<Value, ApiError> {
        let route = route(guild_id, request.endpoint);
        // Shows up in the server's audit log next to every change.
        let reason = (
            HeaderName::from_static("x-audit-log-reason"),
            HeaderValue::from_static("concord apply"),
        );
        let mut builder = Request::builder(&route).headers(std::iter::once(reason));
        if let Some(body) = &request.body {
            builder = builder.json(body);
        }
        let built = builder.build().map_err(|e| ApiError::Transport {
            method: route.method().name(),
            path: route.to_string(),
            message: e.to_string(),
        })?;
        self.send(built).await
    }
}
