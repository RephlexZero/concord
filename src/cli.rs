use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Manage a Discord server's roles and channels as code.
///
/// The bot token is read from the DISCORD_TOKEN environment variable (a
/// `.env` file in the current directory works too).
#[derive(Debug, Parser)]
#[command(name = "concord", version, about, long_about = None)]
pub struct Cli {
    /// Server (guild) ID; overrides `guild_id` in the spec.
    #[arg(long, global = true, value_name = "ID")]
    pub guild: Option<u64>,

    /// Show each request sent to Discord.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Disable coloured output (also: NO_COLOR=1).
    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check a spec offline, without contacting Discord.
    Validate {
        #[arg(short = 'f', long = "file", default_value = "server.yaml")]
        file: PathBuf,
    },

    /// Write a spec and lockfile describing the live server.
    Import {
        /// Where to write the spec.
        #[arg(long, default_value = "server.yaml")]
        out: PathBuf,
        /// Lockfile path [default: concord.lock.json next to the spec].
        #[arg(long)]
        lock: Option<PathBuf>,
        /// Overwrite existing files.
        #[arg(long)]
        force: bool,
        /// Read the server from a saved snapshot instead of Discord.
        #[arg(long, value_name = "JSON")]
        state_file: Option<PathBuf>,
    },

    /// Show what `apply` would change. Exit code 0 = no changes, 2 = changes
    /// pending, 1 = error.
    Plan {
        #[arg(short = 'f', long = "file", default_value = "server.yaml")]
        file: PathBuf,
        /// Lockfile path [default: concord.lock.json next to the spec].
        #[arg(long)]
        lock: Option<PathBuf>,
        /// Include deleting things that aren't in the spec.
        #[arg(long)]
        prune: bool,
        /// Plan against a saved snapshot instead of the live server.
        #[arg(long, value_name = "JSON")]
        state_file: Option<PathBuf>,
    },

    /// Save the live server's state as JSON (for backups, or `--state-file`).
    Snapshot {
        #[arg(long, default_value = "snapshot.json")]
        out: PathBuf,
    },

    /// Edit the lockfile.
    Lock {
        /// Lockfile path.
        #[arg(long, default_value = "concord.lock.json", global = true)]
        lock: PathBuf,
        #[command(subcommand)]
        command: LockCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum LockCommand {
    /// Rename a key, e.g. after moving a channel into a template:
    /// `concord lock mv systems-general systems.main`.
    Mv { from: String, to: String },
    /// List every key and its Discord ID.
    List,
}
