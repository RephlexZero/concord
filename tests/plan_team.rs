//! Plan `examples/racing-team.yaml` against a fixture of the server as it is today.
//! The rendered plan is compared with `tests/fixtures/team_plan.txt`; run
//! with `UPDATE_GOLDEN=1` to regenerate it after an intended change.

mod common;

use concord::diff::{plan, ChannelField, Op, Plan, PlanOptions};
use concord::render::{render_plan, Style};

fn team_plan(prune: bool) -> Plan {
    let desired = common::team_desired();
    let live = common::live_fixture("team_live.json");
    let lock = common::lock_fixture("team.lock.json");
    plan(&desired, &live, &lock, &PlanOptions { prune })
}

#[test]
fn renames_general_channels_to_main() {
    let p = team_plan(false);
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
    let p = team_plan(false);
    assert!(p.has_changes());
    assert!(!p.has_errors(), "{:#?}", p.diagnostics);
}

#[test]
fn prune_never_deletes_protected_channels() {
    let p = team_plan(true);
    for op in &p.ops {
        if let Op::Delete { name, .. } = op {
            assert!(name != "rules" && name != "moderator-only", "{name}");
        }
    }
}

#[test]
fn rendered_plan_matches_golden_file() {
    let rendered = render_plan(&team_plan(false), Style { color: false });
    let golden = common::root().join("tests/fixtures/team_plan.txt");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &rendered).unwrap();
    }
    let expected = std::fs::read_to_string(&golden).unwrap_or_default();
    pretty_assertions::assert_eq!(expected, rendered);
}
