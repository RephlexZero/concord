//! Everything beyond roles and channels: server settings, the widget,
//! onboarding, the welcome screen, the Server Guide, join questions, AutoMod
//! and expressions (emoji, stickers, soundboard sounds).
//!
//! [`expand`] validates the spec's sections, [`convert`] reads Discord's
//! JSON, and [`diff`] produces [`crate::diff::Op::Community`] ops.

pub mod apply;
pub mod convert;
pub mod diff;
pub mod expand;
pub mod import;
pub mod model;
pub mod wire;

pub use model::{Desired, Live};
