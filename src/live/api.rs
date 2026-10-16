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
    /// Server settings.
    UpdateGuild,
    UpdateWidget,
    PutOnboarding,
    UpdateWelcomeScreen,
    /// Join questions (undocumented).
    UpdateJoinQuestions,
    CreateAutomod,
    UpdateAutomod(u64),
    DeleteAutomod(u64),
    CreateEmoji,
    UpdateEmoji(u64),
    DeleteEmoji(u64),
    /// Multipart: the body's fields plus `file`.
    CreateSticker,
    UpdateSticker(u64),
    DeleteSticker(u64),
    CreateSound,
    UpdateSound(u64),
    DeleteSound(u64),
}

impl Endpoint {
    /// Method and path (after `/api/v10/`), for endpoints sent as raw
    /// requests and for messages.
    pub fn method_path(self, guild: u64) -> (&'static str, String) {
        use Endpoint::*;
        let g = format!("guilds/{guild}");
        match self {
            CreateRole => ("POST", format!("{g}/roles")),
            UpdateRole(id) => ("PATCH", format!("{g}/roles/{id}")),
            RolePositions => ("PATCH", format!("{g}/roles")),
            DeleteRole(id) => ("DELETE", format!("{g}/roles/{id}")),
            CreateChannel => ("POST", format!("{g}/channels")),
            UpdateChannel(id) => ("PATCH", format!("channels/{id}")),
            ChannelPositions => ("PATCH", format!("{g}/channels")),
            DeleteChannel(id) => ("DELETE", format!("channels/{id}")),
            UpdateGuild => ("PATCH", g),
            UpdateWidget => ("PATCH", format!("{g}/widget")),
            PutOnboarding => ("PUT", format!("{g}/onboarding")),
            UpdateWelcomeScreen => ("PATCH", format!("{g}/welcome-screen")),
            UpdateJoinQuestions => ("PATCH", format!("{g}/member-verification")),
            CreateAutomod => ("POST", format!("{g}/auto-moderation/rules")),
            UpdateAutomod(id) => ("PATCH", format!("{g}/auto-moderation/rules/{id}")),
            DeleteAutomod(id) => ("DELETE", format!("{g}/auto-moderation/rules/{id}")),
            CreateEmoji => ("POST", format!("{g}/emojis")),
            UpdateEmoji(id) => ("PATCH", format!("{g}/emojis/{id}")),
            DeleteEmoji(id) => ("DELETE", format!("{g}/emojis/{id}")),
            CreateSticker => ("POST", format!("{g}/stickers")),
            UpdateSticker(id) => ("PATCH", format!("{g}/stickers/{id}")),
            DeleteSticker(id) => ("DELETE", format!("{g}/stickers/{id}")),
            CreateSound => ("POST", format!("{g}/soundboard-sounds")),
            UpdateSound(id) => ("PATCH", format!("{g}/soundboard-sounds/{id}")),
            DeleteSound(id) => ("DELETE", format!("{g}/soundboard-sounds/{id}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WriteRequest {
    pub endpoint: Endpoint,
    /// JSON body; `None` for deletes. For multipart endpoints, the text
    /// fields.
    pub body: Option<Value>,
    /// A file to send as multipart form data.
    pub file: Option<FilePart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePart {
    pub filename: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
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
