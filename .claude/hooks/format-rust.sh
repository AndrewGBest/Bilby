#!/bin/sh
# After Claude edits a Rust file, format it, so a change never fails CI on layout.
# Reads the hook's JSON on standard input.
file=$(jq -r '.tool_input.file_path // empty')
case "$file" in
  *.rs) rustfmt --edition 2021 "$file" 2>/dev/null ;;
esac
exit 0
