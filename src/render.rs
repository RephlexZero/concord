//! Human-readable plan output.

use std::fmt::Write;

use owo_colors::OwoColorize;

use crate::diag::{Diagnostic, Severity};
use crate::diff::{
    Change, ChannelField, CreateChannel, EntityKind, Op, Plan, ResolvedOverwrite, RoleField,
    TagChange, TagState, Unmanaged, UnmanagedKind,
};
use crate::model::{ChannelKind, Emoji, Layout, Sort};
use crate::perms::{names, PermContext, Perms};
use crate::spec::raw::{ArchiveDuration, Color};

#[derive(Clone, Copy)]
pub struct Style {
    pub color: bool,
}

#[derive(Clone, Copy)]
enum Tone {
    Add,
    Change,
    Remove,
    Dim,
    Bold,
    Warn,
}

impl Style {
    fn paint(&self, s: impl AsRef<str>, tone: Tone) -> String {
        let s = s.as_ref();
        if !self.color {
            return s.to_owned();
        }
        match tone {
            Tone::Add => s.green().to_string(),
            Tone::Change => s.yellow().to_string(),
            Tone::Remove => s.red().to_string(),
            Tone::Dim => s.dimmed().to_string(),
            Tone::Bold => s.bold().to_string(),
            Tone::Warn => s.magenta().to_string(),
        }
    }
}

pub fn render_diagnostics(diags: &[Diagnostic], style: Style) -> String {
    let mut out = String::new();
    let mut sorted: Vec<&Diagnostic> = diags.iter().collect();
    sorted.sort_by_key(|d| d.severity);
    for d in sorted {
        let (label, tone) = match d.severity {
            Severity::Error => ("error", Tone::Remove),
            Severity::Warning => ("warning", Tone::Warn),
        };
        let _ = write!(out, "{}", style.paint(label, tone));
        if !d.path.is_empty() {
            let _ = write!(out, " {}", style.paint(&d.path, Tone::Bold));
        }
        let _ = writeln!(out, ": {}", d.message);
        if let Some(h) = &d.hint {
            let _ = writeln!(out, "  {} {h}", style.paint("hint:", Tone::Dim));
        }
    }
    out
}

pub fn render_plan(plan: &Plan, style: Style) -> String {
    let mut out = String::new();
    let s = &style;
    let _ = writeln!(
        out,
        "{} {} ({})\n",
        s.paint("concord plan for", Tone::Bold),
        s.paint(&plan.guild_name, Tone::Bold),
        plan.guild_id
    );

    let is_role = |o: &&Op| {
        matches!(
            o,
            Op::Adopt {
                kind: EntityKind::Role,
                ..
            } | Op::CreateRole(_)
                | Op::UpdateRole(_)
                | Op::ReorderRoles(_)
        )
    };
    let is_delete = |o: &&Op| matches!(o, Op::Delete { .. });
    let is_community = |o: &&Op| matches!(o, Op::Community(_));
    let role_ops: Vec<&Op> = plan.ops.iter().filter(is_role).collect();
    let deletes: Vec<&Op> = plan.ops.iter().filter(is_delete).collect();
    let community_ops: Vec<&Op> = plan.ops.iter().filter(is_community).collect();
    let channel_ops: Vec<&Op> = plan
        .ops
        .iter()
        .filter(|o| !is_role(o) && !is_delete(o) && !is_community(o))
        .collect();

    // Channels whose overwrites only change because their category's did.
    let synced: Vec<&crate::diff::SetOverwrites> = channel_ops
        .iter()
        .filter_map(|o| match o {
            Op::SetPermissionOverwrites(so) if so.synced_with.is_some() => Some(so),
            _ => None,
        })
        .collect();

    if !role_ops.is_empty() {
        let _ = writeln!(out, "{}", s.paint("Roles", Tone::Bold));
        for op in &role_ops {
            op_lines(&mut out, op, s, &synced);
        }
        out.push('\n');
    }
    if !channel_ops.is_empty() {
        let _ = writeln!(out, "{}", s.paint("Channels", Tone::Bold));
        for op in &channel_ops {
            if let Op::SetPermissionOverwrites(so) = op {
                // Shown under the category instead, if the category changes too.
                let category_changes = channel_ops.iter().any(|o| {
                    matches!(o, Op::SetPermissionOverwrites(c)
                        if c.kind == ChannelKind::Category && Some(&c.name) == so.synced_with.as_ref())
                });
                if so.synced_with.is_some() && category_changes {
                    continue;
                }
            }
            op_lines(&mut out, op, s, &synced);
        }
        out.push('\n');
    }
    if !community_ops.is_empty() {
        let _ = writeln!(out, "{}", s.paint("Server", Tone::Bold));
        for op in &community_ops {
            op_lines(&mut out, op, s, &synced);
        }
        out.push('\n');
    }
    if !deletes.is_empty() {
        let _ = writeln!(out, "{}", s.paint("Deletions (--prune)", Tone::Bold));
        for op in &deletes {
            op_lines(&mut out, op, s, &synced);
        }
        out.push('\n');
    }

    let kept: Vec<&Unmanaged> = plan.unmanaged.iter().filter(|u| !u.pruned).collect();
    if !kept.is_empty() {
        let _ = writeln!(
            out,
            "{} {}",
            s.paint("Unmanaged", Tone::Bold),
            s.paint(
                "(on Discord but not in the spec; left alone, --prune deletes them)",
                Tone::Dim
            )
        );
        for u in kept {
            let what = match &u.kind {
                UnmanagedKind::Role => format!("role      @{}", u.name),
                UnmanagedKind::Category => format!("category  {}", u.name),
                UnmanagedKind::Channel { kind, parent } => match parent {
                    Some(p) => format!("{kind:<9} #{} in {p}", u.name),
                    None => format!("{kind:<9} #{}", u.name),
                },
                UnmanagedKind::Tag { forum } => format!("tag       {} in #{forum}", u.name),
                UnmanagedKind::Automod => format!("automod   {}", u.name),
                UnmanagedKind::Emoji => format!("emoji     :{}:", u.name),
                UnmanagedKind::Sticker => format!("sticker   {}", u.name),
                UnmanagedKind::Sound => format!("sound     {}", u.name),
            };
            let note = u
                .note
                .as_ref()
                .map(|n| format!("  ({n})"))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "  {} {what}{}",
                s.paint("?", Tone::Dim),
                s.paint(note, Tone::Dim)
            );
        }
        out.push('\n');
    }

    if !plan.diagnostics.is_empty() {
        out.push_str(&render_diagnostics(&plan.diagnostics, style));
        out.push('\n');
    }

    out.push_str(&summary(plan, style));
    out
}

fn summary(plan: &Plan, s: Style) -> String {
    let count = |f: fn(&Op) -> bool| plan.ops.iter().filter(|o| f(o)).count();
    let create = count(|o| match o {
        Op::CreateRole(_) | Op::CreateChannel(_) => true,
        Op::Community(c) => c.is_create(),
        _ => false,
    });
    let update = count(|o| match o {
        Op::UpdateRole(_)
        | Op::UpdateChannel(_)
        | Op::SetForumTags(_)
        | Op::SetPermissionOverwrites(_) => true,
        Op::Community(c) => !c.is_create() && !c.is_delete(),
        _ => false,
    });
    let reorder = count(|o| matches!(o, Op::ReorderChannels(_) | Op::ReorderRoles(_)));
    let adopt = count(|o| matches!(o, Op::Adopt { .. }));
    let delete = count(|o| match o {
        Op::Delete { .. } => true,
        Op::Community(c) => c.is_delete(),
        _ => false,
    });
    let unmanaged = plan.unmanaged.iter().filter(|u| !u.pruned).count();
    let errors = plan.diagnostics.iter().filter(|d| d.is_error()).count();

    if !plan.has_changes() {
        let mut line = s.paint("No changes. The server matches the spec.", Tone::Add);
        if unmanaged > 0 {
            line.push_str(&format!(" ({unmanaged} unmanaged)"));
        }
        return line + "\n";
    }
    let mut parts = vec![
        s.paint(format!("{create} to create"), Tone::Add),
        s.paint(format!("{update} to update"), Tone::Change),
    ];
    if reorder > 0 {
        parts.push(s.paint(format!("{reorder} to reorder"), Tone::Change));
    }
    if adopt > 0 {
        parts.push(format!("{adopt} to adopt"));
    }
    if delete > 0 {
        parts.push(s.paint(format!("{delete} to delete"), Tone::Remove));
    }
    if unmanaged > 0 {
        parts.push(s.paint(format!("{unmanaged} unmanaged"), Tone::Dim));
    }
    let mut line = format!("Plan: {}.", parts.join(", "));
    if errors > 0 {
        line.push_str(&s.paint(
            format!(" {errors} error(s) must be fixed before apply."),
            Tone::Remove,
        ));
    }
    line + "\n"
}

fn sigil(s: &Style, tone: Tone) -> String {
    let c = match tone {
        Tone::Add => "+",
        Tone::Change => "~",
        Tone::Remove => "-",
        _ => "=",
    };
    s.paint(c, tone)
}

fn op_lines(out: &mut String, op: &Op, s: &Style, synced: &[&crate::diff::SetOverwrites]) {
    match op {
        Op::Adopt { kind, name, id, .. } => {
            let what = match kind {
                EntityKind::Role => format!("@{name}"),
                EntityKind::Category => name.clone(),
                EntityKind::Channel => format!("#{name}"),
            };
            let _ = writeln!(
                out,
                "  {} adopt    {what}  {}",
                sigil(s, Tone::Dim),
                s.paint(
                    format!("(already exists as {id}; recorded in the lockfile)"),
                    Tone::Dim
                )
            );
        }
        Op::CreateRole(c) => {
            let mut bits = Vec::new();
            if let Some(col) = c.color.filter(|c| *c != 0) {
                bits.push(Color(col).to_string());
            }
            if c.hoist == Some(true) {
                bits.push("hoisted".into());
            }
            if c.mentionable == Some(true) {
                bits.push("mentionable".into());
            }
            if let Some(p) = c.permissions.filter(|p| !p.is_empty()) {
                bits.push(names(p, PermContext::Role).join(", "));
            }
            let _ = writeln!(
                out,
                "  {} create   @{}  {}",
                sigil(s, Tone::Add),
                s.paint(&c.name, Tone::Add),
                s.paint(bits.join(", "), Tone::Dim)
            );
            if let Some(old) = c.replaces {
                let _ = writeln!(
                    out,
                    "      {}",
                    s.paint(
                        format!("recreating: the lockfile's ID {old} no longer exists"),
                        Tone::Warn
                    )
                );
            }
        }
        Op::UpdateRole(u) => {
            // `@everyone` already starts with an @.
            let at = if u.name.starts_with('@') { "" } else { "@" };
            let _ = writeln!(out, "  {} update   {at}{}", sigil(s, Tone::Change), u.name);
            for f in &u.changes {
                match f {
                    RoleField::Name(c) => field(out, s, "name", &c.before, &c.after),
                    RoleField::Color(c) => field(
                        out,
                        s,
                        "color",
                        &Color(c.before).to_string(),
                        &Color(c.after).to_string(),
                    ),
                    RoleField::Hoist(c) => {
                        field(out, s, "hoist", &c.before.to_string(), &c.after.to_string())
                    }
                    RoleField::Mentionable(c) => field(
                        out,
                        s,
                        "mentionable",
                        &c.before.to_string(),
                        &c.after.to_string(),
                    ),
                    RoleField::Permissions(c) => {
                        perm_delta(out, s, "permissions", c, PermContext::Role)
                    }
                    RoleField::Icon(c) => {
                        let show =
                            |i: &Option<String>| i.clone().unwrap_or_else(|| "none".to_owned());
                        field(out, s, "icon", &show(&c.before), &show(&c.after))
                    }
                }
            }
        }
        Op::Community(c) => community_op(out, s, c),
        Op::ReorderRoles(r) => {
            let _ = writeln!(
                out,
                "  {} reorder  roles (highest first)",
                sigil(s, Tone::Change)
            );
            order_lines(out, s, &r.before, &r.after);
        }
        Op::CreateChannel(c) => create_channel(out, s, c),
        Op::UpdateChannel(u) => {
            let _ = writeln!(
                out,
                "  {} update   {}",
                sigil(s, Tone::Change),
                channel_label(u.kind, &u.name)
            );
            for f in &u.changes {
                channel_field(out, s, f);
            }
        }
        Op::SetForumTags(t) => {
            let _ = writeln!(out, "  {} tags     #{}", sigil(s, Tone::Change), t.name);
            for c in &t.changes {
                let line = match c {
                    TagChange::Add(n) => format!("{} {n}", s.paint("+", Tone::Add)),
                    TagChange::Remove(n) => format!("{} {n}", s.paint("-", Tone::Remove)),
                    TagChange::Rename(c) => {
                        format!("{} {} → {}", s.paint("~", Tone::Change), c.before, c.after)
                    }
                    TagChange::Emoji { name, change } => format!(
                        "{} {name}: emoji {} → {}",
                        s.paint("~", Tone::Change),
                        emoji_label(&change.before),
                        emoji_label(&change.after)
                    ),
                    TagChange::Moderated { name, change } => format!(
                        "{} {name}: moderated {} → {}",
                        s.paint("~", Tone::Change),
                        change.before,
                        change.after
                    ),
                    TagChange::Reorder => format!(
                        "{} order: {}",
                        s.paint("~", Tone::Change),
                        tag_names(&t.after)
                    ),
                };
                let _ = writeln!(out, "      {line}");
            }
            let kept = t.after.iter().filter(|t| t.key.is_none()).count();
            if kept > 0 {
                let _ = writeln!(
                    out,
                    "      {}",
                    s.paint(
                        format!("({kept} unmanaged tag(s) kept; existing tags keep their IDs)"),
                        Tone::Dim
                    )
                );
            }
        }
        Op::SetPermissionOverwrites(o) => {
            let what = channel_label(o.kind, &o.name);
            match &o.synced_with {
                Some(cat) => {
                    let _ = writeln!(
                        out,
                        "  {} perms    {what}  {}",
                        sigil(s, Tone::Change),
                        s.paint(format!("(re-sync with category {cat})"), Tone::Dim)
                    );
                }
                None => {
                    let _ = writeln!(out, "  {} perms    {what}", sigil(s, Tone::Change));
                }
            }
            overwrite_delta(out, s, &o.before, &o.after, o.kind.perm_context());
            if o.kind == ChannelKind::Category {
                let children: Vec<String> = synced
                    .iter()
                    .filter(|c| c.synced_with.as_deref() == Some(o.name.as_str()))
                    .map(|c| channel_label(c.kind, &c.name))
                    .collect();
                if !children.is_empty() {
                    let _ = writeln!(
                        out,
                        "      {}",
                        s.paint(
                            format!(
                                "+ re-syncs {} channel(s): {}",
                                children.len(),
                                children.join(", ")
                            ),
                            Tone::Dim
                        )
                    );
                }
            }
        }
        Op::ReorderChannels(groups) => {
            for g in groups {
                let _ = writeln!(out, "  {} reorder  {}", sigil(s, Tone::Change), g.label);
                order_lines(out, s, &g.before, &g.after);
            }
        }
        Op::Delete { kind, id, name } => {
            let what = match kind {
                EntityKind::Role => format!("role @{name}"),
                EntityKind::Category => format!("category {name}"),
                EntityKind::Channel => format!("channel #{name}"),
            };
            let _ = writeln!(
                out,
                "  {} delete   {}  {}",
                sigil(s, Tone::Remove),
                s.paint(what, Tone::Remove),
                s.paint(format!("({id})"), Tone::Dim)
            );
        }
    }
}

fn channel_label(kind: ChannelKind, name: &str) -> String {
    match kind {
        ChannelKind::Category => format!("category {name}"),
        ChannelKind::Voice => format!("🔊{name}"),
        _ => format!("#{name}"),
    }
}

fn create_channel(out: &mut String, s: &Style, c: &CreateChannel) {
    let parent = c
        .parent
        .as_ref()
        .map(|p| format!(" in {}", p.name))
        .unwrap_or_default();
    let kind = if c.kind == ChannelKind::Category {
        String::new()
    } else {
        format!("  {}", s.paint(format!("({})", c.kind.as_str()), Tone::Dim))
    };
    let _ = writeln!(
        out,
        "  {} create   {}{parent}{kind}",
        sigil(s, Tone::Add),
        s.paint(channel_label(c.kind, &c.name), Tone::Add),
    );
    if let Some(old) = c.replaces {
        let _ = writeln!(
            out,
            "      {}",
            s.paint(
                format!("recreating: the lockfile's ID {old} no longer exists"),
                Tone::Warn
            )
        );
    }
    let mut attr = |k: &str, v: String| {
        let _ = writeln!(out, "      {} {v}", s.paint(format!("{k}:"), Tone::Dim));
    };
    if let Some(t) = &c.topic {
        let k = if c.kind == ChannelKind::Forum {
            "guidelines"
        } else {
            "topic"
        };
        attr(k, ellipsis(t));
    }
    if !c.tags.is_empty() {
        attr("tags", tag_names(&c.tags));
    }
    if c.require_tag == Some(true) {
        attr("require_tag", "true".into());
    }
    if let Some(l) = c.layout {
        attr("layout", layout_label(l).into());
    }
    if let Some(so) = c.sort {
        attr("sort", sort_label(so).into());
    }
    if let Some(Some(e)) = &c.default_reaction {
        attr("default_reaction", e.to_spec());
    }
    if let Some(a) = c.auto_archive {
        if a != crate::model::DEFAULT_AUTO_ARCHIVE {
            attr("auto_archive", ArchiveDuration::label(a));
        }
    }
    if let Some(sl) = c.slowmode.filter(|s| *s > 0) {
        attr("slowmode", format!("{sl}s"));
    }
    if let Some(u) = c.user_limit.filter(|u| *u > 0) {
        attr("user_limit", u.to_string());
    }
    let ctx = c.kind.perm_context();
    for o in &c.overwrites {
        attr(
            "perms",
            format!("{} {}", o.label, describe_overwrite(o, ctx)),
        );
    }
}

fn channel_field(out: &mut String, s: &Style, f: &ChannelField) {
    match f {
        ChannelField::Name(c) => field(out, s, "name", &c.before, &c.after),
        ChannelField::Kind(c) => field(out, s, "type", c.before.as_str(), c.after.as_str()),
        ChannelField::Parent(c) => {
            let label = |p: &Option<crate::diff::Parent>| {
                p.as_ref()
                    .map_or_else(|| "(no category)".to_owned(), |p| p.name.clone())
            };
            field(out, s, "category", &label(&c.before), &label(&c.after));
        }
        ChannelField::Topic(c) => field(out, s, "topic", &quoted(&c.before), &quoted(&c.after)),
        ChannelField::Nsfw(c) => field(out, s, "nsfw", &c.before.to_string(), &c.after.to_string()),
        ChannelField::Slowmode(c) => field(
            out,
            s,
            "slowmode",
            &format!("{}s", c.before),
            &format!("{}s", c.after),
        ),
        ChannelField::Bitrate(c) => field(
            out,
            s,
            "bitrate",
            &c.before.to_string(),
            &c.after.to_string(),
        ),
        ChannelField::UserLimit(c) => field(
            out,
            s,
            "user_limit",
            &c.before.to_string(),
            &c.after.to_string(),
        ),
        ChannelField::RequireTag(c) => field(
            out,
            s,
            "require_tag",
            &c.before.to_string(),
            &c.after.to_string(),
        ),
        ChannelField::Layout(c) => field(
            out,
            s,
            "layout",
            layout_label(c.before),
            layout_label(c.after),
        ),
        ChannelField::Sort(c) => field(out, s, "sort", sort_label(c.before), sort_label(c.after)),
        ChannelField::DefaultReaction(c) => field(
            out,
            s,
            "default_reaction",
            &emoji_label(&c.before),
            &emoji_label(&c.after),
        ),
        ChannelField::AutoArchive(c) => field(
            out,
            s,
            "auto_archive",
            &ArchiveDuration::label(c.before),
            &ArchiveDuration::label(c.after),
        ),
        ChannelField::Region(c) => {
            let show = |r: &Option<String>| r.clone().unwrap_or_else(|| "automatic".to_owned());
            field(out, s, "region", &show(&c.before), &show(&c.after))
        }
        ChannelField::VideoQuality(c) => {
            let show = |v: u8| if v == 2 { "full" } else { "auto" };
            field(out, s, "video_quality", show(c.before), show(c.after))
        }
        ChannelField::PostSlowmode(c) => field(
            out,
            s,
            "post_slowmode",
            &duration(c.before),
            &duration(c.after),
        ),
    }
}

/// A duration in seconds, the way specs write it: `30s`, `5m`, `2h`, `1d`.
pub fn duration(secs: u32) -> String {
    match secs {
        0 => "off".to_owned(),
        s if s % 86_400 == 0 => format!("{}d", s / 86_400),
        s if s % 3600 == 0 => format!("{}h", s / 3600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

fn community_lines(out: &mut String, s: &Style, lines: &[crate::community::diff::Line]) {
    use crate::community::diff::LineTone;
    for l in lines {
        let (sig, tone) = match l.tone {
            LineTone::Add => ("+", Tone::Add),
            LineTone::Change => ("~", Tone::Change),
            LineTone::Remove => ("-", Tone::Remove),
            LineTone::Note => (" ", Tone::Dim),
        };
        let _ = writeln!(out, "      {} {}", s.paint(sig, tone), l.text);
    }
}

fn community_op(out: &mut String, s: &Style, op: &crate::community::diff::CommunityOp) {
    use crate::community::diff::CommunityOp as C;
    let head = |out: &mut String, tone: Tone, verb: &str, what: &str| {
        let _ = writeln!(out, "  {} {verb:<8} {what}", sigil(s, tone));
    };
    match op {
        C::Settings(changes) => {
            head(out, Tone::Change, "update", "server settings");
            for c in changes {
                field(out, s, c.field, &c.before, &c.after);
            }
        }
        C::Widget { lines, .. } => {
            head(out, Tone::Change, "update", "server widget");
            community_lines(out, s, lines);
        }
        C::Onboarding { lines, .. } => {
            head(out, Tone::Change, "update", "onboarding");
            community_lines(out, s, lines);
        }
        C::WelcomeScreen { lines, .. } => {
            head(out, Tone::Change, "update", "welcome screen");
            community_lines(out, s, lines);
        }
        C::Guide { lines, .. } => {
            head(out, Tone::Change, "update", "Server Guide");
            community_lines(out, s, lines);
        }
        C::JoinQuestions { lines, .. } => {
            head(out, Tone::Change, "update", "join questions");
            community_lines(out, s, lines);
        }
        C::CreateAutomod { lines, .. } => {
            head(out, Tone::Add, "create", "AutoMod rule");
            community_lines(out, s, lines);
        }
        C::UpdateAutomod { rule, lines, .. } => {
            head(
                out,
                Tone::Change,
                "update",
                &format!("AutoMod rule \"{}\"", rule.name),
            );
            community_lines(out, s, lines);
        }
        C::DeleteAutomod { name, .. } => head(
            out,
            Tone::Remove,
            "delete",
            &format!("AutoMod rule \"{name}\""),
        ),
        C::CreateEmoji { name, asset, .. } => head(
            out,
            Tone::Add,
            "create",
            &format!("emoji :{name}:  ({})", asset.file),
        ),
        C::UpdateEmoji { name, lines, .. } => {
            head(out, Tone::Change, "update", &format!("emoji :{name}:"));
            community_lines(out, s, lines);
        }
        C::DeleteEmoji { name, .. } => {
            head(out, Tone::Remove, "delete", &format!("emoji :{name}:"))
        }
        C::CreateSticker { name, asset, .. } => head(
            out,
            Tone::Add,
            "create",
            &format!("sticker {name}  ({})", asset.file),
        ),
        C::UpdateSticker { name, lines, .. } => {
            head(out, Tone::Change, "update", &format!("sticker {name}"));
            community_lines(out, s, lines);
        }
        C::DeleteSticker { name, .. } => {
            head(out, Tone::Remove, "delete", &format!("sticker {name}"))
        }
        C::CreateSound { name, asset, .. } => head(
            out,
            Tone::Add,
            "create",
            &format!("sound {name}  ({})", asset.file),
        ),
        C::UpdateSound { name, lines, .. } => {
            head(out, Tone::Change, "update", &format!("sound {name}"));
            community_lines(out, s, lines);
        }
        C::DeleteSound { name, .. } => head(out, Tone::Remove, "delete", &format!("sound {name}")),
    }
}

fn field(out: &mut String, s: &Style, name: &str, before: &str, after: &str) {
    let _ = writeln!(
        out,
        "      {:<12} {} {} {}",
        name,
        s.paint(before, Tone::Remove),
        s.paint("→", Tone::Dim),
        s.paint(after, Tone::Add)
    );
}

fn perm_delta(out: &mut String, s: &Style, label: &str, c: &Change<Perms>, ctx: PermContext) {
    let added = names(c.after.minus(c.before), ctx);
    let removed = names(c.before.minus(c.after), ctx);
    let mut parts: Vec<String> = added
        .iter()
        .map(|n| s.paint(format!("+{n}"), Tone::Add))
        .collect();
    parts.extend(
        removed
            .iter()
            .map(|n| s.paint(format!("-{n}"), Tone::Remove)),
    );
    let _ = writeln!(out, "      {label:<12} {}", parts.join(" "));
}

fn describe_overwrite(o: &ResolvedOverwrite, ctx: PermContext) -> String {
    let mut parts = Vec::new();
    if !o.allow.is_empty() {
        parts.push(format!("allow {}", names(o.allow, ctx).join(", ")));
    }
    if !o.deny.is_empty() {
        parts.push(format!("deny {}", names(o.deny, ctx).join(", ")));
    }
    parts.join("; ")
}

fn overwrite_delta(
    out: &mut String,
    s: &Style,
    before: &[ResolvedOverwrite],
    after: &[ResolvedOverwrite],
    ctx: PermContext,
) {
    let mut targets: Vec<&ResolvedOverwrite> = after.iter().collect();
    for b in before {
        if !after.iter().any(|a| a.target == b.target) {
            targets.push(b);
        }
    }
    for t in targets {
        let b = before.iter().find(|o| o.target == t.target);
        let a = after.iter().find(|o| o.target == t.target);
        let (ba, bd) = b.map_or((Perms::NONE, Perms::NONE), |o| (o.allow, o.deny));
        let (aa, ad) = a.map_or((Perms::NONE, Perms::NONE), |o| (o.allow, o.deny));
        if (ba, bd) == (aa, ad) {
            continue;
        }
        let mut parts = Vec::new();
        for (word, before, after) in [("allow", ba, aa), ("deny", bd, ad)] {
            let mut bits: Vec<String> = names(after.minus(before), ctx)
                .into_iter()
                .map(|n| s.paint(format!("+{n}"), Tone::Add))
                .collect();
            bits.extend(
                names(before.minus(after), ctx)
                    .into_iter()
                    .map(|n| s.paint(format!("-{n}"), Tone::Remove)),
            );
            if !bits.is_empty() {
                parts.push(format!("{word} {}", bits.join(" ")));
            }
        }
        let who = match (b, a) {
            (None, Some(_)) => s.paint(format!("+ {}", t.label), Tone::Add),
            (Some(_), None) => s.paint(format!("- {}", t.label), Tone::Remove),
            _ => format!("  {}", t.label),
        };
        let _ = writeln!(out, "      {who:<16} {}", parts.join("; "));
    }
}

fn order_lines(out: &mut String, s: &Style, before: &[String], after: &[String]) {
    let before = if before.is_empty() {
        "(empty)".to_owned()
    } else {
        before.join(", ")
    };
    let _ = writeln!(out, "      {} {before}", s.paint("before:", Tone::Dim));
    let _ = writeln!(
        out,
        "      {} {}",
        s.paint("after: ", Tone::Dim),
        after.join(", ")
    );
}

fn tag_names(tags: &[TagState]) -> String {
    tags.iter()
        .map(|t| {
            let mut n = String::new();
            if let Some(e) = &t.emoji {
                n.push_str(&emoji_label(&Some(e.clone())));
                n.push(' ');
            }
            n.push_str(&t.name);
            if t.moderated {
                n.push_str(" [mod]");
            }
            n
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn emoji_label(e: &Option<Emoji>) -> String {
    match e {
        None => "none".to_owned(),
        Some(Emoji::Unicode(u)) => u.clone(),
        Some(Emoji::Custom(id)) => format!("<custom {id}>"),
    }
}

fn layout_label(l: Layout) -> &'static str {
    match l {
        Layout::List => "list",
        Layout::Gallery => "gallery",
    }
}

fn sort_label(s: Sort) -> &'static str {
    match s {
        Sort::Activity => "activity",
        Sort::Created => "created",
    }
}

fn quoted(s: &str) -> String {
    if s.is_empty() {
        "(empty)".to_owned()
    } else {
        format!("\"{}\"", ellipsis(s))
    }
}

fn ellipsis(s: &str) -> String {
    let one_line = s.replace('\n', " ⏎ ");
    if one_line.chars().count() > 60 {
        let cut: String = one_line.chars().take(57).collect();
        format!("{cut}...")
    } else {
        one_line
    }
}
