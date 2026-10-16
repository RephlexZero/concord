//! Discord's JSON for server-wide configuration. Lenient like the rest of
//! the wire types: unknown fields are ignored, missing ones defaulted.
//! Fields concord doesn't manage (onboarding responses, guide icons, ...)
//! are kept so they can be sent back unchanged.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::snowflake::Sf;

/// Everything beyond roles and channels, as read in one snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireCommunity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding: Option<WireOnboarding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome_screen: Option<WireWelcomeScreen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide: Option<WireGuide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<WireVerification>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automod: Option<Vec<WireAutomod>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emojis: Option<Vec<WireEmoji>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stickers: Option<Vec<WireSticker>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sounds: Option<Vec<WireSound>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<WireWidget>,
    /// (what, why) for each part that couldn't be read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<(String, String)>,
}

/// An emoji reference as onboarding and the Server Guide send it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEmojiRef {
    #[serde(default)]
    pub id: Option<Sf>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animated: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireOnboarding {
    #[serde(default)]
    pub prompts: Vec<WirePrompt>,
    #[serde(default)]
    pub default_channel_ids: Vec<Sf>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: u8,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WirePrompt {
    pub id: Sf,
    #[serde(rename = "type", default)]
    pub kind: u8,
    #[serde(default)]
    pub options: Vec<WireOption>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub single_select: bool,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub in_onboarding: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireOption {
    pub id: Sf,
    #[serde(default)]
    pub channel_ids: Vec<Sf>,
    #[serde(default)]
    pub role_ids: Vec<Sf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<WireEmojiRef>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireWelcomeScreen {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub welcome_channels: Vec<WireWelcomeChannel>,
    /// Not in Discord's response (it's the `WELCOME_SCREEN_ENABLED` guild
    /// feature); filled in by concord.
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireWelcomeChannel {
    pub channel_id: Sf,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub emoji_id: Option<Sf>,
    #[serde(default)]
    pub emoji_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireGuide {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub welcome_message: Option<WireGuideWelcome>,
    #[serde(default)]
    pub new_member_actions: Vec<WireGuideItem>,
    #[serde(default)]
    pub resource_channels: Vec<WireGuideItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireGuideWelcome {
    #[serde(default)]
    pub author_ids: Vec<Sf>,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireGuideItem {
    pub channel_id: Sf,
    /// Actions only: 0 = view, 1 = chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_type: Option<u8>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<WireEmojiRef>,
    /// An uploaded image; not managed, but kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireVerification {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub form_fields: Vec<WireFormField>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireFormField {
    pub field_type: String,
    #[serde(default)]
    pub label: String,
    /// `TERMS`: the rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    /// `MULTIPLE_CHOICE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireAutomod {
    pub id: Sf,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub event_type: u8,
    #[serde(default)]
    pub trigger_type: u8,
    #[serde(default)]
    pub trigger_metadata: WireTriggerMeta,
    #[serde(default)]
    pub actions: Vec<WireAutomodAction>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub exempt_roles: Vec<Sf>,
    #[serde(default)]
    pub exempt_channels: Vec<Sf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireTriggerMeta {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyword_filter: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regex_patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_list: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mention_total_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mention_raid_protection_enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireAutomodAction {
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default)]
    pub metadata: WireActionMeta,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireActionMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<Sf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_message: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireEmoji {
    #[serde(default)]
    pub id: Option<Sf>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub roles: Vec<Sf>,
    #[serde(default)]
    pub managed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireSticker {
    pub id: Sf,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// The related emoji.
    #[serde(default)]
    pub tags: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireSound {
    pub sound_id: Sf,
    #[serde(default)]
    pub name: String,
    #[serde(default = "one")]
    pub volume: f64,
    #[serde(default)]
    pub emoji_id: Option<Sf>,
    #[serde(default)]
    pub emoji_name: Option<String>,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireWidget {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub channel_id: Option<Sf>,
}
