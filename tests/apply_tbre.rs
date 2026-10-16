//! Apply `examples/tbre.yaml` to an in-memory copy of the fixture server,
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
    let parsed = spec::load(&common::root().join("examples/tbre.yaml")).unwrap();
    let (desired, diags) = spec::expand(&parsed);
    assert!(!concord::diag::has_errors(&diags), "{diags:#?}");
    desired
}

fn snapshot() -> Snapshot {
    let path = common::root().join("tests/fixtures/tbre_live.json");
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
    let (p, lock) = apply(&api, &common::lock_fixture("tbre.lock.json"), false);
    assert!(p.has_changes());
    assert_converged(&api, &lock, false);
}

#[test]
fn apply_with_prune_converges() {
    let api = MemoryApi::new(snapshot());
    let (_, lock) = apply(&api, &common::lock_fixture("tbre.lock.json"), true);
    assert_converged(&api, &lock, true);
    let live = to_live(&api.state());
    let p = plan(&desired(), &live, &lock, &PlanOptions { prune: true });
    assert!(p.unmanaged.iter().all(|u| !u.pruned || u.note.is_some()));
}

#[test]
fn records_every_created_object_in_the_lockfile() {
    let api = MemoryApi::new(snapshot());
    let (_, lock) = apply(&api, &common::lock_fixture("tbre.lock.json"), false);
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
    apply(&api, &common::lock_fixture("tbre.lock.json"), false);
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
    let lock = common::lock_fixture("tbre.lock.json");
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
