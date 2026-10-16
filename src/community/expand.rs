//! Validate the spec's server-wide sections and normalise them into
//! [`Desired`]. Channel keys are checked here; role names are checked when
//! planning, since they may name live roles the spec doesn't manage.

use std::collections::HashSet;
use std::path::Path;

use super::model::*;
use crate::diag::{suggest, Diagnostic};
use crate::model::{ChannelKind, DesiredChannel, DesiredRole, Emoji};
use crate::spec::community::{
    ActionKind, AutomodSpec, BlockSpec, GuideSpec, JoinQuestionsSpec, OnboardingMode,
    OnboardingSpec, OptionSpec, PromptKind, QuestionKind, ServerSpec, WelcomeScreenSpec,
};
use crate::spec::expand::TemplateOption;
use crate::spec::raw::SpecFile;

/// Discord's limits for these sections.
pub mod limits {
    pub const PROMPTS: usize = 15;
    pub const OPTIONS: usize = 50;
    pub const PROMPT_TITLE: usize = 100;
    pub const OPTION_TITLE: usize = 50;
    pub const OPTION_DESCRIPTION: usize = 100;
    pub const WELCOME_CHANNELS: usize = 5;
    pub const GUIDE_ACTIONS: usize = 5;
    pub const GUIDE_RESOURCES: usize = 7;
    pub const GUIDE_WELCOME: usize = 300;
    pub const KEYWORD_RULES: usize = 6;
    pub const KEYWORDS: usize = 1000;
    pub const REGEX: usize = 10;
    pub const MENTION_LIMIT: u32 = 50;
    pub const TIMEOUT: u32 = 2_419_200;
    pub const EMOJI_BYTES: usize = 256 * 1024;
    pub const STICKER_BYTES: usize = 512 * 1024;
    pub const SOUND_BYTES: usize = 512 * 1024;
    pub const AFK_TIMEOUTS: [u32; 5] = [60, 300, 900, 1800, 3600];
}

pub fn expand(
    spec: &SpecFile,
    roles: &[DesiredRole],
    channels: &[DesiredChannel],
    template_options: Vec<TemplateOption>,
    base: &Path,
    diags: &mut Vec<Diagnostic>,
) -> Desired {
    let mut cx = Cx {
        channels,
        roles,
        base,
        diags,
    };
    let mut out = Desired::default();
    if let Some(s) = &spec.server {
        out.settings = Some(cx.settings(s));
        out.widget = s.widget.as_ref().map(|w| Widget {
            path: "server.widget".to_owned(),
            enabled: w.enabled,
            channel: w.channel.as_ref().map(|c| {
                c.as_ref()
                    .map(|k| cx.channel_key(k, "server.widget.channel"))
            }),
        });
    }
    match &spec.onboarding {
        Some(o) => out.onboarding = Some(cx.onboarding(o, template_options)),
        None => {
            for t in &template_options {
                cx.diags.push(
                    Diagnostic::error(
                        &t.path,
                        "an onboarding option, but the spec has no `onboarding:` section",
                    )
                    .with_hint("add `onboarding:` with a prompt for these options"),
                );
            }
        }
    }
    out.welcome_screen = spec.welcome_screen.as_ref().map(|w| cx.welcome(w));
    out.guide = spec.guide.as_ref().map(|g| cx.guide(g));
    out.join_questions = spec.join_questions.as_ref().map(|j| cx.join(j));
    out.automod = spec.automod.as_ref().map(|a| cx.automod(a));
    out.emoji = spec.emoji.as_ref().map(|list| {
        let mut keys = HashSet::new();
        list.iter()
            .enumerate()
            .map(|(i, e)| {
                let path = format!("emoji[{i}]");
                cx.unique_key(&mut keys, &e.key, &path);
                let n = e.name.chars().count();
                if !(2..=32).contains(&n)
                    || !e
                        .name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    cx.err(
                        format!("{path}.name"),
                        "emoji names are 2-32 letters, digits or underscores",
                    );
                }
                DesiredEmoji {
                    file: e.file.as_ref().and_then(|f| {
                        cx.asset(
                            f,
                            &format!("{path}.file"),
                            limits::EMOJI_BYTES,
                            &["png", "gif", "jpg", "jpeg", "webp"],
                        )
                    }),
                    path,
                    key: e.key.clone(),
                    name: e.name.clone(),
                    roles: e.roles.clone(),
                }
            })
            .collect()
    });
    out.stickers = spec.stickers.as_ref().map(|list| {
        let mut keys = HashSet::new();
        list.iter()
            .enumerate()
            .map(|(i, s)| {
                let path = format!("stickers[{i}]");
                cx.unique_key(&mut keys, &s.key, &path);
                if !(2..=30).contains(&s.name.chars().count()) {
                    cx.err(format!("{path}.name"), "sticker names are 2-30 characters");
                }
                if s.emoji.trim().is_empty() {
                    cx.err(format!("{path}.emoji"), "stickers need a related emoji");
                }
                DesiredSticker {
                    file: s.file.as_ref().and_then(|f| {
                        cx.asset(
                            f,
                            &format!("{path}.file"),
                            limits::STICKER_BYTES,
                            &["png", "apng", "gif", "json"],
                        )
                    }),
                    path,
                    key: s.key.clone(),
                    name: s.name.clone(),
                    description: s.description.clone().unwrap_or_default(),
                    emoji: s.emoji.trim().to_owned(),
                }
            })
            .collect()
    });
    out.sounds = spec.sounds.as_ref().map(|list| {
        let mut keys = HashSet::new();
        list.iter()
            .enumerate()
            .map(|(i, s)| {
                let path = format!("sounds[{i}]");
                cx.unique_key(&mut keys, &s.key, &path);
                if !(2..=32).contains(&s.name.chars().count()) {
                    cx.err(format!("{path}.name"), "sound names are 2-32 characters");
                }
                let volume = s.volume.unwrap_or(1.0);
                if !(0.0..=1.0).contains(&volume) {
                    cx.err(format!("{path}.volume"), "volume is between 0.0 and 1.0");
                }
                DesiredSound {
                    file: s.file.as_ref().and_then(|f| {
                        cx.asset(
                            f,
                            &format!("{path}.file"),
                            limits::SOUND_BYTES,
                            &["mp3", "ogg"],
                        )
                    }),
                    emoji: s
                        .emoji
                        .as_ref()
                        .and_then(|e| cx.emoji(e, &format!("{path}.emoji"))),
                    path,
                    key: s.key.clone(),
                    name: s.name.clone(),
                    volume,
                }
            })
            .collect()
    });
    out
}

struct Cx<'a> {
    channels: &'a [DesiredChannel],
    roles: &'a [DesiredRole],
    base: &'a Path,
    diags: &'a mut Vec<Diagnostic>,
}

/// A stable key from a title: `"Which year are you in?"` → `which-year-are-you-in`.
pub fn key_of(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        "item".to_owned()
    } else {
        out
    }
}

fn too_long(s: &str, max: usize) -> bool {
    s.chars().count() > max
}

impl Cx<'_> {
    fn err(&mut self, path: impl Into<String>, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(path, msg));
    }

    fn unique_key(&mut self, keys: &mut HashSet<String>, key: &str, path: &str) {
        if key.trim().is_empty() || key.contains('/') {
            self.err(
                format!("{path}.key"),
                "keys must be non-empty and contain no `/`",
            );
        } else if !keys.insert(key.to_owned()) {
            self.err(format!("{path}.key"), format!("duplicate key `{key}`"));
        }
    }

    /// Check a channel key exists in the spec; returns it unchanged.
    fn channel_key(&mut self, key: &str, path: &str) -> String {
        if !self.channels.iter().any(|c| c.key == key) {
            let by_name = self.channels.iter().find(|c| c.name == key);
            let hint = by_name
                .map(|c| {
                    format!(
                        "channels are referred to by key; `{key}` is the name of `{}`",
                        c.key
                    )
                })
                .or_else(|| suggest(key, self.channels.iter().map(|c| c.key.as_str())));
            self.diags.push(
                Diagnostic::error(path, format!("no channel or category with key `{key}`"))
                    .with_hint_opt(hint),
            );
        }
        key.to_owned()
    }

    fn channel_kind(&self, key: &str) -> Option<ChannelKind> {
        self.channels.iter().find(|c| c.key == key).map(|c| c.kind)
    }

    fn emoji(&mut self, s: &str, path: &str) -> Option<Emoji> {
        match Emoji::parse(s) {
            Ok(e) => Some(e),
            Err(m) => {
                self.err(path, m);
                None
            }
        }
    }

    fn asset(&mut self, file: &str, path: &str, max: usize, exts: &[&str]) -> Option<Asset> {
        let ext = Path::new(file)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default();
        if !exts.contains(&ext.as_str()) {
            self.err(
                path,
                format!("`{file}`: expected a .{} file", exts.join(", .")),
            );
            return None;
        }
        let full = self.base.join(file);
        let bytes = match std::fs::read(&full) {
            Ok(b) => b,
            Err(e) => {
                self.err(path, format!("can't read {}: {e}", full.display()));
                return None;
            }
        };
        if bytes.len() > max {
            self.err(
                path,
                format!(
                    "`{file}` is {} KiB; Discord allows at most {} KiB",
                    bytes.len() / 1024,
                    max / 1024
                ),
            );
        }
        let mime = match ext.as_str() {
            "png" | "apng" => "image/png",
            "gif" => "image/gif",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "json" => "application/json",
            "mp3" => "audio/mpeg",
            "ogg" => "audio/ogg",
            _ => "application/octet-stream",
        };
        let digest = ring::digest::digest(&ring::digest::SHA256, &bytes);
        let sha256 = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
        Some(Asset {
            file: file.to_owned(),
            bytes,
            mime,
            sha256,
        })
    }

    // -- settings ----------------------------------------------------------

    fn settings(&mut self, s: &ServerSpec) -> Settings {
        let mut out = Settings {
            path: "server".to_owned(),
            name: s.name.clone(),
            description: s.description.as_ref().map(|d| d.trim().to_owned()),
            verification_level: s.verification_level.map(|v| v.wire()),
            default_notifications: s.default_notifications.map(|v| v.wire()),
            content_filter: s.content_filter.map(|v| v.wire()),
            locale: s.locale.clone(),
            boost_progress_bar: s.boost_progress_bar,
            ..Default::default()
        };
        if let Some(n) = &s.name {
            if !(2..=100).contains(&n.chars().count()) {
                self.err("server.name", "server names are 2-100 characters");
            }
        }
        if let Some(t) = s.afk_timeout {
            if !limits::AFK_TIMEOUTS.contains(&t.0) {
                self.err(
                    "server.afk_timeout",
                    "Discord allows 1m, 5m, 15m, 30m or 1h",
                );
            }
            out.afk_timeout = Some(t.0);
        }
        if let Some(m) = &s.system_messages {
            use system_flags::*;
            let mut mask = 0;
            let mut value = 0;
            for (bit, want) in [
                (JOIN, m.join),
                (JOIN_REPLIES, m.join_replies),
                (BOOSTS, m.boosts),
                (SETUP_TIPS, m.setup_tips),
                (ROLE_SUBSCRIPTIONS, m.role_subscriptions),
                (ROLE_SUBSCRIPTION_REPLIES, m.role_subscription_replies),
            ] {
                if let Some(on) = want {
                    mask |= bit;
                    if !on {
                        value |= bit; // suppress
                    }
                }
            }
            out.system_flags = Some((mask, value));
        }
        let wanted = [
            (ChannelSetting::System, s.system_channel.clone()),
            (ChannelSetting::Rules, s.rules_channel.clone().map(Some)),
            (ChannelSetting::Updates, s.updates_channel.clone().map(Some)),
            (
                ChannelSetting::SafetyAlerts,
                s.safety_alerts_channel.clone(),
            ),
            (ChannelSetting::Afk, s.afk_channel.clone()),
        ];
        for (setting, value) in wanted {
            let Some(value) = value else { continue };
            let path = format!("server.{}", setting.spec());
            if let Some(k) = &value {
                self.channel_key(k, &path);
                let kind = self.channel_kind(k);
                let want_voice = setting == ChannelSetting::Afk;
                if kind.is_some_and(|k| k.is_voice_like() != want_voice) {
                    self.err(
                        &path,
                        if want_voice {
                            "the AFK channel must be a voice channel"
                        } else {
                            "must be a text channel"
                        },
                    );
                }
            }
            out.channels.push((setting, value));
        }
        out
    }

    // -- onboarding --------------------------------------------------------

    fn onboarding(
        &mut self,
        o: &OnboardingSpec,
        extra: Vec<TemplateOption>,
    ) -> Onboarding<String, String> {
        let mut prompts = Vec::new();
        let mut keys = HashSet::new();
        for (i, p) in o.prompts.iter().enumerate() {
            let path = format!("onboarding.prompts[{i}]");
            let key = p.key.clone().unwrap_or_else(|| key_of(&p.title));
            if !keys.insert(key.clone()) {
                self.diags.push(
                    Diagnostic::error(&path, format!("two prompts have the key `{key}`"))
                        .with_hint("give one a `key:`"),
                );
            }
            if p.title.trim().is_empty() || too_long(&p.title, limits::PROMPT_TITLE) {
                self.err(
                    format!("{path}.title"),
                    "prompt titles are 1-100 characters",
                );
            }
            let options = p
                .options
                .iter()
                .enumerate()
                .map(|(j, x)| self.option(x, format!("{path}.options[{j}]")))
                .collect();
            prompts.push(Prompt {
                path,
                id: None,
                key,
                title: p.title.trim().to_owned(),
                kind: p.kind.unwrap_or(PromptKind::MultipleChoice).wire(),
                single: p.single,
                required: p.required,
                in_onboarding: p.in_onboarding,
                options,
            });
        }
        for t in extra {
            let option = self.option(&t.option, t.path.clone());
            match prompts.iter_mut().find(|p| p.key == t.prompt) {
                Some(p) => p.options.push(option),
                None => {
                    let hint = suggest(&t.prompt, prompts.iter().map(|p| p.key.as_str()));
                    self.diags.push(
                        Diagnostic::error(
                            format!("{}.prompt", t.path),
                            format!("no onboarding prompt with key `{}`", t.prompt),
                        )
                        .with_hint_opt(hint),
                    );
                }
            }
        }
        if prompts.len() > limits::PROMPTS {
            self.err("onboarding.prompts", "Discord allows at most 15 prompts");
        }
        for p in &mut prompts {
            if p.options.len() > limits::OPTIONS {
                self.err(&p.path, "Discord allows at most 50 options per prompt");
            }
            if p.options.is_empty() {
                self.err(&p.path, "a prompt needs at least one option");
            }
            let mut seen = HashSet::new();
            for x in &p.options {
                if !seen.insert(x.key.clone()) {
                    self.diags.push(
                        Diagnostic::error(
                            &x.path,
                            format!("two options in this prompt have the key `{}`", x.key),
                        )
                        .with_hint("give one a `key:`"),
                    );
                }
            }
        }
        let default_channels: Vec<String> = o
            .default_channels
            .iter()
            .enumerate()
            .map(|(i, k)| self.channel_key(k, &format!("onboarding.default_channels[{i}]")))
            .collect();
        if o.enabled {
            // Discord counts a category's channels, and wants 7 default
            // channels, 5 of which members can talk in.
            let count: usize = default_channels
                .iter()
                .map(|k| match self.channel_kind(k) {
                    Some(ChannelKind::Category) => self
                        .channels
                        .iter()
                        .filter(|c| c.parent.as_deref() == Some(k))
                        .count(),
                    _ => 1,
                })
                .sum();
            if count < 7 {
                self.diags.push(
                    Diagnostic::warning(
                        "onboarding.default_channels",
                        format!("{count} default channel(s); Discord requires at least 7 for onboarding"),
                    )
                    .with_hint("add more channels or categories everyone should see"),
                );
            }
        }
        Onboarding {
            path: "onboarding".to_owned(),
            enabled: o.enabled,
            mode: o.mode.unwrap_or(OnboardingMode::Default).wire(),
            default_channels,
            prompts,
        }
    }

    fn option(&mut self, x: &OptionSpec, path: String) -> PromptOption<String, String> {
        if x.title.trim().is_empty() || too_long(&x.title, limits::OPTION_TITLE) {
            self.err(format!("{path}.title"), "option titles are 1-50 characters");
        }
        if x.description
            .as_deref()
            .is_some_and(|d| too_long(d, limits::OPTION_DESCRIPTION))
        {
            self.err(
                format!("{path}.description"),
                "option descriptions are at most 100 characters",
            );
        }
        if x.roles.is_empty() && x.channels.is_empty() {
            self.err(&path, "an option must give at least one role or channel");
        }
        let channels = x
            .channels
            .iter()
            .enumerate()
            .map(|(i, k)| self.channel_key(k, &format!("{path}.channels[{i}]")))
            .collect();
        for (i, r) in x.roles.iter().enumerate() {
            if r == crate::spec::expand::EVERYONE {
                self.err(format!("{path}.roles[{i}]"), "options can't give @everyone");
            }
        }
        let emoji = x
            .emoji
            .as_ref()
            .and_then(|e| self.emoji(e, &format!("{path}.emoji")));
        PromptOption {
            id: None,
            key: x.key.clone().unwrap_or_else(|| key_of(&x.title)),
            title: x.title.trim().to_owned(),
            description: x.description.clone().unwrap_or_default().trim().to_owned(),
            emoji,
            roles: x.roles.clone(),
            channels,
            path,
        }
    }

    // -- welcome screen and guide -------------------------------------------

    fn welcome(&mut self, w: &WelcomeScreenSpec) -> WelcomeScreen<String> {
        if w.channels.len() > limits::WELCOME_CHANNELS {
            self.err(
                "welcome_screen.channels",
                "Discord allows at most 5 welcome channels",
            );
        }
        let channels = w
            .channels
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let path = format!("welcome_screen.channels[{i}]");
                WelcomeChannel {
                    channel: self.channel_key(&c.channel, &format!("{path}.channel")),
                    description: c.description.trim().to_owned(),
                    emoji: c
                        .emoji
                        .as_ref()
                        .and_then(|e| self.emoji(e, &format!("{path}.emoji"))),
                }
            })
            .collect();
        WelcomeScreen {
            path: "welcome_screen".to_owned(),
            enabled: w.enabled,
            description: w.description.clone().unwrap_or_default().trim().to_owned(),
            channels,
        }
    }

    fn guide(&mut self, g: &GuideSpec) -> Guide<String> {
        if g.actions.len() > limits::GUIDE_ACTIONS {
            self.err("guide.actions", "Discord allows at most 5 to-dos");
        }
        if g.enabled && g.actions.len() < 3 {
            self.diags.push(Diagnostic::warning(
                "guide.actions",
                "the Server Guide needs at least 3 to-dos to be enabled",
            ));
        }
        if g.resources.len() > limits::GUIDE_RESOURCES {
            self.err("guide.resources", "Discord allows at most 7 resources");
        }
        if let Some(w) = &g.welcome {
            if too_long(&w.message, limits::GUIDE_WELCOME) {
                self.err("guide.welcome.message", "at most 300 characters");
            }
        }
        let actions = g
            .actions
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let path = format!("guide.actions[{i}]");
                GuideItem {
                    channel: self.channel_key(&a.channel, &format!("{path}.channel")),
                    title: a.title.trim().to_owned(),
                    description: a.description.clone().unwrap_or_default().trim().to_owned(),
                    kind: a.kind.unwrap_or(ActionKind::View).wire(),
                    emoji: a
                        .emoji
                        .as_ref()
                        .and_then(|e| self.emoji(e, &format!("{path}.emoji"))),
                    icon: None,
                }
            })
            .collect();
        let resources = g
            .resources
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let path = format!("guide.resources[{i}]");
                GuideItem {
                    channel: self.channel_key(&r.channel, &format!("{path}.channel")),
                    title: r.title.trim().to_owned(),
                    description: r.description.clone().unwrap_or_default().trim().to_owned(),
                    kind: 0,
                    emoji: r
                        .emoji
                        .as_ref()
                        .and_then(|e| self.emoji(e, &format!("{path}.emoji"))),
                    icon: None,
                }
            })
            .collect();
        Guide {
            path: "guide".to_owned(),
            enabled: g.enabled,
            welcome: g.welcome.as_ref().map(|w| {
                (
                    w.message.trim().to_owned(),
                    w.authors.iter().map(|a| a.0).collect(),
                )
            }),
            actions,
            resources,
        }
    }

    fn join(&mut self, j: &JoinQuestionsSpec) -> JoinQuestions {
        let questions = j
            .questions
            .iter()
            .enumerate()
            .map(|(i, q)| {
                let path = format!("join_questions.questions[{i}]");
                let kind = q.kind.unwrap_or(QuestionKind::Text);
                if kind == QuestionKind::Choice && q.choices.is_empty() {
                    self.err(
                        format!("{path}.choices"),
                        "a `choice` question needs `choices`",
                    );
                }
                if kind != QuestionKind::Choice && !q.choices.is_empty() {
                    self.err(format!("{path}.choices"), "only `type: choice` has choices");
                }
                if q.label.trim().is_empty() || too_long(&q.label, 300) {
                    self.err(format!("{path}.label"), "labels are 1-300 characters");
                }
                Question {
                    kind: kind.wire(),
                    label: q.label.trim().to_owned(),
                    required: q.required,
                    description: q.description.clone().unwrap_or_default(),
                    placeholder: q.placeholder.clone().unwrap_or_default(),
                    choices: q.choices.clone(),
                }
            })
            .collect();
        for (i, r) in j.rules.iter().enumerate() {
            if too_long(r, 300) {
                self.err(
                    format!("join_questions.rules[{i}]"),
                    "rules are at most 300 characters",
                );
            }
        }
        JoinQuestions {
            path: "join_questions".to_owned(),
            description: j.description.clone().unwrap_or_default().trim().to_owned(),
            rules: j.rules.iter().map(|r| r.trim().to_owned()).collect(),
            questions,
        }
    }

    // -- AutoMod -----------------------------------------------------------

    fn automod(&mut self, list: &[AutomodSpec]) -> Vec<Automod<String, String>> {
        use crate::spec::community::AutomodTrigger as T;
        let mut keys = HashSet::new();
        let mut per_trigger = std::collections::HashMap::<T, usize>::new();
        let mut out = Vec::new();
        for (i, r) in list.iter().enumerate() {
            let path = format!("automod[{i}]");
            self.unique_key(&mut keys, &r.key, &path);
            *per_trigger.entry(r.trigger).or_default() += 1;
            let only = |cx: &mut Self, set: bool, field: &str, ok: &[T]| {
                if set && !ok.contains(&r.trigger) {
                    cx.err(
                        format!("{path}.{field}"),
                        format!("`{field}` doesn't apply to `{}` rules", r.trigger),
                    );
                }
            };
            only(
                self,
                !r.keywords.is_empty(),
                "keywords",
                &[T::Keyword, T::MemberProfile],
            );
            only(
                self,
                !r.regex.is_empty(),
                "regex",
                &[T::Keyword, T::MemberProfile],
            );
            only(
                self,
                !r.allow.is_empty(),
                "allow",
                &[T::Keyword, T::KeywordPreset, T::MemberProfile],
            );
            only(self, !r.presets.is_empty(), "presets", &[T::KeywordPreset]);
            only(
                self,
                r.mention_limit.is_some(),
                "mention_limit",
                &[T::MentionSpam],
            );
            only(
                self,
                r.raid_protection.is_some(),
                "raid_protection",
                &[T::MentionSpam],
            );
            match r.trigger {
                T::Keyword | T::MemberProfile if r.keywords.is_empty() && r.regex.is_empty() => {
                    self.err(&path, "needs `keywords` or `regex`")
                }
                T::KeywordPreset if r.presets.is_empty() => {
                    self.err(&path, "needs `presets` (profanity, sexual_content, slurs)")
                }
                T::MentionSpam
                    if !r
                        .mention_limit
                        .is_some_and(|m| (1..=limits::MENTION_LIMIT).contains(&m)) =>
                {
                    self.err(
                        format!("{path}.mention_limit"),
                        "needs `mention_limit` between 1 and 50",
                    );
                }
                _ => {}
            }
            if r.keywords.len() > limits::KEYWORDS || r.regex.len() > limits::REGEX {
                self.err(
                    &path,
                    "Discord allows 1000 keywords and 10 regexes per rule",
                );
            }
            if r.actions.is_empty() {
                self.err(
                    format!("{path}.actions"),
                    "a rule needs at least one action",
                );
            }
            let mut actions = Vec::new();
            for (j, a) in r.actions.iter().enumerate() {
                let apath = format!("{path}.actions[{j}]");
                let set = [
                    a.block.is_some(),
                    a.alert.is_some(),
                    a.timeout.is_some(),
                    a.block_interactions.is_some(),
                ];
                if set.iter().filter(|s| **s).count() != 1 {
                    self.err(&apath, "each action is exactly one of `block`, `alert`, `timeout`, `block_interactions`");
                    continue;
                }
                if let Some(b) = &a.block {
                    actions.push(AutomodAction::Block(match b {
                        BlockSpec::Plain => None,
                        BlockSpec::Message(m) => Some(m.clone()),
                    }));
                } else if let Some(k) = &a.alert {
                    actions.push(AutomodAction::Alert(
                        self.channel_key(k, &format!("{apath}.alert")),
                    ));
                } else if let Some(t) = a.timeout {
                    if !matches!(r.trigger, T::Keyword | T::MentionSpam | T::MemberProfile) {
                        self.err(&apath, "`timeout` only works for keyword, mention_spam and member_profile rules");
                    }
                    if t.0 == 0 || t.0 > limits::TIMEOUT {
                        self.err(&apath, "timeouts are 1s to 4 weeks");
                    }
                    actions.push(AutomodAction::Timeout(t.0));
                } else if a.block_interactions == Some(true) {
                    if r.trigger != T::MemberProfile {
                        self.err(
                            &apath,
                            "`block_interactions` only works for member_profile rules",
                        );
                    }
                    actions.push(AutomodAction::BlockInteractions);
                }
            }
            out.push(Automod {
                path: path.clone(),
                id: None,
                key: r.key.clone(),
                name: r.name.trim().to_owned(),
                enabled: r.enabled,
                trigger: r.trigger.wire(),
                event: if r.trigger == T::MemberProfile { 2 } else { 1 },
                keywords: r.keywords.clone(),
                regex: r.regex.clone(),
                allow: r.allow.clone(),
                presets: r.presets.iter().map(|p| p.wire()).collect(),
                mention_limit: r.mention_limit,
                raid_protection: r
                    .raid_protection
                    .or((r.trigger == T::MentionSpam).then_some(false)),
                actions,
                exempt_roles: r.exempt_roles.clone(),
                exempt_channels: r
                    .exempt_channels
                    .iter()
                    .enumerate()
                    .map(|(j, k)| self.channel_key(k, &format!("{path}.exempt_channels[{j}]")))
                    .collect(),
            });
        }
        for (trigger, n) in per_trigger {
            let max = if trigger == T::Keyword {
                limits::KEYWORD_RULES
            } else {
                1
            };
            if n > max {
                self.err(
                    "automod",
                    format!("Discord allows at most {max} `{trigger}` rule(s)"),
                );
            }
        }
        let _ = self.roles;
        out
    }
}
