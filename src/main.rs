mod cli;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::Parser;

use cli::{Cli, Command, LockCommand};
use concord::apply::{self, Applier};
use concord::diag::{has_errors, Diagnostic};
use concord::diff::{self, PlanOptions};
use concord::live::memory::MemoryApi;
use concord::live::twilight::TwilightApi;
use concord::live::{to_live, DiscordApi, Snapshot};
use concord::lock::{Lockfile, DEFAULT_LOCK_FILE};
use concord::render::{render_diagnostics, render_plan, Style};
use concord::{import, spec};

/// Exit codes, so CI can tell "changes pending" from "broken".
const EXIT_OK: u8 = 0;
const EXIT_ERROR: u8 = 1;
const EXIT_CHANGES: u8 = 2;

fn main() -> ExitCode {
    // clap exits with 2 on bad arguments, which would read as "changes
    // pending"; map it to 1.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() { EXIT_ERROR } else { EXIT_OK });
        }
    };
    let style = Style {
        color: !cli.no_color
            && std::env::var_os("NO_COLOR").is_none()
            && std::io::stdout().is_terminal(),
    };
    match run(&cli, style) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn run(cli: &Cli, style: Style) -> Result<u8> {
    match &cli.command {
        Command::Validate { file } => validate(file, cli, style),
        Command::Plan {
            file,
            lock,
            prune,
            state_file,
        } => plan(
            cli,
            style,
            file,
            lock.as_deref(),
            *prune,
            state_file.as_deref(),
        ),
        Command::Import {
            out,
            lock,
            force,
            state_file,
        } => import_cmd(
            cli,
            style,
            out,
            lock.as_deref(),
            *force,
            state_file.as_deref(),
        ),
        Command::Apply {
            file,
            lock,
            prune,
            yes,
            simulate,
            state_file,
        } => apply_cmd(
            cli,
            style,
            file,
            lock.as_deref(),
            ApplyFlags {
                prune: *prune,
                yes: *yes,
                simulate: *simulate,
            },
            state_file.as_deref(),
        ),
        Command::Snapshot { out } => {
            let guild = cli.guild.context("pass --guild <ID>")?;
            let snap = fetch(guild, cli.verbose > 0)?;
            write_json(out, &snap)?;
            eprintln!("Wrote {}", out.display());
            Ok(EXIT_OK)
        }
        Command::Lock { lock, command } => lock_cmd(lock, command),
    }
}

/// Parse and expand a spec, printing diagnostics. `None` if it has errors.
fn load_spec(
    file: &Path,
    style: Style,
) -> Result<Option<(concord::model::Desired, Vec<Diagnostic>)>> {
    let parsed = match spec::load(file) {
        Ok(s) => s,
        Err(spec::SpecError::Parse(d)) => {
            eprint!("{}", render_diagnostics(&[d], style));
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    };
    // Files (emoji, stickers, sounds) are relative to the spec, not the
    // current directory.
    let base = file.parent().unwrap_or_else(|| Path::new(""));
    let (desired, diags) = spec::expand_at(&parsed, base);
    if has_errors(&diags) {
        eprint!("{}", render_diagnostics(&diags, style));
        let n = diags.iter().filter(|d| d.is_error()).count();
        eprintln!("\n{} has {n} error(s).", file.display());
        return Ok(None);
    }
    Ok(Some((desired, diags)))
}

fn validate(file: &Path, cli: &Cli, style: Style) -> Result<u8> {
    let Some((desired, diags)) = load_spec(file, style)? else {
        return Ok(EXIT_ERROR);
    };
    eprint!("{}", render_diagnostics(&diags, style));
    if desired.guild_id.is_none() && cli.guild.is_none() {
        eprintln!("note: no guild_id in the spec; pass --guild when planning.");
    }
    let roles = desired.roles.len();
    let channels = desired.channels.len();
    println!(
        "{} is valid ({roles} roles, {channels} categories and channels).",
        file.display()
    );
    Ok(EXIT_OK)
}

fn lock_path(spec_file: &Path, explicit: Option<&Path>) -> PathBuf {
    explicit.map_or_else(
        || {
            spec_file
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(DEFAULT_LOCK_FILE)
        },
        Path::to_path_buf,
    )
}

fn plan(
    cli: &Cli,
    style: Style,
    file: &Path,
    lock: Option<&Path>,
    prune: bool,
    state_file: Option<&Path>,
) -> Result<u8> {
    let Some((desired, spec_diags)) = load_spec(file, style)? else {
        return Ok(EXIT_ERROR);
    };
    let guild = cli
        .guild
        .or(desired.guild_id)
        .context("no server to plan against: set guild_id in the spec or pass --guild")?;
    let lock_file = lock_path(file, lock);
    let lock = Lockfile::load(&lock_file)?;
    let snapshot = match state_file {
        Some(path) => read_snapshot(path, guild)?,
        None => fetch(guild, cli.verbose > 0)?,
    };
    let live = to_live(&snapshot);
    let mut plan = diff::plan(&desired, &live, &lock, &PlanOptions { prune });
    let mut diags = spec_diags;
    diags.append(&mut plan.diagnostics);
    plan.diagnostics = diags;

    print!("{}", render_plan(&plan, style));
    Ok(if plan.has_errors() {
        EXIT_ERROR
    } else if plan.has_changes() {
        EXIT_CHANGES
    } else {
        EXIT_OK
    })
}

#[derive(Clone, Copy)]
struct ApplyFlags {
    prune: bool,
    yes: bool,
    simulate: bool,
}

fn apply_cmd(
    cli: &Cli,
    style: Style,
    file: &Path,
    lock: Option<&Path>,
    flags: ApplyFlags,
    state_file: Option<&Path>,
) -> Result<u8> {
    let Some((desired, spec_diags)) = load_spec(file, style)? else {
        return Ok(EXIT_ERROR);
    };
    let guild = cli
        .guild
        .or(desired.guild_id)
        .context("no server to apply to: set guild_id in the spec or pass --guild")?;
    let lock_file = lock_path(file, lock);
    let lock = Lockfile::load(&lock_file)?;
    let runtime = runtime()?;
    let _guard = runtime.enter();
    let verbose = cli.verbose > 0;
    let job = ApplyJob {
        desired: &desired,
        spec_diags,
        lock,
        lock_file: &lock_file,
        backup_dir: &file
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(".concord/backups"),
        flags,
        style,
    };
    if flags.simulate {
        let snapshot = match state_file {
            Some(path) => read_snapshot(path, guild)?,
            None => runtime.block_on(TwilightApi::new(token()?, verbose).snapshot(guild))?,
        };
        let api = MemoryApi::new(snapshot);
        runtime.block_on(job.run(&api, guild))
    } else {
        let api = TwilightApi::new(token()?, verbose);
        runtime.block_on(job.run(&api, guild))
    }
}

struct ApplyJob<'a> {
    desired: &'a concord::model::Desired,
    spec_diags: Vec<Diagnostic>,
    lock: Lockfile,
    lock_file: &'a Path,
    backup_dir: &'a Path,
    flags: ApplyFlags,
    style: Style,
}

impl ApplyJob<'_> {
    async fn run<A: DiscordApi>(self, api: &A, guild: u64) -> Result<u8> {
        let ApplyFlags {
            prune,
            yes,
            simulate,
        } = self.flags;
        let opts = PlanOptions { prune };
        let snapshot = api.snapshot(guild).await?;
        let mut plan = diff::plan(self.desired, &to_live(&snapshot), &self.lock, &opts);
        let mut diags = self.spec_diags;
        diags.append(&mut plan.diagnostics);
        plan.diagnostics = diags;
        print!("{}", render_plan(&plan, self.style));
        if plan.has_errors() {
            eprintln!("\nNothing was changed.");
            return Ok(EXIT_ERROR);
        }
        if !plan.has_changes() {
            return Ok(EXIT_OK);
        }
        let total = plan.ops.len();

        if simulate {
            println!("\nSimulating on an in-memory copy of {}…", plan.guild_name);
        } else {
            if !yes && !confirm(&plan.guild_name, total)? {
                eprintln!("Cancelled. Nothing was changed.");
                return Ok(EXIT_ERROR);
            }
            std::fs::create_dir_all(self.backup_dir)
                .with_context(|| format!("can't create {}", self.backup_dir.display()))?;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let backup = self
                .backup_dir
                .join(format!("snapshot-{guild}-{stamp}.json"));
            write_json(&backup, &snapshot)?;
            println!("\nBacked up the server to {}", backup.display());
        }

        let mut applier = Applier::new(&plan);
        let mut save_error = None;
        let result = applier
            .run_all(api, &plan.ops, |i, op, lock| {
                if matches!(op, concord::diff::Op::Adopt { .. }) {
                    return;
                }
                println!("  ✓ [{}/{total}] {}", i + 1, apply::describe(op));
                if !simulate && save_error.is_none() {
                    save_error = lock.save(self.lock_file).err();
                }
            })
            .await;
        if !simulate {
            applier.lock().save(self.lock_file)?;
        }
        if let Some(e) = save_error {
            return Err(e.into());
        }
        if let Err((i, e)) = result {
            eprintln!(
                "  ✗ [{}/{total}] {}: {e}",
                i + 1,
                apply::describe(&plan.ops[i])
            );
            eprintln!(
                "\nStopped after {i} of {total} changes. {} records everything done so far; \
                 fix the problem and run apply again to finish.",
                self.lock_file.display()
            );
            return Ok(EXIT_ERROR);
        }

        // Check the result: a second plan should be empty.
        let after = api.snapshot(guild).await?;
        let again = diff::plan(self.desired, &to_live(&after), applier.lock(), &opts);
        if again.has_changes() {
            eprintln!(
                "\nwarning: applied {total} changes, but the server still differs from the spec:"
            );
            print!("{}", render_plan(&again, self.style));
            return Ok(EXIT_ERROR);
        }
        if simulate {
            println!("\nSimulation OK: {total} changes, and a second plan shows none left. Nothing was sent to Discord.");
        } else {
            println!("\nDone: applied {total} changes. The server matches the spec.");
        }
        Ok(EXIT_OK)
    }
}

/// Ask on the terminal. Refuses when there's no terminal to ask on.
fn confirm(guild_name: &str, n: usize) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("not asking for confirmation without a terminal; pass --yes to apply anyway");
    }
    print!("\nApply these {n} changes to {guild_name}? Type `yes` to continue: ");
    std::io::Write::flush(&mut std::io::stdout())?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(answer.trim() == "yes")
}

fn import_cmd(
    cli: &Cli,
    style: Style,
    out: &Path,
    lock: Option<&Path>,
    force: bool,
    state_file: Option<&Path>,
) -> Result<u8> {
    let lock_file = lock_path(out, lock);
    if !force {
        for p in [out, lock_file.as_path()] {
            if p.exists() {
                bail!(
                    "{} already exists; pass --force to overwrite it",
                    p.display()
                );
            }
        }
    }
    let snapshot = match (state_file, cli.guild) {
        (Some(path), guild) => read_snapshot(path, guild.unwrap_or(0))?,
        (None, Some(guild)) => fetch(guild, cli.verbose > 0)?,
        (None, None) => bail!("pass --guild <ID> to choose the server to import"),
    };
    let live = to_live(&snapshot);
    let imported = import::import(&live);
    eprint!("{}", render_diagnostics(&imported.warnings, style));
    std::fs::write(out, import::to_yaml(&imported.spec, &live.guild.name))
        .with_context(|| format!("can't write {}", out.display()))?;
    imported.lock.save(&lock_file)?;
    println!(
        "Wrote {} and {}.\nNext: `concord plan -f {}` should report no changes.",
        out.display(),
        lock_file.display(),
        out.display()
    );
    Ok(EXIT_OK)
}

fn lock_cmd(path: &Path, command: &LockCommand) -> Result<u8> {
    let mut lock = Lockfile::load(path)?;
    match command {
        LockCommand::Mv { from, to } => {
            let kind = lock.mv(from, to)?;
            lock.save(path)?;
            println!("Renamed {kind} `{from}` → `{to}` in {}.", path.display());
        }
        LockCommand::List => {
            for (section, map) in [
                ("roles", &lock.roles),
                ("channels", &lock.channels),
                ("tags", &lock.tags),
            ] {
                if map.is_empty() {
                    continue;
                }
                println!("{section}:");
                for (k, v) in map {
                    println!("  {k:<40} {v}");
                }
            }
        }
    }
    Ok(EXIT_OK)
}

fn read_snapshot(path: &Path, guild: u64) -> Result<Snapshot> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("can't read {}", path.display()))?;
    let snap: Snapshot = serde_json::from_str(&text)
        .with_context(|| format!("{} is not a valid snapshot", path.display()))?;
    if guild != 0 && snap.guild.id.0 != guild {
        bail!(
            "{} is a snapshot of server {}, not {guild}",
            path.display(),
            snap.guild.id
        );
    }
    Ok(snap)
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

fn fetch(guild: u64, verbose: bool) -> Result<Snapshot> {
    let token = token()?;
    let runtime = runtime()?;
    // twilight's rate limiter spawns a task when the client is built, so the
    // client must be created inside the runtime.
    let _guard = runtime.enter();
    let api = TwilightApi::new(token, verbose);
    Ok(runtime.block_on(api.snapshot(guild))?)
}

/// The bot token, from DISCORD_TOKEN (or `.env`). Never printed.
fn token() -> Result<String> {
    let _ = dotenvy::dotenv();
    match std::env::var("DISCORD_TOKEN") {
        Ok(t) if !t.trim().is_empty() => Ok(t.trim().to_owned()),
        _ => bail!("DISCORD_TOKEN is not set (export it, or put DISCORD_TOKEN=... in a .env file)"),
    }
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("can't write {}", path.display()))
}
