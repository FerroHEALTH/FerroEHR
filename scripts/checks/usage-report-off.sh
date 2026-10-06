#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Test launchers switch the usage report off.
#
# The shipped default sends a usage report, so every instance a test lane
# starts from the quickstart compose file or the chart would report from a
# throwaway runner and count as a live installation. A launcher is a tracked
# shell script or workflow that runs `docker compose … up`, `helm install` /
# `helm upgrade`, or `docker run` of the server image. Each one carries the
# switch itself (`FERROEHR__USAGE_REPORT__ENABLED=false`, the same key in a
# workflow `env:` map, or `usageReport.enabled=false`), or is only ever sourced
# by scripts that carry it. No openEHR spec governs this — our own design.
#
# Exempt: deploy/hosted/ (the hosted sandbox is a real deployment and reports
# like one) and scripts/checks/ (guards read these commands as data).
#
# Usage: scripts/checks/usage-report-off.sh [--self-test]
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

readonly SWITCH='FERROEHR__USAGE_REPORT__ENABLED(=|:[[:space:]]*["'\'']?)false|usageReport\.enabled=false'
readonly COMPOSE_UP='(docker[ -]compose|(^|[;&|(`[:space:]])dc)([[:space:]].*)?[[:space:]]up([[:space:]]|$)|\[@\]\}"?[[:space:]]+up([[:space:]]|$)'
readonly HELM_INSTALL='(^|[;&|(`[:space:]])helm[[:space:]]+(upgrade|install)[[:space:]]'
readonly DOCKER_RUN='docker[[:space:]]+run[[:space:]].*(ghcr\.io/ferrohealth/ferroehr[:@]|[[:space:]"'\'']ferroehr[:@]|FERROEHR_IMAGE|APP_IMAGE)'
# A one-shot CLI subcommand reads its configuration and exits without serving.
readonly ONE_SHOT='[[:space:]](config[[:space:]]+check|db[[:space:]]+(migrate|verify)|healthcheck|usage-report)([[:space:]]|$)'

# Prints the file's content without comment lines, one logical line per
# backslash-continued command.
logical_lines() {
  awk '/^[[:space:]]*#/ { next }
       { l = $0; if (sub(/\\$/, "", l)) { buf = buf l " "; next } print buf l; buf = "" }
       END { if (buf != "") print buf }' "$1"
}

# The predicates grep a captured copy, never a pipe: `grep -q` exits at the
# first match, and under pipefail the writer's SIGPIPE would read as no match.
is_launcher() {
  local lines runs
  lines="$(logical_lines "$1")"
  grep -qE "$COMPOSE_UP|$HELM_INSTALL" <<<"$lines" && return 0
  runs="$(grep -E "$DOCKER_RUN" <<<"$lines" || true)"
  [[ -n "$runs" ]] && grep -qvE "$ONE_SHOT" <<<"$runs"
}

carries_switch() {
  local lines
  lines="$(logical_lines "$1")"
  grep -qE "$SWITCH" <<<"$lines"
}

# Lists the candidate files under the current directory: shell scripts and
# workflow/action YAML, minus the exempt trees.
candidates() {
  local f
  while IFS= read -r f; do
    case "$f" in
      deploy/hosted/* | scripts/checks/*) continue ;;
      .github/*.yml | .github/*.yaml | *.sh) printf '%s\n' "$f" ;;
      scripts/* | docker/* | deploy/*)
        head -n 1 "$f" 2>/dev/null | grep -qE '^#!.*(ba)?sh' && printf '%s\n' "$f"
        ;;
      *) ;;
    esac
  done < <("$@" | grep -E '^(scripts|docker|deploy|\.github/(workflows|actions))/' | sort)
}

# Prints every candidate that sources `$1` (`. path` or `source path`).
sourcers() {
  local target="$1" list="$2" f
  while IFS= read -r f; do
    [[ "$f" = "$target" ]] && continue
    grep -qE "^[[:space:]]*(\.|source)[[:space:]]+[\"']?([^[:space:]]*/)?${target//./\\.}([\"'[:space:]]|$)" "$f" &&
      printf '%s\n' "$f"
  done <<<"$list"
}

# Prints one finding per launcher that neither carries the switch nor is
# covered by every script that sources it. $@ = the file-listing command.
findings() {
  local list f carriers c covered
  list="$(candidates "$@")"
  while IFS= read -r f; do
    [[ -n "$f" ]] || continue
    is_launcher "$f" || continue
    carries_switch "$f" && continue
    carriers="$(sourcers "$f" "$list")"
    covered=0
    if [[ -n "$carriers" ]]; then
      covered=1
      while IFS= read -r c; do
        carries_switch "$c" || covered=0
      done <<<"$carriers"
    fi
    [[ "$covered" -eq 1 ]] || printf '%s\n' "$f"
  done <<<"$list"
}

self_test() {
  local tmp found failures=0 name
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" RETURN
  mkdir -p "$tmp/scripts/fam" "$tmp/docker" "$tmp/deploy/hosted" "$tmp/.github/workflows" "$tmp/scripts/checks"
  printf '#!/usr/bin/env bash\nexport FERROEHR__USAGE_REPORT__ENABLED=false\ndocker compose -f x.yml up -d\n' >"$tmp/scripts/off.sh"
  printf '#!/usr/bin/env bash\ndocker compose -f docker-compose.yml up -d --wait\n' >"$tmp/scripts/on.sh"
  # shellcheck disable=SC2016 # the fixture is literal shell text, never expanded here
  printf '#!/usr/bin/env bash\nC=(docker compose)\n"${C[@]}" up -d\n' >"$tmp/scripts/array-on.sh"
  # A launch early in a long file: a predicate that piped into grep -q would miss it.
  { printf '#!/usr/bin/env bash\ndocker compose up -d\n'; seq 1 20000 | sed 's/^/echo /'; } >"$tmp/scripts/long-on.sh"
  printf '#!/usr/bin/env bash\nhelm upgrade --install r chart \\\n  --set image.tag=x\n' >"$tmp/scripts/helm-on.sh"
  printf '#!/usr/bin/env bash\nhelm upgrade --install r chart \\\n  --set usageReport.enabled=false\n' >"$tmp/scripts/helm-off.sh"
  printf '#!/usr/bin/env bash\n# docker compose up -d\necho done\n' >"$tmp/scripts/commented.sh"
  # shellcheck disable=SC2016 # the fixture is literal shell text, never expanded here
  printf 'dc() { docker compose "$@"; }\ndc -f docker-compose.yml up -d ferroehr\n' >"$tmp/scripts/fam/covered.sh"
  printf '#!/usr/bin/env bash\nexport FERROEHR__USAGE_REPORT__ENABLED=false\n. scripts/fam/covered.sh\n' >"$tmp/scripts/carrier.sh"
  printf 'dc -f docker-compose.yml up -d ferroehr\n' >"$tmp/scripts/fam/orphan.sh"
  printf '#!/usr/bin/env bash\n. scripts/fam/orphan.sh\n' >"$tmp/scripts/bare-carrier.sh"
  printf 'jobs:\n  a:\n    steps:\n      - run: |\n          docker run -d ghcr.io/ferrohealth/ferroehr:main\n' >"$tmp/.github/workflows/run-on.yml"
  # shellcheck disable=SC2016 # the fixture is literal shell text, never expanded here
  printf 'jobs:\n  a:\n    env:\n      FERROEHR__USAGE_REPORT__ENABLED: "false"\n    steps:\n      - run: docker run -d "$FERROEHR_IMAGE"\n' >"$tmp/.github/workflows/run-off.yml"
  printf '#!/usr/bin/env bash\ndocker compose up -d\n' >"$tmp/deploy/hosted/box.sh"
  printf '#!/usr/bin/env bash\ngrep "docker compose up" x\n' >"$tmp/scripts/checks/guard.sh"
  printf '#!/usr/bin/env bash\ndocker run --rm ferroehr-viewer:smoke\n' >"$tmp/docker/viewer.sh"
  printf '#!/usr/bin/env bash\ndocker run --rm ferroehr:main config check\n' >"$tmp/docker/one-shot.sh"
  # shellcheck disable=SC2016 # the fixture is literal shell text, never expanded here
  printf '#!/usr/bin/env bash\ndocker run --rm -v "$d:/etc/ferroehr:ro" "$IMAGE" x\n' >"$tmp/docker/mount.sh"

  found="$(cd "$tmp" && findings find scripts docker deploy .github -type f)"
  for name in scripts/on.sh scripts/array-on.sh scripts/long-on.sh scripts/helm-on.sh scripts/fam/orphan.sh .github/workflows/run-on.yml; do
    grep -qxF "$name" <<<"$found" || {
      echo "self-test: $name was not caught" >&2
      failures=1
    }
  done
  for name in scripts/off.sh scripts/helm-off.sh scripts/commented.sh scripts/fam/covered.sh scripts/carrier.sh \
    scripts/bare-carrier.sh .github/workflows/run-off.yml deploy/hosted/box.sh scripts/checks/guard.sh docker/viewer.sh \
    docker/one-shot.sh docker/mount.sh; do
    if grep -qxF "$name" <<<"$found"; then
      echo "self-test: $name was wrongly caught" >&2
      failures=1
    fi
  done
  [[ "$failures" -eq 0 ]] || return 1
  echo "usage-report-off: self-test OK (six unswitched launchers caught, twelve clean or exempt files allowed)."
}

report() {
  local found="$1"
  if [[ -z "$found" ]]; then
    echo "usage-report-off: OK (every test launcher switches the usage report off)."
    return 0
  fi
  echo "error: a launcher starts a FerroEHR instance with the usage report on" >&2
  local f
  while IFS= read -r f; do printf '  %s\n' "$f" >&2; done <<<"$found"
  echo "Export FERROEHR__USAGE_REPORT__ENABLED=false (or pass --set usageReport.enabled=false" >&2
  echo "to helm) in the launcher, or in every script that sources it." >&2
  return 1
}

case "${1:-}" in
  --self-test) self_test ;;
  "") report "$(findings git ls-files)" ;;
  *) guard_usage "[--self-test]" ;;
esac
