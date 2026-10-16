//! Discord's JSON → the normalised live model.

use super::model::*;
use super::wire::*;
use crate::live::wire::WireGuild;
use crate::model::Emoji;
use crate::snowflake::Sf;

pub fn settings(g: &WireGuild) -> LiveSettings {
    let channel = |s: ChannelSetting| {
        let v = match s {
            ChannelSetting::System => g.system_channel_id,
            ChannelSetting::Rules => g.rules_channel_id,
            ChannelSetting::Updates => g.public_updates_channel_id,
            ChannelSetting::SafetyAlerts => g.safety_alerts_channel_id,
            ChannelSetting::Afk => g.afk_channel_id,
        };
        (s, v.map(|v| v.0))
    };
    LiveSettings {
        name: g.name.clone(),
        description: g.description.clone().unwrap_or_default(),
        verification_level: g.verification_level.unwrap_or(0),
        default_notifications: g.default_message_notifications.unwrap_or(0),
        content_filter: g.explicit_content_filter.unwrap_or(0),
        locale: g
            .preferred_locale
            .clone()
            .unwrap_or_else(|| "en-US".to_owned()),
        boost_progress_bar: g.premium_progress_bar_enabled.unwrap_or(false),
        afk_timeout: g.afk_timeout.unwrap_or(300),
        system_flags: g.system_channel_flags.unwrap_or(0),
        mfa_level: g.mfa_level.unwrap_or(0),
        channels: ChannelSetting::ALL.iter().map(|s| channel(*s)).collect(),
    }
}

pub fn emoji_ref(e: Option<&WireEmojiRef>) -> Option<Emoji> {
    e.and_then(|e| Emoji::from_wire(e.id.map(|i| i.0), e.name.as_deref()))
}

fn ids(v: &[Sf]) -> Vec<u64> {
    v.iter().map(|s| s.0).collect()
}

pub fn live(g: &WireGuild, wire: Option<&WireCommunity>) -> Live {
    let mut out = Live {
        settings: settings(g),
        ..Default::default()
    };
    let Some(w) = wire else {
        out.errors.push((
            "server-wide settings".to_owned(),
            "this snapshot doesn't include them".to_owned(),
        ));
        return out;
    };
    out.errors.clone_from(&w.errors);
    out.widget = w.widget.as_ref().map(|x| Widget {
        path: String::new(),
        enabled: x.enabled,
        channel: Some(x.channel_id.map(|c| c.0)),
    });
    out.onboarding = w.onboarding.as_ref().map(onboarding);
    out.welcome_screen = w.welcome_screen.as_ref().map(|x| WelcomeScreen {
        path: String::new(),
        enabled: x.enabled,
        description: x.description.clone().unwrap_or_default(),
        channels: x
            .welcome_channels
            .iter()
            .map(|c| WelcomeChannel {
                channel: c.channel_id.0,
                description: c.description.clone(),
                emoji: Emoji::from_wire(c.emoji_id.map(|i| i.0), c.emoji_name.as_deref()),
            })
            .collect(),
    });
    out.guide = w.guide.as_ref().map(guide);
    out.join_questions = w
        .verification
        .as_ref()
        .map(|v| (v.version.clone().unwrap_or_default(), join_questions(v)));
    out.automod = w
        .automod
        .as_ref()
        .map(|rules| rules.iter().map(automod).collect());
    out.emoji = w.emojis.as_ref().map(|v| {
        v.iter()
            .filter_map(|e| {
                Some(LiveEmoji {
                    id: e.id?.0,
                    name: e.name.clone().unwrap_or_default(),
                    roles: ids(&e.roles),
                    managed: e.managed,
                })
            })
            .collect()
    });
    out.stickers = w.stickers.as_ref().map(|v| {
        v.iter()
            .map(|s| LiveSticker {
                id: s.id.0,
                name: s.name.clone(),
                description: s.description.clone().unwrap_or_default(),
                emoji: s.tags.clone(),
            })
            .collect()
    });
    out.sounds = w.sounds.as_ref().map(|v| {
        v.iter()
            .map(|s| LiveSound {
                id: s.sound_id.0,
                name: s.name.clone(),
                volume: s.volume,
                emoji: Emoji::from_wire(s.emoji_id.map(|i| i.0), s.emoji_name.as_deref()),
            })
            .collect()
    });
    out
}

pub fn onboarding(o: &WireOnboarding) -> Onboarding<u64, u64> {
    Onboarding {
        path: String::new(),
        enabled: o.enabled,
        mode: o.mode,
        default_channels: ids(&o.default_channel_ids),
        prompts: o
            .prompts
            .iter()
            .map(|p| Prompt {
                path: String::new(),
                id: Some(p.id.0),
                key: p.title.clone(),
                title: p.title.clone(),
                kind: p.kind,
                single: p.single_select,
                required: p.required,
                in_onboarding: p.in_onboarding,
                options: p
                    .options
                    .iter()
                    .map(|x| PromptOption {
                        path: String::new(),
                        id: Some(x.id.0),
                        key: x.title.clone(),
                        title: x.title.clone(),
                        description: x.description.clone().unwrap_or_default(),
                        emoji: emoji_ref(x.emoji.as_ref()),
                        roles: ids(&x.role_ids),
                        channels: ids(&x.channel_ids),
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub fn guide(g: &WireGuide) -> Guide<u64> {
    let item = |i: &WireGuideItem| GuideItem {
        channel: i.channel_id.0,
        title: i.title.clone(),
        description: i.description.clone(),
        kind: i.action_type.unwrap_or(0),
        emoji: emoji_ref(i.emoji.as_ref()),
        icon: i.icon.clone(),
    };
    Guide {
        path: String::new(),
        enabled: g.enabled,
        // An empty message is what "no welcome message" looks like.
        welcome: g
            .welcome_message
            .as_ref()
            .filter(|w| !w.message.is_empty() || !w.author_ids.is_empty())
            .map(|w| (w.message.clone(), ids(&w.author_ids))),
        actions: g.new_member_actions.iter().map(item).collect(),
        resources: g.resource_channels.iter().map(item).collect(),
    }
}

pub fn join_questions(v: &WireVerification) -> JoinQuestions {
    let mut out = JoinQuestions {
        path: String::new(),
        description: v.description.clone().unwrap_or_default(),
        ..Default::default()
    };
    for f in &v.form_fields {
        let kind = match f.field_type.as_str() {
            "TERMS" => {
                out.rules.extend(f.values.iter().flatten().cloned());
                continue;
            }
            "TEXT_INPUT" => 0,
            "PARAGRAPH" => 1,
            "MULTIPLE_CHOICE" => 2,
            // Unknown future field types are dropped from comparison; the
            // plan will show them being removed if the spec is applied.
            _ => continue,
        };
        out.questions.push(Question {
            kind,
            label: f.label.clone(),
            required: f.required,
            description: f.description.clone().unwrap_or_default(),
            placeholder: f.placeholder.clone().unwrap_or_default(),
            choices: f.choices.clone().unwrap_or_default(),
        });
    }
    out
}

pub fn automod(r: &WireAutomod) -> Automod<u64, u64> {
    let m = &r.trigger_metadata;
    Automod {
        path: String::new(),
        id: Some(r.id.0),
        key: r.name.clone(),
        name: r.name.clone(),
        enabled: r.enabled,
        trigger: r.trigger_type,
        event: r.event_type,
        keywords: m.keyword_filter.clone(),
        regex: m.regex_patterns.clone(),
        allow: m.allow_list.clone(),
        presets: m.presets.clone(),
        mention_limit: m.mention_total_limit,
        raid_protection: m.mention_raid_protection_enabled,
        actions: r
            .actions
            .iter()
            .filter_map(|a| {
                Some(match a.kind {
                    1 => AutomodAction::Block(
                        a.metadata.custom_message.clone().filter(|m| !m.is_empty()),
                    ),
                    2 => AutomodAction::Alert(a.metadata.channel_id?.0),
                    3 => AutomodAction::Timeout(a.metadata.duration_seconds.unwrap_or(60)),
                    4 => AutomodAction::BlockInteractions,
                    _ => return None,
                })
            })
            .collect(),
        exempt_roles: ids(&r.exempt_roles),
        exempt_channels: ids(&r.exempt_channels),
    }
}
