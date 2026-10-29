#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Compose `!reset` guard (#4391): every key a committed compose overlay removes
# with `!reset` is absent from the merged model its launcher composes.
#
# Compose v5.6.1 keeps a key an overlay `!reset`s when a file merged between
# the base and the overlay sets other keys of the same `environment` map
# (docs.docker.com/reference/compose-file/merge/#reset-value describes the
# tag, not this case). The measurement overlay met it (#4281): the limiter it
# switched off came back. So each overlay that uses the tag is registered
# below with the stack its launcher merges, in that order, and the guard reads
# the merged model with `docker compose config`. An overlay that uses the tag
# and is not registered fails the guard.
#
# --self-test reproduces the Compose behaviour with three files and says
# whether this Compose drops the reset, so the behaviour is re-checked on every
# Compose the runners carry, and proves the check refuses a merged model that
# still holds a reset key.
#
# Usage: scripts/checks/compose-reset.sh [--self-test]
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--self-test]" "--self-test" "$@"

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
# shellcheck source=scripts/lib/cnf-compose.sh
. scripts/lib/cnf-compose.sh

command -v docker >/dev/null || { echo "compose-reset: docker is required" >&2; exit 1; }
command -v jq >/dev/null || { echo "compose-reset: jq is required" >&2; exit 1; }

# The keys a compose file removes with `!reset`, one per line.
reset_keys() {
  sed -nE 's/^[[:space:]]*([A-Za-z0-9_.-]+):[[:space:]]*!reset([[:space:]].*)?$/\1/p' "$1"
}

# check <service> <keys file> <compose args…>: 0 when the merged model's
# environment of <service> holds none of the keys.
check() {
  local service="$1" keys="$2" model held
  shift 2
  model="$(docker compose "$@" config --format json)"
  held="$(jq -r --arg s "$service" --rawfile k "$keys" '
    ($k | split("\n") | map(select(length > 0))) as $keys
    | (.services[$s].environment // {}) | keys | map(select(. as $n | $keys | index($n))) | join(" ")
  ' <<<"$model")"
  if [[ -n "$held" ]]; then
    echo "compose-reset: the merged model still sets $held on $service, which the overlay resets" >&2
    return 1
  fi
}

# The stack each registered overlay is merged into, as compose arguments, and
# the service whose environment it resets. A new overlay that uses `!reset`
# adds a case here.
stack_of() {
  case "$1" in
    docker/sut-measurement.yml)
      echo ferroehr
      echo -p
      echo ferroehr-cnf
      echo --project-directory
      echo "$ROOT"
      echo --profile
      echo terminology
      local file
      while IFS= read -r file; do
        echo -f
        echo "$file"
      done < <(cnf_compose_files 1 "$ROOT")
      ;;
    *) return 1 ;;
  esac
}

self_test() {
  local work version
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # the path is expanded now, on purpose
  trap "rm -rf '$work'" EXIT
  printf 'services:\n  app:\n    image: x\n    environment:\n      FOO: "1"\n      BAR: b\n' > "$work/base.yml"
  printf 'services:\n  app:\n    environment:\n      BAZ: z\n' > "$work/between.yml"
  printf 'services:\n  app:\n    environment:\n      FOO: !reset null\n' > "$work/reset.yml"
  echo FOO > "$work/keys"
  version="$(docker compose version --short 2>/dev/null || echo unknown)"
  if ! check app "$work/keys" -p reset-self-test --project-directory "$work" \
    -f "$work/base.yml" -f "$work/reset.yml" -f "$work/between.yml" > /dev/null 2>&1; then
    echo "compose-reset: self-test: Compose $version keeps a key reset right after its base file" >&2
    exit 1
  fi
  if check app "$work/keys" -p reset-self-test --project-directory "$work" \
    -f "$work/base.yml" -f "$work/between.yml" -f "$work/reset.yml" > /dev/null 2>&1; then
    echo "compose-reset: Compose $version honours a reset behind an intermediate file"
  else
    echo "compose-reset: Compose $version drops a reset behind an intermediate file (#4391); overlays sit right after their base"
  fi
  if check app "$work/keys" -p reset-self-test --project-directory "$work" \
    -f "$work/base.yml" > /dev/null 2>&1; then
    echo "compose-reset: self-test: a merged model holding a reset key was not refused" >&2
    exit 1
  fi
  echo "compose-reset: self-test OK (a reset right after its base holds, and a merged model holding a reset key is refused)."
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit 0
fi

failures=0
checked=0
keys="$(mktemp)"
trap 'rm -f "$keys"' EXIT
while IFS= read -r file; do
  reset_keys "$file" > "$keys"
  [[ -s "$keys" ]] || continue
  if ! stack="$(stack_of "$file")"; then
    echo "compose-reset: $file uses !reset and registers no stack in scripts/checks/compose-reset.sh" >&2
    failures=$((failures + 1))
    continue
  fi
  args=()
  while IFS= read -r line; do args+=("$line"); done <<<"$stack"
  service="${args[0]}"
  if check "$service" "$keys" "${args[@]:1}"; then
    echo "compose-reset: $file removes $(paste -sd' ' "$keys") from $service in the stack its launcher merges"
  else
    failures=$((failures + 1))
  fi
  checked=$((checked + 1))
done < <(git ls-files 'docker-compose*.yml' 'docker/*.yml' 'deploy/*.yml' 'deploy/**/*.yml' 'compose*.yml' | sort -u)

if [[ "$failures" -ne 0 ]]; then
  exit 1
fi
echo "compose-reset: OK ($checked overlay(s) with !reset checked)."
