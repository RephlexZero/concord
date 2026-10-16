//! Normalised server-wide configuration. Most types are generic over how
//! they refer to channels (`C`) and roles (`R`):
//!
//! - desired, straight from the spec: channel keys and role names (`String`);
//! - resolved, ready to send: [`Ref`]s, which may be created in the same run;
//! - live, from Discord: IDs, converted to `Ref::Id` for comparison.

use crate::diff::Ref;
use crate::model::Emoji;

/// The desired state of everything beyond roles and channels. `None`: the
/// spec doesn't mention it, so it's left alone.
#[derive(Debug, Clone, Default)]
pub struct Desired {
    pub settings: Option<Settings>,
    pub widget: Option<Widget<String>>,
    pub onboarding: Option<Onboarding<String, String>>,
    pub welcome_screen: Option<WelcomeScreen<String>>,
    pub guide: Option<Guide<String>>,
    pub join_questions: Option<JoinQuestions>,
    pub automod: Option<Vec<Automod<String, String>>>,
    pub emoji: Option<Vec<DesiredEmoji>>,
    pub stickers: Option<Vec<DesiredSticker>>,
    pub sounds: Option<Vec<DesiredSound>>,
}

impl Desired {
    /// Every role name these sections refer to.
    pub fn role_names(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for p in self.onboarding.iter().flat_map(|o| &o.prompts) {
            for x in &p.options {
                out.extend(x.roles.iter().map(String::as_str));
            }
        }
        for r in self.automod.iter().flatten() {
            out.extend(r.exempt_roles.iter().map(String::as_str));
        }
        for e in self.emoji.iter().flatten() {
            out.extend(e.roles.iter().map(String::as_str));
        }
        out
    }
}

/// What concord read from Discord. `None` for parts it couldn't read (the
/// reason is in `errors`).
#[derive(Debug, Clone, Default)]
pub struct Live {
    pub settings: LiveSettings,
    pub widget: Option<Widget<u64>>,
    pub onboarding: Option<Onboarding<u64, u64>>,
    pub welcome_screen: Option<WelcomeScreen<u64>>,
    pub guide: Option<Guide<u64>>,
    pub join_questions: Option<(String, JoinQuestions)>,
    pub automod: Option<Vec<Automod<u64, u64>>>,
    pub emoji: Option<Vec<LiveEmoji>>,
    pub stickers: Option<Vec<LiveSticker>>,
    pub sounds: Option<Vec<LiveSound>>,
    /// (what, why) for each part that couldn't be read.
    pub errors: Vec<(String, String)>,
}

// -- settings ----------------------------------------------------------------

/// A channel the server points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelSetting {
    System,
    Rules,
    Updates,
    SafetyAlerts,
    Afk,
}

impl ChannelSetting {
    pub const ALL: [ChannelSetting; 5] = [
        ChannelSetting::System,
        ChannelSetting::Rules,
        ChannelSetting::Updates,
        ChannelSetting::SafetyAlerts,
        ChannelSetting::Afk,
    ];

    pub fn wire(self) -> &'static str {
        match self {
            ChannelSetting::System => "system_channel_id",
            ChannelSetting::Rules => "rules_channel_id",
            ChannelSetting::Updates => "public_updates_channel_id",
            ChannelSetting::SafetyAlerts => "safety_alerts_channel_id",
            ChannelSetting::Afk => "afk_channel_id",
        }
    }

    pub fn spec(self) -> &'static str {
        match self {
            ChannelSetting::System => "system_channel",
            ChannelSetting::Rules => "rules_channel",
            ChannelSetting::Updates => "updates_channel",
            ChannelSetting::SafetyAlerts => "safety_alerts_channel",
            ChannelSetting::Afk => "afk_channel",
        }
    }

    /// Discord requires these on Community servers.
    pub fn required_for_community(self) -> bool {
        matches!(self, ChannelSetting::Rules | ChannelSetting::Updates)
    }
}

/// System channel flags are *suppress* bits: set means "don't post".
pub mod system_flags {
    pub const JOIN: u64 = 1 << 0;
    pub const BOOSTS: u64 = 1 << 1;
    pub const SETUP_TIPS: u64 = 1 << 2;
    pub const JOIN_REPLIES: u64 = 1 << 3;
    pub const ROLE_SUBSCRIPTIONS: u64 = 1 << 4;
    pub const ROLE_SUBSCRIPTION_REPLIES: u64 = 1 << 5;

    pub const NAMES: [(u64, &str); 6] = [
        (JOIN, "join"),
        (JOIN_REPLIES, "join_replies"),
        (BOOSTS, "boosts"),
        (SETUP_TIPS, "setup_tips"),
        (ROLE_SUBSCRIPTIONS, "role_subscriptions"),
        (ROLE_SUBSCRIPTION_REPLIES, "role_subscription_replies"),
    ];
}

/// Desired server settings; `None` fields are left alone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub path: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub verification_level: Option<u8>,
    pub default_notifications: Option<u8>,
    pub content_filter: Option<u8>,
    pub locale: Option<String>,
    pub boost_progress_bar: Option<bool>,
    pub afk_timeout: Option<u32>,
    /// (bits managed, bits that should be set) of the suppress flags.
    pub system_flags: Option<(u64, u64)>,
    /// Channel key, or `None` for "no channel".
    pub channels: Vec<(ChannelSetting, Option<String>)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveSettings {
    pub name: String,
    pub description: String,
    pub verification_level: u8,
    pub default_notifications: u8,
    pub content_filter: u8,
    pub locale: String,
    pub boost_progress_bar: bool,
    pub afk_timeout: u32,
    pub system_flags: u64,
    pub mfa_level: u8,
    pub channels: Vec<(ChannelSetting, Option<u64>)>,
}

impl LiveSettings {
    pub fn channel(&self, s: ChannelSetting) -> Option<u64> {
        self.channels
            .iter()
            .find(|(k, _)| *k == s)
            .and_then(|(_, v)| *v)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widget<C> {
    pub path: String,
    pub enabled: bool,
    /// `None` inside: no channel. Outer `None` (desired only): leave alone.
    pub channel: Option<Option<C>>,
}

// -- onboarding --------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Onboarding<C, R> {
    pub path: String,
    pub enabled: bool,
    /// 0 = default, 1 = advanced.
    pub mode: u8,
    pub default_channels: Vec<C>,
    pub prompts: Vec<Prompt<C, R>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prompt<C, R> {
    pub path: String,
    /// Live, or matched to a live prompt.
    pub id: Option<u64>,
    pub key: String,
    pub title: String,
    /// 0 = multiple choice, 1 = dropdown.
    pub kind: u8,
    pub single: bool,
    pub required: bool,
    pub in_onboarding: bool,
    pub options: Vec<PromptOption<C, R>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptOption<C, R> {
    pub path: String,
    pub id: Option<u64>,
    pub key: String,
    pub title: String,
    pub description: String,
    pub emoji: Option<Emoji>,
    pub roles: Vec<R>,
    pub channels: Vec<C>,
}

// -- welcome screen and Server Guide -------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeScreen<C> {
    pub path: String,
    pub enabled: bool,
    pub description: String,
    pub channels: Vec<WelcomeChannel<C>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeChannel<C> {
    pub channel: C,
    pub description: String,
    pub emoji: Option<Emoji>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guide<C> {
    pub path: String,
    pub enabled: bool,
    /// (message, author user IDs).
    pub welcome: Option<(String, Vec<u64>)>,
    pub actions: Vec<GuideItem<C>>,
    pub resources: Vec<GuideItem<C>>,
}

/// A Server Guide to-do (`kind` 0 = view, 1 = chat) or resource (`kind` 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideItem<C> {
    pub channel: C,
    pub title: String,
    pub description: String,
    pub kind: u8,
    pub emoji: Option<Emoji>,
    /// An image uploaded in the client: not managed, but sent back.
    pub icon: Option<serde_json::Value>,
}

// -- join questions ------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JoinQuestions {
    pub path: String,
    pub description: String,
    pub rules: Vec<String>,
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// 0 = text, 1 = paragraph, 2 = choice.
    pub kind: u8,
    pub label: String,
    pub required: bool,
    pub description: String,
    pub placeholder: String,
    pub choices: Vec<String>,
}

// -- AutoMod -------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Automod<C, R> {
    pub path: String,
    pub id: Option<u64>,
    pub key: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: u8,
    /// 1 = message sent, 2 = member profile updated.
    pub event: u8,
    pub keywords: Vec<String>,
    pub regex: Vec<String>,
    pub allow: Vec<String>,
    pub presets: Vec<u8>,
    pub mention_limit: Option<u32>,
    pub raid_protection: Option<bool>,
    pub actions: Vec<AutomodAction<C>>,
    pub exempt_roles: Vec<R>,
    pub exempt_channels: Vec<C>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomodAction<C> {
    Block(Option<String>),
    Alert(C),
    Timeout(u32),
    BlockInteractions,
}

impl<C> AutomodAction<C> {
    pub fn wire_type(&self) -> u8 {
        match self {
            AutomodAction::Block(_) => 1,
            AutomodAction::Alert(_) => 2,
            AutomodAction::Timeout(_) => 3,
            AutomodAction::BlockInteractions => 4,
        }
    }
}

// -- expressions ---------------------------------------------------------------

/// A file to upload, read when the spec is loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// As written in the spec, for messages.
    pub file: String,
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    /// Hex SHA-256, kept in the lockfile to notice when the file changes.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredEmoji {
    pub path: String,
    pub key: String,
    pub name: String,
    pub file: Option<Asset>,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveEmoji {
    pub id: u64,
    pub name: String,
    pub roles: Vec<u64>,
    pub managed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredSticker {
    pub path: String,
    pub key: String,
    pub name: String,
    pub file: Option<Asset>,
    pub description: String,
    pub emoji: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSticker {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub emoji: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DesiredSound {
    pub path: String,
    pub key: String,
    pub name: String,
    pub file: Option<Asset>,
    pub volume: f64,
    pub emoji: Option<Emoji>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveSound {
    pub id: u64,
    pub name: String,
    pub volume: f64,
    pub emoji: Option<Emoji>,
}

// -- converting between reference types -----------------------------------------

/// Map a value's channel and role references.
pub trait MapRefs<C, R> {
    type Out<C2, R2>;
    fn map_refs<C2, R2>(
        &self,
        c: &mut impl FnMut(&C) -> C2,
        r: &mut impl FnMut(&R) -> R2,
    ) -> Self::Out<C2, R2>;
}

impl<C, R> MapRefs<C, R> for Onboarding<C, R> {
    type Out<C2, R2> = Onboarding<C2, R2>;
    fn map_refs<C2, R2>(
        &self,
        c: &mut impl FnMut(&C) -> C2,
        r: &mut impl FnMut(&R) -> R2,
    ) -> Onboarding<C2, R2> {
        Onboarding {
            path: self.path.clone(),
            enabled: self.enabled,
            mode: self.mode,
            default_channels: self.default_channels.iter().map(&mut *c).collect(),
            prompts: self
                .prompts
                .iter()
                .map(|p| Prompt {
                    path: p.path.clone(),
                    id: p.id,
                    key: p.key.clone(),
                    title: p.title.clone(),
                    kind: p.kind,
                    single: p.single,
                    required: p.required,
                    in_onboarding: p.in_onboarding,
                    options: p
                        .options
                        .iter()
                        .map(|o| PromptOption {
                            path: o.path.clone(),
                            id: o.id,
                            key: o.key.clone(),
                            title: o.title.clone(),
                            description: o.description.clone(),
                            emoji: o.emoji.clone(),
                            roles: o.roles.iter().map(&mut *r).collect(),
                            channels: o.channels.iter().map(&mut *c).collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

impl<C, R> MapRefs<C, R> for Automod<C, R> {
    type Out<C2, R2> = Automod<C2, R2>;
    fn map_refs<C2, R2>(
        &self,
        c: &mut impl FnMut(&C) -> C2,
        r: &mut impl FnMut(&R) -> R2,
    ) -> Automod<C2, R2> {
        Automod {
            path: self.path.clone(),
            id: self.id,
            key: self.key.clone(),
            name: self.name.clone(),
            enabled: self.enabled,
            trigger: self.trigger,
            event: self.event,
            keywords: self.keywords.clone(),
            regex: self.regex.clone(),
            allow: self.allow.clone(),
            presets: self.presets.clone(),
            mention_limit: self.mention_limit,
            raid_protection: self.raid_protection,
            actions: self
                .actions
                .iter()
                .map(|a| match a {
                    AutomodAction::Block(m) => AutomodAction::Block(m.clone()),
                    AutomodAction::Alert(ch) => AutomodAction::Alert(c(ch)),
                    AutomodAction::Timeout(t) => AutomodAction::Timeout(*t),
                    AutomodAction::BlockInteractions => AutomodAction::BlockInteractions,
                })
                .collect(),
            exempt_roles: self.exempt_roles.iter().map(&mut *r).collect(),
            exempt_channels: self.exempt_channels.iter().map(&mut *c).collect(),
        }
    }
}

impl<C> WelcomeScreen<C> {
    pub fn map_channels<C2>(&self, c: &mut impl FnMut(&C) -> C2) -> WelcomeScreen<C2> {
        WelcomeScreen {
            path: self.path.clone(),
            enabled: self.enabled,
            description: self.description.clone(),
            channels: self
                .channels
                .iter()
                .map(|w| WelcomeChannel {
                    channel: c(&w.channel),
                    description: w.description.clone(),
                    emoji: w.emoji.clone(),
                })
                .collect(),
        }
    }
}

impl<C> Guide<C> {
    pub fn map_channels<C2>(&self, c: &mut impl FnMut(&C) -> C2) -> Guide<C2> {
        let mut item = |i: &GuideItem<C>| GuideItem {
            channel: c(&i.channel),
            title: i.title.clone(),
            description: i.description.clone(),
            kind: i.kind,
            emoji: i.emoji.clone(),
            icon: i.icon.clone(),
        };
        Guide {
            path: self.path.clone(),
            enabled: self.enabled,
            welcome: self.welcome.clone(),
            actions: self.actions.iter().map(&mut item).collect(),
            resources: self.resources.iter().map(&mut item).collect(),
        }
    }
}

/// Live IDs as references, for comparing with resolved desired state.
pub fn id_ref(id: &u64) -> Ref {
    Ref::Id(*id)
}
