#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# scripts/watch/ghcr-prune-orphans.sh — delete the GHCR manifests nothing reaches.
#
# WHY (#3473): scripts/watch/ghcr-orphans.sh finds the residue of image runs
# that pushed by digest and died before `scan-and-tag` applied a tag, and
# deliberately deletes nothing. This is the reviewed deletion step it points at.
#
# A DRY RUN unless `--apply` is given: it prints every manifest it would delete
# with the command, and touches nothing. Three guards hold either way:
#
#   1. The list is recomputed by ghcr-orphans.sh on every run, never read from
#      a file a person saved earlier: a manifest tagged since then is no
#      longer an orphan, and a stale list cannot know that.
#   2. A manifest younger than `--min-age-days` (default 7) is kept. A release
#      in flight has pushed by digest and not yet tagged, so a fresh untagged
#      manifest may be one scan-and-tag is about to adopt.
#   3. With `--apply`, each version is read again just before its delete and
#      skipped unless it still carries the same digest and no tag.
#
# Usage:
#   scripts/watch/ghcr-prune-orphans.sh [--owner X] [--package Y]
#       [--min-age-days N] [--apply]
#   scripts/watch/ghcr-prune-orphans.sh --self-test
#
# Requires: gh, jq, curl; `--apply` needs a token with `delete:packages` over the
# owner's packages. Exits 1 when the orphan probe cannot answer (nothing is
# deleted) or when any delete failed, 2 on a usage error.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
# The probe this script trusts for the orphan list; the self-test points it at
# a stub.
PROBE="${GHCR_PRUNE_PROBE:-$HERE/ghcr-orphans.sh}"

usage() {
  echo "usage: $0 [--owner X] [--package Y] [--min-age-days N] [--apply] | --self-test" >&2
  exit 2
}

prune() {
  local owner="FerroHEALTH" min_age_days=7 apply=0
  local -a probe_args=()
  while [[ "$#" -gt 0 ]]; do
    case "$1" in
    --owner)
      owner="${2:?--owner needs a value}"
      probe_args+=(--owner "$owner")
      shift 2
      ;;
    --package)
      probe_args+=(--package "${2:?--package needs a value}")
      shift 2
      ;;
    --min-age-days)
      min_age_days="${2:?--min-age-days needs a value}"
      [[ "$min_age_days" =~ ^[0-9]+$ ]] || usage
      shift 2
      ;;
    --apply)
      apply=1
      shift
      ;;
    *) usage ;;
    esac
  done
  probe_args+=(--owner "$owner")

  local tool
  for tool in gh jq; do
    command -v "$tool" >/dev/null 2>&1 || {
      echo "$tool is required" >&2
      exit 1
    }
  done

  local work
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $work now, while it is still in scope
  trap "rm -rf '$work'" RETURN

  if ! bash "$PROBE" "${probe_args[@]}" --tsv "$work/orphans.tsv" >"$work/report.md" 2>"$work/probe.log"; then
    cat "$work/probe.log" >&2
    echo "ghcr-prune-orphans: the orphan probe could not answer — nothing deleted." >&2
    exit 1
  fi

  local now cutoff
  now="$(date -u +%s)"
  cutoff=$((now - min_age_days * 86400))
  local kept=0 planned=0 deleted=0 skipped=0 failed=0
  local pkg digest id created created_s current
  while IFS=$'\t' read -r pkg digest id created; do
    [[ -n "$pkg" ]] || continue
    created_s="$(jq -rn --arg t "$created" '$t | fromdate' 2>/dev/null)" || {
      echo "ghcr-prune-orphans: $pkg@$digest has an unreadable created_at '$created' — kept." >&2
      kept=$((kept + 1))
      continue
    }
    if [[ "$created_s" -gt "$cutoff" ]]; then
      echo "keep    $pkg@$digest (created $created, younger than $min_age_days days: a release may still be tagging it)"
      kept=$((kept + 1))
      continue
    fi
    planned=$((planned + 1))
    if [[ "$apply" -eq 0 ]]; then
      echo "delete  $pkg@$digest (created $created) — dry run:"
      echo "        gh api -X DELETE /orgs/$owner/packages/container/$pkg/versions/$id"
      continue
    fi
    # Re-read just before the delete: the digest must still be this version's
    # and no tag may have landed on it since the probe ran.
    if ! current="$(gh api "/orgs/$owner/packages/container/$pkg/versions/$id" \
      --jq '[.name, ((.metadata.container.tags // []) | length | tostring)] | @tsv')"; then
      echo "ghcr-prune-orphans: could not re-read $pkg version $id — skipped." >&2
      skipped=$((skipped + 1))
      continue
    fi
    if [[ "$current" != "$digest"$'\t'"0" ]]; then
      echo "skip    $pkg@$digest (version $id changed since the probe: $current)"
      skipped=$((skipped + 1))
      continue
    fi
    if gh api -X DELETE "/orgs/$owner/packages/container/$pkg/versions/$id" >/dev/null; then
      echo "deleted $pkg@$digest (version $id)"
      deleted=$((deleted + 1))
    else
      echo "ghcr-prune-orphans: deleting $pkg version $id failed." >&2
      failed=$((failed + 1))
    fi
  done <"$work/orphans.tsv"

  if [[ "$apply" -eq 0 ]]; then
    echo "ghcr-prune-orphans: dry run — $planned would be deleted, $kept kept. Re-run with --apply to delete."
  else
    echo "ghcr-prune-orphans: $deleted deleted, $skipped skipped, $failed failed, $kept kept."
  fi
  [[ "$failed" -eq 0 ]]
}

# Proves the guards against a stub probe and a stub gh: the dry run deletes
# nothing, `--apply` deletes only the old orphan whose version is unchanged,
# and a young orphan and a since-tagged one survive.
self_test() {
  local work
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $work now, while it is still in scope
  trap "rm -rf '$work'" RETURN
  mkdir -p "$work/bin"
  local old young
  old="$(date -u -r 0 +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -d @0 +%Y-%m-%dT%H:%M:%SZ)"
  young="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

  cat >"$work/probe.sh" <<PROBE
#!/usr/bin/env bash
set -euo pipefail
while [[ "\$#" -gt 0 ]]; do
  case "\$1" in --tsv) out="\$2"; shift 2 ;; *) shift ;; esac
done
printf 'pkg\tsha256:old\t11\t$old\n' > "\$out"
printf 'pkg\tsha256:young\t22\t$young\n' >> "\$out"
printf 'pkg\tsha256:retagged\t33\t$old\n' >> "\$out"
PROBE

  cat >"$work/bin/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$GH_STUB_LOG"
filter=""
previous=""
for argument in "$@"; do
  [[ "$previous" == --jq ]] && filter="$argument"
  previous="$argument"
done
case "$*" in
  "api -X DELETE "*) exit 0 ;;
  *versions/11*) body='{"name":"sha256:old","metadata":{"container":{"tags":[]}}}' ;;
  *versions/22*) body='{"name":"sha256:young","metadata":{"container":{"tags":[]}}}' ;;
  *versions/33*) body='{"name":"sha256:retagged","metadata":{"container":{"tags":["v9.9.9"]}}}' ;;
  *) echo "stub: an unexpected call: $*" >&2; exit 1 ;;
esac
printf '%s' "$body" | jq -r "$filter"
STUB
  chmod 0755 "$work/bin/gh" "$work/probe.sh"

  local failures=0
  : >"$work/gh.log"
  PATH="$work/bin:$PATH" GH_STUB_LOG="$work/gh.log" GHCR_PRUNE_PROBE="$work/probe.sh" \
    bash "$0" >"$work/dry.out"
  if grep -q DELETE "$work/gh.log"; then
    echo "self-test: the dry run issued a DELETE" >&2
    failures=1
  fi
  grep -q 'would be deleted' "$work/dry.out" || {
    echo "self-test: the dry run did not say what it would delete" >&2
    failures=1
  }

  : >"$work/gh.log"
  PATH="$work/bin:$PATH" GH_STUB_LOG="$work/gh.log" GHCR_PRUNE_PROBE="$work/probe.sh" \
    bash "$0" --apply >"$work/apply.out"
  grep -qx 'api -X DELETE /orgs/FerroHEALTH/packages/container/pkg/versions/11' "$work/gh.log" || {
    echo "self-test: the old unchanged orphan was not deleted" >&2
    failures=1
  }
  if grep -q 'DELETE .*/versions/22' "$work/gh.log"; then
    echo "self-test: a manifest younger than the age floor was deleted" >&2
    failures=1
  fi
  if grep -q 'DELETE .*/versions/33' "$work/gh.log"; then
    echo "self-test: a manifest tagged since the probe was deleted" >&2
    failures=1
  fi

  [[ "$failures" -eq 0 ]] || return 1
  echo "ghcr-prune-orphans: self-test OK (dry run deletes nothing; --apply keeps young and since-tagged manifests)."
}

case "${1:-}" in
--self-test) self_test ;;
*) prune "$@" ;;
esac
