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
# THE VERSION IS SET HERE (#4555). A pull request never edits the lockstep
# version; `plan` picks it from crates.io and `stamp` writes it into the
# publishing checkout, uncommitted. `plan` downloads the archive of the newest
# published version, reads the commit it was packaged from out of the
# `.cargo_vcs_info.json` cargo writes into every package
# (https://doc.rust-lang.org/cargo/commands/cargo-package.html), and asks
# scripts/checks/crate-version-guard.sh whether packaged content changed
# since. Unchanged, the version stays and the publish is a no-op; changed, it
# is the next 0.0.x, never below the manifests' own. `publish` then refuses to
# upload when any crate already holds the stamped version from a commit whose
# packaged content differs: "already exists" counts as done only for the same
# content.
#
# Usage:
#   publish-crates.sh plan            # print the lockstep version to publish
#   publish-crates.sh stamp <version> # write it into the nine manifests, the
#                                     # internal requirements and both locks
#   publish-crates.sh publish         # upload each crate in dependency order
#   publish-crates.sh verify          # read the registry back, with retries
#   publish-crates.sh version         # print the version in the manifests
#   publish-crates.sh list            # print the crates, one per line, in publish order
#
# Requires: cargo, curl, git, jq, sha256sum, tar; `plan` and `publish` need
# the full history (fetch-depth: 0). `publish` additionally requires
# CARGO_REGISTRY_TOKEN in the environment.
set -euo pipefail
cd "$(dirname "$0")/../.."

# shellcheck source=scripts/gh/retry-net.sh
. scripts/gh/retry-net.sh

readonly GUARD=scripts/checks/crate-version-guard.sh
readonly AGENT='User-Agent: ferroehr-publish'

die() {
  echo "::error::publish-crates: $*" >&2
  exit 1
}

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
    "crates/${1:-openehr-base}/Cargo.toml"
}

# The crates.io version list of <crate>, yanked versions included: a yanked
# number can never be uploaded again. A failed request is fatal, because an
# unknown answer must not read as "not published".
versions_body() {
  retry_read "https://crates.io/api/v1/crates/$1/versions" -H "$AGENT" \
    || die "crates.io did not answer for $1"
}

# The commit <crate> <version> was packaged from, out of the archive crates.io
# serves, checked against the index checksum. Nothing when the archive
# records no commit.
packaged_from() {
  local crate="$1" version="$2" body checksum work info
  body="$(versions_body "$crate")"
  checksum="$(jq -r --arg v "$version" '.versions[]? | select(.num == $v) | .checksum' <<<"$body")"
  [[ -n "$checksum" ]] || die "$crate $version is not on crates.io"
  work="$(mktemp -d)"
  if ! retry_read "https://static.crates.io/crates/$crate/$crate-$version.crate" -H "$AGENT" -o "$work/archive.crate"; then
    rm -rf "$work"
    die "could not download $crate-$version.crate from crates.io"
  fi
  if ! echo "$checksum  $work/archive.crate" | sha256sum -c - >/dev/null; then
    rm -rf "$work"
    die "$crate-$version.crate does not match the index checksum $checksum"
  fi
  info="$(tar -xzOf "$work/archive.crate" "$crate-$version/.cargo_vcs_info.json" 2>/dev/null || true)"
  rm -rf "$work"
  [[ -n "$info" ]] || return 0
  jq -r '.git.sha1 // empty' <<<"$info"
}

# Whether the packaged content at HEAD is what <sha> packaged: 0 the same,
# 1 changed, 2 when <sha> is empty or not in this history, so it cannot be
# judged.
same_content_as() {
  local sha="$1" verdict=0
  [[ -n "$sha" ]] || return 2
  git cat-file -e "${sha}^{commit}" 2>/dev/null || return 2
  bash "$GUARD" changed "$sha" HEAD >/dev/null || verdict=$?
  case "$verdict" in
  0) return 1 ;;
  1) return 0 ;;
  *) die "$GUARD could not judge the packaged content since $sha (exit $verdict)" ;;
  esac
}

# The higher of two versions.
higher() {
  local sorted
  sorted="$(printf '%s\n%s\n' "$1" "$2" | sort -V)"
  printf '%s\n' "${sorted##*$'\n'}"
}

# The lockstep version to publish, on stdout; the reasoning on stderr.
do_plan() {
  local crate body versions newest top="" base_versions sha verdict=0 floor next
  for crate in "${CRATES[@]}"; do
    body="$(versions_body "$crate")"
    versions="$(jq -r '.versions[]?.num' <<<"$body")"
    [[ "$crate" == openehr-base ]] && base_versions="$versions"
    [[ -n "$versions" ]] || continue
    newest="$(sort -V <<<"$versions")"
    newest="${newest##*$'\n'}"
    top="$(higher "${top:-$newest}" "$newest")"
  done
  [[ -n "$top" ]] || die "no openehr-* crate is on crates.io; a crate's first release is published by hand"
  [[ "$top" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "the newest published version '$top' is not a plain x.y.z"
  # openehr-base is uploaded first, so it holds the newest version unless a
  # crate was published on its own; then the set needs a fresh number.
  if grep -Fxq "$top" <<<"${base_versions:-}"; then
    sha="$(packaged_from openehr-base "$top")"
    same_content_as "$sha" || verdict=$?
    case "$verdict" in
    0)
      echo "publish-crates: packaged content is unchanged since $top (packaged from $sha): publish $top, a no-op where it is already on crates.io" >&2
      printf '%s\n' "$top"
      return
      ;;
    1) echo "publish-crates: packaged content changed since $top (packaged from $sha)" >&2 ;;
    *) echo "publish-crates: $top records no commit this history holds (${sha:-none}), so it counts as changed" >&2 ;;
    esac
  else
    echo "publish-crates: openehr-base does not hold the newest published version $top, so the set takes a fresh one" >&2
  fi
  next="${top%.*}.$((${top##*.} + 1))"
  floor="$(manifest_version)"
  next="$(higher "$next" "$floor")"
  echo "publish-crates: publish $next (newest on crates.io $top, manifests at $floor)" >&2
  printf '%s\n' "$next"
}

# Write <version> into the nine manifests, every internal `path =
# "../openehr-*", version = "…"` requirement, and the path entries of
# Cargo.lock and fuzz/Cargo.lock. The edit stays in the publishing checkout:
# `cargo package` records it as `"dirty": true` beside the commit.
do_stamp() {
  local new="$1" old crate f lock names got
  [[ "$new" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "'$new' is not a plain x.y.z version"
  old="$(manifest_version)"
  for crate in "${CRATES[@]}"; do
    [[ "$(manifest_version "$crate")" == "$old" ]] || die "crates/$crate is at $(manifest_version "$crate"), not the lockstep $old"
  done
  for crate in "${CRATES[@]}"; do
    f="crates/$crate/Cargo.toml"
    awk -v new="$new" '
      /^\[/ { p = ($0 == "[package]") }
      p && !done && /^version = "/ { sub(/"[^"]*"/, "\"" new "\""); done = 1 }
      { print }
    ' "$f" >"$f.new"
    sed -E "s|(path = \"\.\./openehr-[a-z]+\", version = )\"[^\"]+\"|\1\"$new\"|g" "$f.new" >"$f"
    rm "$f.new"
  done
  names="$(printf '"%s",' "${CRATES[@]}")"
  # A path package carries no source or checksum, so its lock entry is
  # rewritten in place; fuzz/ is its own workspace and has its own lock.
  for lock in Cargo.lock fuzz/Cargo.lock; do
    awk -v names="$names" -v old="\"$old\"" -v new="\"$new\"" '
      BEGIN { n = split(names, list, ","); for (i = 1; i <= n; i++) ours[list[i]] = 1 }
      $1 == "name" { hit = ($3 in ours); print; next }
      hit && $1 == "version" && $3 == old { sub(old, new) }
      { hit = 0; print }
    ' "$lock" >"$lock.new"
    mv "$lock.new" "$lock"
  done
  for crate in "${CRATES[@]}"; do
    got="$(manifest_version "$crate")"
    [[ "$got" == "$new" ]] || die "crates/$crate is at $got after the stamp, not $new"
  done
  got="$(git grep -hoE 'path = "\.\./openehr-[a-z]+", version = "[^"]+"' -- 'crates/*/Cargo.toml' || true)"
  got="$(sed -E 's/.*version = "([^"]+)"/\1/' <<<"$got" | sort -u)"
  [[ "$got" == "$new" ]] || die "internal requirements name ${got//$'\n'/ } after the stamp, not $new"
  for lock in Cargo.lock fuzz/Cargo.lock; do
    got="$(awk -v names="$names" -v old="\"$old\"" '
      BEGIN { n = split(names, list, ","); for (i = 1; i <= n; i++) ours[list[i]] = 1 }
      $1 == "name" { hit = ($3 in ours); next }
      hit && $1 == "version" && $3 == old { print FILENAME }
      { hit = 0 }
    ' "$lock")"
    [[ "$old" == "$new" || -z "$got" ]] || die "$lock still records an openehr-* path package at $old"
  done
  echo "publish-crates: the nine crates, their internal requirements and both locks are at $new (from $old)"
}

# Refuse before any upload when a crate already holds <version> from a commit
# whose packaged content differs from HEAD's, or from a commit that cannot be
# judged: crates.io would keep the old bytes and the set would split.
refuse_double_publish() {
  local want="$1" crate sha verdict
  for crate in "${CRATES[@]}"; do
    grep -Fxq "$want" <<<"$(jq -r '.versions[]?.num' <<<"$(versions_body "$crate")")" || continue
    sha="$(packaged_from "$crate" "$want")"
    verdict=0
    same_content_as "$sha" || verdict=$?
    case "$verdict" in
    0) echo "$crate: $want is on crates.io from $sha with the same packaged content" ;;
    1) die "$crate $want is on crates.io from $sha, and the packaged content changed since: publishing would put changed content under an existing version. Run 'publish-crates.sh plan' and stamp its version." ;;
    *) die "$crate $want is on crates.io from ${sha:-an unrecorded commit} this history does not hold, so its content cannot be compared: refusing to treat it as done." ;;
    esac
  done
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
  refuse_double_publish "$(manifest_version)"
  for crate in "${CRATES[@]}"; do
    echo "::group::$crate"
    # --allow-dirty: the version is stamped into this checkout, uncommitted.
    out="$(cargo publish -p "$crate" --locked --allow-dirty 2>&1)" || true
    printf '%s\n' "$out"
    echo "::endgroup::"
    plain="$(printf '%s' "$out" | strip_ansi)"
    case "$plain" in
    *"already exists on crates.io index"* | *"already uploaded"*)
      echo "$crate: already published at this version — nothing to do"
      ;;
    *)
      if grep -q "Uploaded $crate" <<<"$plain"; then
        echo "$crate: uploaded"
      else
        failed="$failed $crate"
      fi
      ;;
    esac
  done
  [[ -z "$failed" ]] || {
    echo "::error::failed to publish:$failed" >&2
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
      if body="$(curl --proto '=https' -sSL --fail -H 'User-Agent: ferroehr-publish-verify' \
        "https://crates.io/api/v1/crates/$crate/versions" 2>/dev/null)"; then
        got="$(printf '%s' "$body" |
          jq -r --arg v "$want" '.versions[]? | select(.num == $v) | .num' |
          sed -n 1p)" || got=""
      fi
      [[ -n "$got" ]] && break
      sleep 10
    done
    printf '%-16s %s\n' "$crate" "${got:-MISSING}"
    [[ -n "$got" ]] || bad="$bad $crate"
  done
  [[ -z "$bad" ]] || {
    echo "::error::the published set is SPLIT — these are not at $want:$bad" >&2
    return 1
  }
  echo "publish-crates: confirmed on crates.io — all ${#CRATES[@]} crates at $want"
}

case "${1:-}" in
plan) do_plan ;;
stamp)
  [[ "$#" -eq 2 ]] || die "usage: $0 stamp <version>"
  do_stamp "$2"
  ;;
publish) do_publish ;;
verify) do_verify ;;
version) manifest_version ;;
list) printf '%s\n' "${CRATES[@]}" ;;
*)
  echo "publish-crates: expected 'plan', 'stamp', 'publish', 'verify', 'version' or 'list', got '${1:-<none>}'" >&2
  exit 2
  ;;
esac
