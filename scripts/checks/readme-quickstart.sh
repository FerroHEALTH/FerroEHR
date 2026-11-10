#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The README's quick start is what CI runs (no specification governs this:
# our own design, after FerroPIX's tour). docker/smoke-test.sh, which the
# containers workflow runs against the standalone docker-compose.yml, holds
# the quick start's commands between `# ANCHOR: <name>` and
# `# ANCHOR_END: <name>` lines. GitHub renders no include, so README.md
# carries a copy of each, a fenced block under a `<!-- smoke:<name> -->` line,
# and this guard refuses a copy that differs from its anchor by a byte, an
# anchor with no copy, and a copy of an anchor the script lacks.
#
#   scripts/checks/readme-quickstart.sh               check README.md
#   scripts/checks/readme-quickstart.sh --self-test   prove each refusal
#
# Exit 0 when every copy agrees, 1 naming each refusal, 2 on a usage error.
set -euo pipefail

# anchor_body SCRIPT NAME: the lines between `# ANCHOR: NAME` and its end.
anchor_body() {
  awk -v name="$2" '$0 == "# ANCHOR_END: " name { p = 0 } p { print } $0 == "# ANCHOR: " name { p = 1 }' "$1"
}

# readme_body README NAME: the fenced block under `<!-- smoke:NAME -->`.
readme_body() {
  awk -v name="$2" '
    $0 == "<!-- smoke:" name " -->" { want = 1; next }
    want && $0 ~ /^```/ { want = 0; p = 1; next }
    p && $0 == "```" { exit }
    p { print }' "$1"
}

# check SCRIPT README: every anchor of SCRIPT shown verbatim in README.
check() {
  local script=$1 readme=$2 failed=0 name opened
  opened="$(sed -n 's/^# ANCHOR: \([a-z-]*\)$/\1/p' "$script" | sort)"
  if [[ -z "$opened" ]]; then
    echo "readme-quickstart: $script holds no anchor." >&2
    return 1
  fi
  while IFS= read -r name; do
    if ! grep -qx "<!-- smoke:$name -->" "$readme"; then
      echo "readme-quickstart: $readme shows no copy of the anchor '$name' of $script." >&2
      failed=1
    elif [[ "$(readme_body "$readme" "$name")" != "$(anchor_body "$script" "$name")" ]]; then
      echo "readme-quickstart: $readme's copy of the anchor '$name' differs from $script." >&2
      failed=1
    fi
  done <<< "$opened"
  while IFS= read -r name; do
    [[ -z "$name" ]] && continue
    if ! grep -qx "$name" <<< "$opened"; then
      echo "readme-quickstart: $readme shows the anchor '$name', which $script lacks." >&2
      failed=1
    fi
  done < <(sed -n 's/^<!-- smoke:\([a-z-]*\) -->$/\1/p' "$readme")
  return "$failed"
}

self_test() {
  local work fails=0
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' RETURN
  printf '%s\n' 'echo before' '# ANCHOR: run' 'curl -s http://x/' '# ANCHOR_END: run' > "$work/s.sh"
  printf '%s\n' '<!-- smoke:run -->' '```shell' 'curl -s http://x/' '```' > "$work/agree.md"
  printf '%s\n' '<!-- smoke:run -->' '```shell' 'curl -s http://y/' '```' > "$work/differs.md"
  printf '%s\n' 'no copy' > "$work/missing.md"
  printf '%s\n' '<!-- smoke:run -->' '```shell' 'curl -s http://x/' '```' '<!-- smoke:gone -->' '```shell' 'true' '```' > "$work/extra.md"
  check "$work/s.sh" "$work/agree.md" 2> /dev/null || { echo "readme-quickstart: self-test failed: an agreeing copy was refused." >&2; fails=1; }
  for bad in differs missing extra; do
    if check "$work/s.sh" "$work/$bad.md" 2> /dev/null; then
      echo "readme-quickstart: self-test failed: the $bad copy passed." >&2
      fails=1
    fi
  done
  [[ "$fails" == 0 ]] || exit 1
  echo "readme-quickstart: self-test OK."
}

case "${1:-}" in
  --self-test) self_test ;;
  "")
    cd "$(dirname "${BASH_SOURCE[0]}")/../.."
    check docker/smoke-test.sh README.md
    echo "readme-quickstart: README.md shows every step of docker/smoke-test.sh's quick start as CI runs it."
    ;;
  *) echo "usage: $0 [--self-test]" >&2; exit 2 ;;
esac
