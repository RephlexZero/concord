# Changelog

All notable changes to concord are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and concord adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2026-10-17

### Changed

- The Server Guide is check-only. Discord refuses bot tokens on its endpoint
  ("Bots cannot use this endpoint"), so instead of planning a change that
  would fail, the plan warns with the steps to make by hand. Join questions,
  tested the same way, accept bot edits.

## [0.2.0] - 2026-10-17

### Added

- Server-wide configuration, as new top-level spec sections. Each is managed
  as a whole when present and left alone when absent:
  - `server`: name, description, verification level, default notifications,
    explicit content filter, locale, boost progress bar, the system, rules,
    community updates, safety alerts and AFK channels, AFK timeout, which
    system messages are posted, and the server widget. Fields left out are
    left alone in either mode.
  - `onboarding`: mode, default channels, and prompts with their options.
  - `welcome_screen`: description and up to 5 channels.
  - `guide`: the Server Guide's welcome message, to-dos and resources
    (undocumented Discord endpoint).
  - `join_questions`: the member verification form's rules and questions
    (undocumented Discord endpoint).
  - `automod`: keyword, spam, keyword preset, mention spam and member profile
    rules, with block, alert, timeout and block-interactions actions.
  - `emoji`, `stickers` and `sounds`: custom emoji, stickers and soundboard
    sounds, uploaded from files next to the spec, or managed by name only.
- `onboarding_option` on templates and categories adds an option to an
  onboarding prompt for each category, with template variables.
- Role `icon`: a unicode emoji role icon (needs boost level 2).
- `stage` and `media` channel types (previously listed as unmanaged).
- Channel fields `region` (voice and stage), `video_quality` (voice) and
  `post_slowmode` (forum and media).
- `import` writes the new sections from the live server; emoji, stickers and
  sounds are imported by name only.
- The plan checks the bot has Manage Server and Manage Expressions when the
  new sections need them, and that the server has the features they need
  (Community for the rules, updates and safety alerts channels; `ROLE_ICONS`
  for role icons).

### Changed

- The lockfile also maps onboarding prompts and options, AutoMod rules,
  emoji, stickers and sounds to Discord IDs, and records the SHA-256 of each
  uploaded file; a changed file is re-uploaded (delete and create).
- `concord lock mv` also renames AutoMod rule, emoji, sticker and sound
  keys.
- Unmanaged AutoMod rules, emoji, stickers and sounds are listed in the plan
  and deleted only with `--prune`.
- Unicode emoji are sent exactly as written, and compared without variation
  selectors; previously the selector was stripped, which Discord can reject
  (e.g. "⚖" for "⚖️").

### Fixed

- `--prune` no longer deletes live roles that the spec refers to by name
  (in `private`, `overrides`, onboarding, AutoMod or emoji) without defining
  them; they're listed as unmanaged with a note.

## [0.1.0] - 2026-10-16

First release: roles, categories, text, announcement, forum and voice
channels and their permissions from a YAML spec, with `validate`, `import`,
`plan`, `apply`, `snapshot` and `lock`, and a GitHub Action.

[0.2.1]: https://github.com/RephlexZero/concord/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/RephlexZero/concord/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/RephlexZero/concord/releases/tag/v0.1.0
