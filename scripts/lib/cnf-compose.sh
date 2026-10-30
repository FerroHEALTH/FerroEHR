#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The compose files of the conformance SUT stack, in the order scripts/
# conformance.sh merges them, shared with scripts/checks/compose-reset.sh so the
# guard checks the merge the lane runs (#4391).
#
# The measurement overlay sits right after the base file: Compose v5.6.1 keeps
# a key the overlay `!reset`s when a file between the two sets other keys of
# the same `environment` map, which docker/sut-terminology.yml does.
#
# Usage (sourced): cnf_compose_files <measurement 0|1> <repo root>
#   prints one compose file path per line, in merge order.

cnf_compose_files() {
  local measurement="$1" root="$2"
  printf '%s\n' "$root/docker/sut-ferroehr.yml"
  [[ "$measurement" != 1 ]] || printf '%s\n' "$root/docker/sut-measurement.yml"
  printf '%s\n' \
    "$root/docker/sut-smart.yml" \
    "$root/docker/sut-terminology.yml"
}
