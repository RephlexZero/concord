//! `import` then `plan` against the same server must show no changes.

mod common;

use concord::diag::has_errors;
use concord::diff::{plan, Op, PlanOptions};
use concord::lock::Lockfile;
use concord::{import, spec};

#[test]
fn import_then_plan_shows_no_changes() {
    let live = common::live_fixture("team_live.json");
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
    let live = common::live_fixture("team_live.json");
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
    let live = common::live_fixture("team_live.json");
    let imported = import::import(&live);
    for (key, id) in imported.lock.channels.iter() {
        assert!(live.channel(id.0).is_some(), "{key} → {id}");
    }
    for (key, id) in imported.lock.roles.iter() {
        assert!(live.role(id.0).is_some(), "{key} → {id}");
    }
}

#[test]
fn import_includes_server_wide_sections() {
    let live = common::live_fixture("team_live.json");
    let imported = import::import(&live);
    let s = &imported.spec;

    let server = s.server.as_ref().unwrap();
    assert_eq!(server.rules_channel.as_deref(), Some("rules"));
    assert_eq!(
        server.safety_alerts_channel,
        Some(Some("moderator-only".to_owned()))
    );
    assert!(server.widget.is_some());
    let o = s.onboarding.as_ref().unwrap();
    assert_eq!(o.prompts.len(), 3);
    assert_eq!(o.default_channels.len(), 5);
    assert_eq!(s.welcome_screen.as_ref().unwrap().channels.len(), 2);
    let guide = s.guide.as_ref().unwrap();
    assert_eq!((guide.actions.len(), guide.resources.len()), (3, 1));
    let join = s.join_questions.as_ref().unwrap();
    assert_eq!((join.rules.len(), join.questions.len()), (2, 2));
    assert_eq!(s.automod.as_ref().unwrap().len(), 4);
    let emoji: Vec<&str> = s
        .emoji
        .as_ref()
        .unwrap()
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(emoji, ["fs_logo", "old_logo"], "managed emoji are left out");
    assert_eq!(s.stickers.as_ref().unwrap().len(), 1);
    assert_eq!(s.sounds.as_ref().unwrap().len(), 1);

    // Every prompt and option, rule and expression is locked to its ID.
    let lock = &imported.lock;
    assert_eq!(lock.onboarding.len(), 3 + 6);
    assert_eq!(lock.automod.len(), 4);
    assert_eq!(lock.emoji.len(), 2);
    let community = &live.community;
    for id in lock.automod.values() {
        assert!(community
            .automod
            .iter()
            .flatten()
            .any(|r| r.id == Some(id.0)));
    }
    assert!(imported
        .warnings
        .iter()
        .any(|w| w.message.contains("by name only")));
}
