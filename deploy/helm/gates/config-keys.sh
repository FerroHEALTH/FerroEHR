#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Every configuration key a rendered chart hands the server, in the
# ferroehr.toml it mounts or as a FERROEHR__ environment variable, must be a key
# app/ferroehr/assets/ferroehr.default.toml defines (as an active line or a
# `#?` example line). A key the configuration no longer has is otherwise
# caught only by the chart-boot lane, after an image build and a boot (#3901).
#
# Usage: deploy/helm/gates/config-keys.sh <rendered-manifests.yaml> [release]
# Prints one line per unknown key and exits 1 when there is any.
set -euo pipefail

readonly RENDER="$1"
readonly RELEASE="${2:-ferroehr}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
readonly DEFAULT_TOML="${DEFAULT_TOML:-${REPO_ROOT}/app/ferroehr/assets/ferroehr.default.toml}"

# Tables whose keys are the operator's own names (a template id, a user id, a
# predicate name), so any key below them is defined.
readonly MAP_TABLES='
audit.categories.templates
audit.categories.archetypes
authz.assurance.levels
cohort.predicates
'

# Every key path of a TOML document, dotted, array indices dropped; tables are
# paths too, so a table header the default declares empty still counts.
key_paths() {
  yq -p toml -o json '.' | jq -r '
    [paths | map(select(type == "string"))]
    | map(select(length > 0) | join("."))
    | unique[]'
}

defined="$(sed -E 's/^#\? ?//' "$DEFAULT_TOML" | key_paths)"

toml="$(yq "select(.kind == \"ConfigMap\" and .metadata.name == \"${RELEASE}\") | .data.\"ferroehr.toml\"" "$RENDER")"
if [[ -z "$toml" || "$toml" == "null" ]]; then
  toml="$(yq "select(.kind == \"Secret\" and .metadata.name == \"${RELEASE}-config\") | .stringData.\"ferroehr.toml\"" "$RENDER")"
fi
rendered_toml=""
if [[ -n "$toml" && "$toml" != "null" ]]; then
  rendered_toml="$(printf '%s\n' "$toml" | key_paths)"
fi
rendered_env="$(yq -r "select(.kind == \"Deployment\" and .metadata.name == \"${RELEASE}\") | .spec.template.spec.containers[].env[]?.name" "$RENDER" \
  | sed -nE 's/^FERROEHR__//p' | awk '{ gsub(/__/, "."); print tolower($0) }')"

unknown=0
while IFS= read -r key; do
  [[ -n "$key" ]] || continue
  grep -qxF -- "$key" <<<"$defined" && continue
  mapped=0
  while IFS= read -r table; do
    [[ -n "$table" && "$key" == "$table".* ]] && { mapped=1; break; }
  done <<<"$MAP_TABLES"
  [[ "$mapped" -eq 1 ]] && continue
  echo "${key} is not a key of ferroehr.default.toml"
  unknown=1
done < <(printf '%s\n%s\n' "$rendered_toml" "$rendered_env" | sort -u)
exit "$unknown"
