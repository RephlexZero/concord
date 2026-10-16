# concord

[![CI](https://github.com/RephlexZero/concord/actions/workflows/ci.yml/badge.svg)](https://github.com/RephlexZero/concord/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/RephlexZero/concord)](https://github.com/RephlexZero/concord/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Your Discord server's roles, channels and permissions as code.** Describe
the server in a YAML file, review changes as pull requests, and let concord
make Discord match, Terraform-style.

```text
$ concord plan -f server.yaml

Roles
  + create   @Mod  #3498db, hoisted
  ~ update   @Lead
      color        #9b59b6 → #e91e63

Channels
  + create   category driverless 🤖
  + create   #wiki  (forum)
      tags: Guide, Process, Reference, Onboarding
      perms: @everyone deny CREATE_POSTS
      perms: Lead allow CREATE_POSTS
  ~ update   #systems-general
      name         systems-general → systems-main
  ~ perms    category leads
      + Director       allow +VIEW_CHANNEL

Plan: 9 to create, 12 to update, 2 to reorder.
```

## Why

Servers that grow past a handful of channels drift: permissions get edited
in five places, categories end up half-synced, and nobody remembers why a
role can see what it can. Server Settings has no review, no history and no
undo. concord gives you all three:

- **Plan before you change anything.** Every change is shown field by field,
  including the permission bits each channel ends up with.
- **Review it like code.** The spec lives in git; the GitHub Action posts the
  plan on every pull request and applies it on merge.
- **Catch drift.** A scheduled plan fails when someone changed the server by
  hand.

## Features

- **Roles, categories, text, announcement, forum and voice channels**: names,
  colours, hoisting, permissions, topics, slowmode, bitrate, forum tags,
  layouts, sort order, default reactions and more.
- **Templates and tag sets.** Define a category layout once (say, a chat
  channel and a forum per team) and stamp it out with variables.
- **Readable permissions.** `private: [Lead]` hides a channel from everyone
  else; `overrides` layer on top of the category's permissions, bit by bit.
  Forum permissions use the names the Discord client shows (`CREATE_POSTS`).
- **Safe forum tags.** Existing tags are always sent back with their IDs, so
  posts never lose their tags; renames are tracked by key.
- **Stable identity.** A lockfile maps spec keys to Discord IDs, so renaming
  in the spec renames on Discord (keeping history) instead of recreating.
  Objects that already exist are adopted by name.
- **Non-destructive by default.** Anything not in the spec is listed and left
  alone; deleting needs `--prune`. Rules and community channels are never
  deleted.
- **Careful apply.** A backup of the server before every run, the lockfile
  saved after every change (a failed run resumes where it stopped), a second
  plan afterwards to prove the server matches, and every change labelled
  "concord apply" in the audit log.
- **Dry runs.** `apply --simulate` applies the plan to an in-memory copy of
  the server and checks it converges, without sending anything.
- **Import.** `concord import` writes a spec and lockfile from an existing
  server, which plans to zero changes.

## Install

Download a binary for Linux, macOS or Windows from the
[latest release](https://github.com/RephlexZero/concord/releases/latest).
Each archive has a SHA-256 checksum and a
[build provenance attestation](https://docs.github.com/actions/security-for-github-actions/using-artifact-attestations):

```sh
gh attestation verify concord-x86_64-unknown-linux-musl.tar.gz --repo RephlexZero/concord
```

Or build it (Rust 1.89+):

```sh
cargo install --git https://github.com/RephlexZero/concord --locked
```

## Quick start

1. **Create a bot.** In the [Discord Developer Portal](https://discord.com/developers/applications),
   create an application and copy the bot's token (Bot → Reset Token). No
   privileged intents are needed. Untick **Public Bot** so only you can add it.
2. **Invite it** with the `bot` scope and the **Administrator** permission:
   `https://discord.com/oauth2/authorize?client_id=<APP_ID>&scope=bot&permissions=8`.
   Discord only lets a bot grant permissions it has, and a bot without
   Administrator loses access to private channels it creates.
3. **Move its role** (named after the bot) above every role concord should
   manage, in Server Settings → Roles. `plan` tells you if it isn't.
4. **Provide the token** as `DISCORD_TOKEN`, in the environment or a `.env`
   file. concord never prints it.
5. **Import the server**, then commit both files:
   ```sh
   concord import --guild <SERVER_ID> --out server.yaml
   concord plan -f server.yaml             # → "No changes."
   ```
6. **Edit the spec and apply it:**
   ```sh
   concord plan  -f server.yaml            # what would change
   concord apply -f server.yaml --simulate # dry run against a copy
   concord apply -f server.yaml            # shows the plan, asks, applies
   ```
   Commit the updated `concord.lock.json` afterwards.

## GitHub Action

The recommended setup keeps your server's spec in its **own (private)
repository** and uses concord as an action. Pull requests show the plan;
merging applies it.

```text
my-discord/
├── server.yaml
├── concord.lock.json
└── .github/workflows/discord.yml
```

```yaml
# .github/workflows/discord.yml
name: discord
on:
  pull_request:
  push:
    branches: [main]
  schedule:
    - cron: "0 7 * * *"   # daily drift check
  workflow_dispatch:

permissions: {}

concurrency:
  group: discord
  cancel-in-progress: false

jobs:
  plan:
    if: github.event_name == 'pull_request' || github.event_name == 'schedule'
    runs-on: ubuntu-latest
    permissions:
      contents: read
      pull-requests: write   # for the plan comment
    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false
      - uses: RephlexZero/concord@v0
        with:
          command: plan
          discord-token: ${{ secrets.DISCORD_TOKEN }}
          fail-on-changes: ${{ github.event_name == 'schedule' }}

  apply:
    if: github.ref == 'refs/heads/main' && (github.event_name == 'push' || github.event_name == 'workflow_dispatch')
    runs-on: ubuntu-latest
    environment: discord   # optional: require approval per apply
    permissions:
      contents: write        # to commit the updated lockfile
    steps:
      - uses: actions/checkout@v7
        with:
          ref: main          # a queued run sees the previous run's lockfile
      - uses: RephlexZero/concord@v0
        with:
          command: apply
          discord-token: ${{ secrets.DISCORD_TOKEN }}
```

Add the bot token as a repository secret named `DISCORD_TOKEN`. To approve
each apply by hand, add required reviewers to the `discord` environment
(Settings → Environments). If `main` is protected, allow
`github-actions[bot]` to push, so the lockfile can be committed.

| Input | Default | |
|---|---|---|
| `command` | (required) | `plan` or `apply`. |
| `spec` | `server.yaml` | Path to the spec. |
| `lock` | next to the spec | Lockfile path. |
| `discord-token` | | The bot token, from a secret. |
| `prune` | `false` | Also delete what isn't in the spec. |
| `fail-on-changes` | `false` | `plan`: fail if the server differs (drift checks). |
| `comment` | `true` | `plan` on a PR: post the plan as a sticky comment. |
| `commit-lockfile` | `true` | `apply`: commit and push the updated lockfile. |
| `upload-backup` | `true` | `apply`: keep the pre-apply backup as an artifact (90 days). |
| `state-file` | | `plan`: use a snapshot instead of Discord (no token). |
| `version` | the action's ref | A release like `v0.1.0`. Non-release refs build from source. |

Outputs: `exit-code`, `changes` (`true`/`false`) and `output-file`. Every run
also writes the plan or apply log to the job summary.

## Commands

| Command | What it does |
|---|---|
| `concord validate [-f server.yaml]` | Check the spec offline. |
| `concord import --guild ID [--out server.yaml]` | Write a spec and lockfile from the live server. |
| `concord plan [-f server.yaml] [--prune]` | Show the changes. Exit code 0 = none, 2 = changes pending, 1 = error. |
| `concord apply [-f server.yaml] [--prune] [--yes]` | Show the plan, ask, back up, apply, then check that a second plan is empty. Exit code 0 = done, 1 = error. |
| `concord apply --simulate [--state-file snap.json]` | Apply to an in-memory copy and check it converges. Sends nothing, writes no files. |
| `concord snapshot --guild ID [--out snapshot.json]` | Save the live state as JSON. `plan`, `import` and `apply --simulate` accept it as `--state-file`. |
| `concord lock mv OLD NEW` | Rename a key in the lockfile, after refactoring the spec. |
| `concord lock list` | Show every key and its Discord ID. |

Global flags: `--guild ID` (overrides the spec), `-v` (log requests),
`--no-color` (also `NO_COLOR=1`).

## How it works

**Identity.** Every role, category and channel has a **key**.
`concord.lock.json` maps keys to Discord IDs; commit it. Change `name:` to
rename something and keep the key. With no lockfile entry, a live object with
the same name, type and category is **adopted** rather than duplicated. After
refactoring keys (say, moving channels into a template), move their entries
with `concord lock mv`; the plan should still show no changes.

**Unmanaged objects.** Anything on Discord that isn't in the spec is listed
in the plan and left alone, unless you pass `--prune`.

**Order of operations.** Roles are created first, then categories, channels,
settings, tags, permissions and positions, so each step can refer to what the
previous ones created. Positions are compared as the client shows them, so
Discord's messy position numbers never cause phantom changes.

**Apply safety.** Before changing anything, `apply` saves the server to
`.concord/backups/` next to the spec (git-ignore it). The lockfile is saved
after every change; if a run stops partway (network, permissions), fix the
cause and run `apply` again to finish.

## Spec reference

A complete example: [`examples/racing-team.yaml`](examples/racing-team.yaml).

```yaml
guild_id: "1000000000004232083"
concord:
  mode: partial          # or: authoritative (see below)

roles:                   # highest first
  - { key: lead, name: Lead, color: "#e91e63", hoist: true, mentionable: false,
      permissions: [MANAGE_THREADS] }
  - { key: everyone, name: "@everyone", permissions: [VIEW_CHANNEL, SEND_MESSAGES] }

tag_sets:                # reusable forum tag lists
  base: [Question, Issue, Resolved]

templates:               # reusable category contents
  team:
    channels:
      - { key: main, type: text, name: "{slug}-main" }
      - { key: forum, type: forum, name: "{slug}-forum", tags: [base, "{subsystems}"] }

channels:                # uncategorised (always shown above categories)
  - { key: rules, type: text, name: rules }

categories:
  vd:                    # the category's key
    name: "vehicle-dynamics 🪄"
    template: team
    vars: { slug: vd, subsystems: [Kinematics, Tyres] }   # {key} is built in
    private: [Lead]      # optional
    channels: []         # extra channels after the template's
```

Template channels get the key `<category>.<channel>`, e.g. `vd.forum`.

**Modes.** In `partial` mode (the default), fields left out are left as they
are on Discord. In `authoritative` mode, a field left out means Discord's
default (no topic, no colour, no permissions, no tags...). `@everyone`'s
permissions are only managed when written. `import` writes `authoritative`
specs.

**Channel fields.**

| Field | Types | Notes |
|---|---|---|
| `key`, `type`, `name` | all | `type`: `text`, `announcement`, `forum`, `voice`. Text, announcement and forum names must be lower case without spaces. |
| `topic` | text, announcement | |
| `guidelines` | forum | Post guidelines (Discord's forum topic). |
| `nsfw` | all | |
| `slowmode` | text, forum, voice | `30s`, `5m`, `1h`. |
| `bitrate`, `user_limit` | voice | |
| `auto_archive` | text, announcement, forum | `1h`, `1d`, `3d`, `1w`. |
| `tags` | forum | Up to 20; names up to 20 characters. Entries are a tag name, a tag set name, `"{var}"` (a list variable), or `{ name, emoji, moderated, key }`. |
| `require_tag` | forum | |
| `layout` | forum | `list` or `gallery`. |
| `sort` | forum | `activity` or `created`. |
| `default_reaction` | forum | An emoji, or `id:<custom emoji ID>`. |
| `private` | all | `[RoleA, RoleB]`: hidden from `@everyone`, visible to these roles. |
| `overrides` | all | `{ Role: { allow: [...], deny: [...] } }`; `@everyone` is allowed. |
| `sync` | channels in a category | `false`: don't start from the category's permissions. |

**Permissions.** A channel in a category starts from the category's
permissions; `private` and then `overrides` are layered on top, bit by bit.
A channel with neither is "synced to category". concord never touches
per-member overwrites.

**Tag renames.** Tags are matched by lockfile ID, then by name. To rename a
tag without losing it from existing posts, give it a key:
`{ key: wip, name: In Progress }`.

### Permission names

Use Discord's names (case, `-` and spaces are ignored), or these aliases:

| Alias | Discord permission | Where |
|---|---|---|
| `CREATE_POSTS` | `SEND_MESSAGES` | forums only |
| `SEND_IN_POSTS` | `SEND_MESSAGES_IN_THREADS` | forums only |
| `MANAGE_POSTS` | `MANAGE_THREADS` | forums only |
| `SEND_IN_THREADS` | `SEND_MESSAGES_IN_THREADS` | anywhere |
| `VIEW` | `VIEW_CHANNEL` | anywhere |
| `READ_HISTORY` | `READ_MESSAGE_HISTORY` | anywhere |
| `CREATE_INVITE` | `CREATE_INSTANT_INVITE` | anywhere |
| `MANAGE_SERVER` | `MANAGE_GUILD` | anywhere |
| `MANAGE_EXPRESSIONS`, `MANAGE_EMOJIS_AND_STICKERS` | `MANAGE_GUILD_EXPRESSIONS` | anywhere |
| `USE_SLASH_COMMANDS` | `USE_APPLICATION_COMMANDS` | anywhere |
| `USE_VOICE_ACTIVITY` | `USE_VAD` | anywhere |
| `USE_ACTIVITIES` | `USE_EMBEDDED_ACTIVITIES` | anywhere |
| `TIMEOUT_MEMBERS` | `MODERATE_MEMBERS` | anywhere |

Plan output and `import` show forum permissions with the forum names
(`CREATE_POSTS`, `SEND_IN_THREADS`, `MANAGE_POSTS`). Unknown future bits
appear as `BIT_<n>` and round-trip unchanged.

## Limitations

- Not managed: members and their roles, per-member overwrites, emoji,
  stickers, webhooks, events, onboarding, AutoMod, server settings, role
  icons and gradient colours (only the primary colour is set).
- Stage and media channels are listed as unmanaged.
- Roles at or above the bot's own role can't be changed (a Discord rule);
  the plan explains how to fix it.

## Development

```sh
cargo test                                 # no network: fixtures + an in-memory Discord
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo deny check                           # advisories, licences, sources
```

Tests run against JSON fixtures of Discord API responses (`tests/fixtures`)
and `MemoryApi`, an in-memory Discord that enforces the rules concord has
tripped over on real servers. `tests/fixtures/team_plan.txt` is the expected
plan for `examples/racing-team.yaml`; regenerate it with
`UPDATE_GOLDEN=1 cargo test`.

Releases: bump `version` in `Cargo.toml`, commit, and push a matching tag
(`v0.2.0`). The release workflow builds and attests the binaries, publishes
the release and moves the `v0` tag.

## Security

See [SECURITY.md](SECURITY.md) to report a vulnerability.

## License

[MIT](LICENSE)
