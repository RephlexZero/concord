//! Serde types for the server-wide sections of the spec: settings,
//! onboarding, the welcome screen, the Server Guide, join questions, AutoMod
//! and expressions (emoji, stickers, soundboard).
//!
//! Channels are referred to by key; roles by name, as in `private` and
//! `overrides`.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::raw::Seconds;
use crate::snowflake::Sf;

/// `field: null` and a missing field mean different things: "set to none"
/// and "leave alone". Use with `#[serde(default, deserialize_with = "some")]`.
pub fn some<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}

/// Server settings. Every field is optional; a missing one is left alone in
/// either mode, since settings like the verification level are too important
/// to reset by omission.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_level: Option<VerificationLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_notifications: Option<Notifications>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_filter: Option<ContentFilter>,
    /// e.g. `en-GB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boost_progress_bar: Option<bool>,
    /// Channel key, or `null` for none.
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub system_channel: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_messages: Option<SystemMessages>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules_channel: Option<String>,
    /// Where Discord sends community updates for moderators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates_channel: Option<String>,
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub safety_alerts_channel: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub afk_channel: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub afk_timeout: Option<Seconds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<WidgetSpec>,
}

macro_rules! named_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $wire:expr, $text:expr;)+ }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            pub fn wire(self) -> u8 {
                match self { $($name::$variant => $wire),+ }
            }
            pub fn from_wire(v: u8) -> Option<Self> {
                match v { $($wire => Some($name::$variant),)+ _ => None }
            }
            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl Visitor<'_> for V {
                    type Value = $name;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        let all: Vec<String> =
                            $name::ALL.iter().map(|v| format!("`{}`", v.as_str())).collect();
                        write!(f, "one of {}", all.join(", "))
                    }
                    fn visit_str<E: de::Error>(self, v: &str) -> Result<$name, E> {
                        let norm = v.trim().to_lowercase().replace(['-', ' '], "_");
                        $name::ALL
                            .iter()
                            .copied()
                            .find(|x| x.as_str() == norm)
                            .ok_or_else(|| E::invalid_value(de::Unexpected::Str(v), &self))
                    }
                    // YAML 1.1 reads a bare `off` as false.
                    fn visit_bool<E: de::Error>(self, v: bool) -> Result<$name, E> {
                        if v {
                            Err(E::invalid_value(de::Unexpected::Bool(v), &self))
                        } else {
                            self.visit_str("off")
                        }
                    }
                }
                d.deserialize_any(V)
            }
        }
    };
}

named_enum!(
    /// Who can talk before they've been verified.
    VerificationLevel {
        None = 0, "none";
        Low = 1, "low";
        Medium = 2, "medium";
        High = 3, "high";
        Highest = 4, "highest";
    }
);

named_enum!(
    /// Default notification setting for new members.
    Notifications {
        All = 0, "all";
        Mentions = 1, "mentions";
    }
);

named_enum!(
    /// Which members' media is scanned for explicit content.
    ContentFilter {
        Off = 0, "off";
        NoRoles = 1, "no_roles";
        Everyone = 2, "everyone";
    }
);

named_enum!(
    OnboardingMode {
        Default = 0, "default";
        Advanced = 1, "advanced";
    }
);

named_enum!(
    PromptKind {
        MultipleChoice = 0, "multiple_choice";
        Dropdown = 1, "dropdown";
    }
);

named_enum!(
    /// What a Server Guide to-do asks the member to do in its channel.
    ActionKind {
        View = 0, "view";
        Chat = 1, "chat";
    }
);

named_enum!(
    QuestionKind {
        Text = 0, "text";
        Paragraph = 1, "paragraph";
        Choice = 2, "choice";
    }
);

named_enum!(
    AutomodTrigger {
        Keyword = 1, "keyword";
        Spam = 3, "spam";
        KeywordPreset = 4, "keyword_preset";
        MentionSpam = 5, "mention_spam";
        MemberProfile = 6, "member_profile";
    }
);

named_enum!(
    AutomodPreset {
        Profanity = 1, "profanity";
        SexualContent = 2, "sexual_content";
        Slurs = 3, "slurs";
    }
);

named_enum!(
    VideoQuality {
        Auto = 1, "auto";
        Full = 2, "full";
    }
);

/// Which system messages Discord posts in the system channel. Missing fields
/// are left alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemMessages {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_replies: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boosts: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_tips: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_subscriptions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_subscription_replies: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetSpec {
    pub enabled: bool,
    /// Channel key the widget's invite points at, or `null`.
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub channel: Option<Option<String>>,
}

fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}
fn is_false(b: &bool) -> bool {
    !*b
}

/// Onboarding: the questions new members answer, and the channels everyone
/// sees. Managed as a whole when present.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnboardingSpec {
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<OnboardingMode>,
    /// Channel and category keys every member sees.
    #[serde(default)]
    pub default_channels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<PromptSpec>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptSpec {
    /// Stable identity; defaults to the title. Templates add options to a
    /// prompt by this key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub title: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<PromptKind>,
    /// Only one answer allowed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub single: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    /// Asked while joining (otherwise only in Channels & Roles).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub in_onboarding: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<OptionSpec>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionSpec {
    /// Stable identity within the prompt; defaults to the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    /// Role names given to members who pick this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// Channel or category keys shown to members who pick this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
}

/// An onboarding option a template (or category) adds to a prompt.
/// Variables work in every field, e.g. `roles: ["{team}"]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateOptionSpec {
    /// Key of the onboarding prompt to add the option to.
    pub prompt: String,
    #[serde(flatten)]
    pub option: OptionSpec,
}

/// The welcome screen (shown to new members of Discoverable servers).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WelcomeScreenSpec {
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<WelcomeChannelSpec>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WelcomeChannelSpec {
    pub channel: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

/// The Server Guide: welcome message, to-dos and resources. Uses an
/// undocumented Discord endpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideSpec {
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome: Option<GuideWelcomeSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<GuideActionSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<GuideResourceSpec>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideWelcomeSpec {
    /// `[@username]` is replaced with the new member.
    pub message: String,
    /// User IDs shown as the message's author.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<Sf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideActionSpec {
    pub title: String,
    pub channel: String,
    /// `view` (read the channel) or `chat` (send a message there).
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ActionKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideResourceSpec {
    pub title: String,
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

/// The form people fill in to join (member verification). Uses an
/// undocumented Discord endpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinQuestionsSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Rules the applicant must agree to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<QuestionSpec>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionSpec {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<QuestionKind>,
    pub label: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// For `type: choice`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomodSpec {
    pub key: String,
    pub name: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    pub trigger: AutomodTrigger,
    /// `keyword`: words and phrases, with `*` wildcards.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    /// `keyword`: Rust-flavoured regular expressions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regex: Vec<String>,
    /// `keyword`, `keyword_preset`, `member_profile`: exceptions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<AutomodPreset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mention_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raid_protection: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<AutomodActionSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exempt_roles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exempt_channels: Vec<String>,
}

/// One AutoMod action. Exactly one field is set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomodActionSpec {
    /// Block the message: `true`, or the text to show the sender.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<BlockSpec>,
    /// Post an alert in this channel (key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert: Option<String>,
    /// Time the member out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Seconds>,
    /// `member_profile`: stop them interacting until they fix their profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_interactions: Option<bool>,
}

/// `block: true` or `block: "custom message"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockSpec {
    Plain,
    Message(String),
}

impl Serialize for BlockSpec {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            BlockSpec::Plain => s.serialize_bool(true),
            BlockSpec::Message(m) => s.serialize_str(m),
        }
    }
}

impl<'de> Deserialize<'de> for BlockSpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = BlockSpec;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("`true` or a message to show the sender")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<BlockSpec, E> {
                if v {
                    Ok(BlockSpec::Plain)
                } else {
                    Err(E::custom("leave the action out instead of `block: false`"))
                }
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<BlockSpec, E> {
                Ok(BlockSpec::Message(v.to_owned()))
            }
        }
        d.deserialize_any(V)
    }
}

/// A custom emoji. Without `file`, an existing emoji is managed by name only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmojiSpec {
    pub key: String,
    pub name: String,
    /// PNG, GIF, JPEG or WebP, relative to the spec; at most 256 KiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Only these roles can use it (empty: everyone).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StickerSpec {
    pub key: String,
    pub name: String,
    /// PNG, APNG, GIF or Lottie JSON, relative to the spec; at most 512 KiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The emoji Discord suggests it for.
    pub emoji: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundSpec {
    pub key: String,
    pub name: String,
    /// MP3 or OGG, relative to the spec; at most 512 KiB and 5.2 seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// 0.0 to 1.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}
