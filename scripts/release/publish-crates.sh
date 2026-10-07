#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The crates.io upload of the nine `openehr-*` spec crates, and its read-back,
# as ONE implementation.
#
# Two workflows publish these crates — the `crates` leg of `release.yml` on a
# `v*` tag (the primary path, approval-gated) and `publish-crates.yml` on a
# manual dispatch (the dry-run/recovery lane) — and they cannot share a reusable
# WORKFLOW. crates.io Trusted Publishing matches the OIDC `workflow_ref` claim,
# and for a job inside a reusable workflow that claim names the CALLING workflow
# (https://docs.github.com/en/actions/concepts/security/openid-connect and
# https://crates.io/docs/trusted-publishing), so a reusable workflow would
# present the caller's identity anyway while hiding which workflow file each
# Trusted Publisher entry has to name. The shared thing is therefore this
# script, and each workflow keeps its own job — and its own identity.
#
# WHY PER CRATE. `cargo publish --workspace` is all-or-nothing at the START — it
# refuses the whole run if ANY member version already exists — while being
# non-atomic at the END, so a partial publish cannot be finished by re-running
# it. Those two together stranded `openehr-its` and `openehr-adl` at 0.0.10
# while six siblings reached 0.0.15 (issue #2211). Publishing per crate in
# dependency order and treating "already exists" as done makes the lane
# RESUMABLE, and idempotent: a release whose cycle changed no packaged crate
# content publishes nothing and stays green.
#
# Usage:
#   publish-crates.sh publish   # upload each crate in dependency order
#   publish-crates.sh verify    # read the registry back, with retries
#   publish-crates.sh version   # print the lockstep version
#   publish-crates.sh list      # print the crates, one per line, in publish order
#
# Requires: cargo, curl, jq. `publish` additionally requires
# CARGO_REGISTRY_TOKEN in the environment.
set -euo pipefail
cd "$(dirname "$0")/../.."

# Dependency order. `cargo publish -p` resolves nothing for us here — each
# upload must be able to see its siblings already on the index.
readonly CRATES=(
  openehr-base
  openehr-lang
  openehr-term
  openehr-rm
  openehr-am
  openehr-query
  openehr-its
  openehr-sdt
  openehr-adl
)

# The nine versions move in lockstep (`.claude/rules/crates-publishing.md`), so
# one manifest answers for the set. The `[package]` table's own `version`, never
# the first `version = ` line in the file: a manifest carries dependency
# versions too.
manifest_version() {
  awk -F'"' '/^\[package\]/{p=1} p && /^version = /{print $2; exit}' \
    crates/openehr-base/Cargo.toml
}

# cargo colours the STATUS WORD alone, so the bytes are `Uploaded<RESET>
# openehr-adl` and a literal "Uploaded openehr-adl" never matches a perfectly
# successful publish. Built with printf rather than written as an escape in the
# sed program: `\x1b` is a GNU sed extension.
readonly ESC=$'\033'

strip_ansi() {
  sed -E "s/${ESC}\\[[0-9;]*m//g"
}

do_publish() {
  local crate out plain failed=""
  for crate in "${CRATES[@]}"; do
    echo "::group::$crate"
    out="$(cargo publish -p "$crate" --locked 2>&1)" || true
    printf '%s\n' "$out"
    echo "::endgroup::"
    plain="$(printf '%s' "$out" | strip_ansi)"
    case "$plain" in
    *"already exists on crates.io index"* | *"already uploaded"*)
      echo "$crate: already published at this version — nothing to do"
      ;;
    *)
      if printf '%s' "$plain" | grep -q "Uploaded $crate"; then
        echo "$crate: uploaded"
      else
        failed="$failed $crate"
      fi
      ;;
    esac
  done
  [[ -z "$failed" ]] || {
    echo "::error::failed to publish:$failed"
    return 1
  }
  echo "publish-crates: every crate is at $(manifest_version) or was already there"
}

# The set must never be reported as published when it is SPLIT: while the line
# is 0.0.x cargo treats every 0.0.x as its own compatibility set, so a straggler
# makes its siblings' internal requirements unresolvable for every consumer.
# Read the registry rather than trusting the exit code of the upload.
do_verify() {
  local want crate got body bad=""
  want="$(manifest_version)"
  echo "publish-crates: expecting every crate at $want"
  for crate in "${CRATES[@]}"; do
    # The index is eventually consistent right after an upload, so a miss is
    # retried rather than believed. A FAILED REQUEST is not a miss either:
    # without --fail, curl hands an error BODY to jq, jq dies on the absent
    # `.versions`, and `pipefail` ends the whole run — so one transient 429
    # would report a split set that does not exist. Guarding the pipeline in an
    # `if` keeps a bad response as "not seen yet" and lets the remaining polls
    # run.
    got=""
    for _ in 1 2 3 4 5 6; do
      if body="$(curl -sSL --fail -H 'User-Agent: ferroehr-publish-verify' \
        "https://crates.io/api/v1/crates/$crate/versions" 2>/dev/null)"; then
        got="$(printf '%s' "$body" |
          jq -r --arg v "$want" '.versions[]? | select(.num == $v) | .num' |
          head -1)" || got=""
      fi
      [[ -n "$got" ]] && break
      sleep 10
    done
    printf '%-16s %s\n' "$crate" "${got:-MISSING}"
    [[ -n "$got" ]] || bad="$bad $crate"
  done
  [[ -z "$bad" ]] || {
    echo "::error::the published set is SPLIT — these are not at $want:$bad"
    return 1
  }
  echo "publish-crates: confirmed on crates.io — all ${#CRATES[@]} crates at $want"
}

case "${1:-}" in
publish) do_publish ;;
verify) do_verify ;;
version) manifest_version ;;
list) printf '%s\n' "${CRATES[@]}" ;;
*)
  echo "publish-crates: expected 'publish', 'verify', 'version' or 'list', got '${1:-<none>}'" >&2
  exit 2
  ;;
esac
