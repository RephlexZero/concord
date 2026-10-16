#!/usr/bin/env bash
# Commit and push the lockfile if apply changed it. Runs even after a failed
# apply: the lockfile records what was done, so the next run resumes.
set -euo pipefail

if [ ! -f "$LOCKFILE" ]; then exit 0; fi
if git diff --quiet -- "$LOCKFILE" && git ls-files --error-unmatch "$LOCKFILE" >/dev/null 2>&1; then
  echo "Lockfile unchanged."
  exit 0
fi
git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git add -- "$LOCKFILE"
git commit -m "concord: update $LOCKFILE after apply [skip ci]"
git push origin "HEAD:$GITHUB_REF"
