# concord

[![CI](https://github.com/RephlexZero/concord/actions/workflows/ci.yml/badge.svg)](https://github.com/RephlexZero/concord/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/RephlexZero/concord)](https://github.com/RephlexZero/concord/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Your Discord server's roles, channels, permissions and settings as code.**
Describe the server in a YAML file, review changes as pull requests, and let
concord make Discord match, Terraform-style.

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

- **Roles, categories, text, announcement, forum, media, voice and stage
  channels**: names, colours, role icons, hoisting, permissions, topics,
  slowmode, bitrate, voice regions, forum tags, layouts, sort order, default
  reactions and more.
- **Server-wide configuration**: server settings and the widget, onboarding,
  the welcome screen, the Server Guide, join questions, AutoMod rules, custom
  emoji, stickers and soundboard sounds.
- **Templates and tag sets.** Define a category layout once (say, a chat
  channel and a forum per team) and stamp it out with variables. A template
  can also add each team to an onboarding question.
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

**Identity.** Every role, category, channel, AutoMod rule, emoji, sticker
and sound has a **key**. `concord.lock.json` maps keys to Discord IDs (also
forum tags and onboarding prompts and options) and records the SHA-256 of
every file uploaded as an emoji, sticker or sound; commit it. Change `name:`
to rename something and keep the key. With no lockfile entry, a live object
with the same name (and, for channels, type and category) is **adopted**
rather than duplicated. After refactoring keys (say, moving channels into a
template), move their entries with `concord lock mv`; the plan should still
show no changes.

**Unmanaged objects.** Anything on Discord that isn't in the spec is listed
in the plan and left alone, unless you pass `--prune`.

**Order of operations.** Roles are created first, then categories, channels,
channel settings, tags, permissions and positions, then server-wide configuration,
then deletes, so each step can refer to what the previous ones created.
Positions are compared as the client shows them, so Discord's messy position
numbers never cause phantom changes.

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

**Role fields.**

| Field | Notes |
|---|---|
| `key`, `name` | |
| `color` | `"#e91e63"`; `0` for none. |
| `hoist`, `mentionable` | |
| `permissions` | A list of [permission names](#permission-names). |
| `icon` | A unicode emoji shown next to members' names; `""` removes it. Needs boost level 2 (Discord's `ROLE_ICONS` feature): the plan fails without it. Image icons aren't supported. |

`@everyone` can only have `permissions`, and must be the last role.

**Channel fields.**

| Field | Types | Notes |
|---|---|---|
| `key`, `type`, `name` | all | `type`: `text`, `announcement`, `forum`, `media`, `voice`, `stage`. Text, announcement, forum and media names must be lower case without spaces. |
| `topic` | text, announcement | |
| `guidelines` | forum, media | Post guidelines (Discord's forum topic). |
| `nsfw` | all | |
| `slowmode` | text, forum, media, voice, stage | `30s`, `5m`, `1h`; at most `6h`. |
| `post_slowmode` | forum, media | Slowmode inside each post; at most `6h`. |
| `bitrate`, `user_limit` | voice, stage | |
| `region` | voice, stage | Voice region, e.g. `rotterdam`; `""` for automatic. |
| `video_quality` | voice | `auto` or `full`. |
| `auto_archive` | text, announcement, forum, media | `1h`, `1d`, `3d`, `1w`. |
| `tags` | forum, media | Up to 20; names up to 20 characters. Entries are a tag name, a tag set name, `"{var}"` (a list variable), or `{ name, emoji, moderated, key }`. |
| `require_tag` | forum, media | |
| `layout` | forum | `list` or `gallery`. |
| `sort` | forum, media | `activity` or `created`. |
| `default_reaction` | forum, media | An emoji, or `id:<custom emoji ID>`. |
| `private` | all | `[RoleA, RoleB]`: hidden from `@everyone`, visible to these roles. |
| `overrides` | all | `{ Role: { allow: [...], deny: [...] } }`; `@everyone` is allowed. |
| `sync` | channels in a category | `false`: don't start from the category's permissions. |

Discord always shows voice and stage channels below the others in a
category, so list them last (`validate` warns otherwise).

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

### Server-wide configuration

Beyond roles and channels, a spec can have these top-level sections:
`server`, `onboarding`, `welcome_screen`, `guide`, `join_questions`,
`automod`, `emoji`, `stickers` and `sounds`.

- **Left out: left alone.** A section that isn't in the spec isn't read or
  changed, in either mode.
- **Present: managed as a whole.** The spec is the complete onboarding,
  welcome screen, Server Guide or set of join questions; what's on Discord
  is replaced to match. AutoMod rules, emoji, stickers and sounds that
  aren't in the spec are listed as unmanaged and only deleted with
  `--prune`.
- **Except `server`.** Each server setting left out is left alone, in either
  mode: settings like the verification level are too important to reset by
  omission.
- **References.** Channels and categories are referred to by key (`rules`,
  `vd.main`); roles by name (`Lead`), as in `private` and `overrides`. A role
  name may be one the spec doesn't manage, as long as only one live role has
  it. Emoji fields take a unicode emoji or `id:<custom emoji ID>`.
- **Permissions.** The bot needs Manage Server, and Manage Expressions for
  emoji, stickers and sounds (Administrator covers both); the plan checks.
  If a section in the spec can't be read from Discord, the plan fails and
  says which and why; remove the section to leave it alone.
- **Undocumented endpoints.** The Server Guide and join questions have no
  documented Discord API; concord uses the endpoints the Discord client
  uses, so treat these two sections as best-effort. Discord lets bots edit
  join questions, but **not the Server Guide**: concord reads it and the
  plan shows any differences as a warning with the steps to make by hand.

`import` writes every section it can read; the welcome screen and join
questions only if they've been set up.

#### Server settings

```yaml
server:
  name: Racing Team
  description: Formula Student team.
  verification_level: medium
  default_notifications: mentions
  content_filter: everyone
  locale: en-GB
  boost_progress_bar: true
  system_channel: welcome
  system_messages: { join: true, join_replies: false, boosts: true, setup_tips: false }
  rules_channel: rules
  updates_channel: mod-updates
  safety_alerts_channel: mod-updates
  afk_channel: afk
  afk_timeout: 5m
  widget: { enabled: true, channel: rules }
```

| Field | Values | Notes |
|---|---|---|
| `name` | 2-100 characters | |
| `description` | text | |
| `verification_level` | `none`, `low`, `medium`, `high`, `highest` | |
| `default_notifications` | `all`, `mentions` | For new members. |
| `content_filter` | `off`, `no_roles`, `everyone` | Whose media is scanned for explicit content. |
| `locale` | e.g. `en-GB` | |
| `boost_progress_bar` | `true`, `false` | |
| `system_channel` | text channel key, or `null` | Where Discord posts system messages. |
| `system_messages` | `join`, `join_replies`, `boosts`, `setup_tips`, `role_subscriptions`, `role_subscription_replies` | Each `true` or `false`; ones left out are left alone. |
| `rules_channel` | text channel key | Community servers only. Can't be unset. |
| `updates_channel` | text channel key | Community servers only; where Discord sends moderator updates. Can't be unset. |
| `safety_alerts_channel` | text channel key, or `null` | Community servers only. |
| `afk_channel` | voice or stage channel key, or `null` | |
| `afk_timeout` | `1m`, `5m`, `15m`, `30m`, `1h` | |
| `widget.enabled` | `true`, `false` | Required when `widget` is given. |
| `widget.channel` | channel key, or `null` | The channel the widget's invite points at. Left out: left alone. |

#### Onboarding

```yaml
onboarding:
  default_channels: [rules, announcements, general, social]   # a category counts its channels
  prompts:
    - key: team
      title: Which team are you in?
      single: true
      required: true
      options:
        - { title: Committee, roles: [Committee], channels: [committee] }
    - title: What are you into?
      type: dropdown
      options:
        - { title: Socials, emoji: "🎉", description: Trips and nights out, roles: [Socials] }

templates:
  team:
    onboarding_option:       # one option per category made from this template
      prompt: team           # the prompt's key
      title: "{team}"
      roles: ["{team}"]
      channels: ["{key}"]    # the category itself
    channels: []

categories:
  vd:
    name: "vehicle-dynamics 🪄"
    template: team
    vars: { team: Vehicle Dynamics }
```

| Field | Values | Notes |
|---|---|---|
| `enabled` | default `true` | |
| `mode` | `default` (default), `advanced` | Discord's onboarding mode: whether only default channels, or default channels and answers, count towards its requirements. |
| `default_channels` | channel and category keys | Channels every member sees. Discord requires at least 7 (a category counts as its channels); concord warns below that. |
| `prompts` | list | At most 15. |
| `prompts[].title` | 1-100 characters | |
| `prompts[].key` | default: from the title | `Which team are you in?` → `which-team-are-you-in`. Identifies the prompt across renames and for `onboarding_option`. |
| `prompts[].type` | `multiple_choice` (default), `dropdown` | |
| `prompts[].single` | default `false` | Only one answer allowed. |
| `prompts[].required` | default `false` | |
| `prompts[].in_onboarding` | default `true` | `false`: only shown in Channels & Roles. |
| `prompts[].options` | list | 1-50 per prompt, counting options added by templates. |
| `options[].title` | 1-50 characters | |
| `options[].key` | default: from the title | Unique within the prompt. |
| `options[].description` | up to 100 characters | |
| `options[].emoji` | emoji | |
| `options[].roles` | role names | Given to members who pick it. Not `@everyone`. |
| `options[].channels` | channel and category keys | Shown to members who pick it. Each option needs at least one role or channel. |

**Options from templates.** `onboarding_option` on a template adds an option
to the prompt with that key for every category made from the template, after
the prompt's own options, in category order. It takes the option fields above
plus `prompt`, and the category's variables (and `{key}`) work in every
field. A category's own `onboarding_option` replaces its template's. The spec
needs an `onboarding:` section with that prompt.

Prompts and options are matched to Discord's by lockfile, then by title.
Before changing a title, give the prompt or option a `key` so it keeps its
identity.

#### Welcome screen

```yaml
welcome_screen:
  description: Formula Student team.
  channels:
    - { channel: rules, description: Read this first, emoji: "📜" }
    - { channel: general, description: Say hello }
```

| Field | Values | Notes |
|---|---|---|
| `enabled` | default `true` | |
| `description` | text | |
| `channels` | at most 5 | Each has `channel` (key), `description` (required) and `emoji`. |

#### Server Guide

Check-only. Discord refuses bot tokens on the Server Guide's (undocumented)
endpoint, so concord compares the guide with the spec and, if they differ,
the plan shows a warning listing the changes to make in Server Settings →
Onboarding → Server Guide. Nothing is sent, and it doesn't count as a pending
change.

```yaml
guide:
  welcome:
    message: "Welcome [@username]! Start with the to-dos below."
    authors: ["100000000000000001"]
  actions:
    - { title: Read the rules, channel: rules }
    - { title: Say hello, channel: general, type: chat, emoji: "👋" }
    - { title: Find your team, channel: teams, description: Pick a team channel }
  resources:
    - { title: Handbook, channel: wiki, description: Everything else }
```

| Field | Values | Notes |
|---|---|---|
| `enabled` | default `true` | Discord needs at least 3 to-dos to enable the guide; concord warns. |
| `welcome.message` | up to 300 characters | `[@username]` becomes the new member. |
| `welcome.authors` | user IDs | Shown as the message's author. |
| `actions` | at most 5 | The to-dos: `title`, `channel` (key), `type`, `description`, `emoji`. |
| `actions[].type` | `view` (default), `chat` | Read the channel, or send a message in it. |
| `resources` | at most 7 | `title`, `channel` (key), `description`, `emoji`. |

Images on to-dos and resources can't be set from the spec; ones uploaded in
the client are kept as long as the item's channel and title stay the same.

#### Join questions

The form people fill in before they can join (member verification).
Best-effort: uses an undocumented endpoint.

```yaml
join_questions:
  description: Tell us who you are.
  rules:
    - Be kind.
    - No spam.
  questions:
    - { label: "What's your name?", placeholder: Jane Smith }
    - { type: paragraph, label: "Why do you want to join?", required: false }
    - { type: choice, label: "Which year are you in?", choices: [First, Second, Third or later] }
```

| Field | Values | Notes |
|---|---|---|
| `description` | text | |
| `rules` | up to 300 characters each | Rules the applicant must agree to. |
| `questions[].type` | `text` (default), `paragraph`, `choice` | |
| `questions[].label` | 1-300 characters | |
| `questions[].required` | default `true` | |
| `questions[].description`, `placeholder` | text | |
| `questions[].choices` | list | Required for `choice`, not allowed otherwise. |

concord sends back the form version it read, as the client does, so it
doesn't overwrite changes made since.

#### AutoMod

```yaml
automod:
  - key: slurs
    name: Block slurs
    trigger: keyword_preset
    presets: [slurs, sexual_content]
    actions: [{ block: true }, { alert: mod-log }]
  - key: invites
    name: No invite links
    trigger: keyword
    keywords: ["discord.gg/*"]
    allow: [discord.gg/ourteam]
    actions:
      - block: "Invite links aren't allowed here."
      - timeout: 10m
    exempt_roles: [Lead]
    exempt_channels: [promo]
  - key: mentions
    name: Mention spam
    trigger: mention_spam
    mention_limit: 10
    raid_protection: true
    actions: [{ block: true }]
```

| Field | Values | Notes |
|---|---|---|
| `key`, `name` | | Matched by lockfile, then name. |
| `enabled` | default `true` | |
| `trigger` | `keyword`, `spam`, `keyword_preset`, `mention_spam`, `member_profile` | Discord allows 6 `keyword` rules and 1 of each other kind. Changing a rule's trigger replaces it (delete and create). |
| `keywords` | `keyword`, `member_profile` | Words and phrases; `*` is a wildcard. At most 1000. |
| `regex` | `keyword`, `member_profile` | Rust-flavoured regular expressions; at most 10. `keyword` and `member_profile` rules need `keywords` or `regex`. |
| `allow` | `keyword`, `keyword_preset`, `member_profile` | Exceptions. |
| `presets` | `profanity`, `sexual_content`, `slurs` | Required for `keyword_preset`. |
| `mention_limit` | 1-50 | Required for `mention_spam`. |
| `raid_protection` | default `false` | `mention_spam` only. |
| `actions` | list | At least one; see below. |
| `exempt_roles` | role names | |
| `exempt_channels` | channel keys | |

Each action is exactly one of:

| Action | Triggers | |
|---|---|---|
| `block: true` or `block: "text"` | all | Block the message; the text is shown to the sender. |
| `alert: <channel key>` | all | Post an alert in that channel. |
| `timeout: 10m` | `keyword`, `mention_spam`, `member_profile` | Time the member out, 1s to 4 weeks. |
| `block_interactions: true` | `member_profile` | Stop the member interacting until they fix their profile. |

#### Emoji, stickers and sounds

```yaml
emoji:
  - { key: logo, name: team_logo, file: assets/logo.png }
  - { key: lead, name: lead_only, file: assets/lead.gif, roles: [Lead] }
  - { key: legacy, name: old_logo }        # no file: managed by name only
stickers:
  - { key: gg, name: Good Game, file: assets/gg.png, emoji: "🏁", description: Well raced }
sounds:
  - { key: horn, name: Air horn, file: assets/horn.mp3, volume: 0.6, emoji: "📯" }
```

| Field | Values | Notes |
|---|---|---|
| `emoji[].name` | 2-32 letters, digits or underscores | |
| `emoji[].file` | PNG, GIF, JPEG or WebP, at most 256 KiB | |
| `emoji[].roles` | role names | Only these roles can use it; empty: everyone. |
| `stickers[].name` | 2-30 characters | |
| `stickers[].file` | PNG, APNG, GIF or Lottie JSON, at most 512 KiB | |
| `stickers[].emoji` | an emoji | Required: the emoji Discord suggests it for. |
| `stickers[].description` | text | |
| `sounds[].name` | 2-32 characters | |
| `sounds[].file` | MP3 or OGG, at most 512 KiB | Discord also limits sounds to about 5 seconds; concord doesn't check. |
| `sounds[].volume` | 0.0 to 1.0, default 1.0 | |
| `sounds[].emoji` | emoji | |

Every entry has a `key`, and is matched by lockfile, then name. `file` is a
path relative to the spec.

- **Without `file`**, an existing emoji, sticker or sound is managed by name
  only: its name, roles, description, emoji and volume. If it doesn't exist,
  the plan fails, since there's nothing to upload. `import` writes entries
  this way, as it doesn't download files.
- **With `file`**, concord uploads it and keeps its SHA-256 in the lockfile.
  When the file changes, Discord can't replace the image or sound in place,
  so the plan deletes it and uploads the new file (it gets a new ID). An
  entry adopted by name, with no hash recorded yet, is assumed to match its
  file.
- Emoji managed by an integration (Twitch, for example) are ignored.

## Limitations

- Not managed: members and their roles, per-member overwrites, scheduled
  events, webhooks, images on onboarding options and Server Guide items,
  Discovery settings, gradient role colours (only the primary colour is set),
  image role icons, the server's icon and banner, turning Community on or
  off, and the moderator 2FA requirement (only the owner can change it).
- The Server Guide can only be checked, not changed: Discord doesn't let bots
  edit it. Join questions use an undocumented endpoint and may break when
  Discord changes it.
- Emoji fields refer to custom emoji by ID (`id:<ID>`), not by their key in
  `emoji:`.
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
