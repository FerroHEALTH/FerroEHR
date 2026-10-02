#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# scripts/checks/ferroterm-index-layout.sh — the FerroTERM the sandbox posture
# pins reads the index the box carries (#3520).
#
# FerroTERM refuses an artifact whose layout is not its own ("artifact layout
# Some("6"), expected "7""), and the box's index is built off-box from
# licensed releases, so a pin bump that moves the layout takes the sandbox
# down at the release's `sandbox` leg unless the index is rebuilt first. The
# v4.3.2 cut did exactly that. This check runs on the pull request instead:
#
#   * the pinned version is the `ghcr.io/ferrohealth/ferroterm:X.Y.Z` default
#     in deploy/hosted/docker-compose.yml;
#   * the layout that version reads is `LAYOUT_VERSION` in FerroTERM's
#     crates/concept-store/src/tables.rs at tag vX.Y.Z (a public repository);
#   * the layout the box carries is FERROTERM_INDEX_LAYOUT in
#     deploy/hosted/ferroterm-index.env, which the operator updates when the
#     index is rebuilt.
#
# A difference fails with the remedy: rebuild both indexes with that version's
# ferroterm-build, place them on the box, and update the record.
#
# Usage: scripts/checks/ferroterm-index-layout.sh [--self-test]
#   --self-test runs the comparison against a stubbed FerroTERM source: an
#   agreeing layout passes, a differing one and an unreadable one fail.
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
compose="${FERROTERM_LAYOUT_COMPOSE:-$root/deploy/hosted/docker-compose.yml}"
record="${FERROTERM_LAYOUT_RECORD:-$root/deploy/hosted/ferroterm-index.env}"
source_base="${FERROTERM_LAYOUT_SOURCE:-https://raw.githubusercontent.com/FerroHEALTH/FerroTERM}"

if [[ "${1:-}" == "--self-test" ]]; then
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  mkdir -p "$work/v9.9.9/crates/concept-store/src"
  printf 'pub const LAYOUT_VERSION: &str = "7";\n' > "$work/v9.9.9/crates/concept-store/src/tables.rs"
  # shellcheck disable=SC2016 # the ${FERROTERM_IMAGE:-…} is compose syntax written verbatim into the fixture
  printf '    image: ${FERROTERM_IMAGE:-ghcr.io/ferrohealth/ferroterm:9.9.9@sha256:00}\n' > "$work/compose.yml"
  # case NAME WANT LAYOUT: the check over a record of LAYOUT lands on WANT.
  case_() {
    local name=$1 want=$2 layout=$3 rc=0
    printf 'FERROTERM_INDEX_LAYOUT=%s\n' "$layout" > "$work/record.env"
    FERROTERM_LAYOUT_COMPOSE="$work/compose.yml" FERROTERM_LAYOUT_RECORD="$work/record.env" \
      FERROTERM_LAYOUT_SOURCE="file://$work" bash "$0" > "$work/out" 2>&1 || rc=$?
    if [[ "$rc" -ne "$want" ]]; then
      echo "::error::ferroterm-index-layout --self-test: $name exited $rc, wanted $want." >&2
      cat "$work/out" >&2
      exit 1
    fi
  }
  case_ "an agreeing layout" 0 7
  case_ "a differing layout" 1 6
  rm "$work/v9.9.9/crates/concept-store/src/tables.rs"
  case_ "an unreadable FerroTERM source" 1 7
  echo "ok: ferroterm-index-layout --self-test refused a differing and an unreadable layout"
  exit 0
fi

version="$(sed -nE 's#.*ghcr\.io/ferrohealth/ferroterm:([0-9]+\.[0-9]+\.[0-9]+)@sha256:.*#\1#p' "$compose" | head -n 1)"
if [[ -z "$version" ]]; then
  echo "::error file=${compose#"$root"/}::no ghcr.io/ferrohealth/ferroterm:X.Y.Z@sha256 pin found" >&2
  exit 1
fi

carried="$(sed -n 's/^FERROTERM_INDEX_LAYOUT=//p' "$record" | tail -n 1)"
if [[ -z "$carried" ]]; then
  echo "::error file=${record#"$root"/}::FERROTERM_INDEX_LAYOUT is not set" >&2
  exit 1
fi

tables="$(curl -fsSL --retry 3 "$source_base/v$version/crates/concept-store/src/tables.rs")" || {
  echo "::error::could not read FerroTERM v$version's crates/concept-store/src/tables.rs" >&2
  exit 1
}
reads="$(sed -nE 's/^pub const LAYOUT_VERSION: &str = "([0-9]+)";.*/\1/p' <<<"$tables")"
if [[ -z "$reads" ]]; then
  echo "::error::FerroTERM v$version's tables.rs declares no LAYOUT_VERSION this check can read" >&2
  exit 1
fi

if [[ "$reads" != "$carried" ]]; then
  echo "::error file=${compose#"$root"/}::the sandbox pins FerroTERM $version, which reads artifact layout $reads, but the box's index is layout $carried (${record#"$root"/}). Rebuild both indexes with ferroterm-build $version, place them on the box, and update the record in this change." >&2
  exit 1
fi
echo "ok: FerroTERM $version reads layout $reads, the layout of the box's index"
