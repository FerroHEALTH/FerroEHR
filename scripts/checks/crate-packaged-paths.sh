#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Decides whether a list of changed paths touches PACKAGED content of the
# published crates/openehr-* members (.claude/rules/crates-publishing.md).
#
# The one pattern both crate-version guards share: the `crate-version-guard`
# CI job and .claude/hooks/crate_version_bump_guard.sh. Each used to carry its
# own copy, and both copies matched `LICENSE-` only, so the bare `LICENSE` the
# BUSL crates (openehr-adl, openehr-query, openehr-sdt) list in their
# `include` passed without the lockstep bump (#3573).
#
# The scope mirrors the manifests' `include` lists: sources, the embedded
# assets and JSON schemas, the README, every licence text (bare or suffixed)
# and the manifest itself. Anything else (tests, benches, fuzz targets, the
# crate CLAUDE.md) never reaches crates.io.
#
# Usage: scripts/checks/crate-packaged-paths.sh [--self-test]
#   no args     → read changed paths on stdin; exit 0 when at least one is
#                 packaged content, 1 when none is
#   --self-test → prove the classifier in both directions
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"

readonly PACKAGED='^crates/openehr-[a-z]+/(src/|assets/|schemas/json/|README\.md$|LICENSE(-[^/]+)?$|Cargo\.toml$)'

# Exit 0 when a path on stdin is packaged content, 1 otherwise. The whole
# input is read (no `-q`), so a caller piping a long list under `pipefail`
# never sees its writer killed by SIGPIPE and the answer read as "none".
packaged() {
  grep -E "$PACKAGED" >/dev/null
}

self_test() {
  local failures=0 path
  for path in \
    crates/openehr-sdt/LICENSE \
    crates/openehr-query/LICENSE \
    crates/openehr-adl/LICENSE \
    crates/openehr-rm/LICENSE-APACHE-2.0 \
    crates/openehr-term/LICENSE-CC-BY-SA-3.0 \
    crates/openehr-base/src/lib.rs \
    crates/openehr-term/assets/openehr_terminology.xml \
    crates/openehr-its/schemas/json/openehr_rm_1.1.0_all.json \
    crates/openehr-adl/README.md \
    crates/openehr-sdt/Cargo.toml; do
    printf '%s\n' "$path" | packaged || {
      echo "self-test: $path was not counted as packaged content" >&2
      failures=1
    }
  done
  for path in \
    crates/openehr-sdt/tests/flat.rs \
    crates/openehr-adl/CLAUDE.md \
    crates/openehr-query/benches/parse.rs \
    crates/openehr-sdt/LICENSES/extra.txt \
    app/ferroehr/LICENSE \
    LICENSE \
    tools/openehr-codegen/Cargo.toml; do
    if printf '%s\n' "$path" | packaged; then
      echo "self-test: $path was wrongly counted as packaged content" >&2
      failures=1
    fi
  done
  [[ "$failures" -eq 0 ]] || return 1
  echo "crate-packaged-paths: self-test OK (bare and suffixed licence texts counted, unpackaged paths ignored)."
}

case "${1:-}" in
--self-test)
  self_test
  ;;
"")
  packaged
  ;;
*)
  guard_usage "[--self-test]"
  ;;
esac
