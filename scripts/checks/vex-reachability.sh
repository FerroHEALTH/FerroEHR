#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The dependency-path gate: every VEX statement's claim about HOW the affected
# crate enters the build is compared against the graph cargo resolves.
#
# The two gates beside this one both pass on a false statement. The generator
# and `scripts/checks/vex-advisories.sh` check that the accepted id set and the
# published justifications agree; `scripts/checks/advisory-exceptions.sh` checks
# that each accepted advisory still fires. Neither reads the ARGUMENT. The `rsa`
# statement asserted the crate was reached only through `openidconnect`, in a
# path where RSA is used to verify with a public key — while `cargo tree -i rsa`
# had shown a second path through `pgp`, where an RSA operation would be a
# private-key one, the exact operation the advisory is about. Every gate was
# green, because a wrong path claim is only wrong against the dependency graph
# and nothing compared the two.
#
# WHAT IS COMPARED, AND WHY AT THIS LEVEL. Per statement, two exact sets, in
# both directions (a named crate the graph lacks fails; a crate in the graph the
# statement does not name fails):
#
#   direct_dependents  the crates depending DIRECTLY on the affected package —
#                      depth 1 of `cargo tree -i`. This is the level the rsa
#                      defect lived at: `pgp` was a direct dependent all along.
#   workspace_roots    the workspace members the package is reachable from.
#
# Not full paths, and not the intermediate crates between the two. `cargo tree`
# DE-DUPLICATES: a package already displayed is repeated as `(*)` with its
# dependencies omitted (`cargo tree --no-dedupe`,
# https://doc.rust-lang.org/cargo/commands/cargo-tree.html). So its output
# enumerates every NODE exactly, while the set of PATHS it prints is partial —
# and both sets above are node-set facts, invariant under de-duplication, where
# a per-path comparison would be reading structure the output does not carry.
#
# THE GRAPH. `--workspace --all-features --target all -e normal,build,dev`: the
# widest graph cargo can resolve, because a published reachability claim must
# hold for every configuration a consumer can build, not only the default one.
# That is a superset of what `cargo deny` resolves (deny.toml sets no `[graph]`
# overrides, so it takes default features), which is the safe direction: this
# gate can only require a statement to account for MORE of the graph than the
# advisory gate itself sees. `--target all` also makes the answer
# host-independent, so a developer's macOS run and CI's Linux run agree.
#
# THE INPUT is the PUBLISHED document, not the prose file it is generated from:
# it is what a downstream scanner ingests, it needs no TOML reader in this job,
# and `vex-advisories.sh` already fails if it is not exactly what the prose
# generates.
#
# Mutation-proven in both directions: adding a carrier no graph edge supports
# fails naming it, and removing a real one fails naming the unlisted path.
#
# Usage: scripts/checks/vex-reachability.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

readonly DOC='security/vex/rust-advisories.openvex.json'

for tool in cargo jq comm; do
  command -v "$tool" >/dev/null || { echo "vex-reachability: $tool is required" >&2; exit 1; }
done
[[ -f "$DOC" ]] || {
  echo "error: $DOC does not exist — run scripts/security/vex-generate.sh" >&2
  exit 1
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Offline when the local registry cache allows it (a developer's run then never
# touches the network); online otherwise, because a CI job with a cold cargo
# home must still be able to resolve. `--locked` on both, so the graph compared
# is the committed one and nothing can be silently re-resolved.
net=(--locked)
if cargo tree --workspace --all-features --target all --depth 0 --locked --offline \
    > /dev/null 2>&1; then
  net+=(--offline)
fi

# `--no-deps` resolves nothing: the packages it lists are exactly the workspace
# members.
cargo metadata --no-deps --format-version 1 "${net[@]}" \
  | jq -r '.packages[].name' | sort -u > "$work/members"

fail=0
note() { echo "vex-reachability: $*" >&2; fail=1; }

# How many statements MUST be examined. Without this the loop below reads
# silence as agreement: a document with no `statements` array, or a jq failure
# inside the process substitution feeding the loop, would run zero iterations
# and report every statement verified.
expected="$(jq -r '.statements | length' "$DOC")"
[[ "$expected" -gt 0 ]] 2>/dev/null || {
  echo "error: $DOC declares no statements, so nothing would be verified" >&2
  exit 1
}

# One set per file, so `comm` can do the two-direction difference; `comm`
# requires sorted input and treats a blank line as an element, hence the filter.
setfile() { sort -u | sed '/^$/d' > "$1"; }

statements=0
while read -r statement; do
  [[ -n "$statement" ]] || continue
  id="$(jq -r '.vulnerability.name // "<no id>"' <<<"$statement")"

  if ! jq -e 'has("ferroehr:reachability")' <<<"$statement" > /dev/null; then
    note "$id: no 'ferroehr:reachability' block — the statement's dependency-path claim is unchecked"
    continue
  fi
  statements=$((statements + 1))

  spec="$(jq -r '."ferroehr:reachability".package' <<<"$statement")"
  jq -r '."ferroehr:reachability".direct_dependents[]?' <<<"$statement" | setfile "$work/said-direct"
  jq -r '."ferroehr:reachability".workspace_roots[]?' <<<"$statement" | setfile "$work/said-roots"

  # `--prefix depth` prints the depth as a bare number immediately before the
  # package, so a crate name starting with a digit would be unsplittable; the
  # literal '|' in the format string is the separator that makes it exact.
  # An absent package is not an error: cargo prints nothing and exits 0, which
  # is how a `component_not_present` statement (both sets empty) verifies.
  if ! tree="$(cargo tree --invert "$spec" --workspace --all-features --target all \
      --edges normal,build,dev --prefix depth --format '|{p}' "${net[@]}" 2> "$work/err")"; then
    note "$id: cargo tree --invert $spec failed:"
    sed 's/^/    /' "$work/err" >&2
    continue
  fi

  printf '%s\n' "$tree" | sed -n 's/^1|\([^ ]*\).*/\1/p' | setfile "$work/has-direct"
  printf '%s\n' "$tree" | sed -n 's/^[1-9][0-9]*|\([^ ]*\).*/\1/p' | sort -u \
    | comm -12 - "$work/members" | setfile "$work/has-roots"

  for level in direct roots; do
    case "$level" in
      direct) what='direct dependent' ;;
      roots) what='workspace root' ;;
      *) echo "error: unknown dependency level '$level' — no label to report it under" >&2; exit 1 ;;
    esac
    while read -r crate; do
      [[ -n "$crate" ]] || continue
      note "$id: claims $spec has the $what '$crate', which is in no edge of the resolved graph"
    done < <(comm -23 "$work/said-$level" "$work/has-$level")
    while read -r crate; do
      [[ -n "$crate" ]] || continue
      note "$id: '$crate' is a $what of $spec in the resolved graph and the statement does not name it — the impact statement argues about a path set that is not the real one"
    done < <(comm -13 "$work/said-$level" "$work/has-$level")
  done
done < <(jq -c '.statements[]' "$DOC")

[[ "$statements" -eq "$expected" ]] \
  || note "only $statements of the document's $expected statements carried a dependency-path claim"

if [[ "$fail" -ne 0 ]]; then
  echo >&2
  echo "A VEX statement's argument rests on where the affected crate enters the" >&2
  echo "build, so a wrong path makes the published justification a false claim." >&2
  echo "Re-read the argument against the graph, then correct 'carriers' /" >&2
  echo "'workspace_roots' in security/vex/rust-advisories.toml (and the prose" >&2
  echo "beside them) and regenerate:" >&2
  echo "  cargo tree -i <crate>[@<version>] --workspace --all-features --target all -e normal,build,dev" >&2
  echo "  bash scripts/security/vex-generate.sh" >&2
  exit 1
fi

echo "ok: all $statements VEX statements name the dependency paths cargo resolves"
