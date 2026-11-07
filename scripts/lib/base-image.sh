#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The base image of a Dockerfile, read from its `FROM` line (#4953).
#
# The runtime stage's `FROM` pin is the one place a base digest is written.
# Dependabot moves it, and every other value is derived from it here: the
# `org.opencontainers.image.base.name` and `base.digest` labels and
# annotations the reusable build-image.yml lane puts on a published image, and
# the agreement scripts/checks/image-labels.sh holds the tree to. Before this,
# the digest was written in six places, and every base-images pull request
# needed a hand commit to carry it into the other five.
#
# The base is the last digest-pinned `FROM` of the file: a stage built on an
# earlier stage (`FROM runtime-base AS runtime-prebuilt`) names no image, and
# the build stages come first in all three Dockerfiles.
#
# Usage:
#   scripts/lib/base-image.sh labels <dockerfile>   (repository-relative)
#       print the two label lines, `org.opencontainers.image.base.name=<name>`
#       and `org.opencontainers.image.base.digest=<digest>`; exit 1 when the
#       file has no digest-pinned FROM
#   scripts/lib/base-image.sh ref <dockerfile>      (repository-relative)
#       print the `<name>@<digest>` reference itself, the image a workflow
#       service container runs (#5112); exit 1 as for `labels`
#   scripts/lib/base-image.sh --self-test
#
# No specification governs this: our own design. The keys are the OCI image
# spec's predefined annotations (image-spec annotations.md).
set -Eeuo pipefail

# The `<name>@<digest>` reference of the last digest-pinned FROM in $1.
base_ref() {
  local dockerfile="$1"
  [[ -f "$dockerfile" ]] || {
    echo "base-image: no such Dockerfile: $dockerfile" >&2
    return 1
  }
  awk '
    toupper($1) == "FROM" {
      for (i = 2; i <= NF; i++) {
        if ($i ~ /^--/) continue
        if ($i ~ /@sha256:[0-9a-f]{64}$/) ref = $i
        break
      }
    }
    END { if (ref != "") print ref }
  ' "$dockerfile"
}

# The literal `<name>@<digest>` base of $1; refuses a tag-only FROM and one
# naming a build argument.
literal_ref() {
  local dockerfile="$1" ref
  ref="$(base_ref "$dockerfile")" || return 1
  if [[ -z "$ref" ]]; then
    echo "base-image: $dockerfile has no digest-pinned FROM" >&2
    return 1
  fi
  if [[ "${ref%@*}" == *'$'* ]]; then
    echo "base-image: the base of $dockerfile names a build argument (${ref%@*}); a reference needs the literal name" >&2
    return 1
  fi
  printf '%s\n' "$ref"
}

labels() {
  local ref
  ref="$(literal_ref "$1")" || return 1
  printf 'org.opencontainers.image.base.name=%s\n' "${ref%@*}"
  printf 'org.opencontainers.image.base.digest=%s\n' "${ref#*@}"
}

self_test() {
  local dir out failures=0 a b
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN
  a="sha256:$(printf 'a%.0s' {1..64})"
  b="sha256:$(printf 'b%.0s' {1..64})"

  expect() {
    local what="$1" want="$2" got="$3"
    if [[ "$got" != "$want" ]]; then
      printf 'base-image self-test: %s\n  want: %s\n  got:  %s\n' "$what" "$want" "$got" >&2
      failures=$((failures + 1))
    fi
  }

  # shellcheck disable=SC2016 # the Dockerfile's own \${…} is the literal under test
  printf 'ARG RUST_VERSION=1.99.1\nFROM rust:${RUST_VERSION}-slim@%s AS builder\nFROM gcr.io/distroless/cc:nonroot@%s AS runtime-base\nFROM runtime-base AS runtime-prebuilt\n' "$a" "$b" >"$dir/staged"
  out="$(labels "$dir/staged")"
  expect "the runtime stage's FROM, past a build stage and a stage on a stage" \
    "org.opencontainers.image.base.name=gcr.io/distroless/cc:nonroot
org.opencontainers.image.base.digest=$b" "$out"

  # shellcheck disable=SC2016 # the Dockerfile's own \${…} is the literal under test
  printf 'FROM --platform=$BUILDPLATFORM postgres:18.6@%s\n' "$a" >"$dir/flagged"
  out="$(labels "$dir/flagged")"
  expect "a FROM with a flag" \
    "org.opencontainers.image.base.name=postgres:18.6
org.opencontainers.image.base.digest=$a" "$out"
  expect "the reference of a FROM with a flag" "postgres:18.6@$a" "$(literal_ref "$dir/flagged")"

  printf 'FROM postgres:18.6\n' >"$dir/tagged"
  if labels "$dir/tagged" >/dev/null 2>&1; then
    echo "base-image self-test: a tag-only FROM was read as a base" >&2
    failures=$((failures + 1))
  fi

  # shellcheck disable=SC2016 # the Dockerfile's own \${…} is the literal under test
  printf 'ARG BASE=x\nFROM ${BASE}@%s\n' "$a" >"$dir/argument"
  if labels "$dir/argument" >/dev/null 2>&1; then
    echo "base-image self-test: a FROM naming a build argument was read as a base" >&2
    failures=$((failures + 1))
  fi

  if labels "$dir/missing" >/dev/null 2>&1; then
    echo "base-image self-test: a missing Dockerfile was read" >&2
    failures=$((failures + 1))
  fi

  for real in docker/Dockerfile docker/viewer/Dockerfile docker/postgres/Dockerfile; do
    out="$(labels "$real")" || {
      echo "base-image self-test: $real yields no base" >&2
      failures=$((failures + 1))
      continue
    }
    [[ "$(grep -c '^org.opencontainers.image.base.digest=sha256:[0-9a-f]\{64\}$' <<<"$out")" -eq 1 ]] || {
      echo "base-image self-test: $real yields a malformed base: $out" >&2
      failures=$((failures + 1))
    }
  done

  if [[ "$failures" -gt 0 ]]; then
    echo "base-image: self-test FAILED ($failures)" >&2
    return 1
  fi
  echo "base-image: self-test OK (the runtime stage past build stages, a flagged FROM and its reference, the refused tag-only and argument FROMs, a missing file, and the three Dockerfiles)."
}

cd "$(dirname "$0")/../.."
case "${1:-}" in
  labels | ref)
    [[ "$#" -eq 2 ]] || {
      echo "usage: $0 labels|ref <dockerfile> | --self-test" >&2
      exit 2
    }
    if [[ "$1" == ref ]]; then literal_ref "$2"; else labels "$2"; fi
    ;;
  --self-test) self_test ;;
  *)
    echo "usage: $0 labels|ref <dockerfile> | --self-test" >&2
    exit 2
    ;;
esac
