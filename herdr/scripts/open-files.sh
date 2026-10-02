#!/bin/bash
# 포커스된 herdr 패널의 폴더를 ggl 파일 트리로 연다.
# 패널에서 돌고 있는 프로그램(claude 등)의 폴더를 먼저 보고, 없으면 패널 셸의 폴더를 쓴다.
set -uo pipefail
herdr_bin="${HERDR_BIN_PATH:-herdr}"

dir=""
if [ -n "${HERDR_PLUGIN_CONTEXT_JSON:-}" ]; then
  dir="$(printf '%s' "$HERDR_PLUGIN_CONTEXT_JSON" | jq -r '.focused_pane_cwd // empty' 2>/dev/null)"
fi
if [ -z "$dir" ]; then
  dir="$("$herdr_bin" pane list 2>/dev/null \
    | jq -r 'first(.result.panes[] | select(.focused) | .foreground_cwd // .cwd) // empty' 2>/dev/null)"
fi
[ -d "$dir" ] || dir="$HOME"

ggl_open="$HOME/.local/bin/ggl-open"
[ -x "$ggl_open" ] || ggl_open="ggl-open"
exec "$ggl_open" --files "$dir"
