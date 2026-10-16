# Security

concord holds a Discord bot token with Administrator on your server, so
security reports are taken seriously.

## Reporting a vulnerability

Please report privately through
[GitHub's private vulnerability reporting](https://github.com/RephlexZero/concord/security/advisories/new)
rather than a public issue. Include what you found, how to reproduce it, and
the impact. You should hear back within a week.

## Supported versions

Fixes go into the latest release; the `v0` action tag always points at it.

## Handling your token

- concord reads the token from `DISCORD_TOKEN` (or a `.env` file) and never
  prints or logs it.
- In GitHub Actions, store it as a secret and pass it with
  `discord-token: ${{ secrets.DISCORD_TOKEN }}`. Keep the workflow's
  `permissions` minimal, as in the README's example.
- If a token leaks, reset it in the Discord Developer Portal (Bot → Reset
  Token) and update the secret.

## What CI checks

Every push runs `cargo deny` (RustSec advisories, licences, crate sources),
clippy with warnings as errors, actionlint and shellcheck. Third-party actions
are pinned to commit SHAs. Release binaries ship with SHA-256 checksums and
GitHub build provenance attestations, and the action verifies the checksum
before running a download.
