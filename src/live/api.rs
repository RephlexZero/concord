//! The HTTP layer, behind a trait so tests can use a fake.

use std::future::Future;

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

/// Discord REST operations concord needs.
pub trait DiscordApi {
    /// Read everything concord manages, plus who the bot is.
    fn snapshot(&self, guild_id: u64) -> impl Future<Output = Result<Snapshot, ApiError>> + Send;
}
