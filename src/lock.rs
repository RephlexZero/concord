//! `concord.lock.json`: maps spec keys to Discord IDs. Committed to git.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::snowflake::Sf;

pub const LOCK_VERSION: u32 = 1;
pub const DEFAULT_LOCK_FILE: &str = "concord.lock.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lockfile {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<Sf>,
    #[serde(default)]
    pub roles: BTreeMap<String, Sf>,
    /// Categories and channels.
    #[serde(default)]
    pub channels: BTreeMap<String, Sf>,
    /// Forum tags, keyed `<channel key>/<tag key>`.
    #[serde(default)]
    pub tags: BTreeMap<String, Sf>,
}

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("can't read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("{path} is not a valid lockfile: {source}")]
    Parse {
        path: String,
        source: serde_json::Error,
    },
    #[error(
        "{path} has lockfile version {found}; this concord understands version {LOCK_VERSION}"
    )]
    Version { path: String, found: u32 },
    #[error("can't write {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
    #[error("no lock entry `{0}`")]
    NoSuchKey(String),
    #[error("lock entry `{0}` already exists")]
    KeyExists(String),
}

pub fn tag_key(channel_key: &str, tag_key: &str) -> String {
    format!("{channel_key}/{tag_key}")
}

impl Lockfile {
    pub fn new(guild_id: u64) -> Self {
        Lockfile {
            version: LOCK_VERSION,
            guild_id: Some(Sf(guild_id)),
            ..Default::default()
        }
    }

    /// Load a lockfile; a missing file is an empty lockfile.
    pub fn load(path: &Path) -> Result<Self, LockError> {
        let display = path.display().to_string();
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Lockfile {
                    version: LOCK_VERSION,
                    ..Default::default()
                })
            }
            Err(source) => {
                return Err(LockError::Read {
                    path: display,
                    source,
                })
            }
        };
        let lock: Lockfile = serde_json::from_str(&text).map_err(|source| LockError::Parse {
            path: display.clone(),
            source,
        })?;
        if lock.version != LOCK_VERSION {
            return Err(LockError::Version {
                path: display,
                found: lock.version,
            });
        }
        Ok(lock)
    }

    /// Write atomically (temp file + rename), so a crash never leaves a
    /// half-written lockfile.
    pub fn save(&self, path: &Path) -> Result<(), LockError> {
        let err = |source| LockError::Write {
            path: path.display().to_string(),
            source,
        };
        let mut text = serde_json::to_string_pretty(self).expect("lockfile serialises");
        text.push('\n');
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(err)?;
        std::fs::rename(&tmp, path).map_err(err)
    }

    /// Rename a key (after refactoring the spec), including a channel's tags.
    pub fn mv(&mut self, from: &str, to: &str) -> Result<&'static str, LockError> {
        for (kind, map) in [("role", &mut self.roles), ("channel", &mut self.channels)] {
            if let Some(id) = map.get(from).copied() {
                if map.contains_key(to) {
                    return Err(LockError::KeyExists(to.to_owned()));
                }
                map.remove(from);
                map.insert(to.to_owned(), id);
                if kind == "channel" {
                    let prefix = format!("{from}/");
                    let moved: Vec<(String, Sf)> = self
                        .tags
                        .iter()
                        .filter(|(k, _)| k.starts_with(&prefix))
                        .map(|(k, v)| (k.clone(), *v))
                        .collect();
                    for (k, v) in moved {
                        self.tags.remove(&k);
                        self.tags.insert(format!("{to}/{}", &k[prefix.len()..]), v);
                    }
                }
                return Ok(kind);
            }
        }
        if let Some(id) = self.tags.remove(from) {
            self.tags.insert(to.to_owned(), id);
            return Ok("tag");
        }
        Err(LockError::NoSuchKey(from.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mv_moves_channel_and_its_tags() {
        let mut lock = Lockfile::new(1);
        lock.channels.insert("systems-general".into(), Sf(10));
        lock.tags.insert("systems-general/Open".into(), Sf(11));
        lock.tags.insert("other/Open".into(), Sf(12));
        assert_eq!(
            lock.mv("systems-general", "systems.main").unwrap(),
            "channel"
        );
        assert_eq!(lock.channels.get("systems.main"), Some(&Sf(10)));
        assert_eq!(lock.tags.get("systems.main/Open"), Some(&Sf(11)));
        assert_eq!(lock.tags.get("other/Open"), Some(&Sf(12)));
        assert!(lock.mv("nope", "x").is_err());
    }

    #[test]
    fn round_trips_through_json() {
        let mut lock = Lockfile::new(42);
        lock.roles.insert("lead".into(), Sf(7));
        let text = serde_json::to_string(&lock).unwrap();
        assert!(text.contains("\"7\""), "IDs are strings: {text}");
        assert_eq!(serde_json::from_str::<Lockfile>(&text).unwrap(), lock);
    }
}
