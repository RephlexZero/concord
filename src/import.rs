//! `concord import`: turn the live server into a spec and lockfile.
//!
//! The output is flat (no templates or tag sets) and uses `authoritative`
//! mode, writing only values that differ from Discord's defaults. Planning
//! the imported spec against the same server shows no changes.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::diag::Diagnostic;
use crate::lock::{tag_key, Lockfile};
use crate::model::{
    layer_overwrites, ChannelKind, LiveChannel, LiveState, Mode, DEFAULT_AUTO_ARCHIVE,
    DEFAULT_BITRATE,
};
use crate::perms::{names, PermContext, Perms};
use crate::snowflake::Sf;
use crate::spec::expand::EVERYONE;
use crate::spec::raw::{
    ArchiveDuration, CategorySpec, ChannelSpec, Color, OverrideSpec, RoleSpec, Seconds, Settings,
    SpecFile, TagEntry, TagSpec,
};

pub struct Imported {
    pub spec: SpecFile,
    pub lock: Lockfile,
    pub warnings: Vec<Diagnostic>,
}

pub fn import(live: &LiveState) -> Imported {
    let mut im = Importer {
        live,
        keys: HashSet::new(),
        role_names: HashMap::new(),
        lock: Lockfile::new(live.guild.id),
        warnings: Vec::new(),
    };
    let roles = im.roles();
    let (channels, categories) = im.channels();
    Imported {
        spec: SpecFile {
            guild_id: Some(Sf(live.guild.id)),
            concord: Settings {
                mode: Mode::Authoritative,
            },
            tag_sets: IndexMap::new(),
            templates: IndexMap::new(),
            roles,
            channels,
            categories,
        },
        lock: im.lock,
        warnings: im.warnings,
    }
}

/// Serialise an imported spec, with a short header.
pub fn to_yaml(spec: &SpecFile, guild_name: &str) -> String {
    let body = serde_yaml_ng::to_string(spec).expect("spec serialises");
    format!(
        "# Imported from \"{guild_name}\" by `concord import`.\n\
         # Keys identify things (concord.lock.json maps them to Discord IDs);\n\
         # names are what Discord shows. Change a name to rename; keep the key.\n\
         {body}"
    )
}

struct Importer<'a> {
    live: &'a LiveState,
    /// Channel and category keys used so far.
    keys: HashSet<String>,
    /// Role ID → name used in the spec.
    role_names: HashMap<u64, String>,
    lock: Lockfile,
    warnings: Vec<Diagnostic>,
}

type Tuple = (u64, Perms, Perms);

impl Importer<'_> {
    fn roles(&mut self) -> Vec<RoleSpec> {
        let live = self.live;
        let mut sorted: Vec<_> = live
            .roles
            .iter()
            .filter(|r| r.id != live.guild.id)
            .collect();
        sorted.sort_by_key(|r| (std::cmp::Reverse(r.position), r.id));

        let mut out = Vec::new();
        let mut keys = HashSet::new();
        let mut seen_names = HashSet::new();
        for r in &sorted {
            self.role_names.insert(r.id, r.name.clone());
            if r.managed {
                continue;
            }
            if !seen_names.insert(r.name.clone()) {
                self.warnings.push(
                    Diagnostic::warning(
                        "roles",
                        format!(
                            "more than one role is called `{}`; only the highest was imported",
                            r.name
                        ),
                    )
                    .with_hint("rename the duplicates in Discord and import again"),
                );
                continue;
            }
            let key = unique(&slug(&r.name, "role"), &mut keys);
            self.lock.roles.insert(key.clone(), Sf(r.id));
            out.push(RoleSpec {
                key,
                name: r.name.clone(),
                color: (r.color != 0).then_some(Color(r.color)),
                hoist: r.hoist.then_some(true),
                mentionable: r.mentionable.then_some(true),
                permissions: (!r.permissions.is_empty())
                    .then(|| names(r.permissions, PermContext::Role)),
            });
        }
        if let Some(everyone) = live.role(live.guild.id) {
            out.push(RoleSpec {
                key: "everyone".to_owned(),
                name: EVERYONE.to_owned(),
                color: None,
                hoist: None,
                mentionable: None,
                permissions: Some(names(everyone.permissions, PermContext::Role)),
            });
        }
        self.role_names.insert(live.guild.id, EVERYONE.to_owned());
        out
    }

    fn channels(&mut self) -> (Vec<ChannelSpec>, IndexMap<String, CategorySpec>) {
        let live = self.live;
        let supported = |c: &&LiveChannel| c.kind.known().is_some();
        for c in live.channels.iter().filter(|c| c.kind.known().is_none()) {
            self.warnings.push(Diagnostic::warning(
                "",
                format!(
                    "skipped #{}: {} channels aren't supported yet",
                    c.name,
                    c.kind.describe()
                ),
            ));
        }

        let mut top: Vec<&LiveChannel> = live
            .channels
            .iter()
            .filter(supported)
            .filter(|c| c.parent_id.is_none() && !c.kind.is_category())
            .collect();
        sort_effective(&mut top);
        let mut cats: Vec<&LiveChannel> = live
            .channels
            .iter()
            .filter(|c| c.kind.is_category())
            .collect();
        cats.sort_by_key(|c| (c.position, c.id));

        let channels = top.iter().map(|c| self.channel(c, None)).collect();
        let mut categories = IndexMap::new();
        for cat in cats {
            let key = unique(&slug(&cat.name, "category"), &mut self.keys);
            self.lock.channels.insert(key.clone(), Sf(cat.id));
            self.member_warning(cat);
            let mut children: Vec<&LiveChannel> = live
                .channels
                .iter()
                .filter(supported)
                .filter(|c| c.parent_id == Some(cat.id))
                .collect();
            sort_effective(&mut children);
            let spec = CategorySpec {
                name: cat.name.clone(),
                template: None,
                vars: IndexMap::new(),
                private: None,
                overrides: self.overrides(&tuples(cat), PermContext::Category),
                channels: children
                    .iter()
                    .map(|c| self.channel(c, Some(cat)))
                    .collect(),
            };
            categories.insert(key, spec);
        }
        (channels, categories)
    }

    fn channel(&mut self, c: &LiveChannel, parent: Option<&LiveChannel>) -> ChannelSpec {
        let kind = c.kind.known().expect("only supported kinds are imported");
        let key = unique(&slug(&c.name, "channel"), &mut self.keys);
        self.lock.channels.insert(key.clone(), Sf(c.id));
        self.member_warning(c);

        let mut s = ChannelSpec::new(key.clone(), kind, c.name.clone());
        let topic = (!c.topic.is_empty()).then(|| c.topic.clone());
        if kind == ChannelKind::Forum {
            s.guidelines = topic;
        } else {
            s.topic = topic;
        }
        s.nsfw = c.nsfw.then_some(true);
        s.slowmode = (c.slowmode > 0).then_some(Seconds(c.slowmode));
        if kind == ChannelKind::Voice {
            s.bitrate = (c.bitrate != DEFAULT_BITRATE).then_some(c.bitrate);
            s.user_limit = (c.user_limit > 0).then_some(c.user_limit);
        } else {
            s.slowmode = s.slowmode.filter(|_| kind != ChannelKind::Announcement);
        }
        if kind != ChannelKind::Voice {
            s.auto_archive = (c.forum.auto_archive != DEFAULT_AUTO_ARCHIVE)
                .then_some(ArchiveDuration(c.forum.auto_archive));
        }
        if kind == ChannelKind::Forum {
            let f = &c.forum;
            if !f.tags.is_empty() {
                s.tags = Some(
                    f.tags
                        .iter()
                        .map(|t| {
                            self.lock.tags.insert(tag_key(&key, &t.name), Sf(t.id));
                            if t.emoji.is_none() && !t.moderated {
                                TagEntry::Name(t.name.clone())
                            } else {
                                TagEntry::Full(TagSpec {
                                    key: None,
                                    name: t.name.clone(),
                                    emoji: t.emoji.as_ref().map(|e| e.to_spec()),
                                    moderated: t.moderated,
                                })
                            }
                        })
                        .collect(),
                );
            }
            s.require_tag = f.require_tag.then_some(true);
            s.layout = (f.layout != Default::default()).then_some(f.layout);
            s.sort = (f.sort != Default::default()).then_some(f.sort);
            s.default_reaction = f.default_reaction.as_ref().map(|e| e.to_spec());
        }

        let mine = tuples(c);
        let ctx = kind.perm_context();
        match parent {
            None => s.overrides = self.overrides(&mine, ctx),
            Some(p) => {
                let base = tuples(p);
                if mine != base {
                    match layered_delta(&base, &mine) {
                        Some(delta) => s.overrides = self.overrides(&delta, ctx),
                        None => {
                            s.sync = Some(false);
                            s.overrides = self.overrides(&mine, ctx);
                        }
                    }
                }
            }
        }
        s
    }

    fn overrides(
        &mut self,
        list: &[Tuple],
        ctx: PermContext,
    ) -> Option<IndexMap<String, OverrideSpec>> {
        if list.is_empty() {
            return None;
        }
        let live = self.live;
        let mut sorted: Vec<&Tuple> = list.iter().collect();
        sorted.sort_by_key(|(id, _, _)| {
            let pos = if *id == live.guild.id {
                i64::MAX
            } else {
                live.role(*id).map_or(-1, |r| r.position)
            };
            (std::cmp::Reverse(pos), *id)
        });
        let mut map = IndexMap::new();
        for (id, allow, deny) in sorted {
            let Some(name) = self.role_names.get(id).cloned() else {
                self.warnings.push(Diagnostic::warning(
                    "",
                    format!("skipped a permission overwrite for unknown role {id}"),
                ));
                continue;
            };
            map.insert(
                name,
                OverrideSpec {
                    allow: names(*allow, ctx),
                    deny: names(*deny, ctx),
                },
            );
        }
        Some(map)
    }

    fn member_warning(&mut self, c: &LiveChannel) {
        let n = c.member_overwrites().len();
        if n > 0 {
            self.warnings.push(Diagnostic::warning(
                "",
                format!(
                    "{} has {n} per-member permission overwrite(s); concord leaves those alone",
                    c.name
                ),
            ));
        }
    }
}

fn tuples(c: &LiveChannel) -> Vec<Tuple> {
    c.role_overwrites()
}

/// Express `child` as `base` plus layered overrides, if possible. Layering
/// can set a bit to allow or deny but can't reset it to neutral.
fn layered_delta(base: &[Tuple], child: &[Tuple]) -> Option<Vec<Tuple>> {
    let mut delta = Vec::new();
    for (id, ca, cd) in child {
        let (pa, pd) = base
            .iter()
            .find(|(b, _, _)| b == id)
            .map_or((Perms::NONE, Perms::NONE), |(_, a, d)| (*a, *d));
        let ta = ca.minus(pa);
        let td = cd.minus(pd);
        if !(ta.is_empty() && td.is_empty()) {
            delta.push((*id, ta, td));
        }
    }
    let mut result = layer_overwrites(base, &delta);
    result.sort_by_key(|(id, _, _)| *id);
    let mut want = child.to_vec();
    want.sort_by_key(|(id, _, _)| *id);
    (result == want).then_some(delta)
}

fn sort_effective(v: &mut [&LiveChannel]) {
    v.sort_by_key(|c| (c.kind.is_voice_like(), c.position, c.id));
}

/// A readable key: lower case, ASCII letters/digits, hyphens.
pub fn slug(name: &str, fallback: &str) -> String {
    let mut out = String::new();
    for ch in name.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() {
        fallback.to_owned()
    } else {
        out
    }
}

fn unique(base: &str, used: &mut HashSet<String>) -> String {
    let mut key = base.to_owned();
    let mut n = 2;
    while !used.insert(key.clone()) {
        key = format!("{base}-{n}");
        n += 1;
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_readable() {
        assert_eq!(slug("engineering 🏁", "x"), "engineering");
        assert_eq!(slug("vehicle-dynamics 🪄", "x"), "vehicle-dynamics");
        assert_eq!(slug("🏁", "category"), "category");
        assert_eq!(slug("Team Lead", "x"), "team-lead");
    }

    #[test]
    fn layered_delta_falls_back_when_bits_are_reset() {
        let v = Perms::VIEW_CHANNEL;
        let s = Perms::SEND_MESSAGES;
        let base = vec![(1, Perms::NONE, v)];
        // Child adds an allow for role 2: expressible.
        let child = vec![(1, Perms::NONE, v), (2, v, Perms::NONE)];
        assert_eq!(
            layered_delta(&base, &child),
            Some(vec![(2, v, Perms::NONE)])
        );
        // Child resets @everyone's deny to neutral: not expressible.
        let child = vec![(2, s, Perms::NONE)];
        assert_eq!(layered_delta(&base, &child), None);
    }
}
