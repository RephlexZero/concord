//! Identity: which live object (if any) does each spec key refer to?

use std::collections::{HashMap, HashSet};

use crate::diag::Diagnostic;
use crate::lock::Lockfile;
use crate::model::{ChannelKind, Desired, DesiredChannel, LiveState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// The lockfile says so, and it still exists.
    Locked(u64),
    /// Not in the lockfile, but a live object with the same name and type in
    /// the same place exists; take it over instead of creating a duplicate.
    Adopted(u64),
    /// Doesn't exist yet.
    New,
    /// The lockfile points at an ID that no longer exists.
    Recreate(u64),
}

impl Binding {
    pub fn id(&self) -> Option<u64> {
        match self {
            Binding::Locked(id) | Binding::Adopted(id) => Some(*id),
            Binding::New | Binding::Recreate(_) => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct Resolution {
    pub roles: HashMap<String, Binding>,
    pub channels: HashMap<String, Binding>,
}

impl Resolution {
    pub fn role(&self, key: &str) -> &Binding {
        self.roles.get(key).unwrap_or(&Binding::New)
    }
    pub fn channel(&self, key: &str) -> &Binding {
        self.channels.get(key).unwrap_or(&Binding::New)
    }
}

pub fn resolve(
    desired: &Desired,
    live: &LiveState,
    lock: &Lockfile,
    diags: &mut Vec<Diagnostic>,
) -> Resolution {
    let mut res = Resolution::default();

    // -- roles
    let desired_role_keys: HashSet<&str> = desired.roles.iter().map(|r| r.key.as_str()).collect();
    let mut claimed: HashSet<u64> = lock
        .roles
        .iter()
        .filter(|(k, _)| desired_role_keys.contains(k.as_str()))
        .map(|(_, id)| id.0)
        .collect();
    for role in &desired.roles {
        let binding = if role.everyone {
            Binding::Locked(live.guild.id)
        } else if let Some(id) = lock.roles.get(&role.key) {
            if live.role(id.0).is_some() {
                Binding::Locked(id.0)
            } else {
                diags.push(
                    Diagnostic::warning(
                        &role.path,
                        format!(
                            "lockfile says role `{}` is {id}, but that role no longer exists",
                            role.key
                        ),
                    )
                    .with_hint("it will be created again"),
                );
                Binding::Recreate(id.0)
            }
        } else {
            let candidates: Vec<u64> = live
                .roles
                .iter()
                .filter(|r| r.name == role.name && !r.managed && r.id != live.guild.id)
                .filter(|r| !claimed.contains(&r.id))
                .map(|r| r.id)
                .collect();
            match candidates.as_slice() {
                [] => Binding::New,
                [id] => {
                    claimed.insert(*id);
                    Binding::Adopted(*id)
                }
                many => {
                    diags.push(
                        Diagnostic::error(
                            &role.path,
                            format!(
                                "{} live roles are called `{}`; can't tell which one this is",
                                many.len(),
                                role.name
                            ),
                        )
                        .with_hint(format!(
                            "add the right one to concord.lock.json under roles.{} (IDs: {})",
                            role.key,
                            join_ids(many)
                        )),
                    );
                    Binding::New
                }
            }
        };
        res.roles.insert(role.key.clone(), binding);
    }

    // -- channels: categories first, so children can be matched by parent
    let desired_keys: HashSet<&str> = desired.channels.iter().map(|c| c.key.as_str()).collect();
    let mut claimed: HashSet<u64> = lock
        .channels
        .iter()
        .filter(|(k, _)| desired_keys.contains(k.as_str()))
        .map(|(_, id)| id.0)
        .collect();
    let (cats, others): (Vec<&DesiredChannel>, Vec<&DesiredChannel>) = desired
        .channels
        .iter()
        .partition(|c| c.kind == ChannelKind::Category);
    for ch in cats.into_iter().chain(others) {
        let binding = resolve_channel(ch, live, lock, &res, &mut claimed, diags);
        res.channels.insert(ch.key.clone(), binding);
    }
    res
}

fn resolve_channel(
    ch: &DesiredChannel,
    live: &LiveState,
    lock: &Lockfile,
    res: &Resolution,
    claimed: &mut HashSet<u64>,
    diags: &mut Vec<Diagnostic>,
) -> Binding {
    if let Some(id) = lock.channels.get(&ch.key) {
        return match live.channel(id.0) {
            Some(lc) => {
                let compatible = lc.kind.known().is_some_and(|k| k.convertible_to(ch.kind));
                if !compatible {
                    diags.push(
                        Diagnostic::error(
                            &ch.path,
                            format!(
                                "`{}` is a {} channel on Discord but a {} in the spec; Discord can't change that in place",
                                lc.name,
                                lc.kind.describe(),
                                ch.kind.as_str()
                            ),
                        )
                        .with_hint("give it a new key to create a new channel (the old one becomes unmanaged)"),
                    );
                }
                Binding::Locked(id.0)
            }
            None => {
                diags.push(
                    Diagnostic::warning(
                        &ch.path,
                        format!(
                            "lockfile says `{}` is {id}, but that channel no longer exists",
                            ch.key
                        ),
                    )
                    .with_hint("it will be created again"),
                );
                Binding::Recreate(id.0)
            }
        };
    }

    let parent_id = match &ch.parent {
        None => None,
        Some(p) => match res.channel(p).id() {
            Some(id) => Some(id),
            // Parent doesn't exist yet, so neither can this.
            None => return Binding::New,
        },
    };
    let candidates: Vec<u64> = live
        .channels
        .iter()
        .filter(|lc| !claimed.contains(&lc.id))
        .filter(|lc| lc.name == ch.name && lc.parent_id == parent_id)
        .filter(|lc| lc.kind.known().is_some_and(|k| k.convertible_to(ch.kind)))
        .map(|lc| lc.id)
        .collect();
    match candidates.as_slice() {
        [] => Binding::New,
        [id] => {
            claimed.insert(*id);
            Binding::Adopted(*id)
        }
        many => {
            diags.push(
                Diagnostic::error(
                    &ch.path,
                    format!(
                        "{} live channels match `{}`; can't tell which one this is",
                        many.len(),
                        ch.name
                    ),
                )
                .with_hint(format!(
                    "add the right one to concord.lock.json under channels.{} (IDs: {})",
                    ch.key,
                    join_ids(many)
                )),
            );
            Binding::New
        }
    }
}

fn join_ids(ids: &[u64]) -> String {
    ids.iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
