//! Discord's JSON shapes, as returned by the REST API. Lenient: unknown
//! fields are ignored, so new Discord fields don't break us.

use serde::{Deserialize, Serialize};

use crate::snowflake::Sf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireGuild {
    pub id: Sf,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<Sf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules_channel_id: Option<Sf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_updates_channel_id: Option<Sf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_alerts_channel_id: Option<Sf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_channel_id: Option<Sf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub afk_channel_id: Option<Sf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_message_notifications: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_content_filter: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mfa_level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub premium_progress_bar_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub afk_timeout: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_channel_flags: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireRole {
    pub id: Sf,
    pub name: String,
    #[serde(default)]
    pub color: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<WireRoleColors>,
    #[serde(default)]
    pub hoist: bool,
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub permissions: String,
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub mentionable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unicode_emoji: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireRoleColors {
    #[serde(default)]
    pub primary_color: u32,
    #[serde(default)]
    pub secondary_color: Option<u32>,
    #[serde(default)]
    pub tertiary_color: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireChannel {
    pub id: Sf,
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Sf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nsfw: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_user: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_overwrites: Option<Vec<WireOverwrite>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_tags: Option<Vec<WireTag>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reaction_emoji: Option<WireReaction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_sort_order: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_forum_layout: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_auto_archive_duration: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtc_region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_quality_mode: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_thread_rate_limit_per_user: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireOverwrite {
    pub id: Sf,
    /// 0 = role, 1 = member.
    #[serde(rename = "type")]
    pub kind: u8,
    pub allow: String,
    pub deny: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireTag {
    /// Absent when creating a new tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Sf>,
    pub name: String,
    #[serde(default)]
    pub moderated: bool,
    #[serde(default)]
    pub emoji_id: Option<Sf>,
    #[serde(default)]
    pub emoji_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireReaction {
    #[serde(default)]
    pub emoji_id: Option<Sf>,
    #[serde(default)]
    pub emoji_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireUser {
    pub id: Sf,
    #[serde(default)]
    pub username: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireMember {
    #[serde(default)]
    pub roles: Vec<Sf>,
}

/// Who concord is running as.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireBot {
    pub user: WireUser,
    pub member: WireMember,
}

/// Everything concord reads from a server, in Discord's own shapes. Used for
/// test fixtures, `--state-file`, and the backup written before each apply.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub guild: WireGuild,
    pub roles: Vec<WireRole>,
    pub channels: Vec<WireChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot: Option<WireBot>,
    /// Onboarding, AutoMod, expressions... `None` in snapshots that don't
    /// include them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community: Option<crate::community::wire::WireCommunity>,
}
