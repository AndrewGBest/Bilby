#!/bin/sh
# Before Claude says it is done: the two quick checks CI would fail on. Only when
# code or interface files have changed, and only once per stop, so it cannot loop.
input=$(cat)
[ "$(printf '%s' "$input" | jq -r '.stop_hook_active // false')" = "true" ] && exit 0
cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0
git status --porcelain 2>/dev/null | grep -qE '\.(rs|slint)$' || exit 0

problems=""
cargo fmt --all -- --check >/dev/null 2>&1 || problems="${problems}- 'cargo fmt --all' has not been run.
"
python3 scripts/i18n.py --check >/dev/null 2>&1 || problems="${problems}- Interface text changed: run 'python3 scripts/i18n.py' and add the missing translations.
"
[ -z "$problems" ] && exit 0
printf 'Not finished yet:\n%s' "$problems" >&2
exit 2
