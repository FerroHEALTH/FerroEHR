#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# A crate that PACKAGES third-party bytes carries that material's attribution
# INSIDE the package, pinned to the revision the repository records.
#
# The per-tree `PROVENANCE.md` is the repository's record and does not travel:
# it describes the whole vendored tree, and for `openehr-its` that tree is 784
# schema files while the published package ships exactly one of them. So the
# packaged attribution is a SECOND statement of the same pin — which is
# precisely the shape that drifts, because a re-vendor updates the provenance
# record and can leave the packaged text quoting a revision nobody vendors any
# more. This gate makes that a failing build instead.
#
# Two mechanical assertions per row; neither judges prose:
#   1. the upstream revision recorded in the provenance file appears verbatim
#      in the packaged attribution file;
#   2. that attribution file is itself in the crate's `include` list, so it
#      actually reaches a consumer.
#
# Usage: scripts/checks/packaged-attribution.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"

cd "$(dirname "$0")/../.."

# crate | provenance record (repository) | packaged attribution
readonly -a ROWS=(
  "crates/openehr-its|schemas/json/PROVENANCE.md|README.md"
  "crates/openehr-term|assets/PROVENANCE.md|assets/PROVENANCE.md"
)

fail=0
note() { echo "packaged-attribution: $*" >&2; fail=1; }

for row in "${ROWS[@]}"; do
  IFS='|' read -r crate provenance attribution <<<"$row"
  manifest="$crate/Cargo.toml"
  for required in "$crate/$provenance" "$crate/$attribution" "$manifest"; do
    [[ -f "$required" ]] || { note "missing $required"; continue 2; }
  done

  revision=$(grep -oE '[0-9a-f]{40}' "$crate/$provenance" | head -1 || true)
  if [[ -z "$revision" ]]; then
    note "$crate/$provenance records no 40-hex upstream commit — the packaged attribution has nothing to pin to"
    continue
  fi
  if ! grep -qF -- "$revision" "$crate/$attribution"; then
    note "$crate/$attribution does not name the upstream commit $revision recorded in $crate/$provenance — the attribution that travels with the package would point at a revision this repository no longer vendors"
  fi

  # The `include` list is a TOML array that may span lines; the attribution
  # file is matched by its exact path, so a glob covering it (never the case
  # today) would have to be spelled out here deliberately.
  if ! sed -n '/^include *= *\[/,/\]/p' "$manifest" | grep -qF -- "\"$attribution\""; then
    note "$manifest does not \`include\` $attribution — the attribution would not travel in the published package"
  fi
done

[[ "$fail" -eq 0 ]] || {
  echo >&2
  echo "Attribution for redistributed third-party material must be inside the" >&2
  echo "published artifact, not only in the repository it was built from." >&2
  exit 1
}

echo "ok: ${#ROWS[@]} crates package third-party bytes, each with a pinned attribution that travels"
