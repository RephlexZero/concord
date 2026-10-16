//! What the bot is allowed to do: Discord's role hierarchy and the rule that
//! a bot can only grant permissions it has itself.

use super::{EntityKind, Op, Ref, ResolvedOverwrite};
use crate::diag::Diagnostic;
use crate::model::{Desired, LiveState};
use crate::perms::{names, PermContext, Perms};

pub fn run(ops: &[Op], desired: &Desired, live: &LiveState, diags: &mut Vec<Diagnostic>) {
    if ops.is_empty() {
        return;
    }
    let Some(bot) = &live.bot else {
        diags.push(Diagnostic::warning(
            "",
            "no information about the bot in this state; role hierarchy and permissions weren't checked",
        ));
        return;
    };
    let bot_roles: Vec<_> = live
        .roles
        .iter()
        .filter(|r| bot.role_ids.contains(&r.id))
        .collect();
    let top = bot_roles
        .iter()
        .max_by_key(|r| (r.position, std::cmp::Reverse(r.id)));
    let top_pos = top.map_or(0, |r| r.position);
    let top_name = top.map_or_else(|| "(no role)".to_owned(), |r| r.name.clone());
    let everyone = live
        .role(live.guild.id)
        .map_or(Perms::NONE, |r| r.permissions);
    let perms = bot_roles
        .iter()
        .fold(everyone, |acc, r| acc.union(r.permissions));
    let admin = perms.contains(Perms::ADMINISTRATOR);

    let role_path = |key: &str| {
        desired
            .roles
            .iter()
            .find(|r| r.key == key)
            .map_or_else(String::new, |r| r.path.clone())
    };
    let channel_path = |key: &str| {
        desired
            .channels
            .iter()
            .find(|c| c.key == key)
            .map_or_else(String::new, |c| c.path.clone())
    };
    let above = |path: String, name: &str| {
        Diagnostic::error(
            path,
            format!("concord can't change role `{name}`: it isn't below concord's highest role `{top_name}`"),
        )
        .with_hint(format!(
            "in Server Settings → Roles, drag `{top_name}` above `{name}`"
        ))
    };

    let mut needs_roles = false;
    let mut needs_channels = false;
    let mut needs_guild = false;
    let mut needs_expressions = false;
    for op in ops {
        match op {
            Op::UpdateRole(u) => {
                needs_roles = true;
                let pos = live.role(u.id).map_or(0, |r| r.position);
                if u.id != live.guild.id && pos >= top_pos {
                    diags.push(above(role_path(&u.key), &u.name));
                }
                if !admin {
                    for change in &u.changes {
                        if let super::RoleField::Permissions(c) = change {
                            missing(c.after, perms, &role_path(&u.key), &u.name, diags);
                        }
                    }
                }
            }
            Op::CreateRole(c) => {
                needs_roles = true;
                if let (false, Some(p)) = (admin, c.permissions) {
                    missing(p, perms, &role_path(&c.key), &c.name, diags);
                }
            }
            Op::Delete {
                kind: EntityKind::Role,
                id,
                name,
            } => {
                needs_roles = true;
                if live.role(*id).is_some_and(|r| r.position >= top_pos) {
                    diags.push(above(String::new(), name));
                }
            }
            // Hierarchy problems with role order are reported by the diff.
            Op::ReorderRoles(_) => needs_roles = true,
            Op::CreateChannel(c) => {
                needs_channels = true;
                if !c.overwrites.is_empty() {
                    needs_roles = true;
                }
                if !admin {
                    overwrite_checks(
                        &c.overwrites,
                        perms,
                        bot.role_ids.as_slice(),
                        live.guild.id,
                        &channel_path(&c.key),
                        &c.name,
                        c.kind.perm_context(),
                        diags,
                    );
                }
            }
            Op::SetPermissionOverwrites(s) => {
                needs_roles = true;
                if !admin {
                    overwrite_checks(
                        &s.after,
                        perms,
                        bot.role_ids.as_slice(),
                        live.guild.id,
                        &channel_path(&s.key),
                        &s.name,
                        s.kind.perm_context(),
                        diags,
                    );
                }
            }
            Op::UpdateChannel(_)
            | Op::SetForumTags(_)
            | Op::ReorderChannels(_)
            | Op::Delete { .. } => {
                needs_channels = true;
            }
            Op::Adopt { .. } => {}
            Op::Community(c) => {
                use crate::community::diff::CommunityOp as C;
                match c {
                    C::CreateEmoji { .. }
                    | C::UpdateEmoji { .. }
                    | C::DeleteEmoji { .. }
                    | C::CreateSticker { .. }
                    | C::UpdateSticker { .. }
                    | C::DeleteSticker { .. }
                    | C::CreateSound { .. }
                    | C::UpdateSound { .. }
                    | C::DeleteSound { .. } => needs_expressions = true,
                    _ => needs_guild = true,
                }
            }
        }
    }

    if !admin {
        for (needed, flag, what) in [
            (needs_roles, Perms::MANAGE_ROLES, "MANAGE_ROLES"),
            (needs_channels, Perms::MANAGE_CHANNELS, "MANAGE_CHANNELS"),
            (needs_guild, Perms(1 << 5), "MANAGE_GUILD"),
            (
                needs_expressions,
                Perms(1 << 30),
                "MANAGE_GUILD_EXPRESSIONS",
            ),
        ] {
            if needed && !perms.contains(flag) {
                diags.push(
                    Diagnostic::error("", format!("concord's bot doesn't have {what}")).with_hint(
                        "give the bot's role Administrator (recommended) or that permission",
                    ),
                );
            }
        }
    }
}

fn missing(want: Perms, have: Perms, path: &str, name: &str, diags: &mut Vec<Diagnostic>) {
    let lacking = want.minus(have);
    if !lacking.is_empty() {
        diags.push(
            Diagnostic::error(
                path,
                format!(
                    "can't give `{name}` {}: concord's bot doesn't have {}",
                    names(lacking, PermContext::Role).join(", "),
                    if lacking.0.count_ones() == 1 {
                        "it"
                    } else {
                        "them"
                    }
                ),
            )
            .with_hint(
                "Discord only lets a bot grant permissions it has; give the bot Administrator",
            ),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn overwrite_checks(
    overwrites: &[ResolvedOverwrite],
    have: Perms,
    bot_roles: &[u64],
    everyone: u64,
    path: &str,
    name: &str,
    ctx: PermContext,
    diags: &mut Vec<Diagnostic>,
) {
    let mut used = Perms::NONE;
    for o in overwrites {
        used = used.union(o.allow).union(o.deny);
        if o.allow.contains(Perms::MANAGE_ROLES) {
            diags.push(
                Diagnostic::error(
                    path,
                    format!("`{name}` allows MANAGE_ROLES for {}", o.label),
                )
                .with_hint(
                    "only a bot with Administrator can set that; give the bot Administrator",
                ),
            );
        }
    }
    let lacking = used.minus(have);
    if !lacking.is_empty() {
        diags.push(
            Diagnostic::error(
                path,
                format!(
                    "`{name}` sets {}, which concord's bot doesn't have",
                    names(lacking, ctx).join(", ")
                ),
            )
            .with_hint(
                "Discord only lets a bot set permissions it has; give the bot Administrator",
            ),
        );
    }
    let hidden = overwrites
        .iter()
        .any(|o| o.target == Ref::Id(everyone) && o.deny.contains(Perms::VIEW_CHANNEL));
    let bot_allowed = overwrites.iter().any(|o| {
        o.target.id().is_some_and(|id| bot_roles.contains(&id))
            && o.allow.contains(Perms::VIEW_CHANNEL)
    });
    if hidden && !bot_allowed {
        diags.push(
            Diagnostic::warning(
                path,
                format!("concord will lose access to `{name}` (it's hidden from @everyone)"),
            )
            .with_hint("give the bot Administrator, or allow its role VIEW_CHANNEL here"),
        );
    }
}
