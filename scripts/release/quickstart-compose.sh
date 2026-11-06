#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# The quickstart a release attaches (#5003; no specification governs this: our
# own design). On main, docker-compose.yml pulls the `:main` FerroEHR images
# and carries the configuration main reads. At a `vX.Y.Z` tag the release lane
# writes X.Y.Z in place of `:main` on every FerroEHR image of the files it
# attaches, then holds them to the release with
# `scripts/checks/compose-image-tags.sh --release`, so an attached quickstart
# never pulls `:main`.
#
#   quickstart-compose.sh <version> <file>…
#       Rewrites each file in place, then checks it. Run it on the release
#       lane's own checkout, never on a tree that is committed.
#   quickstart-compose.sh --self-test
#       Proves the rewrite of a copy of docker-compose.yml passes the release
#       check, and that a version other than the workspace's fails it.
set -euo pipefail

usage() {
  echo "usage: quickstart-compose.sh <version> <file>… | --self-test" >&2
  exit 2
}

rewrite() {
  local version="$1" file
  shift
  for file in "$@"; do
    sed -E -i.orig "s#(ghcr\.io/ferrohealth/ferroehr(-postgres|-viewer)?):main([}\"[:space:]]|$)#\1:${version}\3#g" "$file"
    rm -f "${file}.orig"
  done
  bash scripts/checks/compose-image-tags.sh --release "$@"
}

self_test() {
  local work version
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' RETURN
  version="$(sed -nE 's/^version = "(.*)"$/\1/p' Cargo.toml | sed -n 1p)"
  cp docker-compose.yml "$work/docker-compose.yml"
  if ! rewrite "$version" "$work/docker-compose.yml" >/dev/null 2>&1; then
    echo "self-test: the release rewrite of docker-compose.yml failed its check" >&2
    return 1
  fi
  if grep -qE 'ghcr\.io/ferrohealth/ferroehr(-postgres|-viewer)?:main' "$work/docker-compose.yml"; then
    echo "self-test: the release rewrite left a :main pin" >&2
    return 1
  fi
  cp docker-compose.yml "$work/other.yml"
  if rewrite "0.0.1" "$work/other.yml" >/dev/null 2>&1; then
    echo "self-test: a version other than the workspace's passed the release check" >&2
    return 1
  fi
  echo "quickstart-compose: self-test OK (the release rewrite passes; another version fails)"
}

case "${1:-}" in
  --self-test)
    [[ "$#" -eq 1 ]] || usage
    self_test
    ;;
  "") usage ;;
  *)
    [[ "$#" -ge 2 ]] || usage
    rewrite "$@"
    ;;
esac
