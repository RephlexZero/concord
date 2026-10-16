//! Apply `examples/racing-team.yaml` to an in-memory copy of the fixture server,
//! then plan again: a correct apply leaves nothing to do.

mod common;

use concord::apply::Applier;
use concord::diff::{plan, Op, Plan, PlanOptions};
use concord::live::memory::MemoryApi;
use concord::live::{to_live, Endpoint, Snapshot};
use concord::lock::Lockfile;
use concord::model::Desired;
use concord::spec;

fn desired() -> Desired {
    common::team_desired()
}

fn snapshot() -> Snapshot {
    let path = common::root().join("tests/fixtures/team_live.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

/// Plan, apply to `api`, and return the plan plus the resulting lockfile.
fn apply(api: &MemoryApi, lock: &Lockfile, prune: bool) -> (Plan, Lockfile) {
    let live = to_live(&api.state());
    let p = plan(&desired(), &live, lock, &PlanOptions { prune });
    assert!(!p.has_errors(), "{:#?}", p.diagnostics);
    let mut applier = Applier::new(&p);
    let mut saves = 0;
    block_on(applier.run_all(api, &p.ops, |_, _, _| saves += 1))
        .unwrap_or_else(|(i, e)| panic!("op {i} ({:?}) failed: {e}", p.ops[i]));
    assert_eq!(saves, p.ops.len());
    (p, applier.lock().clone())
}

fn assert_converged(api: &MemoryApi, lock: &Lockfile, prune: bool) {
    let again = plan(
        &desired(),
        &to_live(&api.state()),
        lock,
        &PlanOptions { prune },
    );
    let left: Vec<&Op> = again
        .ops
        .iter()
        .filter(|o| !matches!(o, Op::Adopt { .. }))
        .collect();
    assert!(left.is_empty(), "still to do after apply: {left:#?}");
    assert!(
        again.ops.is_empty(),
        "everything should be in the lockfile now: {:#?}",
        again.ops
    );
}

#[test]
fn apply_converges() {
    let api = MemoryApi::new(snapshot());
    let (p, lock) = apply(&api, &common::lock_fixture("team.lock.json"), false);
    assert!(p.has_changes());
    assert_converged(&api, &lock, false);
}

#[test]
fn apply_with_prune_converges() {
    let api = MemoryApi::new(snapshot());
    let (_, lock) = apply(&api, &common::lock_fixture("team.lock.json"), true);
    assert_converged(&api, &lock, true);
    let live = to_live(&api.state());
    let p = plan(&desired(), &live, &lock, &PlanOptions { prune: true });
    assert!(p.unmanaged.iter().all(|u| !u.pruned || u.note.is_some()));
}

#[test]
fn records_every_created_object_in_the_lockfile() {
    let api = MemoryApi::new(snapshot());
    let (_, lock) = apply(&api, &common::lock_fixture("team.lock.json"), false);
    for key in ["mod", "lead", "director"] {
        assert!(lock.roles.contains_key(key), "role {key}");
    }
    for key in [
        "wiki",
        "design-reviews",
        "driverless",
        "driverless.forum",
        "trackside",
    ] {
        assert!(lock.channels.contains_key(key), "channel {key}");
    }
    assert!(lock.tags.contains_key("wiki/Guide"), "{:#?}", lock.tags);
    assert!(lock.tags.contains_key("vd.forum/Setup"), "{:#?}", lock.tags);
}

#[test]
fn existing_forum_tags_are_sent_back_with_their_ids() {
    let snap = snapshot();
    let vd_forum = snap
        .channels
        .iter()
        .find(|c| c.name.as_deref() == Some("vd-forum"))
        .unwrap();
    let existing: Vec<String> = vd_forum
        .available_tags
        .iter()
        .flatten()
        .filter_map(|t| t.id.map(|i| i.to_string()))
        .collect();
    assert!(!existing.is_empty());
    let id = vd_forum.id.0;

    let api = MemoryApi::new(snap);
    apply(&api, &common::lock_fixture("team.lock.json"), false);
    let sent = api
        .log()
        .into_iter()
        .find(|r| {
            r.endpoint == Endpoint::UpdateChannel(id)
                && r.body
                    .as_ref()
                    .is_some_and(|b| b.get("available_tags").is_some())
        })
        .expect("vd-forum tags were updated");
    let body = sent.body.unwrap();
    let ids: Vec<&str> = body["available_tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    for want in &existing {
        assert!(ids.contains(&want.as_str()), "tag {want} kept its ID");
    }
}

#[test]
fn a_second_apply_after_an_interrupted_one_finishes_the_job() {
    let api = MemoryApi::new(snapshot());
    let lock = common::lock_fixture("team.lock.json");
    let p = plan(
        &desired(),
        &to_live(&api.state()),
        &lock,
        &PlanOptions::default(),
    );
    let mut applier = Applier::new(&p);
    let half = p.ops.len() / 2;
    block_on(applier.run_all(&api, &p.ops[..half], |_, _, _| {})).unwrap();
    let partial = applier.lock().clone();

    let (_, lock) = apply(&api, &partial, false);
    assert_converged(&api, &lock, false);
}

#[test]
fn requiring_tags_waits_until_the_forum_has_some() {
    // Discord rejects require_tag on a forum with no tags, so the flag must
    // follow the tag update even though plans list updates first.
    use concord::live::wire::{WireChannel, WireGuild, WireRole};
    use concord::snowflake::Sf;
    let snap = Snapshot {
        guild: WireGuild {
            id: Sf(1),
            name: "t".into(),
            ..Default::default()
        },
        roles: vec![WireRole {
            id: Sf(1),
            name: "@everyone".into(),
            permissions: "0".into(),
            ..Default::default()
        }],
        channels: vec![WireChannel {
            id: Sf(5),
            kind: 15,
            name: Some("wiki".into()),
            position: Some(0),
            ..Default::default()
        }],
        bot: None,
        community: None,
    };
    let parsed = spec::parse(
        "guild_id: \"1\"\nchannels:\n  - { key: wiki, type: forum, name: wiki, require_tag: true, tags: [Guide] }\n",
    )
    .unwrap();
    let (desired, diags) = spec::expand(&parsed);
    assert!(!concord::diag::has_errors(&diags), "{diags:#?}");
    let api = MemoryApi::new(snap);
    let p = plan(
        &desired,
        &to_live(&api.state()),
        &Lockfile::default(),
        &PlanOptions::default(),
    );
    let mut applier = Applier::new(&p);
    block_on(applier.run_all(&api, &p.ops, |_, _, _| {}))
        .unwrap_or_else(|(i, e)| panic!("op {i} failed: {e}"));
    let again = plan(
        &desired,
        &to_live(&api.state()),
        applier.lock(),
        &PlanOptions::default(),
    );
    assert!(again.ops.is_empty(), "{:#?}", again.ops);
}

#[test]
fn server_wide_requests_keep_what_discord_already_has() {
    let api = MemoryApi::new(snapshot());
    let (_, lock) = apply(&api, &common::lock_fixture("team.lock.json"), false);
    let log = api.log();
    let body = |e: Endpoint| {
        log.iter()
            .find(|r| r.endpoint == e)
            .and_then(|r| r.body.clone())
            .unwrap_or_else(|| panic!("no {e:?} request"))
    };

    // Discord refuses bots on the Server Guide endpoint, so nothing is sent
    // there; differences are reported as a warning instead.
    let (method, guide_path) = (
        "PUT",
        format!("guilds/{}/new-member-welcome", snapshot().guild.id.0),
    );
    assert!(log
        .iter()
        .all(|r| r.endpoint.method_path(snapshot().guild.id.0) != (method, guide_path.clone())));

    // Existing prompts and options keep their IDs, in the spec's order.
    let onboarding = body(Endpoint::PutOnboarding);
    let ids = |v: &serde_json::Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let prompts = ids(&onboarding["prompts"]);
    assert_eq!(
        prompts[..2],
        ["1000000000004236101", "1000000000004236102"],
        "{prompts:?}"
    );
    assert_eq!(prompts.len(), 2, "the old prompt is dropped");
    assert_eq!(
        ids(&onboarding["prompts"][0]["options"]),
        ["1000000000004236112", "1000000000004236111"]
    );
    let subteams = ids(&onboarding["prompts"][1]["options"]);
    assert_eq!(subteams.len(), 10);
    assert!(subteams.contains(&"1000000000004236121".to_owned()));

    // The join form is sent with the version read, rules first.
    let join = body(Endpoint::UpdateJoinQuestions);
    assert_eq!(join["version"], "2024-05-01T12:00:00.000000+00:00");
    assert_eq!(join["form_fields"][0]["field_type"], "TERMS");
    assert_eq!(
        join["form_fields"][0]["values"].as_array().unwrap().len(),
        3
    );

    // New emoji are uploaded as data URIs.
    let emoji = body(Endpoint::CreateEmoji);
    assert!(emoji["image"]
        .as_str()
        .unwrap()
        .starts_with("data:image/png;base64,"));

    for key in ["mention-spam", "slurs", "banned-words", "spam", "profiles"] {
        assert!(lock.automod.contains_key(key), "automod {key}");
    }
    for key in ["why", "why/alumni", "subteams", "subteams/driverless"] {
        assert!(lock.onboarding.contains_key(key), "onboarding {key}");
    }
    assert!(lock.emoji.contains_key("logo") && lock.emoji.contains_key("chequered"));
    assert!(lock.assets.contains_key("emoji/chequered"));
    assert_eq!(lock.stickers["box-box"].0, 1000000000004236401);
    assert_eq!(lock.sounds["engine-rev"].0, 1000000000004236501);
}
