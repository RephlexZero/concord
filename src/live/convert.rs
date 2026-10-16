//! Convert Discord's JSON into the normalised model.

use super::wire::{Snapshot, WireChannel, WireRole};
use crate::model::{
    BotInfo, Emoji, Layout, LiveChannel, LiveForum, LiveGuild, LiveKind, LiveRole, LiveState,
    LiveTag, Overwrite, OverwriteKind, Sort, DEFAULT_AUTO_ARCHIVE, DEFAULT_BITRATE,
    FLAG_REQUIRE_TAG,
};
use crate::perms::Perms;

pub fn to_live(snap: &Snapshot) -> LiveState {
    let g = &snap.guild;
    let mut protected = Vec::new();
    let mut protect = |id: Option<crate::snowflake::Sf>, why: &'static str| {
        if let Some(id) = id {
            protected.push((id.0, why));
        }
    };
    protect(g.rules_channel_id, "the server's rules channel");
    protect(g.public_updates_channel_id, "the community updates channel");
    protect(g.safety_alerts_channel_id, "the safety alerts channel");
    protect(g.system_channel_id, "the system messages channel");
    protect(g.afk_channel_id, "the AFK channel");

    LiveState {
        guild: LiveGuild {
            id: g.id.0,
            name: g.name.clone(),
            owner_id: g.owner_id.map(|o| o.0),
            protected_channels: protected,
        },
        roles: snap.roles.iter().map(role).collect(),
        channels: snap.channels.iter().map(channel).collect(),
        bot: snap.bot.as_ref().map(|b| BotInfo {
            user_id: b.user.id.0,
            role_ids: b.member.roles.iter().map(|r| r.0).collect(),
        }),
    }
}

fn role(r: &WireRole) -> LiveRole {
    let (color, gradient) = match &r.colors {
        Some(c) => (c.primary_color, c.secondary_color.is_some()),
        None => (r.color, false),
    };
    LiveRole {
        id: r.id.0,
        name: r.name.clone(),
        color,
        gradient,
        hoist: r.hoist,
        mentionable: r.mentionable,
        permissions: Perms::from_wire(&r.permissions).unwrap_or_default(),
        position: r.position,
        managed: r.managed,
    }
}

fn channel(c: &WireChannel) -> LiveChannel {
    let kind = LiveKind::from_wire(c.kind);
    let flags = c.flags.unwrap_or(0);
    let overwrites = c
        .permission_overwrites
        .iter()
        .flatten()
        .map(|o| Overwrite {
            id: o.id.0,
            kind: if o.kind == 1 {
                OverwriteKind::Member
            } else {
                OverwriteKind::Role
            },
            allow: Perms::from_wire(&o.allow).unwrap_or_default(),
            deny: Perms::from_wire(&o.deny).unwrap_or_default(),
        })
        .collect();
    LiveChannel {
        id: c.id.0,
        kind,
        name: c.name.clone().unwrap_or_default(),
        parent_id: c.parent_id.map(|p| p.0),
        position: c.position.unwrap_or(0),
        topic: c.topic.clone().unwrap_or_default(),
        nsfw: c.nsfw.unwrap_or(false),
        slowmode: c.rate_limit_per_user.unwrap_or(0),
        bitrate: c.bitrate.unwrap_or(DEFAULT_BITRATE),
        user_limit: c.user_limit.unwrap_or(0),
        flags,
        overwrites,
        forum: LiveForum {
            tags: c
                .available_tags
                .iter()
                .flatten()
                .map(|t| LiveTag {
                    id: t.id.map_or(0, |i| i.0),
                    name: t.name.clone(),
                    emoji: Emoji::from_wire(t.emoji_id.map(|e| e.0), t.emoji_name.as_deref()),
                    moderated: t.moderated,
                })
                .collect(),
            require_tag: flags & FLAG_REQUIRE_TAG != 0,
            layout: Layout::from_wire(c.default_forum_layout),
            sort: Sort::from_wire(c.default_sort_order),
            default_reaction: c
                .default_reaction_emoji
                .as_ref()
                .and_then(|r| Emoji::from_wire(r.emoji_id.map(|e| e.0), r.emoji_name.as_deref())),
            auto_archive: c
                .default_auto_archive_duration
                .unwrap_or(DEFAULT_AUTO_ARCHIVE),
        },
    }
}
