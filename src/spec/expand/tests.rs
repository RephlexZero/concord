use super::*;
use crate::spec::parse;

/// The example from the brief, verbatim.
const BRIEF_EXAMPLE: &str = r##"
guild_id: "1000000000004232083"
tag_sets:
  base: [Question, Issue, Testing, How-to, Ordering, Open, In Progress, Blocked, Resolved]
templates:
  subteam:
    channels:
      - { key: main,  type: text,  name: "{slug}-main" }
      - { key: forum, type: forum, name: "{slug}-forum",
          tags: [base, "{subsystems}"], require_tag: true,
          layout: list, sort: activity,
          guidelines: "What / why / deadline / who you need" }
roles:
  - { key: lead, name: Lead, color: "#e91e63", hoist: true }
categories:
  engineering:
    name: "engineering 🏁"
    channels:
      - { key: engineering-main, type: text, name: engineering-main }
      - { key: interfaces, type: forum, name: interfaces,
          tags: [{ name: Agreed, moderated: true }, "Under Change"],
          overrides: { "@everyone": { deny: [CREATE_POSTS] }, Lead: { allow: [CREATE_POSTS] } } }
  vd:
    template: subteam
    name: "vehicle-dynamics 🪄"
    vars: { slug: vd, subsystems: [Kinematics, Tyres, Sim, Setup] }
"##;

fn expand_str(yaml: &str) -> (Desired, Vec<Diagnostic>) {
    let spec = parse(yaml).unwrap_or_else(|e| panic!("parse failed: {e}"));
    expand(&spec)
}

fn errors(diags: &[Diagnostic]) -> Vec<String> {
    diags
        .iter()
        .filter(|d| d.is_error())
        .map(|d| format!("{}: {}", d.path, d.message))
        .collect()
}

#[test]
fn brief_example_parses_and_expands() {
    let (d, diags) = expand_str(BRIEF_EXAMPLE);
    assert!(errors(&diags).is_empty(), "{diags:#?}");
    assert_eq!(d.guild_id, Some(1000000000004232083));
    assert_eq!(d.roles.len(), 1);
    assert_eq!(d.roles[0].color, Some(0xe91e63));

    let keys: Vec<&str> = d.channels.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "engineering",
            "engineering-main",
            "interfaces",
            "vd",
            "vd.main",
            "vd.forum"
        ]
    );

    let forum = d.channel("vd.forum").unwrap();
    assert_eq!(forum.name, "vd-forum");
    assert_eq!(forum.parent.as_deref(), Some("vd"));
    assert_eq!(
        forum.topic.as_deref(),
        Some("What / why / deadline / who you need")
    );
    assert_eq!(forum.forum.require_tag, Some(true));
    let tags: Vec<&str> = forum
        .forum
        .tags
        .as_ref()
        .unwrap()
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(
        tags,
        [
            "Question",
            "Issue",
            "Testing",
            "How-to",
            "Ordering",
            "Open",
            "In Progress",
            "Blocked",
            "Resolved",
            "Kinematics",
            "Tyres",
            "Sim",
            "Setup"
        ]
    );

    let vd = d.channel("vd").unwrap();
    assert_eq!(vd.kind, ChannelKind::Category);
    assert_eq!(vd.name, "vehicle-dynamics 🪄");
}

#[test]
fn forum_overrides_use_friendly_names() {
    let (d, _) = expand_str(BRIEF_EXAMPLE);
    let interfaces = d.channel("interfaces").unwrap();
    let DesiredOverwrites::Inherit(layers) = &interfaces.overwrites else {
        panic!("expected inherit, got {:?}", interfaces.overwrites);
    };
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].role, "@everyone");
    assert_eq!(layers[0].deny, Perms::SEND_MESSAGES);
    assert_eq!(layers[1].role, "Lead");
    assert_eq!(layers[1].allow, Perms::SEND_MESSAGES);

    let tags = interfaces.forum.tags.as_ref().unwrap();
    assert!(tags[0].moderated);
    assert_eq!(tags[1].name, "Under Change");
}

#[test]
fn channels_without_overrides_are_synced_to_category() {
    let (d, _) = expand_str(BRIEF_EXAMPLE);
    assert_eq!(
        d.channel("engineering-main").unwrap().overwrites,
        DesiredOverwrites::Inherit(vec![])
    );
    // Partial mode: a category with no permissions declared is left alone.
    assert_eq!(
        d.channel("engineering").unwrap().overwrites,
        DesiredOverwrites::Unmanaged
    );
}

#[test]
fn partial_mode_leaves_omitted_fields_unmanaged() {
    let (d, _) = expand_str(BRIEF_EXAMPLE);
    let main = d.channel("vd.main").unwrap();
    assert_eq!(main.topic, None);
    assert_eq!(main.slowmode, None);
    assert_eq!(d.roles[0].mentionable, None);
}

#[test]
fn authoritative_mode_fills_defaults() {
    let yaml = format!("concord: {{ mode: authoritative }}\n{BRIEF_EXAMPLE}");
    let (d, diags) = expand_str(&yaml);
    assert!(errors(&diags).is_empty(), "{diags:#?}");
    let main = d.channel("vd.main").unwrap();
    assert_eq!(main.topic.as_deref(), Some(""));
    assert_eq!(main.slowmode, Some(0));
    assert_eq!(main.bitrate, None, "bitrate is voice-only");
    assert_eq!(d.roles[0].mentionable, Some(false));
    assert_eq!(d.roles[0].permissions, Some(Perms::NONE));
    assert_eq!(
        d.channel("engineering").unwrap().overwrites,
        DesiredOverwrites::Explicit(vec![])
    );
}

#[test]
fn too_many_tags_is_an_error_naming_the_path() {
    let yaml = r#"
tag_sets:
  base: [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R]
categories:
  x:
    name: x
    channels:
      - { key: f, type: forum, name: f, tags: [base, S, T, U] }
"#;
    let (_, diags) = expand_str(yaml);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(
        errs[0].starts_with("categories.x.channels[0].tags: 21 tags"),
        "{errs:?}"
    );
}

#[test]
fn template_errors_point_at_the_template_and_the_instance() {
    let yaml = r#"
templates:
  subteam:
    channels:
      - { key: main, type: text, name: "{slg}-main" }
categories:
  vd:
    template: subteam
    name: vd
    vars: { slug: vd }
"#;
    let (_, diags) = expand_str(yaml);
    let err = diags.iter().find(|d| d.is_error()).unwrap();
    assert_eq!(
        err.path,
        "templates.subteam.channels[0].name (via categories.vd)"
    );
    assert_eq!(err.message, "unknown variable `{slg}`");
    assert_eq!(err.hint.as_deref(), Some("did you mean `slug`?"));
}

#[test]
fn list_variables_only_splice_into_tags() {
    let yaml = r#"
templates:
  t:
    channels:
      - { key: main, type: text, name: "{subs}-main" }
categories:
  vd:
    template: t
    name: vd
    vars: { subs: [A, B] }
"#;
    let (_, diags) = expand_str(yaml);
    assert!(errors(&diags)[0].contains("is a list and can't be used inside text"));
}

#[test]
fn tag_names_are_limited_to_20_characters() {
    let yaml = r#"
channels:
  - { key: f, type: forum, name: f, tags: ["This tag name is far too long"] }
"#;
    let (_, diags) = expand_str(yaml);
    assert!(
        errors(&diags)[0].contains("Discord allows 1-20"),
        "{diags:?}"
    );
}

#[test]
fn non_canonical_text_channel_names_are_rejected_with_a_fix() {
    let yaml = "channels:\n  - { key: g, type: text, name: General Chat }\n";
    let (_, diags) = expand_str(yaml);
    let err = diags.iter().find(|d| d.is_error()).unwrap();
    assert_eq!(err.path, "channels[0].name");
    assert_eq!(err.hint.as_deref(), Some("write `general-chat`"));
}

#[test]
fn create_posts_on_a_text_channel_is_rejected() {
    let yaml = r#"
channels:
  - { key: g, type: text, name: g, overrides: { "@everyone": { deny: [CREATE_POSTS] } } }
"#;
    let (_, diags) = expand_str(yaml);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(
        errs[0].starts_with("channels[0].overrides.@everyone.deny[0]"),
        "{errs:?}"
    );
}

#[test]
fn fields_for_the_wrong_channel_type_are_rejected() {
    let yaml = "channels:\n  - { key: v, type: voice, name: Voice, tags: [A] }\n";
    let (_, diags) = expand_str(yaml);
    assert!(errors(&diags)[0].contains("`tags` only applies to forum/media channels"));
}

#[test]
fn private_layers_under_overrides() {
    let yaml = r#"
roles:
  - { key: director, name: Director }
categories:
  directors:
    name: directors
    private: [Director]
    channels:
      - { key: d-chat, type: text, name: d-chat }
      - { key: d-announce, type: text, name: d-announce,
          overrides: { "@everyone": { allow: [VIEW_CHANNEL] } } }
"#;
    let (d, diags) = expand_str(yaml);
    assert!(errors(&diags).is_empty(), "{diags:?}");
    let DesiredOverwrites::Explicit(cat) = &d.channel("directors").unwrap().overwrites else {
        panic!()
    };
    assert_eq!(cat.len(), 2);
    assert_eq!(cat[0].role, "@everyone");
    assert_eq!(cat[0].deny, Perms::VIEW_CHANNEL);
    assert_eq!(cat[1].allow, Perms::VIEW_CHANNEL);
}

#[test]
fn tag_set_names_cannot_be_used_as_literal_tags() {
    let yaml = r#"
tag_sets:
  base: [A]
channels:
  - { key: f, type: forum, name: f, tags: [{ name: base, moderated: true }] }
"#;
    let (_, diags) = expand_str(yaml);
    assert!(errors(&diags)[0].contains("same name as a tag set"));
}

#[test]
fn duplicate_keys_are_reported() {
    let yaml = r#"
channels:
  - { key: a, type: text, name: a }
categories:
  a:
    name: A
"#;
    let (_, diags) = expand_str(yaml);
    assert!(errors(&diags)[0].contains("duplicate channel key `a`"));
}

#[test]
fn voice_before_text_warns() {
    let yaml = r#"
categories:
  c:
    name: c
    channels:
      - { key: v, type: voice, name: Voice }
      - { key: t, type: text, name: t }
"#;
    let (_, diags) = expand_str(yaml);
    assert!(errors(&diags).is_empty());
    assert!(diags[0].message.contains("listed after voice channel"));
}
