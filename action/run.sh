#!/usr/bin/env bash
# Run `concord plan` or `concord apply`, record the result as step outputs and
# a Markdown summary. Never fails itself, so the comment, lockfile and backup
# steps still run; result.sh decides whether the job fails.
set -uo pipefail

out="$RUNNER_TEMP/concord-output.txt"
summary="$RUNNER_TEMP/concord-summary.md"
spec_dir=$(dirname "$SPEC")
lockfile="${LOCK:-$spec_dir/concord.lock.json}"

args=(-f "$SPEC" --no-color)
[ -n "$LOCK" ] && args+=(--lock "$LOCK")
[ "$PRUNE" = true ] && args+=(--prune)
[ -z "$DISCORD_TOKEN" ] && unset DISCORD_TOKEN

case "$COMMAND" in
  plan)
    [ -n "$STATE_FILE" ] && args+=(--state-file "$STATE_FILE")
    concord plan "${args[@]}" >"$out" 2>&1
    code=$?
    ;;
  apply)
    concord apply "${args[@]}" --yes >"$out" 2>&1
    code=$?
    ;;
  *)
    echo "::error::command must be \`plan\` or \`apply\`, not \`$COMMAND\`"
    echo "Unknown command \`$COMMAND\`." >"$out"
    code=1
    ;;
esac
cat "$out"

changes=false
case "$COMMAND-$code" in
  plan-0) status="No changes: the server matches \`$SPEC\`." ;;
  plan-2) status="Changes pending. Merging applies them:"; changes=true ;;
  apply-0)
    if grep -q '^Done: applied' "$out"; then changes=true; status="Applied."; else status="Nothing to apply."; fi
    ;;
  *) status="**concord $COMMAND failed** (exit $code). Nothing more will be applied until it's fixed." ;;
esac

{
  echo "### concord $COMMAND: \`$SPEC\`"
  echo
  echo "$status"
  echo
  echo '```text'
  cat "$out"
  echo '```'
} >"$summary"
cat "$summary" >>"$GITHUB_STEP_SUMMARY"

{
  echo "exit-code=$code"
  echo "changes=$changes"
  echo "output-file=$out"
  echo "summary-file=$summary"
  echo "lockfile=$lockfile"
  echo "backup-dir=$spec_dir/.concord/backups"
} >>"$GITHUB_OUTPUT"
