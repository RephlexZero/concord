# concord

Manage a Discord server's roles and channels as code, Terraform-style. A YAML
spec in git describes the server; `concord plan` shows the difference from
the live server, and `concord apply` makes the server match.

## Quick start

1. **Create a bot.** In the [Discord Developer Portal](https://discord.com/developers/applications),
   create an application, add a bot, and copy its token. No privileged
   intents are needed.
2. **Invite it** with the `bot` scope and the **Administrator** permission:
   `https://discord.com/oauth2/authorize?client_id=<APP_ID>&scope=bot&permissions=8`.
   Administrator is recommended: Discord only lets a bot grant permissions it
   has, and a bot without it loses access to private channels it makes.
3. **Place its role** (named after the bot) above every role concord should
   manage, in Server Settings → Roles. `plan` reports it if it isn't.
4. **Provide the token** as `DISCORD_TOKEN`, in the environment or a `.env`
   file (git-ignored). concord never prints it.
5. **Import the server:**
   ```sh
   concord import --guild <SERVER_ID> --out server.yaml
   concord plan -f server.yaml        # → "No changes."
   ```
   Commit `server.yaml` and `concord.lock.json`.
6. **Change the spec, then apply it:**
   ```sh
   concord apply -f server.yaml --simulate   # dry run on an in-memory copy
   concord apply -f server.yaml              # shows the plan, asks, applies
   ```
   Commit the updated `concord.lock.json` afterwards.

## Commands

| Command | What it does |
|---|---|
| `concord validate [-f server.yaml]` | Check the spec offline. |
| `concord import --guild ID [--out server.yaml]` | Write a spec and lockfile from the live server. |
| `concord plan [-f server.yaml] [--prune]` | Show the changes. Exit code 0 = none, 2 = changes pending, 1 = error. |
| `concord apply [-f server.yaml] [--prune] [--yes]` | Show the plan, ask, back up the server, apply, then check a second plan is empty. Exit code 0 = done, 1 = error. |
| `concord apply --simulate [--state-file snap.json]` | Apply to an in-memory copy instead, and check it converges. Sends nothing and writes no files. |
| `concord snapshot --guild ID [--out snapshot.json]` | Save the live state as JSON. `plan --state-file` and `import --state-file` read it. |
| `concord lock mv OLD NEW` | Rename a key in the lockfile (after refactoring the spec). |
| `concord lock list` | Show every key and its Discord ID. |

Global flags: `--guild ID` (overrides the spec), `-v` (show requests),
`--no-color` (also `NO_COLOR=1`).

**Apply safety.** Before changing anything, `apply` saves the server to
`.concord/backups/` next to the spec (git-ignored). It saves the lockfile
after every change, so if a run stops partway (network, permissions), fix the
cause and run `apply` again; it picks up where it left off. Every change is
tagged "concord apply" in the server's audit log.

## How identity works

Every role, category and channel has a **key**. `concord.lock.json` maps keys
to Discord IDs and is committed to git. Change `name:` to rename something;
keep the key. If something has no lockfile entry but a live object with the
same name, type and category exists, concord **adopts** it instead of creating
a duplicate.

Anything on Discord that isn't in the spec is **unmanaged**. The plan lists
it, and concord leaves it alone unless you pass `--prune`. The server's rules
and community-updates channels are never deleted.

After `import`, refactor freely (templates, tag sets). If you change a key,
move its lockfile entry with `concord lock mv`. The plan should keep showing
no changes.

## Spec reference

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
  subteam:
    channels:
      - { key: main, type: text, name: "{slug}-main" }
      - { key: forum, type: forum, name: "{slug}-forum", tags: [base, "{subsystems}"] }

channels:                # uncategorised (always shown above categories)
  - { key: rules, type: text, name: rules }

categories:
  vd:                    # the category's key
    name: "vehicle-dynamics 🪄"
    template: subteam
    vars: { slug: vd, subsystems: [Kinematics, Tyres] }   # {key} is built in
    private: [Lead]      # optional
    channels: []         # extra channels after the template's
```

**Modes.** In `partial` mode (the default), fields left out are left as they
are on Discord. In `authoritative` mode, a field left out means Discord's
default (no topic, no colour, no permissions, no tags...). `@everyone`'s
permissions are only managed when written. `import` writes `authoritative`
specs.

**Channel types and fields.**

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
A channel with neither is "synced to category". Concord never touches
per-member overwrites.

**Tag renames.** Tags are matched by lockfile ID, then by name. To rename a
tag without losing it from existing posts, give it a key:
`{ key: wip, name: In Progress }`.

### Permission names

Use Discord's names (case and `-`/space are ignored), or these aliases:

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

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Tests use JSON fixtures of Discord API responses (`tests/fixtures`); nothing
touches the network. `tests/fixtures/tbre_plan.txt` is the expected plan for
`examples/tbre.yaml`; regenerate it with `UPDATE_GOLDEN=1 cargo test`.
