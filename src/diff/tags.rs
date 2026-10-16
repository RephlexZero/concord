//! Forum tags. Discord's `available_tags` is replaced wholesale on update, and
//! a tag sent without its existing ID is a *new* tag: the old one is deleted
//! and stripped from every post. So every tag that already exists must be
//! matched to its live ID and sent back with it.

use std::collections::HashSet;

use super::{Change, TagChange, TagState};
use crate::lock::{tag_key, Lockfile};
use crate::model::{DesiredTag, LiveTag};

#[derive(Debug, Default)]
pub struct TagDiff {
    /// The full list to send.
    pub after: Vec<TagState>,
    pub changes: Vec<TagChange>,
    /// Lockfile entries (`<channel>/<tag>` → ID) for matched tags.
    pub bindings: Vec<(String, u64)>,
    /// Live tags the spec doesn't mention.
    pub unmanaged: Vec<LiveTag>,
}

pub fn diff_tags(
    channel_key: &str,
    desired: &[DesiredTag],
    live: &[LiveTag],
    lock: &Lockfile,
    prune: bool,
) -> TagDiff {
    let mut matched: Vec<Option<usize>> = vec![None; desired.len()];
    let mut used: HashSet<usize> = HashSet::new();

    // 1. By lockfile ID: survives renames.
    for (d, slot) in desired.iter().zip(matched.iter_mut()) {
        if let Some(id) = lock.tags.get(&tag_key(channel_key, &d.key)) {
            if let Some(i) = live.iter().position(|l| l.id == id.0) {
                if used.insert(i) {
                    *slot = Some(i);
                }
            }
        }
    }
    // 2. By exact name, then 3. ignoring case.
    for exact in [true, false] {
        for (d, slot) in desired.iter().zip(matched.iter_mut()) {
            if slot.is_some() {
                continue;
            }
            let found = live.iter().enumerate().position(|(i, l)| {
                !used.contains(&i)
                    && if exact {
                        l.name == d.name
                    } else {
                        l.name.to_lowercase() == d.name.to_lowercase()
                    }
            });
            if let Some(i) = found {
                used.insert(i);
                *slot = Some(i);
            }
        }
    }

    let mut out = TagDiff::default();
    for (d, slot) in desired.iter().zip(&matched) {
        let live_tag = slot.map(|i| &live[i]);
        out.after.push(TagState {
            id: live_tag.map(|l| l.id),
            key: Some(d.key.clone()),
            name: d.name.clone(),
            emoji: d.emoji.clone(),
            moderated: d.moderated,
        });
        let Some(l) = live_tag else {
            out.changes.push(TagChange::Add(d.name.clone()));
            continue;
        };
        out.bindings.push((tag_key(channel_key, &d.key), l.id));
        if let Some(c) = Change::of(&l.name, &d.name) {
            out.changes.push(TagChange::Rename(c));
        }
        if let Some(change) = Change::of(&l.emoji, &d.emoji) {
            out.changes.push(TagChange::Emoji {
                name: d.name.clone(),
                change,
            });
        }
        if let Some(change) = Change::of(&l.moderated, &d.moderated) {
            out.changes.push(TagChange::Moderated {
                name: d.name.clone(),
                change,
            });
        }
    }

    let order: Vec<usize> = matched.iter().flatten().copied().collect();
    if order.windows(2).any(|w| w[0] > w[1]) {
        out.changes.push(TagChange::Reorder);
    }

    for (i, l) in live.iter().enumerate() {
        if used.contains(&i) {
            continue;
        }
        if prune {
            out.changes.push(TagChange::Remove(l.name.clone()));
        } else {
            out.after.push(TagState {
                id: Some(l.id),
                key: None,
                name: l.name.clone(),
                emoji: l.emoji.clone(),
                moderated: l.moderated,
            });
        }
        out.unmanaged.push(l.clone());
    }
    out
}

pub fn live_states(live: &[LiveTag], after: &[TagState]) -> Vec<TagState> {
    live.iter()
        .map(|l| TagState {
            id: Some(l.id),
            key: after
                .iter()
                .find(|a| a.id == Some(l.id))
                .and_then(|a| a.key.clone()),
            name: l.name.clone(),
            emoji: l.emoji.clone(),
            moderated: l.moderated,
        })
        .collect()
}
