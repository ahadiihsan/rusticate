#!/usr/bin/env sh
# Agent hook (PostToolUse on Edit/Write): format the edited Rust file so
# agents never fight rustfmt. Never blocks; formatting errors surface in the
# Stop gate instead. Reads the hook payload (JSON) on stdin.
command -v jq >/dev/null 2>&1 || exit 0
file=$(jq -r '.tool_input.file_path // empty' 2>/dev/null) || exit 0
case "$file" in
  *.rs) rustfmt --edition 2024 --quiet "$file" >/dev/null 2>&1 || true ;;
esac
exit 0
