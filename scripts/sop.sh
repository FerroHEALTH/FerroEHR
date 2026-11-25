#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The pinned `ferrohealth-sop`, the Stateless OpenPGP command line of
# ferrohealth-openpgp-pq (FerroSYS), which generates the ML-DSA-65+Ed25519
# (RFC 9980 algorithm ID 30) signing keys of the deploy probes and the perf
# gate. docs/VERSIONS.md pins its version.
#
# Usage:
#   scripts/sop.sh pin       print the pinned version
#   scripts/sop.sh install   build exactly that version into target/tools/sop/
#   scripts/sop.sh path      print the bin directory to put on PATH
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

die() {
  echo "sop: $*" >&2
  exit 1
}

pin() {
  local version
  version=$(awk -F'|' '$2 ~ /`ferrohealth-openpgp-pq`/ { split($3, cell, "`"); print cell[2] }' "$ROOT/docs/VERSIONS.md")
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "docs/VERSIONS.md pins no ferrohealth-openpgp-pq version"
  echo "$version"
}

case "${1:-}" in
  pin) pin ;;
  install) cargo install ferrohealth-openpgp-pq --version "=$(pin)" --locked --root "$ROOT/target/tools/sop" ;;
  path) echo "$ROOT/target/tools/sop/bin" ;;
  *) die "usage: $0 pin | install | path" ;;
esac
