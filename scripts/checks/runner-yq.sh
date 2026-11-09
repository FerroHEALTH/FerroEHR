#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/checks/runner-yq.sh: the yq a CI job uses is the runner image's, and
# this check refuses it unless it is mikefarah's yq at or above the version the
# job states in YQ_MIN_VERSION (#5256).
#
# The jobs fetched yq from a GitHub release on every run until 2026-11-09, when
# that download answered 504 all day and turned unrelated pull requests red.
# The GitHub-hosted Ubuntu images ship mikefarah's yq (actions/runner-images,
# images/ubuntu/Ubuntu2404-Readme.md), so the job reads that one and fails here,
# naming both versions, when an image ever ships an older or a different yq.
#
# Usage: YQ_MIN_VERSION=4.53.3 bash scripts/checks/runner-yq.sh
set -euo pipefail

min="${YQ_MIN_VERSION:?set YQ_MIN_VERSION to the oldest yq the job accepts, e.g. 4.53.3}"
min="${min#v}"

if ! command -v yq >/dev/null 2>&1; then
  echo "::error::no yq on PATH; this job needs mikefarah's yq ${min} or newer from the runner image"
  exit 1
fi

banner="$(yq --version 2>&1)"
if [[ "$banner" != *"mikefarah/yq"* ]]; then
  echo "::error::the yq on PATH ($(command -v yq)) is not mikefarah's yq: ${banner}"
  exit 1
fi

have="$(sed -nE 's/.*version v?([0-9]+\.[0-9]+\.[0-9]+).*/\1/p' <<<"$banner")"
if [[ -z "$have" ]]; then
  echo "::error::cannot read a version out of: ${banner}"
  exit 1
fi

oldest="$(printf '%s\n%s\n' "$min" "$have" | sort -V | head -n1)"
if [[ "$oldest" != "$min" ]]; then
  echo "::error::the runner's yq is ${have}; this job needs ${min} or newer"
  exit 1
fi

echo "yq ${have} at $(command -v yq) (needs ${min} or newer)"
