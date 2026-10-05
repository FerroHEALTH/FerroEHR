#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The VEX drift gate: the published OpenVEX document must be exactly what the
# generator produces from `deny.toml` + `security/vex/rust-advisories.toml`.
#
# Two failure modes it exists to catch, both of which have precedent in this
# repository's history of second lists that nothing read:
#
#   1. An advisory is accepted in `deny.toml` and no justification is published,
#      so a downstream scanner sees an unexplained finding while the argument
#      sits in a TOML comment nobody downstream reads. (The generator refuses;
#      this gate is where that refusal reaches CI.)
#   2. The JSON is hand-edited, or the prose changes without regeneration, and
#      the published document quietly stops matching the gate it claims to
#      describe.
#
# Mutation-proven in both directions: adding an ignore to `deny.toml` without a
# prose entry fails, and touching one byte of the generated JSON fails.
#
# What this gate CANNOT see: whether either side still describes reality. An
# ignore and its justification agree perfectly right up until a dependency
# upgrade resolves the advisory, at which point both are stale and both still
# pass here. `scripts/checks/advisory-exceptions.sh` is that half, and it runs in
# the cargo-deny job, which is where the graph is resolvable.
#
# Usage: scripts/checks/vex-advisories.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

readonly OUT='security/vex/rust-advisories.openvex.json'

[[ -f "$OUT" ]] || {
  echo "error: $OUT does not exist — run scripts/security/vex-generate.sh" >&2
  exit 1
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The generator carries the id-set agreement and the OpenVEX vocabulary checks
# and exits non-zero on any of them, so running it IS half this gate.
bash scripts/security/vex-generate.sh --stdout > "$work/expected.json"

if ! diff -u "$OUT" "$work/expected.json" > "$work/diff.txt"; then
  echo "error: $OUT is not what the generator produces" >&2
  echo >&2
  sed 's/^/  /' "$work/diff.txt" >&2
  echo >&2
  echo "The document is GENERATED — never hand-edit it. Change deny.toml or" >&2
  echo "security/vex/rust-advisories.toml, then run:" >&2
  echo "  bash scripts/security/vex-generate.sh" >&2
  exit 1
fi

# Every OTHER document under security/vex/ is hand-authored (they describe
# inherited container layers, where nothing in this repository resolves the
# finding). They still have to be well-formed OpenVEX, so a typo does not ship
# as a document a consumer's parser rejects.
docs=(security/vex/*.openvex.json)
for doc in "${docs[@]}"; do
  jq -e '
    (has("@context") and has("@id") and has("author") and has("timestamp")
     and has("version") and (.statements | type == "array" and length > 0))
    and (.statements | all(
      has("vulnerability") and has("products") and has("status")
      and (.status | IN("not_affected", "affected", "fixed", "under_investigation"))
      and (if .status == "not_affected" then
             (.justification | IN(
               "component_not_present", "vulnerable_code_not_present",
               "vulnerable_code_not_in_execute_path",
               "vulnerable_code_cannot_be_controlled_by_adversary",
               "inline_mitigations_already_exist"))
           else true end)))
  ' "$doc" > /dev/null || {
    echo "error: $doc is not a well-formed OpenVEX document" >&2
    echo "  (https://github.com/openvex/spec/blob/main/OPENVEX-SPEC.md)" >&2
    exit 1
  }
done

echo "ok: ${#docs[@]} VEX documents well-formed; $OUT matches deny.toml"
