#![allow(dead_code)]

use std::path::PathBuf;

use concord::live::{to_live, Snapshot};
use concord::lock::Lockfile;
use concord::model::LiveState;

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn live_fixture(name: &str) -> LiveState {
    let text = std::fs::read_to_string(root().join("tests/fixtures").join(name)).unwrap();
    let snap: Snapshot = serde_json::from_str(&text).unwrap();
    to_live(&snap)
}

pub fn lock_fixture(name: &str) -> Lockfile {
    Lockfile::load(&root().join("tests/fixtures").join(name)).unwrap()
}
