//! Compare desired and live server-wide configuration. Each section is
//! compared on a normalised form (sets sorted, IDs matched), so Discord's
//! ordering never causes phantom changes.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use super::model::*;
use crate::diag::{suggest, Diagnostic};
use crate::diff::{LockSection, Op, Ref, Unmanaged, UnmanagedKind};
use crate::lock::Lockfile;
use crate::model::Emoji;
use crate::spec::community::{
    AutomodPreset, AutomodTrigger, ContentFilter, Notifications, OnboardingMode, VerificationLevel,
};

/// A display line under a community op in the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub tone: LineTone,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineTone {
    Add,
    Change,
    Remove,
    Note,
}

fn add(text: impl Into<String>) -> Line {
    Line {
        tone: LineTone::Add,
        text: text.into(),
    }
}
fn change(text: impl Into<String>) -> Line {
    Line {
        tone: LineTone::Change,
        text: text.into(),
    }
}
fn remove(text: impl Into<String>) -> Line {
    Line {
        tone: LineTone::Remove,
        text: text.into(),
    }
}
fn note(text: impl Into<String>) -> Line {
    Line {
        tone: LineTone::Note,
        text: text.into(),
    }
}
fn field_line<T: std::fmt::Display>(name: &str, before: T, after: T) -> Line {
    change(format!("{name:<20} {before} → {after}"))
}

/// One server setting to change.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingChange {
    pub field: &'static str,
    pub before: String,
    pub after: String,
    pub wire: &'static str,
    pub value: SettingValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Json(Value),
    Channel(Option<Ref>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommunityOp {
    Settings(Vec<SettingChange>),
    Widget {
        enabled: bool,
        /// `None`: leave the channel as it is.
        channel: Option<Option<Ref>>,
        lines: Vec<Line>,
    },
    Onboarding {
        after: Onboarding<Ref, Ref>,
        lines: Vec<Line>,
    },
    WelcomeScreen {
        after: WelcomeScreen<Ref>,
        lines: Vec<Line>,
    },
    Guide {
        after: Guide<Ref>,
        lines: Vec<Line>,
    },
    JoinQuestions {
        /// Discord's form version, sent back to avoid clobbering edits.
        version: String,
        after: JoinQuestions,
        lines: Vec<Line>,
    },
    CreateAutomod {
        rule: Automod<Ref, Ref>,
        lines: Vec<Line>,
    },
    UpdateAutomod {
        id: u64,
        rule: Automod<Ref, Ref>,
        lines: Vec<Line>,
    },
    DeleteAutomod {
        id: u64,
        name: String,
    },
    CreateEmoji {
        key: String,
        name: String,
        asset: Asset,
        roles: Vec<Ref>,
    },
    UpdateEmoji {
        key: String,
        id: u64,
        name: String,
        roles: Vec<Ref>,
        lines: Vec<Line>,
    },
    DeleteEmoji {
        id: u64,
        name: String,
    },
    CreateSticker {
        key: String,
        name: String,
        asset: Asset,
        description: String,
        emoji: String,
    },
    UpdateSticker {
        key: String,
        id: u64,
        name: String,
        description: String,
        emoji: String,
        lines: Vec<Line>,
    },
    DeleteSticker {
        id: u64,
        name: String,
    },
    CreateSound {
        key: String,
        name: String,
        asset: Asset,
        volume: f64,
        emoji: Option<Emoji>,
    },
    UpdateSound {
        key: String,
        id: u64,
        name: String,
        volume: f64,
        emoji: Option<Emoji>,
        lines: Vec<Line>,
    },
    DeleteSound {
        id: u64,
        name: String,
    },
}

impl CommunityOp {
    pub fn is_create(&self) -> bool {
        matches!(
            self,
            CommunityOp::CreateAutomod { .. }
                | CommunityOp::CreateEmoji { .. }
                | CommunityOp::CreateSticker { .. }
                | CommunityOp::CreateSound { .. }
        )
    }
    pub fn is_delete(&self) -> bool {
        matches!(
            self,
            CommunityOp::DeleteAutomod { .. }
                | CommunityOp::DeleteEmoji { .. }
                | CommunityOp::DeleteSticker { .. }
                | CommunityOp::DeleteSound { .. }
        )
    }
}

/// How the planner refers to roles and channels, and how it names them.
pub struct Refs<'a> {
    /// Role name → reference; `None` if several live roles share the name.
    pub roles: &'a HashMap<String, Option<Ref>>,
    /// Channel key → reference.
    pub channels: HashMap<String, Ref>,
    /// Display labels (`#general`, `category voice 🔊`, `@Lead`).
    pub labels: HashMap<Ref, String>,
}

impl Refs<'_> {
    fn channel(&self, key: &str) -> Ref {
        self.channels
            .get(key)
            .cloned()
            .unwrap_or_else(|| Ref::New(key.to_owned()))
    }

    fn label(&self, r: &Ref) -> String {
        self.labels.get(r).cloned().unwrap_or_else(|| match r {
            Ref::Id(id) => format!("<{id}>"),
            Ref::New(k) => k.clone(),
        })
    }

    fn labels(&self, rs: &[Ref]) -> String {
        if rs.is_empty() {
            return "none".to_owned();
        }
        rs.iter()
            .map(|r| self.label(r))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Resolve role names, reporting unknown or ambiguous ones.
    fn roles(&self, names: &[String], path: &str, diags: &mut Vec<Diagnostic>) -> Vec<Ref> {
        let mut out = Vec::new();
        for (i, n) in names.iter().enumerate() {
            match self.roles.get(n) {
                Some(Some(r)) => out.push(r.clone()),
                Some(None) => diags.push(
                    Diagnostic::error(
                        format!("{path}[{i}]"),
                        format!("several live roles are called `{n}`"),
                    )
                    .with_hint("rename one in Discord, or add the role to the spec"),
                ),
                None => {
                    let hint = suggest(n, self.roles.keys().map(String::as_str));
                    diags.push(
                        Diagnostic::error(format!("{path}[{i}]"), format!("no role called `{n}`"))
                            .with_hint_opt(hint),
                    );
                }
            }
        }
        out
    }
}

#[derive(Debug, Default)]
pub struct CommunityPlan {
    pub ops: Vec<Op>,
    pub bindings: Vec<(LockSection, String, u64)>,
    pub assets: Vec<(String, String)>,
    pub unmanaged: Vec<Unmanaged>,
}

pub struct Inputs<'a> {
    pub desired: &'a Desired,
    pub live: &'a Live,
    pub lock: &'a Lockfile,
    pub refs: &'a Refs<'a>,
    pub prune: bool,
    pub features: &'a [String],
}

pub fn plan(input: &Inputs<'_>, diags: &mut Vec<Diagnostic>) -> CommunityPlan {
    let mut p = Planner {
        input,
        out: CommunityPlan::default(),
        diags,
    };
    let d = input.desired;
    if let Some(s) = &d.settings {
        p.settings(s);
    }
    if let Some(w) = &d.widget {
        p.widget(w);
    }
    if let Some(list) = &d.automod {
        p.automod(list);
    }
    if let Some(list) = &d.emoji {
        p.emoji(list);
    }
    if let Some(list) = &d.stickers {
        p.stickers(list);
    }
    if let Some(list) = &d.sounds {
        p.sounds(list);
    }
    if let Some(o) = &d.onboarding {
        p.onboarding(o);
    }
    if let Some(w) = &d.welcome_screen {
        p.welcome(w);
    }
    if let Some(g) = &d.guide {
        p.guide(g);
    }
    if let Some(j) = &d.join_questions {
        p.join(j);
    }
    p.out
}

struct Planner<'a, 'b> {
    input: &'a Inputs<'a>,
    out: CommunityPlan,
    diags: &'b mut Vec<Diagnostic>,
}

fn sorted<T: Ord + Clone>(v: &[T]) -> Vec<T> {
    let mut v = v.to_vec();
    v.sort();
    v
}

fn quoted(s: &str) -> String {
    let short: String = s.chars().take(60).collect();
    let more = if s.chars().count() > 60 { "..." } else { "" };
    format!("\"{}{more}\"", short.replace('\n', " ⏎ "))
}

fn emoji_label(e: &Option<Emoji>) -> String {
    match e {
        Some(e) => e.to_spec(),
        None => "none".to_owned(),
    }
}

fn label_of<T: Copy, F: Fn(u8) -> Option<T>>(v: u8, f: F, name: fn(T) -> &'static str) -> String {
    f(v).map_or_else(|| v.to_string(), |x| name(x).to_owned())
}

impl Planner<'_, '_> {
    fn op(&mut self, op: CommunityOp) {
        self.out.ops.push(Op::Community(op));
    }

    /// The live part, or an error if it couldn't be read.
    fn readable<'x, T>(&mut self, live: &'x Option<T>, what: &str, path: &str) -> Option<&'x T> {
        if live.is_none() {
            let why = self
                .input
                .live
                .errors
                .iter()
                .find(|(w, _)| w == what || w == "server-wide settings")
                .map_or("not available", |(_, why)| why.as_str());
            self.diags.push(
                Diagnostic::error(
                    path,
                    format!("couldn't read the {what} from Discord ({why})"),
                )
                .with_hint(
                    "check the bot has Manage Server; or remove this section to leave it alone",
                ),
            );
        }
        live.as_ref()
    }

    fn unmanaged(&mut self, kind: UnmanagedKind, id: u64, name: &str) {
        self.out.unmanaged.push(Unmanaged {
            kind,
            id,
            name: name.to_owned(),
            note: None,
            pruned: self.input.prune,
        });
    }

    // -- settings ----------------------------------------------------------

    fn settings(&mut self, d: &Settings) {
        let l = &self.input.live.settings;
        let refs = self.input.refs;
        let mut changes = Vec::new();
        let mut push = |field: &'static str,
                        wire: &'static str,
                        before: String,
                        after: String,
                        value: Value| {
            if before != after {
                changes.push(SettingChange {
                    field,
                    before,
                    after,
                    wire,
                    value: SettingValue::Json(value),
                });
            }
        };
        if let Some(n) = &d.name {
            push("name", "name", quoted(&l.name), quoted(n), json!(n));
        }
        if let Some(v) = &d.description {
            let show = |s: &str| {
                if s.is_empty() {
                    "(empty)".to_owned()
                } else {
                    quoted(s)
                }
            };
            push(
                "description",
                "description",
                show(&l.description),
                show(v),
                json!(v),
            );
        }
        if let Some(v) = d.verification_level {
            let name = |x| label_of(x, VerificationLevel::from_wire, VerificationLevel::as_str);
            push(
                "verification_level",
                "verification_level",
                name(l.verification_level),
                name(v),
                json!(v),
            );
        }
        if let Some(v) = d.default_notifications {
            let name = |x| label_of(x, Notifications::from_wire, Notifications::as_str);
            push(
                "default_notifications",
                "default_message_notifications",
                name(l.default_notifications),
                name(v),
                json!(v),
            );
        }
        if let Some(v) = d.content_filter {
            let name = |x| label_of(x, ContentFilter::from_wire, ContentFilter::as_str);
            push(
                "content_filter",
                "explicit_content_filter",
                name(l.content_filter),
                name(v),
                json!(v),
            );
        }
        if let Some(v) = &d.locale {
            push(
                "locale",
                "preferred_locale",
                l.locale.clone(),
                v.clone(),
                json!(v),
            );
        }
        if let Some(v) = d.boost_progress_bar {
            push(
                "boost_progress_bar",
                "premium_progress_bar_enabled",
                l.boost_progress_bar.to_string(),
                v.to_string(),
                json!(v),
            );
        }
        if let Some(v) = d.afk_timeout {
            let show = |s: u32| format!("{}m", s / 60);
            push(
                "afk_timeout",
                "afk_timeout",
                show(l.afk_timeout),
                show(v),
                json!(v),
            );
        }
        if let Some((mask, value)) = d.system_flags {
            let new = (l.system_flags & !mask) | value;
            let show = |flags: u64| {
                let on: Vec<&str> = system_flags::NAMES
                    .iter()
                    .filter(|(bit, _)| flags & bit == 0)
                    .map(|(_, n)| *n)
                    .collect();
                if on.is_empty() {
                    "none".to_owned()
                } else {
                    on.join(", ")
                }
            };
            push(
                "system_messages",
                "system_channel_flags",
                show(l.system_flags),
                show(new),
                json!(new),
            );
        }
        for (setting, key) in &d.channels {
            let after = key.as_deref().map(|k| refs.channel(k));
            let before = l.channel(*setting).map(Ref::Id);
            if after != before {
                let show = |r: &Option<Ref>| {
                    r.as_ref()
                        .map_or_else(|| "none".to_owned(), |r| refs.label(r))
                };
                changes.push(SettingChange {
                    field: setting.spec(),
                    before: show(&before),
                    after: show(&after),
                    wire: setting.wire(),
                    value: SettingValue::Channel(after),
                });
            }
        }
        let community = self.input.features.iter().any(|f| f == "COMMUNITY");
        for c in &changes {
            if !community
                && matches!(
                    c.field,
                    "rules_channel" | "updates_channel" | "safety_alerts_channel"
                )
            {
                self.diags.push(Diagnostic::error(
                    format!("{}.{}", d.path, c.field),
                    "only Community servers have this channel (Server Settings → Enable Community)",
                ));
            }
        }
        if !changes.is_empty() {
            self.op(CommunityOp::Settings(changes));
        }
    }

    fn widget(&mut self, d: &Widget<String>) {
        let Some(l) = self.readable(&self.input.live.widget, "widget", &d.path) else {
            return;
        };
        let refs = self.input.refs;
        let channel = d
            .channel
            .as_ref()
            .map(|c| c.as_deref().map(|k| refs.channel(k)));
        let before_ch = l.channel.flatten().map(Ref::Id);
        let mut lines = Vec::new();
        if d.enabled != l.enabled {
            lines.push(field_line("enabled", l.enabled, d.enabled));
        }
        if let Some(after) = &channel {
            if *after != before_ch {
                let show = |r: &Option<Ref>| {
                    r.as_ref()
                        .map_or_else(|| "none".to_owned(), |r| refs.label(r))
                };
                lines.push(field_line("channel", show(&before_ch), show(after)));
            }
        }
        if !lines.is_empty() {
            self.op(CommunityOp::Widget {
                enabled: d.enabled,
                channel,
                lines,
            });
        }
    }

    // -- onboarding --------------------------------------------------------

    fn onboarding(&mut self, d: &Onboarding<String, String>) {
        let Some(l) = self.readable(&self.input.live.onboarding, "onboarding", &d.path) else {
            return;
        };
        let l = l.map_refs(&mut id_ref, &mut id_ref);
        let refs = self.input.refs;
        let lock = self.input.lock;

        // Resolve roles (reporting errors), then everything else.
        let mut role_refs: HashMap<(usize, usize), Vec<Ref>> = HashMap::new();
        for (i, p) in d.prompts.iter().enumerate() {
            for (j, o) in p.options.iter().enumerate() {
                role_refs.insert(
                    (i, j),
                    refs.roles(&o.roles, &format!("{}.roles", o.path), self.diags),
                );
            }
        }
        let mut after = Onboarding {
            path: d.path.clone(),
            enabled: d.enabled,
            mode: d.mode,
            default_channels: d.default_channels.iter().map(|k| refs.channel(k)).collect(),
            prompts: d
                .prompts
                .iter()
                .enumerate()
                .map(|(i, p)| Prompt {
                    path: p.path.clone(),
                    id: None,
                    key: p.key.clone(),
                    title: p.title.clone(),
                    kind: p.kind,
                    single: p.single,
                    required: p.required,
                    in_onboarding: p.in_onboarding,
                    options: p
                        .options
                        .iter()
                        .enumerate()
                        .map(|(j, o)| PromptOption {
                            path: o.path.clone(),
                            id: None,
                            key: o.key.clone(),
                            title: o.title.clone(),
                            description: o.description.clone(),
                            emoji: o.emoji.clone(),
                            roles: role_refs.remove(&(i, j)).unwrap_or_default(),
                            channels: o.channels.iter().map(|k| refs.channel(k)).collect(),
                        })
                        .collect(),
                })
                .collect(),
        };

        // Match prompts and options to live ones: lockfile, then title.
        let mut used_prompts = HashSet::new();
        for p in &mut after.prompts {
            let by_lock = lock
                .onboarding
                .get(&p.key)
                .and_then(|id| l.prompts.iter().position(|x| x.id == Some(id.0)));
            let found = by_lock
                .filter(|i| !used_prompts.contains(i))
                .or_else(|| {
                    l.prompts
                        .iter()
                        .enumerate()
                        .position(|(i, x)| !used_prompts.contains(&i) && x.title == p.title)
                })
                .or_else(|| {
                    l.prompts.iter().enumerate().position(|(i, x)| {
                        !used_prompts.contains(&i)
                            && x.title.to_lowercase() == p.title.to_lowercase()
                    })
                });
            let Some(i) = found else { continue };
            used_prompts.insert(i);
            let lp = &l.prompts[i];
            p.id = lp.id;
            self.out
                .bindings
                .push((LockSection::Onboarding, p.key.clone(), lp.id.unwrap_or(0)));
            let mut used = HashSet::new();
            for o in &mut p.options {
                let okey = format!("{}/{}", p.key, o.key);
                let found = lock
                    .onboarding
                    .get(&okey)
                    .and_then(|id| lp.options.iter().position(|x| x.id == Some(id.0)))
                    .filter(|j| !used.contains(j))
                    .or_else(|| {
                        lp.options
                            .iter()
                            .enumerate()
                            .position(|(j, x)| !used.contains(&j) && x.title == o.title)
                    });
                if let Some(j) = found {
                    used.insert(j);
                    o.id = lp.options[j].id;
                    self.out.bindings.push((
                        LockSection::Onboarding,
                        okey,
                        lp.options[j].id.unwrap_or(0),
                    ));
                }
            }
        }

        if canon_onboarding(&after) == canon_onboarding(&l) {
            return;
        }
        let lines = onboarding_lines(&l, &after, refs);
        self.op(CommunityOp::Onboarding { after, lines });
    }

    // -- welcome screen, guide, join questions ------------------------------

    fn welcome(&mut self, d: &WelcomeScreen<String>) {
        let Some(l) = self.readable(&self.input.live.welcome_screen, "welcome screen", &d.path)
        else {
            return;
        };
        let refs = self.input.refs;
        let after = d.map_channels(&mut |k| refs.channel(k));
        let l = l.map_channels(&mut id_ref);
        let canon = |w: &WelcomeScreen<Ref>| {
            (
                w.enabled,
                w.description.clone(),
                w.channels
                    .iter()
                    .map(|c| (c.channel.clone(), c.description.clone(), c.emoji.clone()))
                    .collect::<Vec<_>>(),
            )
        };
        if canon(&after) == canon(&l) {
            return;
        }
        let mut lines = Vec::new();
        if after.enabled != l.enabled {
            lines.push(field_line("enabled", l.enabled, after.enabled));
        }
        if after.description != l.description {
            lines.push(field_line(
                "description",
                quoted(&l.description),
                quoted(&after.description),
            ));
        }
        list_lines(
            &mut lines,
            "channel",
            &l.channels,
            &after.channels,
            |c| c.channel.clone(),
            |c| format!("{}: {}", refs.label(&c.channel), quoted(&c.description)),
        );
        self.op(CommunityOp::WelcomeScreen { after, lines });
    }

    fn guide(&mut self, d: &Guide<String>) {
        let Some(l) = self.readable(&self.input.live.guide, "Server Guide", &d.path) else {
            return;
        };
        let refs = self.input.refs;
        let mut after = d.map_channels(&mut |k| refs.channel(k));
        let l = l.map_channels(&mut id_ref);
        // Keep images uploaded in the client.
        for (mine, theirs) in [
            (&mut after.actions, &l.actions),
            (&mut after.resources, &l.resources),
        ] {
            for item in mine.iter_mut() {
                item.icon = theirs
                    .iter()
                    .find(|t| t.channel == item.channel && t.title == item.title)
                    .and_then(|t| t.icon.clone());
            }
        }
        let canon = |g: &Guide<Ref>| {
            let items = |v: &[GuideItem<Ref>]| {
                v.iter()
                    .map(|i| {
                        (
                            i.channel.clone(),
                            i.title.clone(),
                            i.description.clone(),
                            i.kind,
                            i.emoji.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            (
                g.enabled,
                g.welcome.clone(),
                items(&g.actions),
                items(&g.resources),
            )
        };
        if canon(&after) == canon(&l) {
            return;
        }
        let mut lines = Vec::new();
        if after.enabled != l.enabled {
            lines.push(field_line("enabled", l.enabled, after.enabled));
        }
        if after.welcome != l.welcome {
            let msg = |w: &Option<(String, Vec<u64>)>| {
                w.as_ref()
                    .map_or_else(|| "none".to_owned(), |(m, _)| quoted(m))
            };
            lines.push(field_line(
                "welcome message",
                msg(&l.welcome),
                msg(&after.welcome),
            ));
        }
        let item_label =
            |i: &GuideItem<Ref>| format!("{} → {}", quoted(&i.title), refs.label(&i.channel));
        list_lines(
            &mut lines,
            "to-do",
            &l.actions,
            &after.actions,
            |i| (i.channel.clone(), i.title.clone()),
            item_label,
        );
        list_lines(
            &mut lines,
            "resource",
            &l.resources,
            &after.resources,
            |i| (i.channel.clone(), i.title.clone()),
            item_label,
        );
        self.op(CommunityOp::Guide { after, lines });
    }

    fn join(&mut self, d: &JoinQuestions) {
        let Some((version, l)) =
            self.readable(&self.input.live.join_questions, "join questions", &d.path)
        else {
            return;
        };
        let canon =
            |j: &JoinQuestions| (j.description.clone(), j.rules.clone(), j.questions.clone());
        if canon(d) == canon(l) {
            return;
        }
        let mut lines = Vec::new();
        if d.description != l.description {
            lines.push(field_line(
                "description",
                quoted(&l.description),
                quoted(&d.description),
            ));
        }
        if d.rules != l.rules {
            let before = lines.len();
            for r in &d.rules {
                if !l.rules.contains(r) {
                    lines.push(add(format!("rule {}", quoted(r))));
                }
            }
            for r in &l.rules {
                if !d.rules.contains(r) {
                    lines.push(remove(format!("rule {}", quoted(r))));
                }
            }
            if lines.len() == before {
                lines.push(change("rules reordered"));
            }
        }
        let kind = |k: u8| {
            ["text", "paragraph", "choice"]
                .get(usize::from(k))
                .copied()
                .unwrap_or("?")
        };
        list_lines(
            &mut lines,
            "question",
            &l.questions,
            &d.questions,
            |q| q.label.clone(),
            |q| {
                format!(
                    "{} ({}{})",
                    quoted(&q.label),
                    kind(q.kind),
                    if q.required { ", required" } else { "" }
                )
            },
        );
        self.op(CommunityOp::JoinQuestions {
            version: version.clone(),
            after: d.clone(),
            lines,
        });
    }

    // -- AutoMod -----------------------------------------------------------

    fn automod(&mut self, list: &[Automod<String, String>]) {
        let Some(live) = self.readable(&self.input.live.automod, "AutoMod rules", "automod") else {
            return;
        };
        let refs = self.input.refs;
        let lock = self.input.lock;
        let mut used = HashSet::new();
        for d in list {
            let roles = refs.roles(
                &d.exempt_roles,
                &format!("{}.exempt_roles", d.path),
                self.diags,
            );
            let mut rule = Automod {
                path: d.path.clone(),
                id: None,
                key: d.key.clone(),
                name: d.name.clone(),
                enabled: d.enabled,
                trigger: d.trigger,
                event: d.event,
                keywords: d.keywords.clone(),
                regex: d.regex.clone(),
                allow: d.allow.clone(),
                presets: d.presets.clone(),
                mention_limit: d.mention_limit,
                raid_protection: d.raid_protection,
                actions: d
                    .actions
                    .iter()
                    .map(|a| match a {
                        AutomodAction::Block(m) => AutomodAction::Block(m.clone()),
                        AutomodAction::Alert(k) => AutomodAction::Alert(refs.channel(k)),
                        AutomodAction::Timeout(t) => AutomodAction::Timeout(*t),
                        AutomodAction::BlockInteractions => AutomodAction::BlockInteractions,
                    })
                    .collect(),
                exempt_roles: roles,
                exempt_channels: d.exempt_channels.iter().map(|k| refs.channel(k)).collect(),
            };
            let found = lock
                .automod
                .get(&d.key)
                .and_then(|id| live.iter().position(|r| r.id == Some(id.0)))
                .or_else(|| live.iter().position(|r| r.name == d.name))
                .filter(|i| used.insert(*i));
            let Some(i) = found else {
                let lines = vec![note(automod_summary(&rule, refs))];
                self.op(CommunityOp::CreateAutomod { rule, lines });
                continue;
            };
            let l = live[i].map_refs(&mut id_ref, &mut id_ref);
            let id = l.id.unwrap_or(0);
            rule.id = Some(id);
            self.out
                .bindings
                .push((LockSection::Automod, d.key.clone(), id));
            if canon_automod(&rule) == canon_automod(&l) {
                continue;
            }
            if rule.trigger != l.trigger {
                // Discord can't change a rule's trigger; replace it.
                self.op(CommunityOp::DeleteAutomod {
                    id,
                    name: l.name.clone(),
                });
                rule.id = None;
                let lines = vec![note(automod_summary(&rule, refs))];
                self.op(CommunityOp::CreateAutomod { rule, lines });
                continue;
            }
            let lines = automod_lines(&l, &rule, refs);
            self.op(CommunityOp::UpdateAutomod { id, rule, lines });
        }
        for (i, r) in live.iter().enumerate() {
            if used.contains(&i) {
                continue;
            }
            let id = r.id.unwrap_or(0);
            self.unmanaged(UnmanagedKind::Automod, id, &r.name);
            if self.input.prune {
                self.op(CommunityOp::DeleteAutomod {
                    id,
                    name: r.name.clone(),
                });
            }
        }
    }

    // -- expressions --------------------------------------------------------

    /// Match a desired expression to a live one: lockfile, then name.
    #[allow(clippy::too_many_arguments)]
    fn match_expr<T>(
        &self,
        section: &std::collections::BTreeMap<String, crate::snowflake::Sf>,
        key: &str,
        name: &str,
        live: &[T],
        id: impl Fn(&T) -> u64,
        lname: impl Fn(&T) -> &str,
        used: &mut HashSet<usize>,
    ) -> Option<usize> {
        section
            .get(key)
            .and_then(|sf| live.iter().position(|x| id(x) == sf.0))
            .or_else(|| {
                live.iter()
                    .enumerate()
                    .position(|(i, x)| !used.contains(&i) && lname(x) == name)
            })
            .filter(|i| used.insert(*i))
    }

    /// Whether the file changed since the last upload. Records the hash.
    fn asset_changed(&mut self, section: &str, key: &str, asset: Option<&Asset>) -> bool {
        let Some(a) = asset else { return false };
        let lock_key = format!("{section}/{key}");
        let old = self.input.lock.assets.get(&lock_key).cloned();
        self.out.assets.push((lock_key, a.sha256.clone()));
        // No hash yet (adopted): assume it's the same picture.
        old.is_some_and(|h| h != a.sha256)
    }

    fn emoji(&mut self, list: &[DesiredEmoji]) {
        let Some(live) = self.readable(&self.input.live.emoji, "emoji", "emoji") else {
            return;
        };
        let live: Vec<&LiveEmoji> = live.iter().filter(|e| !e.managed).collect();
        let refs = self.input.refs;
        let mut used = HashSet::new();
        for d in list {
            let roles = refs.roles(&d.roles, &format!("{}.roles", d.path), self.diags);
            let found = self.match_expr(
                &self.input.lock.emoji,
                &d.key,
                &d.name,
                &live,
                |e| e.id,
                |e| &e.name,
                &mut used,
            );
            match found {
                Some(i) if !self.asset_changed("emoji", &d.key, d.file.as_ref()) => {
                    let l = live[i];
                    self.out
                        .bindings
                        .push((LockSection::Emoji, d.key.clone(), l.id));
                    let live_roles: Vec<Ref> = l.roles.iter().map(id_ref).collect();
                    let mut lines = Vec::new();
                    if l.name != d.name {
                        lines.push(field_line("name", &l.name, &d.name));
                    }
                    if sorted(&live_roles) != sorted(&roles) {
                        lines.push(field_line(
                            "roles",
                            refs.labels(&live_roles),
                            refs.labels(&roles),
                        ));
                    }
                    if !lines.is_empty() {
                        self.op(CommunityOp::UpdateEmoji {
                            key: d.key.clone(),
                            id: l.id,
                            name: d.name.clone(),
                            roles,
                            lines,
                        });
                    }
                }
                found => {
                    if let Some(i) = found {
                        // The image changed: Discord can't replace it in place.
                        self.op(CommunityOp::DeleteEmoji {
                            id: live[i].id,
                            name: live[i].name.clone(),
                        });
                    }
                    match &d.file {
                        Some(asset) => self.op(CommunityOp::CreateEmoji {
                            key: d.key.clone(),
                            name: d.name.clone(),
                            asset: asset.clone(),
                            roles,
                        }),
                        None => self.diags.push(
                            Diagnostic::error(
                                &d.path,
                                format!("emoji `{}` doesn't exist and has no `file`", d.name),
                            )
                            .with_hint("add `file:` to upload it"),
                        ),
                    }
                }
            }
        }
        for (i, e) in live.iter().enumerate() {
            if !used.contains(&i) {
                self.unmanaged(UnmanagedKind::Emoji, e.id, &e.name);
                if self.input.prune {
                    self.op(CommunityOp::DeleteEmoji {
                        id: e.id,
                        name: e.name.clone(),
                    });
                }
            }
        }
    }

    fn stickers(&mut self, list: &[DesiredSticker]) {
        let Some(live) = self.readable(&self.input.live.stickers, "stickers", "stickers") else {
            return;
        };
        let mut used = HashSet::new();
        for d in list {
            let found = self.match_expr(
                &self.input.lock.stickers,
                &d.key,
                &d.name,
                live,
                |s| s.id,
                |s| &s.name,
                &mut used,
            );
            match found {
                Some(i) if !self.asset_changed("stickers", &d.key, d.file.as_ref()) => {
                    let l = &live[i];
                    self.out
                        .bindings
                        .push((LockSection::Sticker, d.key.clone(), l.id));
                    let mut lines = Vec::new();
                    if l.name != d.name {
                        lines.push(field_line("name", &l.name, &d.name));
                    }
                    if l.description != d.description {
                        lines.push(field_line(
                            "description",
                            quoted(&l.description),
                            quoted(&d.description),
                        ));
                    }
                    if Emoji::unicode(&l.emoji) != Emoji::unicode(&d.emoji) {
                        lines.push(field_line("emoji", &l.emoji, &d.emoji));
                    }
                    if !lines.is_empty() {
                        self.op(CommunityOp::UpdateSticker {
                            key: d.key.clone(),
                            id: l.id,
                            name: d.name.clone(),
                            description: d.description.clone(),
                            emoji: d.emoji.clone(),
                            lines,
                        });
                    }
                }
                found => {
                    if let Some(i) = found {
                        self.op(CommunityOp::DeleteSticker {
                            id: live[i].id,
                            name: live[i].name.clone(),
                        });
                    }
                    match &d.file {
                        Some(asset) => self.op(CommunityOp::CreateSticker {
                            key: d.key.clone(),
                            name: d.name.clone(),
                            asset: asset.clone(),
                            description: d.description.clone(),
                            emoji: d.emoji.clone(),
                        }),
                        None => self.diags.push(
                            Diagnostic::error(
                                &d.path,
                                format!("sticker `{}` doesn't exist and has no `file`", d.name),
                            )
                            .with_hint("add `file:` to upload it"),
                        ),
                    }
                }
            }
        }
        for (i, s) in live.iter().enumerate() {
            if !used.contains(&i) {
                self.unmanaged(UnmanagedKind::Sticker, s.id, &s.name);
                if self.input.prune {
                    self.op(CommunityOp::DeleteSticker {
                        id: s.id,
                        name: s.name.clone(),
                    });
                }
            }
        }
    }

    fn sounds(&mut self, list: &[DesiredSound]) {
        let Some(live) = self.readable(&self.input.live.sounds, "soundboard", "sounds") else {
            return;
        };
        let mut used = HashSet::new();
        for d in list {
            let found = self.match_expr(
                &self.input.lock.sounds,
                &d.key,
                &d.name,
                live,
                |s| s.id,
                |s| &s.name,
                &mut used,
            );
            match found {
                Some(i) if !self.asset_changed("sounds", &d.key, d.file.as_ref()) => {
                    let l = &live[i];
                    self.out
                        .bindings
                        .push((LockSection::Sound, d.key.clone(), l.id));
                    let mut lines = Vec::new();
                    if l.name != d.name {
                        lines.push(field_line("name", &l.name, &d.name));
                    }
                    if (l.volume - d.volume).abs() > 0.005 {
                        lines.push(field_line("volume", l.volume, d.volume));
                    }
                    if l.emoji != d.emoji {
                        lines.push(field_line(
                            "emoji",
                            emoji_label(&l.emoji),
                            emoji_label(&d.emoji),
                        ));
                    }
                    if !lines.is_empty() {
                        self.op(CommunityOp::UpdateSound {
                            key: d.key.clone(),
                            id: l.id,
                            name: d.name.clone(),
                            volume: d.volume,
                            emoji: d.emoji.clone(),
                            lines,
                        });
                    }
                }
                found => {
                    if let Some(i) = found {
                        self.op(CommunityOp::DeleteSound {
                            id: live[i].id,
                            name: live[i].name.clone(),
                        });
                    }
                    match &d.file {
                        Some(asset) => self.op(CommunityOp::CreateSound {
                            key: d.key.clone(),
                            name: d.name.clone(),
                            asset: asset.clone(),
                            volume: d.volume,
                            emoji: d.emoji.clone(),
                        }),
                        None => self.diags.push(
                            Diagnostic::error(
                                &d.path,
                                format!("sound `{}` doesn't exist and has no `file`", d.name),
                            )
                            .with_hint("add `file:` to upload it"),
                        ),
                    }
                }
            }
        }
        for (i, s) in live.iter().enumerate() {
            if !used.contains(&i) {
                self.unmanaged(UnmanagedKind::Sound, s.id, &s.name);
                if self.input.prune {
                    self.op(CommunityOp::DeleteSound {
                        id: s.id,
                        name: s.name.clone(),
                    });
                }
            }
        }
    }
}

// -- comparison and display helpers ---------------------------------------------

type CanonOption = (
    Option<u64>,
    String,
    String,
    Option<Emoji>,
    Vec<Ref>,
    Vec<Ref>,
);
type CanonPrompt = (Option<u64>, String, u8, bool, bool, bool, Vec<CanonOption>);

fn canon_onboarding(o: &Onboarding<Ref, Ref>) -> (bool, u8, Vec<Ref>, Vec<CanonPrompt>) {
    (
        o.enabled,
        o.mode,
        sorted(&o.default_channels),
        o.prompts
            .iter()
            .map(|p| {
                (
                    p.id,
                    p.title.clone(),
                    p.kind,
                    p.single,
                    p.required,
                    p.in_onboarding,
                    p.options
                        .iter()
                        .map(|x| {
                            (
                                x.id,
                                x.title.clone(),
                                x.description.clone(),
                                x.emoji.clone(),
                                sorted(&x.roles),
                                sorted(&x.channels),
                            )
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}

fn option_summary(o: &PromptOption<Ref, Ref>, refs: &Refs<'_>) -> String {
    let mut gives: Vec<String> = o.roles.iter().map(|r| refs.label(r)).collect();
    gives.extend(o.channels.iter().map(|c| refs.label(c)));
    let emoji = o
        .emoji
        .as_ref()
        .map(|e| format!("{} ", e.to_spec()))
        .unwrap_or_default();
    format!("{emoji}{} → {}", quoted(&o.title), gives.join(", "))
}

fn onboarding_lines(
    l: &Onboarding<Ref, Ref>,
    a: &Onboarding<Ref, Ref>,
    refs: &Refs<'_>,
) -> Vec<Line> {
    let mut lines = Vec::new();
    if l.enabled != a.enabled {
        lines.push(field_line("enabled", l.enabled, a.enabled));
    }
    if l.mode != a.mode {
        let name = |m| label_of(m, OnboardingMode::from_wire, OnboardingMode::as_str);
        lines.push(field_line("mode", name(l.mode), name(a.mode)));
    }
    for c in &a.default_channels {
        if !l.default_channels.contains(c) {
            lines.push(add(format!("default channel {}", refs.label(c))));
        }
    }
    for c in &l.default_channels {
        if !a.default_channels.contains(c) {
            lines.push(remove(format!("default channel {}", refs.label(c))));
        }
    }
    let flag = |b: bool| if b { "yes" } else { "no" };
    for p in &a.prompts {
        let Some(lp) =
            p.id.and_then(|id| l.prompts.iter().find(|x| x.id == Some(id)))
        else {
            let how = format!(
                "{}{}{}",
                if p.in_onboarding {
                    "asked on join"
                } else {
                    "Channels & Roles only"
                },
                if p.required { ", required" } else { "" },
                if p.single { ", one answer" } else { "" }
            );
            lines.push(add(format!("prompt {} ({how})", quoted(&p.title))));
            for o in &p.options {
                lines.push(add(format!("  option {}", option_summary(o, refs))));
            }
            continue;
        };
        let mut sub = Vec::new();
        if lp.title != p.title {
            sub.push(field_line("  title", quoted(&lp.title), quoted(&p.title)));
        }
        if lp.in_onboarding != p.in_onboarding {
            sub.push(field_line(
                "  asked on join",
                flag(lp.in_onboarding),
                flag(p.in_onboarding),
            ));
        }
        if lp.required != p.required {
            sub.push(field_line(
                "  required",
                flag(lp.required),
                flag(p.required),
            ));
        }
        if lp.single != p.single {
            sub.push(field_line(
                "  one answer only",
                flag(lp.single),
                flag(p.single),
            ));
        }
        if lp.kind != p.kind {
            sub.push(field_line(
                "  dropdown",
                flag(lp.kind == 1),
                flag(p.kind == 1),
            ));
        }
        for o in &p.options {
            match o
                .id
                .and_then(|id| lp.options.iter().find(|x| x.id == Some(id)))
            {
                None => sub.push(add(format!("  option {}", option_summary(o, refs)))),
                Some(lo) => {
                    if lo.title != o.title
                        || lo.description != o.description
                        || lo.emoji != o.emoji
                        || sorted(&lo.roles) != sorted(&o.roles)
                        || sorted(&lo.channels) != sorted(&o.channels)
                    {
                        sub.push(change(format!(
                            "  option {}  (was {})",
                            option_summary(o, refs),
                            option_summary(lo, refs)
                        )));
                    }
                }
            }
        }
        for lo in &lp.options {
            if !p.options.iter().any(|o| o.id.is_some() && o.id == lo.id) {
                sub.push(remove(format!("  option {}", option_summary(lo, refs))));
            }
        }
        let pi_old: Vec<Option<u64>> = lp.options.iter().map(|o| o.id).collect();
        let pi_new: Vec<Option<u64>> = p
            .options
            .iter()
            .map(|o| o.id)
            .filter(|id| pi_old.contains(id))
            .collect();
        let pi_old: Vec<Option<u64>> = pi_old
            .into_iter()
            .filter(|id| pi_new.contains(id))
            .collect();
        if pi_old != pi_new {
            sub.push(change("  options reordered"));
        }
        if !sub.is_empty() {
            lines.push(change(format!("prompt {}", quoted(&p.title))));
            lines.extend(sub);
        }
    }
    for lp in &l.prompts {
        if !a.prompts.iter().any(|p| p.id.is_some() && p.id == lp.id) {
            lines.push(remove(format!("prompt {}", quoted(&lp.title))));
        }
    }
    if lines.is_empty() {
        lines.push(change("prompts reordered"));
    }
    lines
}

/// +/~/- lines for an ordered list matched by `id_of`.
fn list_lines<T: PartialEq, K: PartialEq>(
    lines: &mut Vec<Line>,
    what: &str,
    before: &[T],
    after: &[T],
    id_of: impl Fn(&T) -> K,
    show: impl Fn(&T) -> String,
) {
    let mut any = false;
    for a in after {
        match before.iter().find(|b| id_of(b) == id_of(a)) {
            None => {
                lines.push(add(format!("{what} {}", show(a))));
                any = true;
            }
            Some(b) if b != a => {
                lines.push(change(format!("{what} {}", show(a))));
                any = true;
            }
            _ => {}
        }
    }
    for b in before {
        if !after.iter().any(|a| id_of(a) == id_of(b)) {
            lines.push(remove(format!("{what} {}", show(b))));
            any = true;
        }
    }
    let order = |v: &[T]| v.iter().map(&id_of).collect::<Vec<K>>();
    if !any && order(before) != order(after) {
        lines.push(change(format!("{what}s reordered")));
    }
}

type CanonAction = (u8, Option<String>, Option<Ref>, u32);

fn canon_automod(r: &Automod<Ref, Ref>) -> impl PartialEq {
    let mut actions: Vec<CanonAction> = r
        .actions
        .iter()
        .map(|a| match a {
            AutomodAction::Block(m) => (1, m.clone(), None, 0),
            AutomodAction::Alert(c) => (2, None, Some(c.clone()), 0),
            AutomodAction::Timeout(t) => (3, None, None, *t),
            AutomodAction::BlockInteractions => (4, None, None, 0),
        })
        .collect();
    actions.sort();
    (
        (
            r.name.clone(),
            r.enabled,
            r.trigger,
            r.event,
            sorted(&r.keywords),
            sorted(&r.regex),
            sorted(&r.allow),
        ),
        (
            sorted(&r.presets),
            r.mention_limit,
            r.raid_protection.unwrap_or(false),
            actions,
            sorted(&r.exempt_roles),
            sorted(&r.exempt_channels),
        ),
    )
}

fn action_label(a: &AutomodAction<Ref>, refs: &Refs<'_>) -> String {
    match a {
        AutomodAction::Block(None) => "block".to_owned(),
        AutomodAction::Block(Some(m)) => format!("block ({})", quoted(m)),
        AutomodAction::Alert(c) => format!("alert {}", refs.label(c)),
        AutomodAction::Timeout(t) => format!("timeout {}", crate::render::duration(*t)),
        AutomodAction::BlockInteractions => "block interactions".to_owned(),
    }
}

fn trigger_label(t: u8) -> &'static str {
    AutomodTrigger::from_wire(t).map_or("?", AutomodTrigger::as_str)
}

fn automod_summary(r: &Automod<Ref, Ref>, refs: &Refs<'_>) -> String {
    let mut what = vec![trigger_label(r.trigger).to_owned()];
    if !r.keywords.is_empty() {
        what.push(format!("{} keyword(s)", r.keywords.len()));
    }
    if !r.regex.is_empty() {
        what.push(format!("{} regex(es)", r.regex.len()));
    }
    if !r.presets.is_empty() {
        let names: Vec<&str> = r
            .presets
            .iter()
            .map(|p| AutomodPreset::from_wire(*p).map_or("?", AutomodPreset::as_str))
            .collect();
        what.push(names.join("+"));
    }
    if let Some(m) = r.mention_limit {
        what.push(format!("over {m} mentions"));
    }
    let actions: Vec<String> = r.actions.iter().map(|a| action_label(a, refs)).collect();
    let mut s = format!(
        "{}: {} → {}",
        quoted(&r.name),
        what.join(", "),
        actions.join(", ")
    );
    if !r.enabled {
        s.push_str(" (disabled)");
    }
    s
}

fn automod_lines(l: &Automod<Ref, Ref>, a: &Automod<Ref, Ref>, refs: &Refs<'_>) -> Vec<Line> {
    let mut lines = Vec::new();
    if l.name != a.name {
        lines.push(field_line("name", quoted(&l.name), quoted(&a.name)));
    }
    if l.enabled != a.enabled {
        lines.push(field_line("enabled", l.enabled, a.enabled));
    }
    let set_diff = |lines: &mut Vec<Line>, what: &str, b: &[String], n: &[String]| {
        for x in n {
            if !b.contains(x) {
                lines.push(add(format!("{what} {}", quoted(x))));
            }
        }
        for x in b {
            if !n.contains(x) {
                lines.push(remove(format!("{what} {}", quoted(x))));
            }
        }
    };
    set_diff(&mut lines, "keyword", &l.keywords, &a.keywords);
    set_diff(&mut lines, "regex", &l.regex, &a.regex);
    set_diff(&mut lines, "allowed", &l.allow, &a.allow);
    if sorted(&l.presets) != sorted(&a.presets) {
        let names = |v: &[u8]| {
            v.iter()
                .map(|p| AutomodPreset::from_wire(*p).map_or("?", AutomodPreset::as_str))
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(field_line("presets", names(&l.presets), names(&a.presets)));
    }
    if l.mention_limit != a.mention_limit {
        let show = |m: Option<u32>| m.map_or_else(|| "none".to_owned(), |m| m.to_string());
        lines.push(field_line(
            "mention_limit",
            show(l.mention_limit),
            show(a.mention_limit),
        ));
    }
    if l.raid_protection.unwrap_or(false) != a.raid_protection.unwrap_or(false) {
        lines.push(field_line(
            "raid_protection",
            l.raid_protection.unwrap_or(false),
            a.raid_protection.unwrap_or(false),
        ));
    }
    let acts =
        |v: &[AutomodAction<Ref>]| v.iter().map(|x| action_label(x, refs)).collect::<Vec<_>>();
    let (before, after) = (acts(&l.actions), acts(&a.actions));
    // Order doesn't matter to Discord.
    if sorted(&before) != sorted(&after) {
        lines.push(field_line("actions", before.join(", "), after.join(", ")));
    }
    if sorted(&l.exempt_roles) != sorted(&a.exempt_roles) {
        lines.push(field_line(
            "exempt_roles",
            refs.labels(&l.exempt_roles),
            refs.labels(&a.exempt_roles),
        ));
    }
    if sorted(&l.exempt_channels) != sorted(&a.exempt_channels) {
        lines.push(field_line(
            "exempt_channels",
            refs.labels(&l.exempt_channels),
            refs.labels(&a.exempt_channels),
        ));
    }
    lines
}
