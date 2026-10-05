#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# scripts/checks/ixit-declarations.sh — every per-instance ixit parameter the
# pinned instrument defines is DECLARED on every instance, or its absence is
# adjudicated (#3041).
#
# The instrument's ixit schema is explicit that an absent instance parameter
# is undeclared, never a default: a case whose `requires` clause names it
# records not_applicable, and the capability that case evidences gains an
# `unevidenced` slot. That happened silently at the 0.1.4 pin bump, when the
# new `administrative` parameter guarded six role-boundary cases out and the
# only trace was a diff of results.json outcome rows. This check makes the
# cost visible before the run: a pin that introduces a parameter fails here
# until the party either declares it on its instances or records, in a
# register beside the ixit, why an instance leaves it undeclared.
#
# What counts as declared, per instance and per optional parameter:
#   1. the instance carries the key; or
#   2. the party carries the same top-level key AND the schema says an absent
#      instance value inherits it ("Absent => the top-level … applies"); or
#   3. the register `ixit-undeclared.json` beside the ixit names the instance
#      (or `*`) and the parameter with a reason.
# Anything else fails, naming the instance and the parameter. A register
# entry naming a parameter the schema no longer defines, or an instance the
# ixit no longer declares, fails too: an adjudication that has nothing to
# adjudicate is stale.
#
# The parameter set is READ FROM THE PINNED SCHEMA, never hand-maintained:
# `--schema PATH` for a caller that already has it (the pipeline, CI), else
# the pinned catalogue checkout `scripts/lib/veredictum.sh` resolves.
#
# Usage: scripts/checks/ixit-declarations.sh [--schema PATH] <party-dir>...
#   party-dir  a directory holding ixit.json (and optionally ixit-undeclared.json)

set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--schema PATH] <party-dir>..." "--schema" "$@"
cd "$(dirname "$0")/../.."

command -v jq >/dev/null || { echo "error: jq is required" >&2; exit 1; }

schema=""
parties=()
while [[ $# -gt 0 ]]; do
  case "$1" in
  --schema)
    schema="${2:?--schema needs a path}"
    shift 2
    ;;
  -h | --help)
    sed -n '4,40p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  *)
    parties+=("$1")
    shift
    ;;
  esac
done
[[ ${#parties[@]} -gt 0 ]] || { echo "usage: $0 [--schema PATH] <party-dir>..." >&2; exit 2; }

if [[ -z "$schema" ]]; then
  # shellcheck source=scripts/lib/veredictum.sh
  source scripts/lib/veredictum.sh
  schema="$(veredictum_root)/schemas/ixit.schema.json"
fi
[[ -f "$schema" ]] || { echo "error: ixit schema not found: $schema" >&2; exit 1; }

# The optional per-instance parameters, and which of them the schema lets an
# instance inherit from the party level. Both come from the schema text: the
# inheritance rule is the sentence the schema itself writes for those keys.
optional=$(jq -r '
  .properties.instances.additionalProperties as $inst
  | ($inst.required // []) as $req
  | $inst.properties | keys[] | select(. as $k | $req | index($k) | not)
' "$schema")
inheriting=$(jq -r '
  .properties.instances.additionalProperties.properties
  | to_entries[]
  | select((.value.description // "") | test("Absent => the top-level"))
  | .key
' "$schema")

fail=0
note() { echo "ixit-declarations: $*" >&2; fail=1; }
checked=0

for party in "${parties[@]}"; do
  ixit="$party/ixit.json"
  register="$party/ixit-undeclared.json"
  [[ -f "$ixit" ]] || { note "missing $ixit"; continue; }
  if [[ -f "$register" ]]; then
    jq -e 'type == "object" and all(.[]; type == "object" and all(.[]; type == "string" and length > 0))' "$register" >/dev/null \
      || { note "$register is not an object of instance -> {parameter: reason} with non-empty reasons"; continue; }
  fi
  reg='{}'
  [[ -f "$register" ]] && reg=$(cat "$register")

  while read -r instance; do
    [[ -n "$instance" ]] || continue
    checked=$((checked + 1))
    while read -r param; do
      [[ -n "$param" ]] || continue
      declared=$(jq -r --arg i "$instance" --arg p "$param" --argjson reg "$reg" --arg inherit "$inheriting" '
        (.instances[$i] | has($p))
        or ((has($p)) and (($inherit | split("\n")) | index($p) != null))
        or ($reg[$i][$p]? != null)
        or ($reg["*"][$p]? != null)
      ' "$ixit")
      if [[ "$declared" != "true" ]]; then
        note "$ixit: instance '$instance' leaves '$param' undeclared — the instrument records every case that requires it not_applicable. Declare it on the instance, or record the reason in $register under \"$instance\" (or \"*\")."
      fi
    done <<<"$optional"
  done < <(jq -r '.instances | keys[]' "$ixit")

  # Stale adjudications: a register may only name instances the ixit declares
  # (or `*`) and parameters the schema defines.
  while IFS=$'\t' read -r instance param; do
    [[ -n "$instance" ]] || continue
    if [[ "$instance" != "*" ]] && ! jq -e --arg i "$instance" '.instances | has($i)' "$ixit" >/dev/null; then
      note "$register adjudicates instance '$instance', which $ixit does not declare — remove the stale entry"
    fi
    if ! grep -qxF -- "$param" <<<"$optional"; then
      note "$register adjudicates '$param' on '$instance', but the pinned schema defines no such optional instance parameter — remove the stale entry"
    fi
  done < <(jq -r 'to_entries[] | .key as $i | .value | keys[] | "\($i)\t\(.)"' <<<"$reg")
done

[[ "$fail" -eq 0 ]] || {
  echo >&2
  echo "An undeclared instance parameter is not a default: the pinned instrument" >&2
  echo "guards every case that requires it out of the run, and the capability it" >&2
  echo "evidences loses coverage silently. Declare, or adjudicate the absence." >&2
  exit 1
}
echo "ixit-declarations: OK ($checked instance(s) across ${#parties[@]} party set(s), $(wc -l <<<"$optional" | tr -d ' ') optional parameter(s) from $(basename "$(dirname "$schema")")/$(basename "$schema"))"
