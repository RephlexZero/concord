//! `concord import` for server-wide configuration: the live state as spec
//! sections. Planning the result against the same server shows no changes.

use std::collections::HashMap;

use super::expand::key_of;
use super::model::*;
use crate::diag::Diagnostic;
use crate::lock::Lockfile;
use crate::model::{Emoji, LiveState};
use crate::snowflake::Sf;
use crate::spec::community::*;
use crate::spec::raw::Seconds;

/// The imported sections, ready to put in a [`crate::spec::raw::SpecFile`].
#[derive(Debug, Default)]
pub struct Sections {
    pub server: Option<ServerSpec>,
    pub onboarding: Option<OnboardingSpec>,
    pub welcome_screen: Option<WelcomeScreenSpec>,
    pub guide: Option<GuideSpec>,
    pub join_questions: Option<JoinQuestionsSpec>,
    pub automod: Option<Vec<AutomodSpec>>,
    pub emoji: Option<Vec<EmojiSpec>>,
    pub stickers: Option<Vec<StickerSpec>>,
    pub sounds: Option<Vec<SoundSpec>>,
}

pub struct Importer<'a> {
    pub live: &'a LiveState,
    /// Channel ID → spec key.
    pub channel_keys: HashMap<u64, String>,
    /// Role ID → spec name.
    pub role_names: &'a HashMap<u64, String>,
    pub lock: &'a mut Lockfile,
    pub warnings: &'a mut Vec<Diagnostic>,
}

fn emoji_spec(e: &Option<Emoji>) -> Option<String> {
    e.as_ref().map(Emoji::to_spec)
}

fn nonempty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

/// A key that's unique among `taken`.
fn unique(base: String, taken: &mut Vec<String>) -> String {
    let mut key = base.clone();
    let mut n = 2;
    while taken.contains(&key) {
        key = format!("{base}-{n}");
        n += 1;
    }
    taken.push(key.clone());
    key
}

impl Importer<'_> {
    fn channel(&mut self, id: u64, what: &str) -> Option<String> {
        let key = self.channel_keys.get(&id).cloned();
        if key.is_none() {
            self.warnings.push(Diagnostic::warning(
                what,
                format!("refers to channel {id}, which wasn't imported; left out"),
            ));
        }
        key
    }

    fn role(&mut self, id: u64, what: &str) -> Option<String> {
        let name = self.role_names.get(&id).cloned();
        if name.is_none() {
            self.warnings.push(Diagnostic::warning(
                what,
                format!("refers to role {id}, which wasn't imported; left out"),
            ));
        }
        name
    }

    pub fn import(&mut self) -> Sections {
        let c = &self.live.community;
        let mut out = Sections {
            server: Some(self.server()),
            ..Default::default()
        };
        if let Some(o) = c.onboarding.clone() {
            out.onboarding = Some(self.onboarding(&o));
        }
        if let Some(w) = c.welcome_screen.clone() {
            if w.enabled || !w.channels.is_empty() || !w.description.is_empty() {
                out.welcome_screen = Some(WelcomeScreenSpec {
                    enabled: w.enabled,
                    description: nonempty(&w.description),
                    channels: w
                        .channels
                        .iter()
                        .filter_map(|x| {
                            Some(WelcomeChannelSpec {
                                channel: self.channel(x.channel, "welcome_screen")?,
                                description: x.description.clone(),
                                emoji: emoji_spec(&x.emoji),
                            })
                        })
                        .collect(),
                });
            }
        }
        if let Some(g) = c.guide.clone() {
            out.guide = Some(self.guide(&g));
        }
        if let Some((_, j)) = c.join_questions.clone() {
            if !j.rules.is_empty() || !j.questions.is_empty() {
                out.join_questions = Some(JoinQuestionsSpec {
                    description: nonempty(&j.description),
                    rules: j.rules.clone(),
                    questions: j
                        .questions
                        .iter()
                        .map(|q| QuestionSpec {
                            kind: QuestionKind::from_wire(q.kind)
                                .filter(|k| *k != QuestionKind::Text),
                            label: q.label.clone(),
                            required: q.required,
                            description: nonempty(&q.description),
                            placeholder: nonempty(&q.placeholder),
                            choices: q.choices.clone(),
                        })
                        .collect(),
                });
            }
        }
        if let Some(rules) = c.automod.clone() {
            out.automod = Some(self.automod(&rules));
        }
        let mut no_files = false;
        if let Some(list) = c.emoji.clone() {
            let mut taken = Vec::new();
            out.emoji = Some(
                list.iter()
                    .filter(|e| !e.managed)
                    .map(|e| {
                        let key = unique(key_of(&e.name), &mut taken);
                        self.lock.emoji.insert(key.clone(), Sf(e.id));
                        no_files = true;
                        EmojiSpec {
                            key,
                            name: e.name.clone(),
                            file: None,
                            roles: e
                                .roles
                                .iter()
                                .filter_map(|r| self.role(*r, "emoji"))
                                .collect(),
                        }
                    })
                    .collect(),
            );
        }
        if let Some(list) = c.stickers.clone() {
            let mut taken = Vec::new();
            out.stickers = Some(
                list.iter()
                    .map(|s| {
                        let key = unique(key_of(&s.name), &mut taken);
                        self.lock.stickers.insert(key.clone(), Sf(s.id));
                        no_files = true;
                        StickerSpec {
                            key,
                            name: s.name.clone(),
                            file: None,
                            description: nonempty(&s.description),
                            emoji: s.emoji.clone(),
                        }
                    })
                    .collect(),
            );
        }
        if let Some(list) = c.sounds.clone() {
            let mut taken = Vec::new();
            out.sounds = Some(
                list.iter()
                    .map(|s| {
                        let key = unique(key_of(&s.name), &mut taken);
                        self.lock.sounds.insert(key.clone(), Sf(s.id));
                        no_files = true;
                        SoundSpec {
                            key,
                            name: s.name.clone(),
                            file: None,
                            volume: ((s.volume - 1.0).abs() > 0.005).then_some(s.volume),
                            emoji: emoji_spec(&s.emoji),
                        }
                    })
                    .collect(),
            );
        }
        if no_files {
            self.warnings.push(
                Diagnostic::warning(
                    "",
                    "emoji, stickers and sounds were imported by name only; their files weren't downloaded",
                )
                .with_hint("add `file:` to an entry to manage its image or sound"),
            );
        }
        for (what, why) in &c.errors {
            self.warnings.push(Diagnostic::warning(
                "",
                format!("couldn't read the {what} ({why}); not imported"),
            ));
        }
        out
    }

    fn server(&mut self) -> ServerSpec {
        let s = self.live.community.settings.clone();
        let mut out = ServerSpec {
            name: Some(s.name.clone()),
            description: Some(s.description.clone()),
            verification_level: VerificationLevel::from_wire(s.verification_level),
            default_notifications: Notifications::from_wire(s.default_notifications),
            content_filter: ContentFilter::from_wire(s.content_filter),
            locale: Some(s.locale.clone()),
            boost_progress_bar: Some(s.boost_progress_bar),
            afk_timeout: Some(Seconds(s.afk_timeout)),
            ..Default::default()
        };
        let flag = |bit: u64| Some(s.system_flags & bit == 0);
        out.system_messages = Some(SystemMessages {
            join: flag(system_flags::JOIN),
            join_replies: flag(system_flags::JOIN_REPLIES),
            boosts: flag(system_flags::BOOSTS),
            setup_tips: flag(system_flags::SETUP_TIPS),
            role_subscriptions: flag(system_flags::ROLE_SUBSCRIPTIONS),
            role_subscription_replies: flag(system_flags::ROLE_SUBSCRIPTION_REPLIES),
        });
        for setting in ChannelSetting::ALL {
            let id = s.channel(setting);
            let key = match id {
                Some(id) => match self.channel(id, &format!("server.{}", setting.spec())) {
                    Some(k) => Some(k),
                    None => continue,
                },
                None => None,
            };
            match setting {
                ChannelSetting::System => out.system_channel = Some(key),
                ChannelSetting::Rules => out.rules_channel = key,
                ChannelSetting::Updates => out.updates_channel = key,
                ChannelSetting::SafetyAlerts => out.safety_alerts_channel = Some(key),
                ChannelSetting::Afk => out.afk_channel = Some(key),
            }
        }
        if let Some(w) = self.live.community.widget.clone() {
            let channel = match w.channel.flatten() {
                Some(id) => self.channel(id, "server.widget").map(Some),
                None => Some(None),
            };
            out.widget = Some(WidgetSpec {
                enabled: w.enabled,
                channel,
            });
        }
        out
    }

    fn onboarding(&mut self, o: &Onboarding<u64, u64>) -> OnboardingSpec {
        let mut prompt_keys = Vec::new();
        let prompts = o
            .prompts
            .iter()
            .map(|p| {
                let key = unique(key_of(&p.title), &mut prompt_keys);
                if let Some(id) = p.id {
                    self.lock.onboarding.insert(key.clone(), Sf(id));
                }
                let mut option_keys = Vec::new();
                let options = p
                    .options
                    .iter()
                    .map(|x| {
                        let okey = unique(key_of(&x.title), &mut option_keys);
                        if let Some(id) = x.id {
                            self.lock.onboarding.insert(format!("{key}/{okey}"), Sf(id));
                        }
                        let roles = x
                            .roles
                            .iter()
                            .filter_map(|r| self.role(*r, "onboarding"))
                            .collect();
                        let channels = x
                            .channels
                            .iter()
                            .filter_map(|c| self.channel(*c, "onboarding"))
                            .collect();
                        OptionSpec {
                            key: (okey != key_of(&x.title)).then_some(okey),
                            title: x.title.clone(),
                            description: nonempty(&x.description),
                            emoji: emoji_spec(&x.emoji),
                            roles,
                            channels,
                        }
                    })
                    .collect();
                PromptSpec {
                    key: (key != key_of(&p.title)).then_some(key),
                    title: p.title.clone(),
                    kind: (p.kind == 1).then_some(PromptKind::Dropdown),
                    single: p.single,
                    required: p.required,
                    in_onboarding: p.in_onboarding,
                    options,
                }
            })
            .collect();
        OnboardingSpec {
            enabled: o.enabled,
            mode: (o.mode == 1).then_some(OnboardingMode::Advanced),
            default_channels: o
                .default_channels
                .iter()
                .filter_map(|c| self.channel(*c, "onboarding.default_channels"))
                .collect(),
            prompts,
        }
    }

    fn guide(&mut self, g: &Guide<u64>) -> GuideSpec {
        let actions = g
            .actions
            .iter()
            .filter_map(|a| {
                Some(GuideActionSpec {
                    title: a.title.clone(),
                    channel: self.channel(a.channel, "guide.actions")?,
                    kind: (a.kind == 1).then_some(ActionKind::Chat),
                    description: nonempty(&a.description),
                    emoji: emoji_spec(&a.emoji),
                })
            })
            .collect();
        let resources = g
            .resources
            .iter()
            .filter_map(|r| {
                Some(GuideResourceSpec {
                    title: r.title.clone(),
                    channel: self.channel(r.channel, "guide.resources")?,
                    description: nonempty(&r.description),
                    emoji: emoji_spec(&r.emoji),
                })
            })
            .collect();
        GuideSpec {
            enabled: g.enabled,
            welcome: g.welcome.as_ref().map(|(m, authors)| GuideWelcomeSpec {
                message: m.clone(),
                authors: authors.iter().map(|a| Sf(*a)).collect(),
            }),
            actions,
            resources,
        }
    }

    fn automod(&mut self, rules: &[Automod<u64, u64>]) -> Vec<AutomodSpec> {
        let mut taken = Vec::new();
        let mut out = Vec::new();
        for r in rules {
            let Some(trigger) = AutomodTrigger::from_wire(r.trigger) else {
                self.warnings.push(Diagnostic::warning(
                    "automod",
                    format!(
                        "skipped rule \"{}\": unknown trigger type {}",
                        r.name, r.trigger
                    ),
                ));
                continue;
            };
            let key = unique(key_of(&r.name), &mut taken);
            if let Some(id) = r.id {
                self.lock.automod.insert(key.clone(), Sf(id));
            }
            let actions = r
                .actions
                .iter()
                .filter_map(|a| {
                    Some(match a {
                        AutomodAction::Block(m) => AutomodActionSpec {
                            block: Some(m.clone().map_or(BlockSpec::Plain, BlockSpec::Message)),
                            ..Default::default()
                        },
                        AutomodAction::Alert(c) => AutomodActionSpec {
                            alert: Some(self.channel(*c, "automod")?),
                            ..Default::default()
                        },
                        AutomodAction::Timeout(t) => AutomodActionSpec {
                            timeout: Some(Seconds(*t)),
                            ..Default::default()
                        },
                        AutomodAction::BlockInteractions => AutomodActionSpec {
                            block_interactions: Some(true),
                            ..Default::default()
                        },
                    })
                })
                .collect();
            out.push(AutomodSpec {
                key,
                name: r.name.clone(),
                enabled: r.enabled,
                trigger,
                keywords: r.keywords.clone(),
                regex: r.regex.clone(),
                allow: r.allow.clone(),
                presets: r
                    .presets
                    .iter()
                    .filter_map(|p| AutomodPreset::from_wire(*p))
                    .collect(),
                mention_limit: r.mention_limit,
                raid_protection: r
                    .raid_protection
                    .filter(|_| trigger == AutomodTrigger::MentionSpam),
                actions,
                exempt_roles: r
                    .exempt_roles
                    .iter()
                    .filter_map(|x| self.role(*x, "automod"))
                    .collect(),
                exempt_channels: r
                    .exempt_channels
                    .iter()
                    .filter_map(|x| self.channel(*x, "automod"))
                    .collect(),
            });
        }
        out
    }
}
