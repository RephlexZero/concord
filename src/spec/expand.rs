//! Expand templates, variables and tag sets into the flat desired model, and
//! validate everything that can be checked without talking to Discord.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use super::community::{OptionSpec, TemplateOptionSpec};
use super::raw::{
    CategorySpec, ChannelSpec, OverrideSpec, RoleSpec, SpecFile, TagEntry, TagSpec, VarValue,
};
use crate::diag::{suggest, Diagnostic};
use crate::model::{
    layer_overwrites, ChannelKind, Desired, DesiredChannel, DesiredForum, DesiredOverwrites,
    DesiredRole, DesiredTag, Emoji, Layout, Mode, NamedOverwrite, Sort, DEFAULT_AUTO_ARCHIVE,
    DEFAULT_BITRATE,
};
use crate::perms::{parse_perm, PermContext, Perms};

pub const EVERYONE: &str = "@everyone";

/// Discord limits, checked offline.
pub mod limits {
    pub const ROLES: usize = 250;
    pub const CHANNELS: usize = 500;
    pub const CHANNELS_PER_CATEGORY: usize = 50;
    pub const TAGS_PER_FORUM: usize = 20;
    pub const TAG_NAME: usize = 20;
    pub const NAME: usize = 100;
    pub const TOPIC: usize = 1024;
    pub const GUIDELINES: usize = 4096;
    pub const SLOWMODE: u32 = 21_600;
    pub const USER_LIMIT: u32 = 99;
    pub const BITRATE: std::ops::RangeInclusive<u32> = 8_000..=384_000;
}

/// Expand and validate a spec. Returns the desired model even if there are
/// errors, so callers can report every problem at once. Files (emoji,
/// stickers, sounds) are read relative to the current directory.
pub fn expand(spec: &SpecFile) -> (Desired, Vec<Diagnostic>) {
    expand_at(spec, std::path::Path::new("."))
}

/// [`expand`], reading files relative to `base` (the spec's directory).
pub fn expand_at(spec: &SpecFile, base: &std::path::Path) -> (Desired, Vec<Diagnostic>) {
    let mut cx = Cx {
        mode: spec.concord.mode,
        tag_sets: &spec.tag_sets,
        diags: Vec::new(),
        template_options: Vec::new(),
    };

    let roles = cx.roles(&spec.roles);
    cx.check_tag_sets();

    let mut channels = Vec::new();
    for (i, c) in spec.channels.iter().enumerate() {
        let scope = Scope::top(format!("channels[{i}]"));
        if let Some(ch) = cx.channel(c, &scope) {
            channels.push(ch);
        }
    }
    for (key, cat) in &spec.categories {
        cx.category(key, cat, spec, &mut channels);
    }
    cx.check_channels(&channels);

    let community = crate::community::expand::expand(
        spec,
        &roles,
        &channels,
        std::mem::take(&mut cx.template_options),
        base,
        &mut cx.diags,
    );
    let desired = Desired {
        guild_id: spec.guild_id.map(|g| g.0),
        mode: spec.concord.mode,
        roles,
        channels,
        community,
    };
    (desired, cx.diags)
}

/// An onboarding option contributed by a category or template, with its
/// variables substituted.
#[derive(Debug, Clone)]
pub struct TemplateOption {
    pub prompt: String,
    pub path: String,
    pub option: OptionSpec,
}

struct Cx<'a> {
    mode: Mode,
    tag_sets: &'a IndexMap<String, Vec<TagEntry>>,
    diags: Vec<Diagnostic>,
    template_options: Vec<TemplateOption>,
}

/// Where a channel definition comes from, and which variables it can use.
struct Scope<'a> {
    /// Path to the channel definition itself.
    path: String,
    /// Suffix for messages when the definition is in a template.
    via: Option<String>,
    key_prefix: Option<String>,
    parent: Option<String>,
    vars: Option<&'a Vars>,
}

impl Scope<'_> {
    fn top(path: String) -> Self {
        Scope {
            path,
            via: None,
            key_prefix: None,
            parent: None,
            vars: None,
        }
    }

    fn here(&self) -> String {
        match &self.via {
            Some(via) => format!("{} (via {via})", self.path),
            None => self.path.clone(),
        }
    }

    fn at(&self, field: &str) -> String {
        let mut p = if field.starts_with('[') {
            format!("{}{field}", self.path)
        } else {
            format!("{}.{field}", self.path)
        };
        if let Some(via) = &self.via {
            p.push_str(&format!(" (via {via})"));
        }
        p
    }
}

struct Vars {
    values: IndexMap<String, VarValue>,
    used: RefCell<HashSet<String>>,
}

impl Cx<'_> {
    fn err(&mut self, path: impl Into<String>, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(path, msg));
    }

    // -- roles -------------------------------------------------------------

    fn roles(&mut self, specs: &[RoleSpec]) -> Vec<DesiredRole> {
        let mut out = Vec::new();
        let mut keys = HashSet::new();
        let mut names = HashSet::new();
        for (i, r) in specs.iter().enumerate() {
            let path = format!("roles[{i}]");
            self.check_key(&r.key, &format!("{path}.key"));
            if !keys.insert(r.key.clone()) {
                self.err(
                    format!("{path}.key"),
                    format!("duplicate role key `{}`", r.key),
                );
            }
            if r.name.trim().is_empty() || r.name.chars().count() > limits::NAME {
                self.err(
                    format!("{path}.name"),
                    "role names must be 1-100 characters",
                );
            }
            if !names.insert(r.name.clone()) {
                self.diags.push(
                    Diagnostic::error(
                        format!("{path}.name"),
                        format!("duplicate role name `{}`", r.name),
                    )
                    .with_hint("roles are referenced by name, so names must be unique"),
                );
            }
            let everyone = r.name == EVERYONE;
            if everyone {
                if r.color.is_some()
                    || r.hoist.is_some()
                    || r.mentionable.is_some()
                    || r.icon.is_some()
                {
                    self.err(&path, "@everyone can only have `permissions`");
                }
                if i + 1 != specs.len() {
                    self.diags.push(
                        Diagnostic::error(&path, "@everyone must be the last role")
                            .with_hint("@everyone is always the lowest role"),
                    );
                }
            }
            let permissions = r.permissions.as_ref().map(|list| {
                let mut p = Perms::NONE;
                for (j, name) in list.iter().enumerate() {
                    match parse_perm(name, PermContext::Role) {
                        Ok(bits) => p = p.union(bits),
                        Err(e) => self.err(format!("{path}.permissions[{j}]"), e),
                    }
                }
                p
            });
            let icon = r.icon.as_ref().map(|i| {
                let i = i.trim();
                if i.is_empty() {
                    return None;
                }
                if let Err(e) = Emoji::parse(i).and_then(|e| match e {
                    Emoji::Unicode(_) => Ok(()),
                    Emoji::Custom(_) => Err("role icons must be unicode emoji".to_owned()),
                }) {
                    self.err(format!("{path}.icon"), e);
                }
                Some(i.to_owned())
            });
            let auth = self.mode == Mode::Authoritative && !everyone;
            out.push(DesiredRole {
                key: r.key.clone(),
                path,
                name: r.name.clone(),
                everyone,
                color: r.color.map(|c| c.0).or(auth.then_some(0)),
                hoist: r.hoist.or(auth.then_some(false)),
                mentionable: r.mentionable.or(auth.then_some(false)),
                // @everyone's permissions are never defaulted: resetting them
                // would hide the whole server.
                permissions: permissions.or(auth.then_some(Perms::NONE)),
                icon: icon.or(auth.then_some(None)),
            });
        }
        if out.len() > limits::ROLES {
            self.err(
                "roles",
                format!("Discord allows at most {} roles", limits::ROLES),
            );
        }
        out
    }

    fn check_key(&mut self, key: &str, path: &str) {
        if key.trim().is_empty() {
            self.err(path, "key must not be empty");
        } else if key.contains('/') {
            self.err(path, format!("key `{key}` must not contain `/`"));
        }
    }

    // -- tag sets ----------------------------------------------------------

    fn check_tag_sets(&mut self) {
        for (set, entries) in self.tag_sets {
            for (i, entry) in entries.iter().enumerate() {
                let path = format!("tag_sets.{set}[{i}]");
                match entry {
                    TagEntry::Name(n) if self.tag_sets.contains_key(n) => self.diags.push(
                        Diagnostic::error(
                            path,
                            format!("tag sets can't contain other tag sets (`{n}`)"),
                        )
                        .with_hint("list both sets in the channel's `tags` instead"),
                    ),
                    TagEntry::Name(n) if n.contains('{') => {
                        self.err(path, "tag sets can't use template variables")
                    }
                    _ => {}
                }
            }
        }
    }

    // -- categories ----------------------------------------------------------

    fn category(
        &mut self,
        key: &str,
        cat: &CategorySpec,
        spec: &SpecFile,
        channels: &mut Vec<DesiredChannel>,
    ) {
        let path = format!("categories.{key}");
        self.check_key(key, &path);
        let template = match &cat.template {
            Some(t) => match spec.templates.get(t) {
                Some(tpl) => Some((t.as_str(), tpl)),
                None => {
                    let hint = suggest(t, spec.templates.keys().map(String::as_str));
                    self.diags.push(
                        Diagnostic::error(
                            format!("{path}.template"),
                            format!("unknown template `{t}`"),
                        )
                        .with_hint_opt(hint),
                    );
                    None
                }
            },
            None => None,
        };
        if cat.vars.contains_key("key") {
            self.err(
                format!("{path}.vars.key"),
                "`key` is a built-in variable (the category key)",
            );
        }
        let mut values = cat.vars.clone();
        values.insert("key".to_owned(), VarValue::One(key.to_owned()));
        let vars = Vars {
            values,
            used: RefCell::new(HashSet::new()),
        };

        let scope = Scope {
            path: path.clone(),
            via: None,
            key_prefix: None,
            parent: None,
            vars: Some(&vars),
        };
        let name = self.subst(&cat.name, &scope, "name");
        self.check_name_len(&name, &scope.at("name"));

        // Category permissions: template's, with the category's own on top.
        let mut layers: Option<Vec<NamedOverwrite>> = None;
        if let Some((tname, tpl)) = template {
            let tscope = Scope {
                path: format!("templates.{tname}"),
                via: Some(path.clone()),
                key_prefix: None,
                parent: None,
                vars: Some(&vars),
            };
            if let Some(l) = self.perm_layers(
                tpl.private.as_deref(),
                tpl.overrides.as_ref(),
                PermContext::Category,
                &tscope,
            ) {
                layers = Some(l);
            }
        }
        if let Some(own) = self.perm_layers(
            cat.private.as_deref(),
            cat.overrides.as_ref(),
            PermContext::Category,
            &scope,
        ) {
            layers = Some(merge_named(layers.as_deref().unwrap_or(&[]), &own));
        }
        let overwrites = match layers {
            Some(l) => DesiredOverwrites::Explicit(l),
            None if self.mode == Mode::Authoritative => DesiredOverwrites::Explicit(Vec::new()),
            None => DesiredOverwrites::Unmanaged,
        };

        channels.push(DesiredChannel {
            key: key.to_owned(),
            path: path.clone(),
            kind: ChannelKind::Category,
            name,
            parent: None,
            topic: None,
            nsfw: None,
            slowmode: None,
            bitrate: None,
            user_limit: None,
            region: None,
            video_quality: None,
            forum: DesiredForum::default(),
            overwrites,
        });

        // An onboarding option for this category: its own, or the template's.
        let option = match (&cat.onboarding_option, template) {
            (Some(o), _) => Some((o, scope.at("onboarding_option"))),
            (None, Some((tname, tpl))) => tpl.onboarding_option.as_ref().map(|o| {
                (
                    o,
                    format!("templates.{tname}.onboarding_option (via {path})"),
                )
            }),
            _ => None,
        };
        if let Some((o, opath)) = option {
            let o = self.subst_option(o, &scope);
            self.template_options.push(TemplateOption {
                prompt: o.prompt,
                path: opath,
                option: o.option,
            });
        }

        if let Some((tname, tpl)) = template {
            for (i, c) in tpl.channels.iter().enumerate() {
                let scope = Scope {
                    path: format!("templates.{tname}.channels[{i}]"),
                    via: Some(path.clone()),
                    key_prefix: Some(format!("{key}.")),
                    parent: Some(key.to_owned()),
                    vars: Some(&vars),
                };
                if let Some(ch) = self.channel(c, &scope) {
                    channels.push(ch);
                }
            }
        }
        for (i, c) in cat.channels.iter().enumerate() {
            let scope = Scope {
                path: format!("{path}.channels[{i}]"),
                via: None,
                key_prefix: None,
                parent: Some(key.to_owned()),
                vars: Some(&vars),
            };
            if let Some(ch) = self.channel(c, &scope) {
                channels.push(ch);
            }
        }

        let used = vars.used.borrow();
        for name in cat.vars.keys() {
            if !used.contains(name) {
                self.diags.push(Diagnostic::warning(
                    format!("{path}.vars.{name}"),
                    format!("variable `{name}` is never used"),
                ));
            }
        }
    }

    // -- channels ----------------------------------------------------------

    fn channel(&mut self, c: &ChannelSpec, scope: &Scope<'_>) -> Option<DesiredChannel> {
        let raw_key = self.subst(&c.key, scope, "key");
        self.check_key(&raw_key, &scope.at("key"));
        let key = match &scope.key_prefix {
            Some(prefix) => format!("{prefix}{raw_key}"),
            None => raw_key,
        };
        let kind = c.kind;
        if kind == ChannelKind::Category {
            self.diags.push(Diagnostic::error(
                scope.at("type"),
                "categories go under `categories:`, not in a channel list",
            ));
            return None;
        }

        let name = self.subst(&c.name, scope, "name");
        self.check_name_len(&name, &scope.at("name"));
        if kind.has_slug_name() {
            let canonical = slug_name(&name);
            if canonical != name {
                self.diags.push(
                    Diagnostic::error(
                        scope.at("name"),
                        format!(
                            "Discord stores {} channel names in lower case without spaces",
                            kind.as_str()
                        ),
                    )
                    .with_hint(format!("write `{canonical}`")),
                );
            }
        }

        let only = |cx: &mut Self, set: bool, field: &str, kinds: &[ChannelKind]| {
            if set && !kinds.contains(&kind) {
                let names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
                cx.err(
                    scope.at(field),
                    format!(
                        "`{field}` only applies to {} channels, not {}",
                        names.join("/"),
                        kind.as_str()
                    ),
                );
            }
        };
        use ChannelKind::{Announcement, Forum, Media, Stage, Text, Voice};
        only(
            self,
            c.topic.is_some(),
            "topic",
            &[Text, Announcement, Forum, Media],
        );
        only(self, c.guidelines.is_some(), "guidelines", &[Forum, Media]);
        only(
            self,
            c.slowmode.is_some(),
            "slowmode",
            &[Text, Forum, Media, Voice, Stage],
        );
        only(self, c.bitrate.is_some(), "bitrate", &[Voice, Stage]);
        only(self, c.user_limit.is_some(), "user_limit", &[Voice, Stage]);
        only(self, c.region.is_some(), "region", &[Voice, Stage]);
        only(self, c.video_quality.is_some(), "video_quality", &[Voice]);
        only(self, c.tags.is_some(), "tags", &[Forum, Media]);
        only(
            self,
            c.require_tag.is_some(),
            "require_tag",
            &[Forum, Media],
        );
        only(self, c.layout.is_some(), "layout", &[Forum]);
        only(self, c.sort.is_some(), "sort", &[Forum, Media]);
        only(
            self,
            c.default_reaction.is_some(),
            "default_reaction",
            &[Forum, Media],
        );
        only(
            self,
            c.auto_archive.is_some(),
            "auto_archive",
            &[Text, Announcement, Forum, Media],
        );
        only(
            self,
            c.post_slowmode.is_some(),
            "post_slowmode",
            &[Forum, Media],
        );
        if scope.parent.is_none() && c.sync.is_some() {
            self.diags.push(Diagnostic::warning(
                scope.at("sync"),
                "`sync` has no effect on a channel without a category",
            ));
        }

        // Topic / guidelines.
        if c.topic.is_some() && c.guidelines.is_some() {
            self.err(
                scope.at("guidelines"),
                "set `guidelines` or `topic`, not both (they are the same field)",
            );
        }
        let (topic_field, topic) = match (&c.guidelines, &c.topic) {
            (Some(g), _) => ("guidelines", Some(g)),
            (None, Some(t)) => ("topic", Some(t)),
            _ => ("topic", None),
        };
        // Discord trims topics, and YAML `|` blocks end in a newline; without
        // this every plan would try to set them again.
        let topic = topic.map(|t| self.subst(t, scope, topic_field).trim().to_owned());
        if let Some(t) = &topic {
            let max = if kind.is_forum_like() {
                limits::GUIDELINES
            } else {
                limits::TOPIC
            };
            if t.chars().count() > max {
                self.err(
                    scope.at(topic_field),
                    format!("must be at most {max} characters"),
                );
            }
        }

        for (field, s) in [("slowmode", c.slowmode), ("post_slowmode", c.post_slowmode)] {
            if s.is_some_and(|s| s.0 > limits::SLOWMODE) {
                self.err(scope.at(field), format!("{field} can be at most 6h"));
            }
        }
        if let Some(b) = c.bitrate {
            if !limits::BITRATE.contains(&b) {
                self.err(
                    scope.at("bitrate"),
                    "bitrate must be between 8000 and 384000",
                );
            }
        }
        if let Some(u) = c.user_limit {
            if u > limits::USER_LIMIT {
                self.err(
                    scope.at("user_limit"),
                    "user_limit can be at most 99 (0 = unlimited)",
                );
            }
        }

        let tags = c.tags.as_ref().map(|t| self.tags(t, scope));
        if c.require_tag == Some(true) && tags.as_ref().is_some_and(Vec::is_empty) {
            self.err(
                scope.at("require_tag"),
                "`require_tag` needs at least one tag",
            );
        }
        let default_reaction = c.default_reaction.as_ref().map(|e| match Emoji::parse(e) {
            Ok(e) => Some(e),
            Err(msg) => {
                self.err(scope.at("default_reaction"), msg);
                None
            }
        });

        let layers = self.perm_layers(
            c.private.as_deref(),
            c.overrides.as_ref(),
            kind.perm_context(),
            scope,
        );
        let overwrites = match (&scope.parent, layers) {
            (Some(_), layers) if c.sync == Some(false) => {
                DesiredOverwrites::Explicit(layers.unwrap_or_default())
            }
            (Some(_), layers) => DesiredOverwrites::Inherit(layers.unwrap_or_default()),
            (None, Some(layers)) => DesiredOverwrites::Explicit(layers),
            (None, None) if self.mode == Mode::Authoritative => {
                DesiredOverwrites::Explicit(Vec::new())
            }
            (None, None) => DesiredOverwrites::Unmanaged,
        };

        let auth = self.mode == Mode::Authoritative;
        let def = |v: bool| auth && v;
        let has_topic = matches!(kind, Text | Announcement | Forum | Media);
        let has_slowmode = matches!(kind, Text | Forum | Media | Voice | Stage);
        let forum = kind.is_forum_like();
        let voice = kind.is_voice_like();
        let has_archive = has_topic;
        let region = c.region.as_ref().map(|r| {
            let r = r.trim();
            (!r.is_empty()).then(|| r.to_owned())
        });
        Some(DesiredChannel {
            key,
            path: scope.here(),
            kind,
            name,
            parent: scope.parent.clone(),
            topic: topic.or_else(|| def(has_topic).then(String::new)),
            nsfw: c.nsfw.or(def(true).then_some(false)),
            slowmode: c.slowmode.map(|s| s.0).or(def(has_slowmode).then_some(0)),
            bitrate: c.bitrate.or(def(voice).then_some(DEFAULT_BITRATE)),
            user_limit: c.user_limit.or(def(voice).then_some(0)),
            region: region.or(def(voice).then_some(None)),
            video_quality: c
                .video_quality
                .map(|v| v.wire())
                .or(def(kind == Voice).then_some(1)),
            forum: DesiredForum {
                tags: tags.or_else(|| def(forum).then(Vec::new)),
                require_tag: c.require_tag.or(def(forum).then_some(false)),
                layout: c.layout.or(def(kind == Forum).then_some(Layout::List)),
                sort: c.sort.or(def(forum).then_some(Sort::Activity)),
                default_reaction: default_reaction.or(def(forum).then_some(None)),
                auto_archive: c
                    .auto_archive
                    .map(|a| a.0)
                    .or(def(has_archive).then_some(DEFAULT_AUTO_ARCHIVE)),
                post_slowmode: c.post_slowmode.map(|s| s.0).or(def(forum).then_some(0)),
            },
            overwrites,
        })
    }

    fn subst_option(&mut self, o: &TemplateOptionSpec, scope: &Scope<'_>) -> TemplateOptionSpec {
        let s =
            |cx: &mut Self, v: &str, f: &str| cx.subst(v, scope, &format!("onboarding_option.{f}"));
        let opt = &o.option;
        TemplateOptionSpec {
            prompt: s(self, &o.prompt, "prompt"),
            option: OptionSpec {
                key: opt.key.as_ref().map(|k| s(self, k, "key")),
                title: s(self, &opt.title, "title"),
                description: opt.description.as_ref().map(|d| s(self, d, "description")),
                emoji: opt.emoji.as_ref().map(|e| s(self, e, "emoji")),
                roles: opt.roles.iter().map(|r| s(self, r, "roles")).collect(),
                channels: opt
                    .channels
                    .iter()
                    .map(|c| s(self, c, "channels"))
                    .collect(),
            },
        }
    }

    fn check_name_len(&mut self, name: &str, path: &str) {
        let n = name.chars().count();
        if n == 0 || n > limits::NAME {
            self.err(path, "names must be 1-100 characters");
        }
    }

    // -- tags --------------------------------------------------------------

    fn tags(&mut self, entries: &[TagEntry], scope: &Scope<'_>) -> Vec<DesiredTag> {
        let mut out: Vec<(DesiredTag, String)> = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            let path = scope.at(&format!("tags[{i}]"));
            match entry {
                TagEntry::Name(raw) => {
                    for name in self.subst_list(raw, scope, &format!("tags[{i}]")) {
                        if let Some(set) = self.tag_sets.get(&name) {
                            for (j, e) in set.iter().enumerate() {
                                let set_path = format!("tag_sets.{name}[{j}]");
                                if let Some(t) = self.tag_from_entry(e, &set_path) {
                                    out.push((t, path.clone()));
                                }
                            }
                        } else {
                            out.push((plain_tag(name), path.clone()));
                        }
                    }
                }
                TagEntry::Full(t) => {
                    let t = TagSpec {
                        key: t
                            .key
                            .as_ref()
                            .map(|k| self.subst(k, scope, &format!("tags[{i}].key"))),
                        name: self.subst(&t.name, scope, &format!("tags[{i}].name")),
                        emoji: t.emoji.clone(),
                        moderated: t.moderated,
                    };
                    if let Some(tag) = self.tag_from_entry(&TagEntry::Full(t), &path) {
                        out.push((tag, path));
                    }
                }
            }
        }

        let mut names = HashMap::new();
        let mut keys = HashSet::new();
        for (tag, path) in &out {
            let n = tag.name.chars().count();
            if n == 0 || n > limits::TAG_NAME {
                self.diags.push(Diagnostic::error(
                    path,
                    format!("tag `{}` is {n} characters; Discord allows 1-20", tag.name),
                ));
            }
            if names
                .insert(tag.name.to_lowercase(), path.clone())
                .is_some()
            {
                self.err(path, format!("tag `{}` appears more than once", tag.name));
            } else if !keys.insert(tag.key.clone()) {
                self.err(
                    path,
                    format!("tag key `{}` appears more than once", tag.key),
                );
            }
        }
        if out.len() > limits::TAGS_PER_FORUM {
            let names: Vec<&str> = out.iter().map(|(t, _)| t.name.as_str()).collect();
            self.diags.push(
                Diagnostic::error(
                    scope.at("tags"),
                    format!(
                        "{} tags after expansion; Discord allows at most {}",
                        out.len(),
                        limits::TAGS_PER_FORUM
                    ),
                )
                .with_hint(format!("expanded to: {}", names.join(", "))),
            );
        }
        out.into_iter().map(|(t, _)| t).collect()
    }

    fn tag_from_entry(&mut self, entry: &TagEntry, path: &str) -> Option<DesiredTag> {
        match entry {
            TagEntry::Name(n) => Some(plain_tag(n.clone())),
            TagEntry::Full(t) => {
                if self.tag_sets.contains_key(&t.name) {
                    self.diags.push(
                        Diagnostic::error(
                            path,
                            format!("tag `{}` has the same name as a tag set", t.name),
                        )
                        .with_hint("rename the tag set so plain names stay unambiguous"),
                    );
                }
                let key = t.key.clone().unwrap_or_else(|| t.name.clone());
                if key.contains('/') {
                    self.err(path, format!("tag key `{key}` must not contain `/`"));
                }
                let emoji = match t.emoji.as_deref().map(Emoji::parse) {
                    Some(Ok(e)) => Some(e),
                    Some(Err(msg)) => {
                        self.err(format!("{path}.emoji"), msg);
                        None
                    }
                    None => None,
                };
                Some(DesiredTag {
                    key,
                    name: t.name.clone(),
                    emoji,
                    moderated: t.moderated,
                })
            }
        }
    }

    // -- permissions -------------------------------------------------------

    /// `private` then `overrides`, layered. `None` if neither is set.
    fn perm_layers(
        &mut self,
        private: Option<&[String]>,
        overrides: Option<&IndexMap<String, OverrideSpec>>,
        ctx: PermContext,
        scope: &Scope<'_>,
    ) -> Option<Vec<NamedOverwrite>> {
        if private.is_none() && overrides.is_none() {
            return None;
        }
        let mut layers: Vec<NamedOverwrite> = Vec::new();
        if let Some(roles) = private {
            let path = scope.at("private");
            let mut l = vec![NamedOverwrite {
                role: EVERYONE.to_owned(),
                allow: Perms::NONE,
                deny: Perms::VIEW_CHANNEL,
                path: path.clone(),
            }];
            for (i, r) in roles.iter().enumerate() {
                let role = self.subst(r, scope, &format!("private[{i}]"));
                if role == EVERYONE {
                    self.err(
                        &path,
                        "`private` lists the roles that *can* see it; @everyone can't be one",
                    );
                    continue;
                }
                l.push(NamedOverwrite {
                    role,
                    allow: Perms::VIEW_CHANNEL,
                    deny: Perms::NONE,
                    path: format!("{path}[{i}]"),
                });
            }
            layers = merge_named(&layers, &l);
        }
        if let Some(map) = overrides {
            let mut l = Vec::new();
            for (role, ov) in map {
                let field = format!("overrides.{role}");
                let path = scope.at(&field);
                let role = self.subst(role, scope, &field);
                let mut parse = |list: &[String], what: &str| {
                    let mut p = Perms::NONE;
                    for (j, name) in list.iter().enumerate() {
                        match parse_perm(name, ctx) {
                            Ok(bits) => p = p.union(bits),
                            Err(e) => self.err(format!("{path}.{what}[{j}]"), e),
                        }
                    }
                    p
                };
                let allow = parse(&ov.allow, "allow");
                let deny = parse(&ov.deny, "deny");
                if allow.intersects(deny) {
                    let both = crate::perms::names(allow.intersect(deny), ctx).join(", ");
                    self.err(&path, format!("{both} is both allowed and denied"));
                }
                if role.trim().is_empty() {
                    self.err(&path, "role name must not be empty");
                }
                l.push(NamedOverwrite {
                    role,
                    allow,
                    deny,
                    path,
                });
            }
            layers = merge_named(&layers, &l);
        }
        Some(layers)
    }

    // -- variables ---------------------------------------------------------

    /// Substitute `{var}`s in a string. `{{` and `}}` are literal braces.
    fn subst(&mut self, s: &str, scope: &Scope<'_>, field: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut rest = s;
        while let Some(start) = rest.find(['{', '}']) {
            out.push_str(&rest[..start]);
            let tail = &rest[start..];
            if tail.starts_with("{{") || tail.starts_with("}}") {
                out.push_str(&tail[..1]);
                rest = &tail[2..];
                continue;
            }
            if let Some(after) = tail.strip_prefix('}') {
                out.push('}');
                rest = after;
                continue;
            }
            let Some(end) = tail.find('}') else {
                out.push_str(tail);
                rest = "";
                break;
            };
            let name = &tail[1..end];
            let is_var = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !is_var {
                out.push_str(&tail[..=end]);
            } else {
                match scope.vars.and_then(|v| {
                    v.used.borrow_mut().insert(name.to_owned());
                    v.values.get(name)
                }) {
                    Some(VarValue::One(v)) => out.push_str(v),
                    Some(VarValue::Many(_)) => {
                        self.diags.push(
                            Diagnostic::error(
                                scope.at(field),
                                format!("`{{{name}}}` is a list and can't be used inside text"),
                            )
                            .with_hint(format!(
                                "lists can only be a whole tag entry, like tags: [\"{{{name}}}\"]"
                            )),
                        );
                    }
                    None => {
                        let hint = scope
                            .vars
                            .and_then(|v| suggest(name, v.values.keys().map(String::as_str)))
                            .or_else(|| {
                                scope.vars.is_none().then(|| {
                                    "variables only work in categories (and templates); \
                                     write `{{` for a literal brace"
                                        .to_owned()
                                })
                            });
                        self.diags.push(
                            Diagnostic::error(
                                scope.at(field),
                                format!("unknown variable `{{{name}}}`"),
                            )
                            .with_hint_opt(hint),
                        );
                    }
                }
            }
            rest = &tail[end + 1..];
        }
        out.push_str(rest);
        out
    }

    /// Like [`Self::subst`], but `"{var}"` on its own with a list value
    /// expands to every item.
    fn subst_list(&mut self, s: &str, scope: &Scope<'_>, field: &str) -> Vec<String> {
        let trimmed = s.trim();
        if let Some(name) = trimmed.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
            if let Some(v) = scope.vars {
                if let Some(VarValue::Many(items)) = v.values.get(name) {
                    v.used.borrow_mut().insert(name.to_owned());
                    return items.clone();
                }
            }
        }
        vec![self.subst(s, scope, field)]
    }

    // -- whole-spec checks -------------------------------------------------

    fn check_channels(&mut self, channels: &[DesiredChannel]) {
        let mut keys: HashMap<&str, &str> = HashMap::new();
        for c in channels {
            if let Some(first) = keys.insert(&c.key, &c.path) {
                self.diags.push(
                    Diagnostic::error(&c.path, format!("duplicate channel key `{}`", c.key))
                        .with_hint(format!("also used by {first}")),
                );
            }
        }
        if channels.len() > limits::CHANNELS {
            self.err(
                "",
                format!("Discord allows at most {} channels", limits::CHANNELS),
            );
        }

        // Group by parent, preserving order.
        let mut groups: IndexMap<Option<&str>, Vec<&DesiredChannel>> = IndexMap::new();
        for c in channels.iter().filter(|c| c.kind != ChannelKind::Category) {
            groups.entry(c.parent.as_deref()).or_default().push(c);
        }
        for (parent, members) in &groups {
            if let Some(p) = parent {
                if members.len() > limits::CHANNELS_PER_CATEGORY {
                    self.err(
                        format!("categories.{p}"),
                        format!(
                            "{} channels; Discord allows at most 50 per category",
                            members.len()
                        ),
                    );
                }
            }
            let mut seen_voice = None;
            for c in members {
                if c.kind.is_voice_like() {
                    seen_voice.get_or_insert(&c.name);
                } else if let Some(v) = seen_voice {
                    self.diags.push(
                        Diagnostic::warning(&c.path, format!("`{}` is listed after voice channel `{v}`", c.name))
                            .with_hint("Discord always shows voice channels below text channels; list them last"),
                    );
                    break;
                }
            }
            let mut names = HashSet::new();
            for c in members {
                if !names.insert((c.kind.has_slug_name(), c.name.as_str())) {
                    self.diags.push(Diagnostic::warning(
                        &c.path,
                        format!(
                            "another channel in the same place is also called `{}`",
                            c.name
                        ),
                    ));
                }
            }
        }
    }
}

fn plain_tag(name: String) -> DesiredTag {
    DesiredTag {
        key: name.clone(),
        name,
        emoji: None,
        moderated: false,
    }
}

fn merge_named(base: &[NamedOverwrite], top: &[NamedOverwrite]) -> Vec<NamedOverwrite> {
    let to_tuple = |o: &NamedOverwrite| (o.role.clone(), o.allow, o.deny);
    let b: Vec<_> = base.iter().map(to_tuple).collect();
    let t: Vec<_> = top.iter().map(to_tuple).collect();
    layer_overwrites(&b, &t)
        .into_iter()
        .map(|(role, allow, deny)| {
            let path = top
                .iter()
                .chain(base)
                .find(|o| o.role == role)
                .map(|o| o.path.clone())
                .unwrap_or_default();
            NamedOverwrite {
                role,
                allow,
                deny,
                path,
            }
        })
        .collect()
}

/// How Discord stores a text/forum channel name.
pub fn slug_name(name: &str) -> String {
    name.trim().to_lowercase().replace(' ', "-")
}

#[cfg(test)]
mod tests;
