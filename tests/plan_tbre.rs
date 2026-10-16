//! Plan `examples/tbre.yaml` against a fixture of the server as it is today.
//! The rendered plan is compared with `tests/fixtures/tbre_plan.txt`; run
//! with `UPDATE_GOLDEN=1` to regenerate it after an intended change.

mod common;

use concord::diff::{plan, ChannelField, Op, Plan, PlanOptions};
use concord::render::{render_plan, Style};
use concord::spec;

fn tbre_plan(prune: bool) -> Plan {
    let parsed = spec::load(&common::root().join("examples/tbre.yaml")).unwrap();
    let (desired, diags) = spec::expand(&parsed);
    assert!(!concord::diag::has_errors(&diags), "{diags:#?}");
    let live = common::live_fixture("tbre_live.json");
    let lock = common::lock_fixture("tbre.lock.json");
    plan(&desired, &live, &lock, &PlanOptions { prune })
}

#[test]
fn renames_general_channels_to_main() {
    let p = tbre_plan(false);
    let renames: Vec<String> = p
        .ops
        .iter()
        .filter_map(|o| match o {
            Op::UpdateChannel(u) => u.changes.iter().find_map(|c| match c {
                ChannelField::Name(n) => Some(format!("{} -> {}", n.before, n.after)),
                _ => None,
            }),
            _ => None,
        })
        .collect();
    assert_eq!(
        renames,
        [
            "systems-general -> systems-main",
            "controls-general -> controls-main"
        ]
    );
}

#[test]
fn plan_has_changes_and_no_errors() {
    let p = tbre_plan(false);
    assert!(p.has_changes());
    assert!(!p.has_errors(), "{:#?}", p.diagnostics);
}

#[test]
fn prune_never_deletes_protected_channels() {
    let p = tbre_plan(true);
    for op in &p.ops {
        if let Op::Delete { name, .. } = op {
            assert!(name != "rules" && name != "moderator-only", "{name}");
        }
    }
}

#[test]
fn rendered_plan_matches_golden_file() {
    let rendered = render_plan(&tbre_plan(false), Style { color: false });
    let golden = common::root().join("tests/fixtures/tbre_plan.txt");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &rendered).unwrap();
    }
    let expected = std::fs::read_to_string(&golden).unwrap_or_default();
    pretty_assertions::assert_eq!(expected, rendered);
}
