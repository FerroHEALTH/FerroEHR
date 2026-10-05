#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The resolved-exception gate: no `[advisories].ignore` entry in `deny.toml` may
# outlive the finding it describes.
#
# `deny.toml`'s header states the rule in both directions, and this is the half
# no amount of reading keeps true: a dependency upgrade resolves an advisory, the
# ignore goes on passing because there is nothing left to ignore, and the
# published VEX justification keeps describing a dependency graph that no longer
# exists. That happened here — two quick-xml advisories accepted "via
# object_store, which pins ^0.40" stayed accepted after object_store moved to the
# patched 0.41.
#
# cargo-deny already detects the condition: `advisory-not-detected` fires for an
# `ignore` entry that no crate in the graph matches
# (https://embarkstudios.github.io/cargo-deny/checks/advisories/cfg.html). It is
# a WARNING by default, so `cargo deny check` stays green on one; this gate runs
# the check with that diagnostic promoted to an error and names the entries.
#
# Mutation-proven in both directions: a bogus ignore fails the gate naming its
# id, and removing it turns the gate green again.
#
# Usage: scripts/checks/advisory-exceptions.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

for tool in cargo jq; do
  command -v "$tool" >/dev/null || { echo "advisory-exceptions: $tool is required" >&2; exit 1; }
done
cargo deny --version >/dev/null 2>&1 || {
  echo "advisory-exceptions: cargo-deny is required (cargo install cargo-deny)" >&2
  exit 1
}

# `--format json` writes one JSON object per diagnostic plus a final summary
# object. Non-JSON lines (a cargo file-lock notice, a database fetch message) are
# filtered out rather than fed to jq, which would abort on them.
set +e
report="$(cargo deny --format json check -D advisory-not-detected --show-stats advisories 2>&1)"
set -e
json="$(printf '%s\n' "$report" | grep '^{' || true)"

# The summary object is the proof the check actually RAN. Without this, a failed
# advisory-database fetch would produce no diagnostics and the gate would read
# silence as agreement — the exact failure mode it exists to prevent.
if ! printf '%s\n' "$json" | jq -e 'select(.type == "summary")' >/dev/null 2>&1; then
  echo "error: the advisories check did not complete, so nothing was verified" >&2
  echo >&2
  # shellcheck disable=SC2001 # indents EVERY line of a multi-line report; ${//} has no ^ anchor
  sed 's/^/  /' <<<"$report" >&2
  exit 1
fi

resolved="$(printf '%s\n' "$json" | jq -r '
  select(.type == "diagnostic" and .fields.code == "advisory-not-detected")
  | .fields.labels[]
  | select(.message == "no crate matched advisory criteria")
  | .span')"

if [[ -n "$resolved" ]]; then
  echo "error: deny.toml keeps an exception for an advisory the gate no longer raises:" >&2
  # shellcheck disable=SC2001 # indents EVERY line of a multi-line list; ${//} has no ^ anchor
  sed 's/^/  /' <<<"$resolved" >&2
  echo >&2
  echo "No crate in the dependency graph matches these any more — the finding is" >&2
  echo "resolved, so the exception records nothing. Drop each id from deny.toml's" >&2
  echo "[advisories].ignore AND remove its [[accepted]] entry from" >&2
  echo "security/vex/rust-advisories.toml, then regenerate the VEX document:" >&2
  echo "  bash scripts/security/vex-generate.sh" >&2
  exit 1
fi

entries="$(grep -cE '^[[:space:]]*\{[[:space:]]*id[[:space:]]*=' deny.toml || true)"
echo "ok: all $entries deny.toml advisory exceptions still name a finding cargo-deny raises"
