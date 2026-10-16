//! Server-wide sections: validation, planning against small hand-written
//! servers, applying to the in-memory Discord, and its rules.

mod common;

use concord::apply::Applier;
use concord::community::diff::{CommunityOp, LineTone};
use concord::diag::Diagnostic;
use concord::diff::{plan, LockSection, Op, Plan, PlanOptions};
use concord::live::memory::MemoryApi;
use concord::live::{to_live, ApiError, DiscordApi, Endpoint, Snapshot, WriteRequest};
use concord::lock::Lockfile;
use concord::model::Desired;
use concord::spec;
use serde_json::{json, Value};

// -- helpers -------------------------------------------------------------------

/// A Community server: `rules`, `general`, `mods` and a `chats` category of
/// five channels (seven channels in all, as onboarding wants), a voice
/// channel, and two roles the spec doesn't define (`Mod`, `Member`).
fn snapshot(community: Value) -> Snapshot {
    let text = |id: &str, name: &str, parent: Option<&str>| json!({ "id": id, "type": 0, "name": name, "position": 0, "parent_id": parent });
    let mut channels = vec![
        text("21", "rules", None),
        text("22", "general", None),
        text("23", "mods", None),
        json!({ "id": "29", "type": 2, "name": "Lounge", "position": 0 }),
        json!({ "id": "30", "type": 4, "name": "chats", "position": 0 }),
    ];
    for i in 1..=5 {
        channels.push(text(
            &format!("2{}", i + 3),
            &format!("chat-{i}"),
            Some("30"),
        ));
    }
    serde_json::from_value(json!({
        "guild": {
            "id": "1",
            "name": "Test",
            "features": ["COMMUNITY"],
            "rules_channel_id": "21",
            "public_updates_channel_id": "23",
        },
        "roles": [
            { "id": "1", "name": "@everyone", "permissions": "0", "position": 0 },
            { "id": "10", "name": "Mod", "permissions": "0", "position": 2 },
            { "id": "11", "name": "Member", "permissions": "0", "position": 1 },
            { "id": "12", "name": "Concord", "permissions": "8", "position": 3, "managed": true },
        ],
        "channels": channels,
        "bot": { "user": { "id": "2", "username": "concord" }, "member": { "roles": ["12"] } },
        "community": community,
    }))
    .unwrap()
}

/// Every section readable and empty, with `patch`'s sections instead.
fn community(patch: Value) -> Value {
    let mut c = json!({
        "onboarding": { "prompts": [], "default_channel_ids": [], "enabled": false, "mode": 0 },
        "welcome_screen": { "description": null, "welcome_channels": [], "enabled": false },
        "guide": { "enabled": false, "new_member_actions": [], "resource_channels": [] },
        "verification": { "version": "1", "form_fields": [] },
        "automod": [],
        "emojis": [],
        "stickers": [],
        "sounds": [],
        "widget": { "enabled": false, "channel_id": null },
    });
    for (k, v) in patch.as_object().unwrap() {
        c[k] = v.clone();
    }
    c
}

/// Enabled onboarding with the spec's default channels and these prompts.
fn onboarding(prompts: Value) -> Value {
    json!({
        "enabled": true,
        "mode": 0,
        "default_channel_ids": ["21", "22", "30"],
        "prompts": prompts,
    })
}

const CHANNELS: &str = r#"
guild_id: "1"
channels:
  - { key: rules, type: text, name: rules }
  - { key: general, type: text, name: general }
  - { key: mods, type: text, name: mods }
  - { key: lounge, type: voice, name: Lounge }
categories:
  chats:
    name: chats
    channels:
      - { key: chat-1, type: text, name: chat-1 }
      - { key: chat-2, type: text, name: chat-2 }
      - { key: chat-3, type: text, name: chat-3 }
      - { key: chat-4, type: text, name: chat-4 }
      - { key: chat-5, type: text, name: chat-5 }
"#;

const ONBOARDING: &str = "
onboarding:
  default_channels: [rules, general, chats]
  prompts:
";

fn spec_with(sections: &str) -> String {
    format!("{CHANNELS}{sections}")
}

/// Expand, reading files from `examples/` (where `assets/chequered.png` is).
fn expand(yaml: &str) -> (Desired, Vec<Diagnostic>) {
    let parsed = spec::parse(yaml).unwrap_or_else(|e| panic!("{e}\n---\n{yaml}"));
    spec::expand_at(&parsed, &common::root().join("examples"))
}

fn desired(yaml: &str) -> Desired {
    let (d, diags) = expand(yaml);
    assert!(!concord::diag::has_errors(&diags), "{diags:#?}");
    d
}

fn errors(diags: &[Diagnostic]) -> Vec<String> {
    diags
        .iter()
        .filter(|d| d.is_error())
        .map(|d| format!("{}: {}", d.path, d.message))
        .collect()
}

fn diag<'a>(diags: &'a [Diagnostic], path: &str) -> &'a Diagnostic {
    diags
        .iter()
        .find(|d| d.path == path)
        .unwrap_or_else(|| panic!("no diagnostic at {path}: {diags:#?}"))
}

fn lock(v: Value) -> Lockfile {
    let mut v = v;
    v["version"] = json!(1);
    serde_json::from_value(v).unwrap()
}

fn plan_of(yaml: &str, snap: &Snapshot, lock: &Lockfile) -> Plan {
    let p = plan(
        &desired(yaml),
        &to_live(snap),
        lock,
        &PlanOptions::default(),
    );
    assert!(!p.has_errors(), "{:#?}", p.diagnostics);
    p
}

fn community_ops(p: &Plan) -> Vec<&CommunityOp> {
    p.ops
        .iter()
        .filter_map(|o| match o {
            Op::Community(c) => Some(c),
            _ => None,
        })
        .collect()
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

/// Plan, apply to an in-memory copy of `snap`, and check a second plan is
/// empty. Returns the server and lockfile afterwards.
fn converge(yaml: &str, snap: Snapshot, lock: &Lockfile) -> (MemoryApi, Lockfile) {
    let d = desired(yaml);
    let api = MemoryApi::new(snap);
    let p = plan(&d, &to_live(&api.state()), lock, &PlanOptions::default());
    assert!(!p.has_errors(), "{:#?}", p.diagnostics);
    let mut applier = Applier::new(&p);
    block_on(applier.run_all(&api, &p.ops, |_, _, _| {}))
        .unwrap_or_else(|(i, e)| panic!("op {i} ({:?}) failed: {e}", p.ops[i]));
    let lock = applier.lock().clone();
    let again = plan(&d, &to_live(&api.state()), &lock, &PlanOptions::default());
    assert!(again.ops.is_empty(), "still to do: {:#?}", again.ops);
    (api, lock)
}

fn write(api: &MemoryApi, endpoint: Endpoint, body: Value) -> Result<Value, ApiError> {
    let req = WriteRequest {
        endpoint,
        body: Some(body),
        file: None,
    };
    block_on(api.write(1, &req))
}

fn is_400(r: &Result<Value, ApiError>) -> bool {
    matches!(r, Err(ApiError::Status { status: 400, .. }))
}

// -- validation ------------------------------------------------------------------

#[test]
fn unknown_channel_keys_suggest_the_right_one() {
    let (_, diags) = expand(&spec_with(
        "
welcome_screen:
  channels:
    - { channel: rulez, description: Read these }
    - { channel: Lounge, description: Hang out }
",
    ));
    let d = diag(&diags, "welcome_screen.channels[0].channel");
    assert_eq!(d.message, "no channel or category with key `rulez`");
    assert!(d.hint.as_deref().unwrap().contains("rules"), "{d:?}");
    // A channel's name instead of its key.
    let d = diag(&diags, "welcome_screen.channels[1].channel");
    assert_eq!(
        d.hint.as_deref(),
        Some("channels are referred to by key; `Lounge` is the name of `lounge`")
    );
}

#[test]
fn automod_rules_are_checked_for_their_trigger() {
    let (_, diags) = expand(&spec_with(
        "
automod:
  - { key: a, name: A, trigger: mention_spam, keywords: [x], actions: [{ block: true }] }
  - { key: b, name: B, trigger: keyword, actions: [{ block: true, alert: mods }] }
  - { key: c, name: C, trigger: keyword_preset, presets: [slurs], actions: [{ timeout: 1m }] }
  - { key: d, name: D, trigger: spam, actions: [] }
  - { key: d, name: E, trigger: spam, actions: [{ block_interactions: true }] }
  - { key: f, name: F, trigger: keyword, keywords: [x], actions: [{ alert: modz }] }
",
    ));
    let errs = errors(&diags);
    for want in [
        "automod[0].keywords: `keywords` doesn't apply to `mention_spam` rules",
        "automod[0].mention_limit: needs `mention_limit` between 1 and 50",
        "automod[1]: needs `keywords` or `regex`",
        "automod[1].actions[0]: each action is exactly one of `block`, `alert`, `timeout`, `block_interactions`",
        "automod[2].actions[0]: `timeout` only works for keyword, mention_spam and member_profile rules",
        "automod[3].actions: a rule needs at least one action",
        "automod[4].key: duplicate key `d`",
        "automod[4].actions[0]: `block_interactions` only works for member_profile rules",
        "automod: Discord allows at most 1 `spam` rule(s)",
        "automod[5].actions[0].alert: no channel or category with key `modz`",
    ] {
        assert!(errs.iter().any(|e| e == want), "missing `{want}` in {errs:#?}");
    }
}

#[test]
fn automod_values_are_parsed_strictly() {
    let err = spec::parse(&spec_with(
        "automod:\n  - { key: a, name: A, trigger: keyword, keywords: [x], actions: [{ block: false }] }\n",
    ))
    .unwrap_err();
    assert!(err.message.contains("leave the action out"), "{err}");
    let err = spec::parse(&spec_with(
        "automod:\n  - { key: a, name: A, trigger: mentions, actions: [{ block: true }] }\n",
    ))
    .unwrap_err();
    assert!(err.message.contains("`mention_spam`"), "{err}");
    // Spellings are forgiving.
    let parsed = spec::parse(&spec_with(
        "automod:\n  - { key: a, name: A, trigger: Keyword-Preset, presets: [Sexual Content], actions: [{ block: true }] }\n",
    ))
    .unwrap();
    let automod = parsed.automod.unwrap();
    let rule = &automod[0];
    assert_eq!(rule.trigger.as_str(), "keyword_preset");
    assert_eq!(rule.presets[0].as_str(), "sexual_content");
}

/// The base spec with more `categories` (made from the `team` template,
/// whose option goes in the prompt named by its `prompt` variable), then
/// `rest`.
fn with_teams(categories: &str, rest: &str) -> String {
    format!("{CHANNELS}{categories}{TEAM_TEMPLATE}{rest}")
}

const PROMPT_TEAMS: &str =
    "    - { key: teams, title: Which teams?, options: [{ title: None, channels: [general] }] }\n";

const TEAM_TEMPLATE: &str = r#"
templates:
  team:
    channels: [{ key: chat, type: text, name: "{key}-chat" }]
    onboarding_option: { prompt: "{prompt}", title: "{key}", channels: ["{key}"], emoji: "🏁" }
"#;

#[test]
fn template_options_need_an_onboarding_section() {
    let yaml = with_teams(
        "  aero: { template: team, name: aero, vars: { prompt: teams } }\n",
        "",
    );
    let (_, diags) = expand(&yaml);
    let d = diag(
        &diags,
        "templates.team.onboarding_option (via categories.aero)",
    );
    assert!(d.is_error());
    assert!(d.message.contains("no `onboarding:` section"), "{d:?}");
}

#[test]
fn template_options_land_in_their_prompt_with_variables_substituted() {
    let yaml = with_teams(
        "  aero: { template: team, name: aero, vars: { prompt: teams } }
  vd:
    template: team
    name: vd
    onboarding_option: { prompt: teams, title: Vehicle dynamics, roles: [Member] }
",
        &format!("{ONBOARDING}{PROMPT_TEAMS}"),
    );
    let d = desired(&yaml);
    let o = d.community.onboarding.unwrap();
    let p = &o.prompts[0];
    let titles: Vec<&str> = p.options.iter().map(|x| x.title.as_str()).collect();
    assert_eq!(titles, ["None", "aero", "Vehicle dynamics"]);
    assert_eq!(p.options[1].channels, ["aero"]);
    assert_eq!(p.options[1].key, "aero");
    // A category's own option replaces the template's.
    assert_eq!(p.options[2].roles, ["Member"]);
    assert!(p.options[2].channels.is_empty());
}

#[test]
fn a_misspelt_prompt_key_is_reported_with_a_suggestion() {
    let yaml = with_teams(
        "  aero: { template: team, name: aero, vars: { prompt: tems } }\n",
        &format!("{ONBOARDING}{PROMPT_TEAMS}"),
    );
    let (_, diags) = expand(&yaml);
    let d = diag(
        &diags,
        "templates.team.onboarding_option (via categories.aero).prompt",
    );
    assert_eq!(d.message, "no onboarding prompt with key `tems`");
    assert!(d.hint.as_deref().unwrap().contains("teams"), "{d:?}");
}

#[test]
fn onboarding_problems_are_reported() {
    let (_, diags) = expand(&spec_with(
        "
onboarding:
  default_channels: [rules, general]
  prompts:
    - title: Who are you?
      options:
        - { title: Nothing }
        - { title: Everyone, roles: ['@everyone'] }
    - { title: Who are you?, options: [{ title: x, channels: [general] }] }
    - { title: Empty }
",
    ));
    let errs = errors(&diags);
    for want in [
        "onboarding.prompts[0].options[0]: an option must give at least one role or channel",
        "onboarding.prompts[0].options[1].roles[0]: options can't give @everyone",
        "onboarding.prompts[1]: two prompts have the key `who-are-you`",
        "onboarding.prompts[2]: a prompt needs at least one option",
    ] {
        assert!(
            errs.iter().any(|e| e == want),
            "missing `{want}` in {errs:#?}"
        );
    }
    // Too few default channels is only a warning: Discord has the last word.
    let d = diag(&diags, "onboarding.default_channels");
    assert!(!d.is_error());
    assert!(d.message.starts_with("2 default channel(s)"), "{d:?}");
}

#[test]
fn guide_join_questions_and_settings_are_checked() {
    let (_, diags) = expand(&spec_with(
        "
server:
  afk_channel: general
  system_channel: lounge
  afk_timeout: 7m
guide:
  actions:
    - { title: Read, channel: rules }
join_questions:
  questions:
    - { label: Year?, type: choice }
    - { label: Name?, choices: [a] }
",
    ));
    let errs = errors(&diags);
    for want in [
        "server.afk_channel: the AFK channel must be a voice channel",
        "server.system_channel: must be a text channel",
        "server.afk_timeout: Discord allows 1m, 5m, 15m, 30m or 1h",
        "join_questions.questions[0].choices: a `choice` question needs `choices`",
        "join_questions.questions[1].choices: only `type: choice` has choices",
    ] {
        assert!(
            errs.iter().any(|e| e == want),
            "missing `{want}` in {errs:#?}"
        );
    }
    let d = diag(&diags, "guide.actions");
    assert!(!d.is_error(), "{d:?}");
}

#[test]
fn expression_files_and_names_are_checked() {
    let (_, diags) = expand(&spec_with(
        "
emoji:
  - { key: a, name: a, file: assets/chequered.png }
  - { key: b, name: ok_name, file: assets/missing.png }
  - { key: c, name: ok_name2, file: racing-team.yaml }
stickers:
  - { key: s, name: Box, emoji: ' ' }
sounds:
  - { key: s, name: Rev, volume: 1.5 }
",
    ));
    let errs = errors(&diags);
    assert!(
        errs.contains(
            &"emoji[0].name: emoji names are 2-32 letters, digits or underscores".to_owned()
        ),
        "{errs:#?}"
    );
    assert!(
        errs.iter()
            .any(|e| e.starts_with("emoji[1].file: can't read")),
        "{errs:#?}"
    );
    assert!(
        errs.iter()
            .any(|e| e.starts_with("emoji[2].file: `racing-team.yaml`: expected a .png")),
        "{errs:#?}"
    );
    assert!(
        errs.contains(&"stickers[0].emoji: stickers need a related emoji".to_owned()),
        "{errs:#?}"
    );
    assert!(
        errs.contains(&"sounds[0].volume: volume is between 0.0 and 1.0".to_owned()),
        "{errs:#?}"
    );
}

// -- planning --------------------------------------------------------------------

#[test]
fn reordered_live_lists_are_not_changes() {
    let snap = snapshot(community(json!({
        "onboarding": {
            "enabled": true,
            "mode": 0,
            "default_channel_ids": ["30", "22", "21"],
            "prompts": [{
                "id": "100", "type": 0, "title": "Who are you?", "in_onboarding": true,
                "options": [{
                    "id": "110", "title": "Staff", "description": null,
                    "role_ids": ["11", "10"], "channel_ids": ["23", "22"],
                }],
            }],
        },
        "automod": [
            {
                "id": "400", "name": "Words", "event_type": 1, "trigger_type": 1,
                "trigger_metadata": {
                    "keyword_filter": ["b", "a"], "regex_patterns": ["y+", "x+"], "allow_list": ["d", "c"],
                },
                "actions": [
                    { "type": 2, "metadata": { "channel_id": "23" } },
                    { "type": 1, "metadata": {} },
                ],
                "enabled": true, "exempt_roles": ["11", "10"], "exempt_channels": ["23", "22"],
            },
            {
                "id": "401", "name": "Presets", "event_type": 1, "trigger_type": 4,
                "trigger_metadata": { "presets": [3, 1, 2] },
                "actions": [{ "type": 1, "metadata": {} }], "enabled": true,
            },
            {
                // Discord leaves raid protection out when it's off.
                "id": "402", "name": "Mentions", "event_type": 1, "trigger_type": 5,
                "trigger_metadata": { "mention_total_limit": 5 },
                "actions": [{ "type": 1, "metadata": {} }], "enabled": true,
            },
        ],
        "emojis": [{ "id": "300", "name": "flag", "roles": ["11", "10"] }],
    })));
    let yaml = format!(
        "{}{ONBOARDING}    - title: Who are you?\n      options:\n        \
         - {{ title: Staff, roles: [Mod, Member], channels: [general, mods] }}\n{}",
        spec_with(""),
        r#"
automod:
  - key: words
    name: Words
    trigger: keyword
    keywords: [a, b]
    regex: ["x+", "y+"]
    allow: [c, d]
    actions: [{ block: true }, { alert: mods }]
    exempt_roles: [Mod, Member]
    exempt_channels: [general, mods]
  - { key: presets, name: Presets, trigger: keyword_preset, presets: [profanity, sexual_content, slurs], actions: [{ block: true }] }
  - { key: mentions, name: Mentions, trigger: mention_spam, mention_limit: 5, actions: [{ block: true }] }
emoji:
  - { key: flag, name: flag, roles: [Mod, Member] }
"#
    );
    let p = plan_of(&yaml, &snap, &Lockfile::default());
    assert!(community_ops(&p).is_empty(), "{:#?}", community_ops(&p));
}

#[test]
fn prompts_and_options_match_by_lockfile_id_then_title() {
    let snap = snapshot(community(json!({
        "onboarding": onboarding(json!([
            {
                "id": "100", "title": "Old title", "in_onboarding": true,
                "options": [{ "id": "110", "title": "Old option", "channel_ids": ["22"] }],
            },
            {
                "id": "101", "title": "Same title", "in_onboarding": true,
                "options": [{ "id": "111", "title": "x", "channel_ids": ["23"] }],
            },
        ])),
    })));
    let yaml = format!(
        "{}{ONBOARDING}{}",
        spec_with(""),
        "    - key: renamed
      title: New title
      options:
        - { key: opt, title: New option, channels: [general] }
    - title: Same title
      options:
        - { title: x, channels: [mods] }
"
    );
    let locked = lock(json!({ "onboarding": { "renamed": "100", "renamed/opt": "110" } }));

    let p = plan_of(&yaml, &snap, &locked);
    let ops = community_ops(&p);
    let [CommunityOp::Onboarding { after, lines }] = ops[..] else {
        panic!("{ops:#?}")
    };
    assert_eq!(after.prompts[0].id, Some(100));
    assert_eq!(after.prompts[0].options[0].id, Some(110));
    assert_eq!(after.prompts[1].id, Some(101));
    assert_eq!(after.prompts[1].options[0].id, Some(111));
    assert!(
        lines.iter().all(|l| l.tone != LineTone::Remove),
        "{lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.text.contains("\"Old title\" → \"New title\"")),
        "{lines:#?}"
    );
    for (key, id) in [
        ("renamed", 100),
        ("renamed/opt", 110),
        ("same-title", 101),
        ("same-title/x", 111),
    ] {
        assert!(
            p.bindings
                .community
                .contains(&(LockSection::Onboarding, key.to_owned(), id)),
            "{key}: {:#?}",
            p.bindings.community
        );
    }

    // Without the lockfile, the renamed prompt is a new one.
    let p = plan_of(&yaml, &snap, &Lockfile::default());
    let ops = community_ops(&p);
    let [CommunityOp::Onboarding { after, lines }] = ops[..] else {
        panic!("{ops:#?}")
    };
    assert_eq!(after.prompts[0].id, None);
    assert_eq!(after.prompts[1].id, Some(101));
    let shown: Vec<(LineTone, &str)> = lines.iter().map(|l| (l.tone, l.text.as_str())).collect();
    assert!(
        shown.contains(&(LineTone::Add, "prompt \"New title\" (asked on join)")),
        "{shown:#?}"
    );
    assert!(
        shown.contains(&(LineTone::Remove, "prompt \"Old title\"")),
        "{shown:#?}"
    );

    // Applying keeps the matched IDs.
    let (api, lock) = converge(&yaml, snap, &locked);
    let state = api.state().community.unwrap().onboarding.unwrap();
    let ids: Vec<u64> = state.prompts.iter().map(|p| p.id.0).collect();
    assert_eq!(ids, [100, 101]);
    assert_eq!(lock.onboarding["renamed/opt"].0, 110);
}

#[test]
fn a_changed_file_replaces_the_emoji() {
    let snap = snapshot(community(json!({
        "emojis": [{ "id": "300", "name": "flag", "roles": [] }],
    })));
    let yaml = spec_with("emoji:\n  - { key: flag, name: flag, file: assets/chequered.png }\n");

    // Adopted by name: the file is assumed to be the same, and its hash noted.
    let p = plan_of(&yaml, &snap, &Lockfile::default());
    assert!(community_ops(&p).is_empty(), "{:#?}", p.ops);
    let [(key, hash)] = &p.bindings.assets[..] else {
        panic!("{:#?}", p.bindings.assets)
    };
    assert_eq!(key, "emoji/flag");
    assert_eq!(hash.len(), 64);

    let same = lock(json!({ "emoji": { "flag": "300" }, "assets": { "emoji/flag": hash } }));
    assert!(community_ops(&plan_of(&yaml, &snap, &same)).is_empty());

    let old =
        lock(json!({ "emoji": { "flag": "300" }, "assets": { "emoji/flag": "0".repeat(64) } }));
    let p = plan_of(&yaml, &snap, &old);
    match &community_ops(&p)[..] {
        [CommunityOp::DeleteEmoji { id: 300, .. }, CommunityOp::CreateEmoji { key, asset, .. }] => {
            assert_eq!(key, "flag");
            assert_eq!(&asset.sha256, hash);
        }
        other => panic!("{other:#?}"),
    }

    let (api, lock) = converge(&yaml, snap, &old);
    let emojis = api.state().community.unwrap().emojis.unwrap();
    let [e] = &emojis[..] else {
        panic!("{emojis:#?}")
    };
    assert_ne!(e.id.unwrap().0, 300);
    assert_eq!(lock.emoji["flag"], e.id.unwrap());
    assert_eq!(&lock.assets["emoji/flag"], hash);
}

#[test]
fn changing_an_automod_trigger_replaces_the_rule() {
    let snap = snapshot(community(json!({
        "automod": [{
            "id": "400", "name": "Words", "event_type": 1, "trigger_type": 1,
            "trigger_metadata": { "keyword_filter": ["x"] },
            "actions": [{ "type": 1, "metadata": {} }], "enabled": true,
        }],
    })));
    let yaml = spec_with(
        "automod:\n  - { key: words, name: Words, trigger: keyword_preset, presets: [profanity], actions: [{ block: true }] }\n",
    );
    let locked = lock(json!({ "automod": { "words": "400" } }));
    let p = plan_of(&yaml, &snap, &locked);
    match &community_ops(&p)[..] {
        [CommunityOp::DeleteAutomod { id: 400, .. }, CommunityOp::CreateAutomod { rule, .. }] => {
            assert_eq!(rule.trigger, 4);
            assert_eq!(rule.id, None);
        }
        other => panic!("{other:#?}"),
    }

    let (api, lock) = converge(&yaml, snap, &locked);
    let created = api
        .log()
        .into_iter()
        .find(|r| r.endpoint == Endpoint::CreateAutomod)
        .unwrap();
    assert_eq!(created.body.unwrap()["trigger_type"], 4);
    assert_ne!(lock.automod["words"].0, 400);
}

#[test]
fn automod_updates_show_only_real_changes() {
    let snap = snapshot(community(json!({
        "automod": [{
            "id": "400", "name": "Words", "event_type": 1, "trigger_type": 1,
            "trigger_metadata": { "keyword_filter": ["x"] },
            "actions": [
                { "type": 2, "metadata": { "channel_id": "23" } },
                { "type": 1, "metadata": {} },
            ],
            "enabled": true,
        }],
    })));
    let yaml = spec_with(
        "automod:\n  - { key: words, name: Words, trigger: keyword, keywords: [x, y], actions: [{ block: true }, { alert: mods }] }\n",
    );
    let p = plan_of(&yaml, &snap, &Lockfile::default());
    let ops = community_ops(&p);
    let [CommunityOp::UpdateAutomod { id: 400, lines, .. }] = ops[..] else {
        panic!("{ops:#?}")
    };
    let shown: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(shown, ["keyword \"y\""]);
    // Updates never send the trigger type, which Discord can't change.
    let (api, _) = converge(&yaml, snap, &Lockfile::default());
    let update = api
        .log()
        .into_iter()
        .find(|r| r.endpoint == Endpoint::UpdateAutomod(400))
        .unwrap();
    assert!(update.body.unwrap().get("trigger_type").is_none());
}

#[test]
fn reordered_join_rules_are_shown_alongside_other_changes() {
    let snap = snapshot(community(json!({
        "verification": {
            "version": "1",
            "description": "Old",
            "form_fields": [{ "field_type": "TERMS", "label": "Rules", "values": ["a", "b"], "required": true }],
        },
    })));
    let yaml = spec_with("join_questions:\n  description: New\n  rules: [b, a]\n");
    let p = plan_of(&yaml, &snap, &Lockfile::default());
    let ops = community_ops(&p);
    let [CommunityOp::JoinQuestions { version, lines, .. }] = ops[..] else {
        panic!("{ops:#?}")
    };
    assert_eq!(version, "1");
    let shown: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(shown.len(), 2, "{shown:#?}");
    assert!(shown[0].starts_with("description"), "{shown:#?}");
    assert_eq!(shown[1], "rules reordered");
}

#[test]
fn every_section_converges_from_empty() {
    let yaml = format!(
        "{}{ONBOARDING}{}",
        spec_with(
            "
server:
  system_channel: general
  system_messages: { join: false }
  safety_alerts_channel: mods
  afk_channel: lounge
  afk_timeout: 1h
  widget: { enabled: true, channel: general }
welcome_screen:
  description: Hello
  channels:
    - { channel: rules, description: Rules, emoji: '📜' }
guide:
  welcome: { message: 'Hi [@username]', authors: ['2'] }
  actions:
    - { title: Read the rules, channel: rules }
    - { title: Say hi, channel: general, type: chat, emoji: '👋' }
    - { title: Chat, channel: chat-1, type: chat }
  resources:
    - { title: Mods, channel: mods }
join_questions:
  rules: [Be nice]
  questions:
    - { label: Name? }
    - { label: Why?, type: paragraph, required: false, description: Tell us }
    - { label: Year?, type: choice, choices: ['1', '2'] }
automod:
  - { key: k, name: K, trigger: keyword, regex: ['b[a4]d'], actions: [{ timeout: 1m }, { block: Nope }] }
  - { key: s, name: S, trigger: spam, actions: [{ alert: mods }] }
  - { key: p, name: P, trigger: member_profile, keywords: [x], actions: [{ block_interactions: true }] }
emoji:
  - { key: flag, name: flag, file: assets/chequered.png, roles: [Mod] }
"
        ),
        "    - title: Who are you?
      required: true
      single: true
      type: dropdown
      options:
        - { title: Staff, roles: [Mod], channels: [mods], emoji: '🛠️', description: Staff only }
        - { title: Member, roles: [Member] }
    - title: Later
      in_onboarding: false
      options: [{ title: Chats, channels: [chats] }]
"
    );
    converge(&yaml, snapshot(community(json!({}))), &Lockfile::default());
}

#[test]
fn a_guide_without_a_welcome_message_converges() {
    let yaml = spec_with(
        "
guide:
  actions:
    - { title: Read the rules, channel: rules }
    - { title: Say hi, channel: general, type: chat }
    - { title: Chat, channel: chat-1, type: chat }
",
    );
    converge(&yaml, snapshot(community(json!({}))), &Lockfile::default());
}

#[test]
fn unreadable_sections_are_errors_only_when_managed() {
    let mut c = community(json!({}));
    c.as_object_mut().unwrap().remove("verification");
    c["errors"] = json!([["join questions", "403 Missing Access"]]);
    let snap = snapshot(c);
    let yaml = spec_with("join_questions:\n  rules: [Be nice]\n");
    let p = plan(
        &desired(&yaml),
        &to_live(&snap),
        &Lockfile::default(),
        &PlanOptions::default(),
    );
    let d = diag(&p.diagnostics, "join_questions");
    assert!(d.is_error());
    assert!(d.message.contains("403 Missing Access"), "{d:?}");
    // Left out of the spec: no complaint.
    plan_of(&spec_with(""), &snap, &Lockfile::default());
}

#[test]
fn prune_keeps_roles_the_spec_uses_by_name() {
    let snap = snapshot(community(json!({})));
    let yaml = spec_with("automod:\n  - { key: s, name: S, trigger: spam, actions: [{ block: true }], exempt_roles: [Mod] }\n");
    let p = plan(
        &desired(&yaml),
        &to_live(&snap),
        &Lockfile::default(),
        &PlanOptions { prune: true },
    );
    let deleted: Vec<&str> = p
        .ops
        .iter()
        .filter_map(|o| match o {
            Op::Delete { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deleted, ["Member"]);
    let m = p.unmanaged.iter().find(|u| u.name == "Mod").unwrap();
    assert!(!m.pruned && m.note.is_some(), "{m:?}");
}

#[test]
fn unmanaged_server_objects_are_listed_and_pruned_on_request() {
    let snap = snapshot(community(json!({
        "automod": [{ "id": "400", "name": "Old", "trigger_type": 3, "event_type": 1, "actions": [] }],
        "emojis": [
            { "id": "300", "name": "old" },
            { "id": "301", "name": "integration", "managed": true },
        ],
        "stickers": [{ "id": "500", "name": "Old", "tags": "x" }],
        "sounds": [{ "sound_id": "600", "name": "Old" }],
    })));
    let yaml = spec_with("automod: []\nemoji: []\nstickers: []\nsounds: []\n");
    let p = plan_of(&yaml, &snap, &Lockfile::default());
    assert!(community_ops(&p).is_empty());
    let mut ids: Vec<u64> = p
        .unmanaged
        .iter()
        .map(|u| u.id)
        .filter(|id| *id >= 300)
        .collect();
    ids.sort();
    assert_eq!(ids, [300, 400, 500, 600], "managed emoji are never listed");

    let p = plan(
        &desired(&yaml),
        &to_live(&snap),
        &Lockfile::default(),
        &PlanOptions { prune: true },
    );
    let deletes = community_ops(&p).iter().filter(|o| o.is_delete()).count();
    assert_eq!(deletes, 4);
}

#[test]
fn expressions_without_a_file_must_exist() {
    let snap = snapshot(community(json!({})));
    let yaml = spec_with("stickers:\n  - { key: s, name: Box Box, emoji: '🛞' }\n");
    let p = plan(
        &desired(&yaml),
        &to_live(&snap),
        &Lockfile::default(),
        &PlanOptions::default(),
    );
    let d = diag(&p.diagnostics, "stickers[0]");
    assert_eq!(
        d.message,
        "sticker `Box Box` doesn't exist and has no `file`"
    );
}

// -- the in-memory Discord's rules -----------------------------------------------

#[test]
fn memory_api_wants_seven_default_channels_for_onboarding() {
    let api = MemoryApi::new(snapshot(community(json!({}))));
    let body = |ids: Value, enabled: bool| json!({ "prompts": [], "default_channel_ids": ids, "enabled": enabled, "mode": 0 });
    let r = write(
        &api,
        Endpoint::PutOnboarding,
        body(json!(["21", "22", "23"]), true),
    );
    assert!(is_400(&r), "{r:?}");
    // A category counts its channels.
    let r = write(
        &api,
        Endpoint::PutOnboarding,
        body(json!(["21", "22", "30"]), true),
    );
    assert!(r.is_ok(), "{r:?}");
    let r = write(&api, Endpoint::PutOnboarding, body(json!(["21"]), false));
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn memory_api_checks_the_join_questions_version() {
    let api = MemoryApi::new(snapshot(community(json!({}))));
    let body = |v: &str| json!({ "version": v, "description": "", "form_fields": [] });
    let r = write(&api, Endpoint::UpdateJoinQuestions, body("0"));
    assert!(is_400(&r), "{r:?}");
    let new = write(&api, Endpoint::UpdateJoinQuestions, body("1")).unwrap();
    assert_ne!(new["version"], "1");
    let r = write(&api, Endpoint::UpdateJoinQuestions, body("1"));
    assert!(is_400(&r), "stale version: {r:?}");
}

#[test]
fn memory_api_wants_three_guide_actions() {
    let api = MemoryApi::new(snapshot(community(json!({}))));
    let action =
        json!({ "channel_id": "21", "action_type": 0, "title": "Read", "description": "" });
    let body = |enabled: bool| json!({ "enabled": enabled, "new_member_actions": [action, action], "resource_channels": [] });
    assert!(is_400(&write(&api, Endpoint::PutGuide, body(true))));
    assert!(write(&api, Endpoint::PutGuide, body(false)).is_ok());
}

#[test]
fn memory_api_refuses_to_change_an_automod_trigger() {
    let api = MemoryApi::new(snapshot(community(json!({}))));
    let r = write(
        &api,
        Endpoint::CreateAutomod,
        json!({ "name": "S", "event_type": 1, "trigger_type": 3, "actions": [] }),
    )
    .unwrap();
    let id: u64 = r["id"].as_str().unwrap().parse().unwrap();
    let r = write(
        &api,
        Endpoint::UpdateAutomod(id),
        json!({ "trigger_type": 1 }),
    );
    assert!(is_400(&r), "{r:?}");
}

#[test]
fn memory_api_ids_never_collide_with_existing_objects() {
    let api = MemoryApi::new(snapshot(community(json!({
        "emojis": [{ "id": "999999", "name": "big" }],
    }))));
    let r = write(
        &api,
        Endpoint::CreateAutomod,
        json!({ "name": "S", "event_type": 1, "trigger_type": 3, "actions": [] }),
    )
    .unwrap();
    let id: u64 = r["id"].as_str().unwrap().parse().unwrap();
    assert!(id > 999_999, "{id}");
}

#[test]
fn applying_onboarding_with_too_few_channels_fails_with_discords_message() {
    let yaml = format!(
        "{}onboarding:\n  default_channels: [rules, general]\n  prompts:\n    \
         - {{ title: Hi, options: [{{ title: x, channels: [general] }}] }}\n",
        spec_with("")
    );
    let (d, diags) = expand(&yaml);
    assert!(!concord::diag::has_errors(&diags));
    assert!(diags
        .iter()
        .any(|d| d.path == "onboarding.default_channels"));
    let api = MemoryApi::new(snapshot(community(json!({}))));
    let p = plan(
        &d,
        &to_live(&api.state()),
        &Lockfile::default(),
        &PlanOptions::default(),
    );
    let mut applier = Applier::new(&p);
    let (_, e) = block_on(applier.run_all(&api, &p.ops, |_, _, _| {})).unwrap_err();
    assert!(e.to_string().contains("too few default channels"), "{e}");
}

#[test]
fn join_questions_edited_since_the_plan_are_not_overwritten() {
    let yaml = spec_with("join_questions:\n  rules: [Be nice]\n");
    let api = MemoryApi::new(snapshot(community(json!({}))));
    let p = plan(
        &desired(&yaml),
        &to_live(&api.state()),
        &Lockfile::default(),
        &PlanOptions::default(),
    );
    // Someone edits the form in the client before the plan is applied.
    write(
        &api,
        Endpoint::UpdateJoinQuestions,
        json!({ "version": "1", "form_fields": [] }),
    )
    .unwrap();
    let mut applier = Applier::new(&p);
    let (_, e) = block_on(applier.run_all(&api, &p.ops, |_, _, _| {})).unwrap_err();
    assert!(e.to_string().contains("version mismatch"), "{e}");
}
