//! Channel and role ordering. Discord positions are relative and messy, so we
//! compare *effective* order (what the client shows), and only emit a reorder
//! when it actually differs.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use super::resolve::Resolution;
use super::{Ref, ReorderGroup, ReorderRoles};
use crate::diag::Diagnostic;
use crate::model::{ChannelKind, Desired, LiveChannel, LiveState};

/// Merge the desired order of managed items with the live order. Unmanaged
/// live items stay attached to the managed item they currently follow (or the
/// start of the list), so concord never shuffles things it doesn't own.
fn merge_order(desired: &[Ref], live: &[u64], managed: &HashSet<u64>) -> Vec<Ref> {
    let in_group: HashSet<&Ref> = desired.iter().collect();
    let mut anchored: HashMap<Option<u64>, Vec<u64>> = HashMap::new();
    let mut anchor: Option<u64> = None;
    for id in live {
        if !managed.contains(id) {
            anchored.entry(anchor).or_default().push(*id);
        } else if in_group.contains(&Ref::Id(*id)) {
            anchor = Some(*id);
        }
        // Managed items leaving this group are skipped entirely.
    }
    let mut out: Vec<Ref> = Vec::new();
    let push_anchored = |out: &mut Vec<Ref>, key: Option<u64>| {
        if let Some(items) = anchored.get(&key) {
            out.extend(items.iter().map(|id| Ref::Id(*id)));
        }
    };
    push_anchored(&mut out, None);
    for r in desired {
        out.push(r.clone());
        if let Ref::Id(id) = r {
            push_anchored(&mut out, Some(*id));
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Group {
    Top,
    Categories,
    Children(Ref),
}

fn effective_sort(channels: &mut [&LiveChannel]) {
    channels.sort_by_key(|c| (c.kind.is_voice_like(), c.position, c.id));
}

/// Groups whose order needs fixing, plus positions for channels created in
/// groups that are entirely new (they're created in place, no reorder needed).
pub fn reorder_channels(
    desired: &Desired,
    live: &LiveState,
    res: &Resolution,
) -> (Vec<ReorderGroup>, HashMap<String, u64>) {
    let ref_of = |key: &str| match res.channel(key).id() {
        Some(id) => Ref::Id(id),
        None => Ref::New(key.to_owned()),
    };
    let managed: HashSet<u64> = res.channels.values().filter_map(|b| b.id()).collect();

    // Desired groups, in spec order.
    let mut groups: IndexMap<Group, Vec<Ref>> = IndexMap::new();
    let mut names: HashMap<Ref, String> = HashMap::new();
    let mut voice: HashSet<Ref> = HashSet::new();
    for c in &desired.channels {
        let r = ref_of(&c.key);
        names.insert(r.clone(), c.name.clone());
        if c.kind == ChannelKind::Voice {
            voice.insert(r.clone());
        }
        let g = match (&c.parent, c.kind) {
            (_, ChannelKind::Category) => Group::Categories,
            (None, _) => Group::Top,
            (Some(p), _) => Group::Children(ref_of(p)),
        };
        groups.entry(g).or_default().push(r);
    }
    for lc in &live.channels {
        names
            .entry(Ref::Id(lc.id))
            .or_insert_with(|| lc.name.clone());
        if lc.kind.is_voice_like() {
            voice.insert(Ref::Id(lc.id));
        }
    }

    let mut out = Vec::new();
    let mut create_positions = HashMap::new();
    for (group, mut wanted) in groups {
        // Voice-like channels always display below text-like ones.
        wanted.sort_by_key(|r| voice.contains(r));

        let mut live_seq: Vec<&LiveChannel> = live
            .channels
            .iter()
            .filter(|lc| match &group {
                Group::Top => lc.parent_id.is_none() && !lc.kind.is_category(),
                Group::Categories => lc.kind.is_category(),
                Group::Children(Ref::Id(p)) => lc.parent_id == Some(*p),
                Group::Children(Ref::New(_)) => false,
            })
            .collect();
        effective_sort(&mut live_seq);
        let live_ids: Vec<u64> = live_seq.iter().map(|c| c.id).collect();

        let mut merged = merge_order(&wanted, &live_ids, &managed);
        merged.sort_by_key(|r| voice.contains(r));

        // What the group looks like now, minus channels that are leaving it.
        let current: Vec<Ref> = live_ids
            .iter()
            .filter(|id| merged.contains(&Ref::Id(**id)))
            .map(|id| Ref::Id(*id))
            .collect();
        if merged == current {
            continue;
        }
        if current.is_empty() && merged.iter().all(|r| matches!(r, Ref::New(_))) {
            for (i, r) in merged.iter().enumerate() {
                if let Ref::New(key) = r {
                    create_positions.insert(key.clone(), i as u64);
                }
            }
            continue;
        }
        let label = match &group {
            Group::Top => "uncategorised channels".to_owned(),
            Group::Categories => "categories".to_owned(),
            Group::Children(r) => names.get(r).cloned().unwrap_or_default(),
        };
        let name = |r: &Ref| names.get(r).cloned().unwrap_or_default();
        out.push(ReorderGroup {
            label,
            before: live_ids.iter().map(|id| name(&Ref::Id(*id))).collect(),
            after: merged.iter().map(name).collect(),
            positions: merged
                .iter()
                .enumerate()
                .map(|(i, r)| (r.clone(), i as u64))
                .collect(),
        });
    }
    (out, create_positions)
}

pub fn reorder_roles(
    desired: &Desired,
    live: &LiveState,
    res: &Resolution,
    diags: &mut Vec<Diagnostic>,
) -> Option<ReorderRoles> {
    let mut live_roles: Vec<_> = live
        .roles
        .iter()
        .filter(|r| r.id != live.guild.id)
        .collect();
    live_roles.sort_by_key(|r| (std::cmp::Reverse(r.position), r.id));
    let live_ids: Vec<u64> = live_roles.iter().map(|r| r.id).collect();
    let managed: HashSet<u64> = res.roles.values().filter_map(|b| b.id()).collect();

    let mut names: HashMap<Ref, String> = live_roles
        .iter()
        .map(|r| (Ref::Id(r.id), r.name.clone()))
        .collect();
    let wanted: Vec<Ref> = desired
        .roles
        .iter()
        .filter(|r| !r.everyone)
        .map(|r| {
            let rf = match res.role(&r.key).id() {
                Some(id) => Ref::Id(id),
                None => Ref::New(r.key.clone()),
            };
            names.insert(rf.clone(), r.name.clone());
            rf
        })
        .collect();

    let merged = merge_order(&wanted, &live_ids, &managed);
    let current: Vec<Ref> = live_ids.iter().map(|id| Ref::Id(*id)).collect();
    if merged == current {
        return None;
    }
    let name = |r: &Ref| names.get(r).cloned().unwrap_or_default();

    // Roles at or above the bot's highest role can't be moved by the bot, so
    // they must already be in place. Everything below gets positions counted
    // up from the bottom (@everyone is 0). Creating a role shifts every
    // position up by one, so absolute positions are only stable this way.
    let bot_top = live.bot.as_ref().and_then(|b| {
        live.roles
            .iter()
            .filter(|r| b.role_ids.contains(&r.id))
            .map(|r| r.position)
            .max()
    });
    let fixed = bot_top.map_or(0, |top| {
        live_roles.iter().take_while(|r| r.position >= top).count()
    });
    if merged.len() < fixed || merged[..fixed] != current[..fixed] {
        let top_name = name(&current[fixed - 1]);
        let moved: Vec<String> = merged
            .iter()
            .take(fixed)
            .zip(&current)
            .filter(|(a, b)| a != b)
            .map(|(a, _)| name(a))
            .collect();
        diags.push(
            Diagnostic::error(
                "roles",
                format!(
                    "the spec puts roles in a different order above concord's own role `{top_name}` ({}), which concord can't change",
                    moved.join(", ")
                ),
            )
            .with_hint(format!(
                "in Server Settings → Roles, drag `{top_name}` above them, or match their current order in the spec"
            )),
        );
    }
    let movable = &merged[fixed.min(merged.len())..];
    let k = movable.len() as u64;
    Some(ReorderRoles {
        before: current.iter().map(name).collect(),
        after: merged.iter().map(name).collect(),
        positions: movable
            .iter()
            .enumerate()
            .map(|(i, r)| (r.clone(), k - i as u64))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[u64]) -> Vec<Ref> {
        v.iter().map(|i| Ref::Id(*i)).collect()
    }

    #[test]
    fn unmanaged_items_stay_after_their_predecessor() {
        // live: 1 (managed), 9 (unmanaged), 2 (managed)
        let managed: HashSet<u64> = [1, 2].into();
        let merged = merge_order(&ids(&[2, 1]), &[1, 9, 2], &managed);
        assert_eq!(merged, ids(&[2, 1, 9]));
    }

    #[test]
    fn leading_unmanaged_items_stay_first() {
        let managed: HashSet<u64> = [1, 2].into();
        let merged = merge_order(&ids(&[1, 2]), &[9, 1, 2], &managed);
        assert_eq!(merged, ids(&[9, 1, 2]));
    }

    #[test]
    fn new_items_go_where_the_spec_says() {
        let managed: HashSet<u64> = [1].into();
        let wanted = vec![Ref::New("x".into()), Ref::Id(1)];
        assert_eq!(merge_order(&wanted, &[1], &managed), wanted);
    }
}
