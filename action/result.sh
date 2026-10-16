#!/usr/bin/env bash
# Fail the job for errors, and for pending changes when asked to.
set -euo pipefail

case "$COMMAND-$CODE" in
  plan-0 | apply-0) exit 0 ;;
  plan-2)
    if [ "$FAIL_ON_CHANGES" = true ]; then
      echo "::error::The server no longer matches $SPEC (someone changed it in Discord). Put the change in the spec, or re-run apply to undo it."
      exit 1
    fi
    exit 0
    ;;
  *)
    echo "::error::concord $COMMAND failed (exit ${CODE:-?}); see the step above."
    exit 1
    ;;
esac
