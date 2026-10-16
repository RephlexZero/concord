//! `import` then `plan` against the same server must show no changes.

mod common;

use concord::diag::has_errors;
use concord::diff::{plan, Op, PlanOptions};
use concord::lock::Lockfile;
use concord::{import, spec};

#[test]
fn import_then_plan_shows_no_changes() {
    let live = common::live_fixture("tbre_live.json");
    let imported = import::import(&live);
    let yaml = import::to_yaml(&imported.spec, &live.guild.name);

    // The YAML must parse and validate like a hand-written spec.
    let parsed = spec::parse(&yaml).unwrap_or_else(|e| panic!("{e}\n---\n{yaml}"));
    let (desired, diags) = spec::expand(&parsed);
    assert!(!has_errors(&diags), "{diags:#?}\n---\n{yaml}");

    let p = plan(&desired, &live, &imported.lock, &PlanOptions::default());
    assert!(
        p.ops.is_empty(),
        "expected no changes, got {:#?}\n---\n{yaml}",
        p.ops
    );
    assert!(!p.has_errors(), "{:#?}", p.diagnostics);
    assert!(
        p.unmanaged.is_empty(),
        "everything is imported: {:#?}",
        p.unmanaged
    );
}

#[test]
fn imported_spec_without_a_lockfile_only_adopts() {
    let live = common::live_fixture("tbre_live.json");
    let imported = import::import(&live);
    let (desired, _) = spec::expand(&imported.spec);

    let p = plan(
        &desired,
        &live,
        &Lockfile::default(),
        &PlanOptions::default(),
    );
    assert!(!p.has_errors(), "{:#?}", p.diagnostics);
    let not_adopt: Vec<&Op> = p
        .ops
        .iter()
        .filter(|o| !matches!(o, Op::Adopt { .. }))
        .collect();
    assert!(not_adopt.is_empty(), "{not_adopt:#?}");
}

#[test]
fn imported_lockfile_matches_the_server() {
    let live = common::live_fixture("tbre_live.json");
    let imported = import::import(&live);
    for (key, id) in imported.lock.channels.iter() {
        assert!(live.channel(id.0).is_some(), "{key} → {id}");
    }
    for (key, id) in imported.lock.roles.iter() {
        assert!(live.role(id.0).is_some(), "{key} → {id}");
    }
}
