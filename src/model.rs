//! The shared, normalised representation that both the spec (desired state)
//! and the live server are converted into. Everything here is already
//! normalised, so the diff can compare with `==`.

use serde::{Deserialize, Serialize};

use crate::perms::{PermContext, Perms};

pub type Snowflake = u64;

/// Discord's default for `default_auto_archive_duration` when it isn't set.
pub const DEFAULT_AUTO_ARCHIVE: u32 = 1440;
/// Discord's default voice bitrate.
pub const DEFAULT_BITRATE: u32 = 64_000;
/// Forum channel flag: posts must have at least one tag.
pub const FLAG_REQUIRE_TAG: u64 = 1 << 4;

/// Whether fields left out of the spec are left alone or reset to defaults.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Fields left out of the spec are not managed.
    #[default]
    Partial,
    /// The spec is the whole truth: a field left out means Discord's default.
    Authoritative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelKind {
    Category,
    Text,
    Announcement,
    Forum,
    Voice,
    Stage,
    /// A forum for images and videos.
    Media,
}

impl ChannelKind {
    pub fn from_wire(t: u8) -> Option<Self> {
        Some(match t {
            0 => Self::Text,
            2 => Self::Voice,
            4 => Self::Category,
            5 => Self::Announcement,
            13 => Self::Stage,
            15 => Self::Forum,
            16 => Self::Media,
            _ => return None,
        })
    }

    pub fn wire(self) -> u8 {
        match self {
            Self::Text => 0,
            Self::Voice => 2,
            Self::Category => 4,
            Self::Announcement => 5,
            Self::Stage => 13,
            Self::Forum => 15,
            Self::Media => 16,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Category => "category",
            Self::Text => "text",
            Self::Announcement => "announcement",
            Self::Forum => "forum",
            Self::Voice => "voice",
            Self::Stage => "stage",
            Self::Media => "media",
        }
    }

    /// Text, announcement and forum names are lower-cased by Discord.
    pub fn has_slug_name(self) -> bool {
        matches!(
            self,
            Self::Text | Self::Announcement | Self::Forum | Self::Media
        )
    }

    /// Forum and media channels: posts, tags, guidelines.
    pub fn is_forum_like(self) -> bool {
        matches!(self, Self::Forum | Self::Media)
    }

    /// Voice and stage channels: bitrate, user limit, region; always listed
    /// below text-like channels.
    pub fn is_voice_like(self) -> bool {
        matches!(self, Self::Voice | Self::Stage)
    }

    pub fn perm_context(self) -> PermContext {
        match self {
            Self::Category => PermContext::Category,
            Self::Text | Self::Announcement => PermContext::Text,
            Self::Forum | Self::Media => PermContext::Forum,
            Self::Voice | Self::Stage => PermContext::Voice,
        }
    }

    /// Text and announcement channels can be converted into each other in
    /// place; every other type change needs a new channel.
    pub fn convertible_to(self, other: ChannelKind) -> bool {
        self == other
            || matches!(
                (self, other),
                (Self::Text, Self::Announcement) | (Self::Announcement, Self::Text)
            )
    }
}

/// A live channel's type, including types concord doesn't manage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveKind {
    Known(ChannelKind),
    Other(u8),
}

impl LiveKind {
    pub fn from_wire(t: u8) -> Self {
        ChannelKind::from_wire(t).map_or(Self::Other(t), Self::Known)
    }

    pub fn known(self) -> Option<ChannelKind> {
        match self {
            Self::Known(k) => Some(k),
            Self::Other(_) => None,
        }
    }

    pub fn is_category(self) -> bool {
        self == Self::Known(ChannelKind::Category)
    }

    /// Voice-like channels (voice = 2, stage = 13) always sort below
    /// text-like channels in the Discord client.
    pub fn is_voice_like(self) -> bool {
        self.known().is_some_and(ChannelKind::is_voice_like)
    }

    pub fn describe(self) -> String {
        match self {
            Self::Known(k) => k.as_str().to_owned(),
            Self::Other(t) => format!("type {t}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Emoji {
    /// A unicode emoji, stored without variation selectors.
    Unicode(String),
    /// A custom guild emoji, by ID.
    Custom(Snowflake),
}

impl Emoji {
    /// Parse the spec form: a unicode emoji, `id:<snowflake>`, or a Discord
    /// mention like `<:name:id>` / `<a:name:id>`.
    pub fn parse(s: &str) -> Result<Emoji, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("emoji must not be empty".to_owned());
        }
        if let Some(id) = s.strip_prefix("id:") {
            return id
                .parse()
                .map(Emoji::Custom)
                .map_err(|_| format!("invalid custom emoji ID `{id}`"));
        }
        if s.starts_with('<') && s.ends_with('>') {
            let id = s
                .trim_end_matches('>')
                .rsplit(':')
                .next()
                .and_then(|id| id.parse().ok());
            return id
                .map(Emoji::Custom)
                .ok_or_else(|| format!("invalid custom emoji `{s}`, expected `<:name:id>`"));
        }
        if s.chars().count() > 16 || s.is_ascii() {
            return Err(format!(
                "`{s}` doesn't look like an emoji; use a unicode emoji like \"✅\" \
                 or a custom one as \"id:123456789\""
            ));
        }
        Ok(Emoji::unicode(s))
    }

    pub fn unicode(s: &str) -> Emoji {
        Emoji::Unicode(s.chars().filter(|c| *c != '\u{FE0F}').collect())
    }

    /// From Discord's `{emoji_id, emoji_name}` pair.
    pub fn from_wire(id: Option<Snowflake>, name: Option<&str>) -> Option<Emoji> {
        match (id, name) {
            (Some(id), _) if id != 0 => Some(Emoji::Custom(id)),
            (_, Some(name)) if !name.is_empty() => Some(Emoji::unicode(name)),
            _ => None,
        }
    }

    /// The spec form, as accepted by [`Emoji::parse`].
    pub fn to_spec(&self) -> String {
        match self {
            Emoji::Unicode(s) => s.clone(),
            Emoji::Custom(id) => format!("id:{id}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    #[default]
    List,
    Gallery,
}

impl Layout {
    pub fn from_wire(v: Option<u8>) -> Layout {
        if v == Some(2) {
            Layout::Gallery
        } else {
            // 0 = "not set", which the client shows as a list.
            Layout::List
        }
    }
    pub fn wire(self) -> u8 {
        match self {
            Layout::List => 1,
            Layout::Gallery => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    #[default]
    #[serde(alias = "latest_activity")]
    Activity,
    #[serde(alias = "creation", alias = "creation_date")]
    Created,
}

impl Sort {
    pub fn from_wire(v: Option<u8>) -> Sort {
        if v == Some(1) {
            Sort::Created
        } else {
            Sort::Activity
        }
    }
    pub fn wire(self) -> u8 {
        match self {
            Sort::Activity => 0,
            Sort::Created => 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Desired state (from the spec)

#[derive(Debug, Clone, Default)]
pub struct Desired {
    pub guild_id: Option<Snowflake>,
    pub mode: Mode,
    /// Highest first. `@everyone`, if present, is last.
    pub roles: Vec<DesiredRole>,
    /// Spec order: uncategorised channels, then each category followed by its
    /// channels.
    pub channels: Vec<DesiredChannel>,
    /// Server settings, onboarding, AutoMod, expressions...
    pub community: crate::community::Desired,
}

impl Desired {
    pub fn channel(&self, key: &str) -> Option<&DesiredChannel> {
        self.channels.iter().find(|c| c.key == key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredRole {
    pub key: String,
    pub path: String,
    pub name: String,
    pub everyone: bool,
    pub color: Option<u32>,
    pub hoist: Option<bool>,
    pub mentionable: Option<bool>,
    pub permissions: Option<Perms>,
    /// Unicode role icon; `Some(None)` removes it.
    pub icon: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredChannel {
    pub key: String,
    pub path: String,
    pub kind: ChannelKind,
    pub name: String,
    /// Key of the parent category.
    pub parent: Option<String>,
    pub topic: Option<String>,
    pub nsfw: Option<bool>,
    pub slowmode: Option<u32>,
    pub bitrate: Option<u32>,
    pub user_limit: Option<u32>,
    /// Voice region; `Some(None)` is automatic.
    pub region: Option<Option<String>>,
    pub video_quality: Option<u8>,
    pub forum: DesiredForum,
    pub overwrites: DesiredOverwrites,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesiredForum {
    pub tags: Option<Vec<DesiredTag>>,
    pub require_tag: Option<bool>,
    pub layout: Option<Layout>,
    pub sort: Option<Sort>,
    pub default_reaction: Option<Option<Emoji>>,
    pub auto_archive: Option<u32>,
    /// Slowmode inside each post, in seconds.
    pub post_slowmode: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredTag {
    pub key: String,
    pub name: String,
    pub emoji: Option<Emoji>,
    pub moderated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesiredOverwrites {
    /// Leave the live overwrites alone (partial mode, nothing declared).
    Unmanaged,
    /// The parent category's overwrites, with these layered on top. An empty
    /// list means "synced to category".
    Inherit(Vec<NamedOverwrite>),
    /// Exactly these overwrites.
    Explicit(Vec<NamedOverwrite>),
}

/// An overwrite that refers to its role by name (`@everyone` included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedOverwrite {
    pub role: String,
    pub allow: Perms,
    pub deny: Perms,
    pub path: String,
}

/// Layer `top` on top of `base`: for each target, bits mentioned in `top`
/// (allowed or denied) take `top`'s value; other bits keep `base`'s.
pub fn layer_overwrites<K: PartialEq + Clone>(
    base: &[(K, Perms, Perms)],
    top: &[(K, Perms, Perms)],
) -> Vec<(K, Perms, Perms)> {
    let mut out: Vec<(K, Perms, Perms)> = base.to_vec();
    for (target, allow, deny) in top {
        let mentioned = allow.union(*deny);
        match out.iter_mut().find(|(t, _, _)| t == target) {
            Some((_, a, d)) => {
                *a = a.minus(mentioned).union(*allow);
                *d = d.minus(mentioned).union(*deny);
            }
            None => out.push((target.clone(), *allow, *deny)),
        }
    }
    out.retain(|(_, a, d)| !(a.is_empty() && d.is_empty()));
    out
}

// ---------------------------------------------------------------------------
// Live state (from Discord)

#[derive(Debug, Clone, Default)]
pub struct LiveState {
    pub guild: LiveGuild,
    pub roles: Vec<LiveRole>,
    pub channels: Vec<LiveChannel>,
    pub bot: Option<BotInfo>,
    pub community: crate::community::Live,
}

impl LiveState {
    pub fn role(&self, id: Snowflake) -> Option<&LiveRole> {
        self.roles.iter().find(|r| r.id == id)
    }
    pub fn channel(&self, id: Snowflake) -> Option<&LiveChannel> {
        self.channels.iter().find(|c| c.id == id)
    }
    pub fn role_name(&self, id: Snowflake) -> String {
        if id == self.guild.id {
            return "@everyone".to_owned();
        }
        self.role(id)
            .map_or_else(|| format!("<role {id}>"), |r| r.name.clone())
    }
    pub fn protected_reason(&self, id: Snowflake) -> Option<&'static str> {
        self.guild
            .protected_channels
            .iter()
            .find(|(c, _)| *c == id)
            .map(|(_, why)| *why)
    }
}

#[derive(Debug, Clone, Default)]
pub struct LiveGuild {
    pub id: Snowflake,
    pub name: String,
    pub owner_id: Option<Snowflake>,
    /// Discord feature flags, e.g. `COMMUNITY`, `ROLE_ICONS`.
    pub features: Vec<String>,
    /// Channels Discord won't let us delete (rules, community updates, ...).
    pub protected_channels: Vec<(Snowflake, &'static str)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRole {
    pub id: Snowflake,
    pub name: String,
    pub color: u32,
    /// Uses Discord's gradient/holographic colours; only the primary colour is
    /// managed.
    pub gradient: bool,
    pub hoist: bool,
    pub mentionable: bool,
    pub permissions: Perms,
    pub position: i64,
    /// Owned by an integration (bot role, booster role); can't be edited.
    pub managed: bool,
    /// Unicode role icon.
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveChannel {
    pub id: Snowflake,
    pub kind: LiveKind,
    pub name: String,
    pub parent_id: Option<Snowflake>,
    pub position: i64,
    pub topic: String,
    pub nsfw: bool,
    pub slowmode: u32,
    pub bitrate: u32,
    pub user_limit: u32,
    pub region: Option<String>,
    pub video_quality: u8,
    pub flags: u64,
    pub overwrites: Vec<Overwrite>,
    pub forum: LiveForum,
}

impl LiveChannel {
    /// Role overwrites only, sorted by target, empty ones dropped.
    pub fn role_overwrites(&self) -> Vec<(Snowflake, Perms, Perms)> {
        let mut v: Vec<_> = self
            .overwrites
            .iter()
            .filter(|o| o.kind == OverwriteKind::Role && !(o.allow.is_empty() && o.deny.is_empty()))
            .map(|o| (o.id, o.allow, o.deny))
            .collect();
        v.sort_by_key(|(id, _, _)| *id);
        v
    }

    pub fn member_overwrites(&self) -> Vec<Overwrite> {
        self.overwrites
            .iter()
            .filter(|o| o.kind == OverwriteKind::Member)
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveForum {
    pub tags: Vec<LiveTag>,
    pub require_tag: bool,
    pub layout: Layout,
    pub sort: Sort,
    pub default_reaction: Option<Emoji>,
    pub auto_archive: u32,
    pub post_slowmode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTag {
    pub id: Snowflake,
    pub name: String,
    pub emoji: Option<Emoji>,
    pub moderated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteKind {
    Role,
    Member,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overwrite {
    pub id: Snowflake,
    pub kind: OverwriteKind,
    pub allow: Perms,
    pub deny: Perms,
}

/// Who concord is running as, for hierarchy and permission checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotInfo {
    pub user_id: Snowflake,
    pub role_ids: Vec<Snowflake>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layering_overrides_only_mentioned_bits() {
        let view = Perms::VIEW_CHANNEL;
        let send = Perms::SEND_MESSAGES;
        let base = vec![("everyone", Perms::NONE, view.union(send))];
        let top = vec![("everyone", view, Perms::NONE), ("lead", send, Perms::NONE)];
        let out = layer_overwrites(&base, &top);
        assert_eq!(
            out,
            vec![("everyone", view, send), ("lead", send, Perms::NONE)]
        );
    }

    #[test]
    fn emoji_normalises_variation_selectors() {
        assert_eq!(
            Emoji::parse("✅").unwrap(),
            Emoji::parse("✅\u{FE0F}").unwrap()
        );
        assert_eq!(Emoji::parse("<:team:123>").unwrap(), Emoji::Custom(123));
        assert_eq!(Emoji::parse("id:42").unwrap(), Emoji::Custom(42));
        assert!(Emoji::parse("check").is_err());
    }
}
