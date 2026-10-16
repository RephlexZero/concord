use pretty_assertions::assert_eq;

use super::*;
use crate::lock::Lockfile;
use crate::model::*;
use crate::snowflake::Sf;
use crate::spec;

const GUILD: u64 = 1;
const BOT_ROLE: u64 = 10;
const LEAD: u64 = 11;

fn role(id: u64, name: &str, position: i64) -> LiveRole {
    LiveRole {
        id,
        name: name.into(),
        color: 0,
        gradient: false,
        hoist: false,
        mentionable: false,
        permissions: Perms::NONE,
        position,
        managed: false,
    }
}

fn chan(id: u64, kind: ChannelKind, name: &str, parent: Option<u64>, position: i64) -> LiveChannel {
    LiveChannel {
        id,
        kind: LiveKind::Known(kind),
        name: name.into(),
        parent_id: parent,
        position,
        topic: String::new(),
        nsfw: false,
        slowmode: 0,
        bitrate: DEFAULT_BITRATE,
        user_limit: 0,
        flags: 0,
        overwrites: vec![],
        forum: LiveForum {
            auto_archive: DEFAULT_AUTO_ARCHIVE,
            ..Default::default()
        },
    }
}

fn tag(id: u64, name: &str) -> LiveTag {
    LiveTag {
        id,
        name: name.into(),
        emoji: None,
        moderated: false,
    }
}

fn ow(id: u64, allow: Perms, deny: Perms) -> Overwrite {
    Overwrite {
        id,
        kind: OverwriteKind::Role,
        allow,
        deny,
    }
}

/// A server with @everyone, an admin bot role on top, and Lead.
fn server() -> LiveState {
    let mut bot = role(BOT_ROLE, "Concord", 5);
    bot.permissions = Perms::ADMINISTRATOR;
    bot.managed = true;
    LiveState {
        guild: LiveGuild {
            id: GUILD,
            name: "Test".into(),
            owner_id: None,
            protected_channels: vec![],
        },
        roles: vec![role(GUILD, "@everyone", 0), bot, role(LEAD, "Lead", 2)],
        channels: vec![],
        bot: Some(BotInfo {
            user_id: 99,
            role_ids: vec![BOT_ROLE],
        }),
    }
}

fn plan_with(yaml: &str, live: &LiveState, lock: &Lockfile, prune: bool) -> Plan {
    let parsed = spec::parse(yaml).unwrap_or_else(|e| panic!("{e}"));
    let (desired, diags) = spec::expand(&parsed);
    assert!(!crate::diag::has_errors(&diags), "{diags:#?}");
    plan(&desired, live, lock, &PlanOptions { prune })
}

fn plan_of(yaml: &str, live: &LiveState) -> Plan {
    plan_with(yaml, live, &Lockfile::default(), false)
}

fn errors(p: &Plan) -> Vec<String> {
    p.diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect()
}

fn only_op(p: &Plan) -> &Op {
    assert_eq!(p.ops.len(), 1, "expected one op, got {:#?}", p.ops);
    &p.ops[0]
}

// -- roles ------------------------------------------------------------------

#[test]
fn matching_role_has_no_changes() {
    let p = plan_of("roles:\n  - { key: lead, name: Lead }\n", &server());
    // Adopted by name: recorded, but nothing changes on Discord.
    assert!(matches!(only_op(&p), Op::Adopt { id: LEAD, .. }));

    let mut lock = Lockfile::default();
    lock.roles.insert("lead".into(), Sf(LEAD));
    let p = plan_with(
        "roles:\n  - { key: lead, name: Lead }\n",
        &server(),
        &lock,
        false,
    );
    assert!(p.ops.is_empty(), "{:#?}", p.ops);
}

#[test]
fn role_updates_show_each_field() {
    let mut lock = Lockfile::default();
    lock.roles.insert("lead".into(), Sf(LEAD));
    let yaml = r##"roles:
  - { key: lead, name: Team Lead, color: "#e91e63", hoist: true, permissions: [MANAGE_THREADS] }
"##;
    let p = plan_with(yaml, &server(), &lock, false);
    let Op::UpdateRole(u) = only_op(&p) else {
        panic!()
    };
    assert_eq!(
        u.changes,
        vec![
            RoleField::Name(Change {
                before: "Lead".into(),
                after: "Team Lead".into()
            }),
            RoleField::Color(Change {
                before: 0,
                after: 0xe91e63
            }),
            RoleField::Hoist(Change {
                before: false,
                after: true
            }),
            RoleField::Permissions(Change {
                before: Perms::NONE,
                after: Perms::MANAGE_THREADS
            }),
        ]
    );
}

#[test]
fn new_role_is_created_and_placed() {
    let yaml = "roles:\n  - { key: director, name: Director }\n  - { key: lead, name: Lead }\n";
    let p = plan_of(yaml, &server());
    assert!(matches!(&p.ops[1], Op::CreateRole(c) if c.name == "Director"));
    let Op::ReorderRoles(r) = &p.ops[2] else {
        panic!("{:#?}", p.ops)
    };
    assert_eq!(r.after, ["Concord", "Director", "Lead"]);
    // The bot's own role is never sent; the rest count up from the bottom.
    assert_eq!(
        r.positions,
        vec![(Ref::New("director".into()), 2), (Ref::Id(LEAD), 1)]
    );
}

#[test]
fn recreates_when_the_locked_role_was_deleted() {
    let mut lock = Lockfile::default();
    lock.roles.insert("mod".into(), Sf(555));
    let p = plan_with(
        "roles:\n  - { key: mod, name: Mod }\n",
        &server(),
        &lock,
        false,
    );
    assert!(matches!(&p.ops[0], Op::CreateRole(c) if c.replaces == Some(555)));
    assert!(p.diagnostics[0].message.contains("no longer exists"));
}

#[test]
fn editing_a_role_above_the_bot_is_an_error_with_a_fix() {
    let mut live = server();
    live.roles[2].position = 9; // Lead above Concord
    let p = plan_of(
        "roles:\n  - { key: lead, name: Lead, hoist: true }\n",
        &live,
    );
    let err = p
        .diagnostics
        .iter()
        .find(|d| d.is_error())
        .expect("an error");
    assert!(
        err.message
            .contains("isn't below concord's highest role `Concord`"),
        "{}",
        err.message
    );
    assert_eq!(
        err.hint.as_deref(),
        Some("in Server Settings → Roles, drag `Concord` above `Lead`")
    );
}

#[test]
fn granting_permissions_the_bot_lacks_is_an_error() {
    let mut live = server();
    live.roles[1].permissions = Perms::MANAGE_ROLES.union(Perms::MANAGE_CHANNELS);
    let p = plan_of(
        "roles:\n  - { key: lead, name: Lead, permissions: [BAN_MEMBERS] }\n",
        &live,
    );
    assert_eq!(
        errors(&p),
        ["can't give `Lead` BAN_MEMBERS: concord's bot doesn't have it"]
    );
}

#[test]
fn integration_roles_are_not_listed_as_unmanaged() {
    let p = plan_of("roles: []\n", &server());
    let names: Vec<&str> = p.unmanaged.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["Lead"]);
}

// -- channels ---------------------------------------------------------------

#[test]
fn renaming_a_channel_keeps_its_identity() {
    let mut live = server();
    live.channels
        .push(chan(20, ChannelKind::Text, "systems-general", None, 0));
    let mut lock = Lockfile::default();
    lock.channels.insert("systems-main".into(), Sf(20));
    let p = plan_with(
        "channels:\n  - { key: systems-main, type: text, name: systems-main }\n",
        &live,
        &lock,
        false,
    );
    let Op::UpdateChannel(u) = only_op(&p) else {
        panic!()
    };
    assert_eq!(
        u.changes,
        vec![ChannelField::Name(Change {
            before: "systems-general".into(),
            after: "systems-main".into()
        })]
    );
}

#[test]
fn adoption_requires_same_name_type_and_parent() {
    let mut live = server();
    live.channels
        .push(chan(30, ChannelKind::Category, "eng", None, 0));
    live.channels
        .push(chan(31, ChannelKind::Text, "chat", Some(30), 0));
    live.channels
        .push(chan(32, ChannelKind::Voice, "chat-voice", Some(30), 1));
    let yaml = r#"
categories:
  eng:
    name: eng
    channels:
      - { key: chat, type: text, name: chat }
      - { key: other, type: text, name: chat-voice }
"#;
    let p = plan_of(yaml, &live);
    let adopted: Vec<u64> = p
        .ops
        .iter()
        .filter_map(|o| match o {
            Op::Adopt { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(adopted, [30, 31]);
    // `chat-voice` is voice live but text in the spec: a new channel.
    assert!(p
        .ops
        .iter()
        .any(|o| matches!(o, Op::CreateChannel(c) if c.key == "other")));
}

#[test]
fn changing_type_in_place_is_rejected() {
    let mut live = server();
    live.channels
        .push(chan(20, ChannelKind::Text, "ideas", None, 0));
    let mut lock = Lockfile::default();
    lock.channels.insert("ideas".into(), Sf(20));
    let p = plan_with(
        "channels:\n  - { key: ideas, type: forum, name: ideas }\n",
        &live,
        &lock,
        false,
    );
    assert!(errors(&p)[0].contains("can't change that in place"));
}

#[test]
fn text_can_become_an_announcement_channel() {
    let mut live = server();
    live.channels
        .push(chan(20, ChannelKind::Text, "news", None, 0));
    let p = plan_of(
        "channels:\n  - { key: news, type: announcement, name: news }\n",
        &live,
    );
    assert!(p.ops.iter().any(|o| matches!(o, Op::UpdateChannel(u)
        if u.changes == vec![ChannelField::Kind(Change { before: ChannelKind::Text, after: ChannelKind::Announcement })])));
}

#[test]
fn empty_and_missing_topics_are_equal() {
    let mut live = server();
    live.channels
        .push(chan(20, ChannelKind::Text, "a", None, 0));
    let yaml = "concord: { mode: authoritative }\nchannels:\n  - { key: a, type: text, name: a }\n";
    let p = plan_of(yaml, &live);
    assert!(matches!(only_op(&p), Op::Adopt { .. }), "{:#?}", p.ops);
}

// -- forum tags -------------------------------------------------------------

fn forum_server(tags: Vec<LiveTag>) -> (LiveState, Lockfile) {
    let mut live = server();
    let mut f = chan(40, ChannelKind::Forum, "help", None, 0);
    f.forum.tags = tags;
    live.channels.push(f);
    let mut lock = Lockfile::default();
    lock.channels.insert("help".into(), Sf(40));
    (live, lock)
}

#[test]
fn forum_tag_updates_send_existing_tag_ids_back() {
    // Discord replaces the whole tag list. A tag sent without its ID is
    // treated as new, and the old one is stripped from every post.
    let (live, lock) = forum_server(vec![
        tag(100, "Question"),
        tag(101, "Bug"),
        tag(102, "Legacy"),
    ]);
    let yaml = r#"
channels:
  - { key: help, type: forum, name: help, tags: [Bug, Question, { name: Resolved, moderated: true }] }
"#;
    let p = plan_with(yaml, &live, &lock, false);
    let Op::SetForumTags(t) = only_op(&p) else {
        panic!("{:#?}", p.ops)
    };
    let sent: Vec<(Option<u64>, &str)> = t.after.iter().map(|t| (t.id, t.name.as_str())).collect();
    assert_eq!(
        sent,
        [
            (Some(101), "Bug"),
            (Some(100), "Question"),
            (None, "Resolved"),
            // Not in the spec: kept, with its ID, unless --prune.
            (Some(102), "Legacy"),
        ]
    );
    assert_eq!(
        t.changes,
        [TagChange::Add("Resolved".into()), TagChange::Reorder]
    );
    assert!(p.unmanaged.iter().any(|u| u.name == "Legacy" && !u.pruned));
}

#[test]
fn forum_tags_can_be_renamed_by_key() {
    let (live, mut lock) = forum_server(vec![tag(100, "WIP")]);
    lock.tags.insert("help/wip".into(), Sf(100));
    let yaml = r#"
channels:
  - { key: help, type: forum, name: help, tags: [{ key: wip, name: In Progress }] }
"#;
    let p = plan_with(yaml, &live, &lock, false);
    let Op::SetForumTags(t) = only_op(&p) else {
        panic!()
    };
    assert_eq!(t.after[0].id, Some(100));
    assert_eq!(
        t.changes,
        [TagChange::Rename(Change {
            before: "WIP".into(),
            after: "In Progress".into()
        })]
    );
}

#[test]
fn forum_tags_prune_removes_unlisted_tags() {
    let (live, lock) = forum_server(vec![tag(100, "Question"), tag(102, "Legacy")]);
    let yaml = "channels:\n  - { key: help, type: forum, name: help, tags: [Question] }\n";
    let p = plan_with(yaml, &live, &lock, true);
    let t = p
        .ops
        .iter()
        .find_map(|o| match o {
            Op::SetForumTags(t) => Some(t),
            _ => None,
        })
        .expect("a tag update");
    assert_eq!(t.after.len(), 1);
    assert_eq!(t.changes, [TagChange::Remove("Legacy".into())]);
}

#[test]
fn moderated_and_emoji_changes_are_detected_with_normalised_emoji() {
    let mut t = tag(100, "Done");
    t.emoji = Some(Emoji::unicode("✅"));
    let (live, lock) = forum_server(vec![t]);
    // Same emoji with a variation selector: not a change. Moderated is.
    let yaml = "channels:\n  - { key: help, type: forum, name: help, tags: [{ name: Done, emoji: \"✅\u{FE0F}\", moderated: true }] }\n";
    let p = plan_with(yaml, &live, &lock, false);
    let Op::SetForumTags(t) = only_op(&p) else {
        panic!()
    };
    assert_eq!(
        t.changes,
        [TagChange::Moderated {
            name: "Done".into(),
            change: Change {
                before: false,
                after: true
            }
        }]
    );
}

#[test]
fn too_many_tags_including_unmanaged_ones_is_an_error() {
    let live_tags = (0..15).map(|i| tag(100 + i, &format!("Old{i}"))).collect();
    let (live, lock) = forum_server(live_tags);
    let yaml = "channels:\n  - { key: help, type: forum, name: help, tags: [A, B, C, D, E, F] }\n";
    let p = plan_with(yaml, &live, &lock, false);
    assert!(
        errors(&p)[0].contains("would have 21 tags"),
        "{:?}",
        errors(&p)
    );
}

// -- permission overwrites --------------------------------------------------

fn private_category(child_overwrites: Vec<Overwrite>) -> (LiveState, Lockfile) {
    let mut live = server();
    let private = vec![
        ow(GUILD, Perms::NONE, Perms::VIEW_CHANNEL),
        ow(LEAD, Perms::VIEW_CHANNEL, Perms::NONE),
    ];
    let mut cat = chan(50, ChannelKind::Category, "leads", None, 0);
    cat.overwrites = private;
    let mut child = chan(51, ChannelKind::Text, "leads-chat", Some(50), 0);
    child.overwrites = child_overwrites;
    live.channels.extend([cat, child]);
    let mut lock = Lockfile::default();
    lock.channels.insert("leads".into(), Sf(50));
    lock.channels.insert("leads-chat".into(), Sf(51));
    (live, lock)
}

const LEADS_SPEC: &str = r#"
categories:
  leads:
    name: leads
    private: [Lead]
    channels:
      - { key: leads-chat, type: text, name: leads-chat }
"#;

#[test]
fn synced_channels_compare_effective_overwrites() {
    // Same overwrites in a different order: no diff.
    let (live, lock) = private_category(vec![
        ow(LEAD, Perms::VIEW_CHANNEL, Perms::NONE),
        ow(GUILD, Perms::NONE, Perms::VIEW_CHANNEL),
        ow(77, Perms::NONE, Perms::NONE), // empty overwrite: ignored
    ]);
    let p = plan_with(LEADS_SPEC, &live, &lock, false);
    assert!(p.ops.is_empty(), "{:#?}", p.ops);
}

#[test]
fn out_of_sync_channel_is_resynced_and_member_overwrites_kept() {
    let member = Overwrite {
        id: 4242,
        kind: OverwriteKind::Member,
        allow: Perms::SEND_MESSAGES,
        deny: Perms::NONE,
    };
    let (live, lock) = private_category(vec![
        ow(GUILD, Perms::NONE, Perms::VIEW_CHANNEL),
        member.clone(),
    ]);
    let p = plan_with(LEADS_SPEC, &live, &lock, false);
    let Op::SetPermissionOverwrites(s) = only_op(&p) else {
        panic!("{:#?}", p.ops)
    };
    assert_eq!(s.synced_with.as_deref(), Some("leads"));
    assert_eq!(s.after.len(), 2);
    assert_eq!(s.members, [member]);
}

#[test]
fn channel_overrides_layer_on_top_of_the_category() {
    let (live, lock) = private_category(vec![]);
    let yaml = r#"
categories:
  leads:
    name: leads
    private: [Lead]
    channels:
      - { key: leads-chat, type: text, name: leads-chat,
          overrides: { Lead: { allow: [MANAGE_MESSAGES] } } }
"#;
    let p = plan_with(yaml, &live, &lock, false);
    let Op::SetPermissionOverwrites(s) = only_op(&p) else {
        panic!()
    };
    let lead = s.after.iter().find(|o| o.target == Ref::Id(LEAD)).unwrap();
    assert_eq!(lead.allow, Perms::VIEW_CHANNEL.union(Perms(1 << 13)));
    // Still private: @everyone's deny comes from the category.
    assert!(s
        .after
        .iter()
        .any(|o| o.target == Ref::Id(GUILD) && o.deny == Perms::VIEW_CHANNEL));
}

#[test]
fn partial_mode_inherits_an_unmanaged_categorys_live_permissions() {
    let (live, lock) = private_category(vec![
        ow(GUILD, Perms::NONE, Perms::VIEW_CHANNEL),
        ow(LEAD, Perms::VIEW_CHANNEL, Perms::NONE),
    ]);
    // The category declares no permissions; the child is synced to whatever
    // the category has.
    let yaml = "categories:\n  leads:\n    name: leads\n    channels:\n      - { key: leads-chat, type: text, name: leads-chat }\n";
    let p = plan_with(yaml, &live, &lock, false);
    assert!(p.ops.is_empty(), "{:#?}", p.ops);
}

#[test]
fn unknown_role_in_overrides_is_an_error_with_suggestion() {
    let yaml =
        "channels:\n  - { key: a, type: text, name: a, overrides: { Laed: { allow: [VIEW] } } }\n";
    let p = plan_of(yaml, &server());
    let err = p.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert_eq!(err.message, "no role called `Laed`");
    assert_eq!(err.hint.as_deref(), Some("did you mean `Lead`?"));
}

// -- ordering ---------------------------------------------------------------

fn three_channels(positions: [i64; 3]) -> (LiveState, Lockfile) {
    let mut live = server();
    let mut lock = Lockfile::default();
    for (i, (name, pos)) in ["a", "b", "c"].iter().zip(positions).enumerate() {
        let id = 60 + i as u64;
        live.channels
            .push(chan(id, ChannelKind::Text, name, None, pos));
        lock.channels.insert((*name).into(), Sf(id));
    }
    (live, lock)
}

const ABC: &str = "channels:\n  - { key: a, type: text, name: a }\n  - { key: b, type: text, name: b }\n  - { key: c, type: text, name: c }\n";

#[test]
fn no_reorder_when_effective_order_matches() {
    // Messy positions (gaps, a tie broken by ID) but the right order.
    let (live, lock) = three_channels([3, 9, 9]);
    let p = plan_with(ABC, &live, &lock, false);
    assert!(p.ops.is_empty(), "{:#?}", p.ops);
}

#[test]
fn reorder_when_effective_order_differs() {
    let (live, lock) = three_channels([2, 1, 0]);
    let p = plan_with(ABC, &live, &lock, false);
    let Op::ReorderChannels(groups) = only_op(&p) else {
        panic!()
    };
    assert_eq!(groups[0].before, ["c", "b", "a"]);
    assert_eq!(groups[0].after, ["a", "b", "c"]);
}

#[test]
fn unmanaged_channels_keep_their_place() {
    let (mut live, lock) = three_channels([0, 2, 3]);
    live.channels
        .push(chan(70, ChannelKind::Text, "x", None, 1)); // between a and b
    let yaml = "channels:\n  - { key: b, type: text, name: b }\n  - { key: a, type: text, name: a }\n  - { key: c, type: text, name: c }\n";
    let p = plan_with(yaml, &live, &lock, false);
    let Op::ReorderChannels(groups) = only_op(&p) else {
        panic!()
    };
    assert_eq!(groups[0].after, ["b", "a", "x", "c"]);
}

#[test]
fn voice_channels_sort_below_text_regardless_of_position() {
    let mut live = server();
    live.channels
        .push(chan(80, ChannelKind::Voice, "Lounge", None, 0));
    live.channels
        .push(chan(81, ChannelKind::Text, "chat", None, 5));
    let mut lock = Lockfile::default();
    lock.channels.insert("lounge".into(), Sf(80));
    lock.channels.insert("chat".into(), Sf(81));
    let yaml = "channels:\n  - { key: chat, type: text, name: chat }\n  - { key: lounge, type: voice, name: Lounge }\n";
    let p = plan_with(yaml, &live, &lock, false);
    assert!(p.ops.is_empty(), "{:#?}", p.ops);
}

#[test]
fn channels_in_a_new_category_are_created_in_place() {
    let yaml = "categories:\n  new:\n    name: new\n    channels:\n      - { key: x, type: text, name: x }\n      - { key: y, type: text, name: y }\n";
    let p = plan_of(yaml, &server());
    assert!(
        !p.ops.iter().any(|o| matches!(o, Op::ReorderChannels(_))),
        "{:#?}",
        p.ops
    );
    let positions: Vec<Option<u64>> = p
        .ops
        .iter()
        .filter_map(|o| match o {
            Op::CreateChannel(c) if c.kind == ChannelKind::Text => Some(c.position),
            _ => None,
        })
        .collect();
    assert_eq!(positions, [Some(0), Some(1)]);
}

// -- unmanaged and prune ----------------------------------------------------

#[test]
fn unmanaged_things_are_listed_and_only_pruned_on_request() {
    let mut live = server();
    live.channels
        .push(chan(90, ChannelKind::Text, "random", None, 0));
    live.channels
        .push(chan(91, ChannelKind::Text, "rules", None, 1));
    live.guild.protected_channels = vec![(91, "the server's rules channel")];

    let p = plan_of("roles: []\n", &live);
    assert!(p.ops.is_empty());
    assert_eq!(p.unmanaged.len(), 3); // Lead, random, rules

    let p = plan_with("roles: []\n", &live, &Lockfile::default(), true);
    let deleted: Vec<&str> = p
        .ops
        .iter()
        .filter_map(|o| match o {
            Op::Delete { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deleted, ["random", "Lead"]);
    let rules = p.unmanaged.iter().find(|u| u.name == "rules").unwrap();
    assert!(!rules.pruned);
    assert!(rules
        .note
        .as_deref()
        .unwrap()
        .contains("won't let it be deleted"));
}

#[test]
fn lockfile_for_another_server_is_an_error() {
    let lock = Lockfile {
        guild_id: Some(Sf(12345)),
        ..Default::default()
    };
    let p = plan_with("roles: []\n", &server(), &lock, false);
    assert!(errors(&p)[0].contains("the lockfile is for server 12345"));
}
