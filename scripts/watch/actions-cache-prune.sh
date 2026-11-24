#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# scripts/watch/actions-cache-prune.sh — expire the Actions cache entries no
# lane restores.
#
# WHY (#6388): the repository's caches share one 10 GB quota, and GitHub
# evicts the least recently used entries past it, which took main's build
# caches. Only main writes a cache, so this keeps main's newest entry of each
# kind and deletes:
#
#   1. every entry on another ref: a pull request or a merge-queue run reads
#      main's caches and writes none, so what is left there is stale;
#   2. every main entry that a newer entry of the same family replaced, the
#      family being the key without its trailing hash, date or run id
#      segments (`v0-rust-test-Linux-x64-23d6fd24-610c27a8` is the family
#      `v0-rust-test-Linux-x64`); rust-cache and actions/cache restore the
#      newest prefix match, so an older one is never read again;
#   3. every sccache object on main not read for `--sccache-days` days
#      (default 3): sccache keys each compile unit by its own hash, so an
#      object nothing read lately belongs to a tree main has moved past.
#
# A DRY RUN unless `--apply` is given: it prints every entry it would delete
# with its reason and size, and touches nothing.
#
# Usage:
#   scripts/watch/actions-cache-prune.sh [--repo OWNER/NAME] [--sccache-days N] [--apply]
#   scripts/watch/actions-cache-prune.sh --self-test
#
# Requires: gh, jq; `--apply` needs a token with `actions: write`. Exits 1
# when the cache list cannot be read (nothing is deleted) or a delete failed,
# 2 on a usage error.
set -euo pipefail

usage() {
  echo "usage: $0 [--repo OWNER/NAME] [--sccache-days N] [--apply] | --self-test" >&2
  exit 2
}

# The entries to delete, read from `id key ref created_at last_accessed_at
# size` lines (tab-separated) on stdin, newest created first; prints
# `id reason size key` lines. $1 is the access cutoff for sccache objects as
# an ISO 8601 instant, which compares as text.
decide() {
  local cutoff="$1"
  awk -F'\t' -v cutoff="$cutoff" -v OFS='\t' '
    $3 != "refs/heads/main" { print $1, "not main", $6, $2; next }
    $2 ~ /^sccache\// {
      if ($5 < cutoff) print $1, "sccache unread", $6, $2
      next
    }
    {
      family = $2
      while (match(family, /-([0-9a-f]{8,}|[0-9]+)$/)) family = substr(family, 1, RSTART - 1)
      if (family in kept) print $1, "replaced by " kept[family], $6, $2
      else kept[family] = $2
    }
  '
}

self_test() {
  local input expected got
  input=$(printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    1 v0-rust-test-Linux-x64-23d6fd24-610c27a8 refs/heads/main 2026-11-24T00:53:00Z 2026-11-24T00:53:00Z 700 \
    2 v0-rust-test-Linux-x64-23d6fd24-f2a99494 refs/heads/main 2026-11-23T10:00:00Z 2026-11-24T00:10:00Z 690 \
    3 v0-rust-ui-docs-shots-Linux-x64-23d6fd24-f2a99494 refs/pull/6449/merge 2026-11-24T00:43:00Z 2026-11-24T00:43:00Z 137 \
    4 sccache/1/1/8/1185b1 refs/heads/main 2026-11-20T00:00:00Z 2026-11-24T00:48:00Z 1 \
    5 sccache/6/4/5/645f13 refs/heads/main 2026-11-20T00:00:00Z 2026-11-20T00:00:00Z 1 \
    6 ui-e2e-server-bin-1.99.1-2ddaa810 refs/heads/main 2026-11-24T00:55:00Z 2026-11-24T00:55:00Z 65 \
    7 ui-e2e-server-bin-1.99.1-d61e7606 refs/heads/main 2026-11-23T00:55:00Z 2026-11-23T00:55:00Z 65 \
    8 cache-trivy-2026-11-24 refs/heads/main 2026-11-24T00:51:00Z 2026-11-24T00:51:00Z 102 \
    9 cache-trivy-2026-11-23 refs/heads/main 2026-11-23T00:51:00Z 2026-11-23T00:51:00Z 102 \
    10 fuzz-corpus-aql_query-45292419460 refs/heads/main 2026-11-24T02:41:00Z 2026-11-24T02:41:00Z 3 \
    11 docker.io--tonistiigi--binfmt-latest-linux-x64 refs/heads/main 2026-11-24T00:51:00Z 2026-11-24T00:51:00Z 33 \
    12 v0-rust-msrv-core-Linux-x64-23d6fd24-610c27a8 refs/heads/main 2026-11-22T00:53:00Z 2026-11-24T00:53:00Z 472)
  expected=$(printf '%s\t%s\t%s\t%s\n' \
    2 "replaced by v0-rust-test-Linux-x64-23d6fd24-610c27a8" 690 v0-rust-test-Linux-x64-23d6fd24-f2a99494 \
    3 "not main" 137 v0-rust-ui-docs-shots-Linux-x64-23d6fd24-f2a99494 \
    5 "sccache unread" 1 sccache/6/4/5/645f13 \
    7 "replaced by ui-e2e-server-bin-1.99.1-2ddaa810" 65 ui-e2e-server-bin-1.99.1-d61e7606 \
    9 "replaced by cache-trivy-2026-11-24" 102 cache-trivy-2026-11-23)
  got=$(decide 2026-11-21T00:00:00Z <<<"$input")
  if [[ "$got" != "$expected" ]]; then
    echo "self-test FAILED" >&2
    diff <(echo "$expected") <(echo "$got") >&2 || true
    exit 1
  fi
  echo "self-test passed"
}

prune() {
  local repo="${GITHUB_REPOSITORY:-FerroHEALTH/FerroEHR}" days=3 apply=0
  while [[ "$#" -gt 0 ]]; do
    case "$1" in
    --repo)
      repo="${2:?--repo needs a value}"
      shift 2
      ;;
    --sccache-days)
      days="${2:?--sccache-days needs a value}"
      [[ "$days" =~ ^[0-9]+$ ]] || usage
      shift 2
      ;;
    --apply)
      apply=1
      shift
      ;;
    *) usage ;;
    esac
  done
  local cutoff listing doomed
  cutoff=$(date -u -d "$days days ago" +%Y-%m-%dT%H:%M:%SZ)
  if ! listing=$(gh api --paginate "repos/$repo/actions/caches?per_page=100&sort=created_at&direction=desc" \
    --jq '.actions_caches[] | [.id, .key, .ref, .created_at, .last_accessed_at, .size_in_bytes] | @tsv'); then
    echo "::error::could not read the cache list of $repo; nothing was deleted" >&2
    exit 1
  fi
  doomed=$(decide "$cutoff" <<<"$listing")
  local total count failed=0
  total=$(awk -F'\t' '{ s += $6 } END { printf "%.0f", s / 1e6 }' <<<"$listing")
  count=$(grep -c . <<<"$doomed" || true)
  echo "$repo holds $(grep -c . <<<"$listing") entries, $total MB; $count to delete:"
  [[ -n "$doomed" ]] || return 0
  awk -F'\t' '{ printf "  %s (%s, %.0f MB): %s\n", $4, $2, $3 / 1e6, $1 }' <<<"$doomed"
  if [[ "$apply" -eq 0 ]]; then
    echo "dry run: pass --apply to delete them"
    return 0
  fi
  local id
  while IFS=$'\t' read -r id _; do
    if ! gh api -X DELETE "repos/$repo/actions/caches/$id" >/dev/null; then
      echo "::warning::could not delete cache entry $id"
      failed=1
    fi
  done <<<"$doomed"
  local freed
  freed=$(awk -F'\t' '{ s += $3 } END { printf "%.0f", s / 1e6 }' <<<"$doomed")
  echo "deleted $count entries, $freed MB"
  return "$failed"
}

case "${1:-}" in
--self-test) self_test ;;
*) prune "$@" ;;
esac
