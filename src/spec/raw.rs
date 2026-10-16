//! Serde types for the YAML spec, exactly as written. Templates and tag sets
//! are not expanded here; see [`super::expand`](mod@super::expand).

use std::fmt;

use indexmap::IndexMap;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::community::{
    AutomodSpec, EmojiSpec, GuideSpec, JoinQuestionsSpec, OnboardingSpec, ServerSpec, SoundSpec,
    StickerSpec, TemplateOptionSpec, VideoQuality, WelcomeScreenSpec,
};
use crate::model::{ChannelKind, Layout, Mode, Sort};
use crate::snowflake::Sf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<Sf>,
    #[serde(default, skip_serializing_if = "Settings::is_default")]
    pub concord: Settings,
    /// Server settings; fields left out are left alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerSpec>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub tag_sets: IndexMap<String, Vec<TagEntry>>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub templates: IndexMap<String, Template>,
    /// Highest role first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<RoleSpec>,
    /// Channels that aren't in a category. Discord always shows these above
    /// every category.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<ChannelSpec>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub categories: IndexMap<String, CategorySpec>,

    // Each section below is managed as a whole when present, and left alone
    // when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding: Option<OnboardingSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome_screen: Option<WelcomeScreenSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide: Option<GuideSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_questions: Option<JoinQuestionsSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automod: Option<Vec<AutomodSpec>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<Vec<EmojiSpec>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stickers: Option<Vec<StickerSpec>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sounds: Option<Vec<SoundSpec>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub mode: Mode,
}

impl Settings {
    fn is_default(&self) -> bool {
        *self == Settings::default()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleSpec {
    pub key: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hoist: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentionable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>,
    /// A unicode emoji shown next to members' names (needs boost level 2).
    /// `""` removes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    #[serde(default)]
    pub channels: Vec<ChannelSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<IndexMap<String, OverrideSpec>>,
    /// An onboarding option for each category made from this template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding_option: Option<TemplateOptionSpec>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategorySpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub vars: IndexMap<String, VarValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<IndexMap<String, OverrideSpec>>,
    /// Channels in this category, after any from the template.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<ChannelSpec>,
    /// An onboarding option for this category (replaces the template's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding_option: Option<TemplateOptionSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelSpec {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: ChannelKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// Forum post guidelines (Discord stores these as the forum's topic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidelines: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nsfw: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slowmode: Option<Seconds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_limit: Option<u32>,
    /// Voice region, e.g. `rotterdam`; `""` for automatic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_quality: Option<VideoQuality>,
    /// Forum and media: slowmode inside each post.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_slowmode: Option<Seconds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<TagEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_tag: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<Sort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reaction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_archive: Option<ArchiveDuration>,
    /// Shorthand: hide from `@everyone`, show to these roles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<IndexMap<String, OverrideSpec>>,
    /// `false`: don't start from the category's permissions; use only this
    /// channel's `private`/`overrides`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<bool>,
}

impl ChannelSpec {
    pub fn new(key: impl Into<String>, kind: ChannelKind, name: impl Into<String>) -> Self {
        ChannelSpec {
            key: key.into(),
            kind,
            name: name.into(),
            topic: None,
            guidelines: None,
            nsfw: None,
            slowmode: None,
            bitrate: None,
            user_limit: None,
            region: None,
            video_quality: None,
            post_slowmode: None,
            tags: None,
            require_tag: None,
            layout: None,
            sort: None,
            default_reaction: None,
            auto_archive: None,
            private: None,
            overrides: None,
            sync: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverrideSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagSpec {
    /// Stable identity, so a tag can be renamed without being recreated.
    /// Defaults to the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub moderated: bool,
}

/// An entry in a `tags:` list: a tag name, a tag set name, a `{var}`, or a
/// full tag definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagEntry {
    Name(String),
    Full(TagSpec),
}

impl Serialize for TagEntry {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            TagEntry::Name(n) => s.serialize_str(n),
            TagEntry::Full(t) => t.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for TagEntry {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = TagEntry;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a tag name, a tag set name, or a tag like { name: Agreed, moderated: true }",
                )
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<TagEntry, E> {
                Ok(TagEntry::Name(v.to_owned()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<TagEntry, E> {
                Ok(TagEntry::Name(v.to_string()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<TagEntry, E> {
                Ok(TagEntry::Name(v.to_string()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<TagEntry, E> {
                Ok(TagEntry::Name(v.to_string()))
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<TagEntry, E> {
                Ok(TagEntry::Name(v.to_string()))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<TagEntry, A::Error> {
                TagSpec::deserialize(de::value::MapAccessDeserializer::new(map)).map(TagEntry::Full)
            }
        }
        d.deserialize_any(V)
    }
}

/// A template variable: one string, or a list (which can be spliced into a
/// `tags:` list with `"{var}"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarValue {
    One(String),
    Many(Vec<String>),
}

impl Serialize for VarValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            VarValue::One(v) => s.serialize_str(v),
            VarValue::Many(v) => v.serialize(s),
        }
    }
}

/// Any YAML scalar, as a string.
struct Scalar(String);

impl<'de> Deserialize<'de> for Scalar {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Scalar;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Scalar, E> {
                Ok(Scalar(v.to_owned()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Scalar, E> {
                Ok(Scalar(v.to_string()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Scalar, E> {
                Ok(Scalar(v.to_string()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Scalar, E> {
                Ok(Scalar(v.to_string()))
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Scalar, E> {
                Ok(Scalar(v.to_string()))
            }
        }
        d.deserialize_any(V)
    }
}

impl<'de> Deserialize<'de> for VarValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = VarValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string or a list of strings")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<VarValue, E> {
                Ok(VarValue::One(v.to_owned()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<VarValue, E> {
                Ok(VarValue::One(v.to_string()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<VarValue, E> {
                Ok(VarValue::One(v.to_string()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<VarValue, E> {
                Ok(VarValue::One(v.to_string()))
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<VarValue, E> {
                Ok(VarValue::One(v.to_string()))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<VarValue, A::Error> {
                let mut out = Vec::new();
                while let Some(Scalar(s)) = seq.next_element()? {
                    out.push(s);
                }
                Ok(VarValue::Many(out))
            }
        }
        d.deserialize_any(V)
    }
}

/// A role colour: `"#e91e63"`, `"e91e63"` or an integer. `0` means no colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    pub fn parse(s: &str) -> Result<Color, String> {
        let hex = s.trim().trim_start_matches('#');
        let expanded = match hex.len() {
            3 => hex.chars().flat_map(|c| [c, c]).collect(),
            6 => hex.to_owned(),
            _ => return Err(format!("invalid colour `{s}`, expected \"#rrggbb\"")),
        };
        u32::from_str_radix(&expanded, 16)
            .map(Color)
            .map_err(|_| format!("invalid colour `{s}`, expected \"#rrggbb\""))
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            f.write_str("none")
        } else {
            write!(f, "#{:06x}", self.0)
        }
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("#{:06x}", self.0))
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Color;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a colour like \"#e91e63\"")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Color, E> {
                u32::try_from(v)
                    .ok()
                    .filter(|v| *v <= 0xFF_FFFF)
                    .map(Color)
                    .ok_or_else(|| E::custom("colour out of range"))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Color, E> {
                u64::try_from(v)
                    .map_err(|_| E::custom("colour out of range"))
                    .and_then(|v| self.visit_u64(v))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Color, E> {
                Color::parse(v).map_err(E::custom)
            }
        }
        d.deserialize_any(V)
    }
}

/// A duration in seconds: `30`, `"30s"`, `"5m"`, `"2h"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seconds(pub u32);

impl Serialize for Seconds {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let v = self.0;
        let text = if v != 0 && v.is_multiple_of(3600) {
            format!("{}h", v / 3600)
        } else if v != 0 && v.is_multiple_of(60) {
            format!("{}m", v / 60)
        } else {
            format!("{v}s")
        };
        s.serialize_str(&text)
    }
}

impl<'de> Deserialize<'de> for Seconds {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Seconds;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a duration like \"30s\", \"5m\" or \"1h\"")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Seconds, E> {
                u32::try_from(v)
                    .map(Seconds)
                    .map_err(|_| E::custom("duration too large"))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Seconds, E> {
                u32::try_from(v)
                    .map(Seconds)
                    .map_err(|_| E::custom("duration must be positive"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Seconds, E> {
                let v = v.trim();
                let err = || {
                    E::custom(format!(
                        "invalid duration `{v}`, expected e.g. \"30s\", \"5m\", \"1h\""
                    ))
                };
                let (num, mult) = match v.char_indices().last() {
                    Some((i, 's')) => (&v[..i], 1),
                    Some((i, 'm')) => (&v[..i], 60),
                    Some((i, 'h')) => (&v[..i], 3600),
                    _ => (v, 1),
                };
                num.trim()
                    .parse::<u32>()
                    .ok()
                    .and_then(|n| n.checked_mul(mult))
                    .map(Seconds)
                    .ok_or_else(err)
            }
        }
        d.deserialize_any(V)
    }
}

/// How long a forum post stays open without activity: `1h`, `1d`, `3d`, `1w`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveDuration(pub u32);

impl ArchiveDuration {
    const VALUES: &'static [(u32, &'static str)] =
        &[(60, "1h"), (1440, "1d"), (4320, "3d"), (10080, "1w")];

    pub fn label(minutes: u32) -> String {
        Self::VALUES
            .iter()
            .find(|(m, _)| *m == minutes)
            .map_or_else(|| format!("{minutes}min"), |(_, l)| (*l).to_owned())
    }

    fn from_minutes(m: u64) -> Option<ArchiveDuration> {
        Self::VALUES
            .iter()
            .find(|(v, _)| u64::from(*v) == m)
            .map(|(v, _)| ArchiveDuration(*v))
    }
}

impl Serialize for ArchiveDuration {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&Self::label(self.0))
    }
}

impl<'de> Deserialize<'de> for ArchiveDuration {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        const EXPECTED: &str = "one of \"1h\", \"1d\", \"3d\", \"1w\" (Discord only allows these)";
        impl Visitor<'_> for V {
            type Value = ArchiveDuration;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(EXPECTED)
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<ArchiveDuration, E> {
                ArchiveDuration::from_minutes(v).ok_or_else(|| {
                    E::custom(format!("invalid auto_archive `{v}`, expected {EXPECTED}"))
                })
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<ArchiveDuration, E> {
                self.visit_u64(u64::try_from(v).unwrap_or(0))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<ArchiveDuration, E> {
                let minutes = match v.trim() {
                    "1h" | "60m" => 60,
                    "1d" | "24h" => 1440,
                    "3d" | "72h" => 4320,
                    "1w" | "7d" => 10080,
                    _ => {
                        return Err(E::custom(format!(
                            "invalid auto_archive `{v}`, expected {EXPECTED}"
                        )))
                    }
                };
                Ok(ArchiveDuration(minutes))
            }
        }
        d.deserialize_any(V)
    }
}
