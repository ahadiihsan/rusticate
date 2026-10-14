#!/usr/bin/env sh
# Agent hook (Stop): an agent may not declare a task finished while the fast
# gate fails. Exit code 2 returns the message to the agent so it keeps
# working; `stop_hook_active` (set when the agent is already continuing
# because of this hook) prevents an endless loop.
#
# Works with any agent harness that runs a command before stopping. The full
# gate (`cargo xtask verify`, with database tests) still runs in CI.
payload=$(cat)
case "$payload" in
  *'"stop_hook_active":true'* | *'"stop_hook_active": true'*) exit 0 ;;
esac

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}" || exit 0

# Nothing relevant changed in a git checkout: nothing to verify.
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  if git diff --quiet HEAD -- '*.rs' '*.toml' '*.sql' 2>/dev/null \
    && [ -z "$(git ls-files --others --exclude-standard -- '*.rs' '*.toml' '*.sql')" ]; then
    exit 0
  fi
fi

if ! output=$(cargo xtask verify --fast 2>&1); then
  printf '%s\n' "$output" | tail -n 60 >&2
  printf '\n`cargo xtask verify --fast` failed. Fix the cause named by the rule IDs above (see AGENTS.md) before finishing; do not weaken a check.\n' >&2
  exit 2
fi
exit 0
