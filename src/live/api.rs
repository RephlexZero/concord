//! The HTTP layer, behind a trait so tests can use a fake.

use std::future::Future;

use serde_json::Value;

use super::wire::Snapshot;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Discord rejected the token (401). Check DISCORD_TOKEN.")]
    Unauthorized,
    #[error("Discord returned {status} for {method} /{path}: {message}")]
    Status {
        status: u16,
        method: &'static str,
        path: String,
        message: String,
    },
    #[error("request {method} /{path} failed: {message}")]
    Transport {
        method: &'static str,
        path: String,
        message: String,
    },
    #[error("couldn't understand Discord's response to /{path}: {message}")]
    Decode { path: String, message: String },
}

/// The endpoints `apply` writes to. IDs are channel or role IDs; the guild ID
/// is passed separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    CreateRole,
    UpdateRole(u64),
    RolePositions,
    DeleteRole(u64),
    CreateChannel,
    UpdateChannel(u64),
    ChannelPositions,
    DeleteChannel(u64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WriteRequest {
    pub endpoint: Endpoint,
    /// JSON body; `None` for deletes.
    pub body: Option<Value>,
}

/// Discord REST operations concord needs.
pub trait DiscordApi {
    /// Read everything concord manages, plus who the bot is.
    fn snapshot(&self, guild_id: u64) -> impl Future<Output = Result<Snapshot, ApiError>> + Send;

    /// Send one write. Returns the response body (`Null` when Discord sends
    /// none).
    fn write(
        &self,
        guild_id: u64,
        request: &WriteRequest,
    ) -> impl Future<Output = Result<Value, ApiError>> + Send;
}
