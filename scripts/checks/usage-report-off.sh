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
# by scripts that carry it. A `docker run` of the server image carries it on
# the command itself (`-e FERROEHR__USAGE_REPORT__ENABLED…`): a container
# inherits nothing from the shell or the job environment.
#
# The launchers start sibling family servers too, each reporting under its own
# key: a Compose service running a family image (FerroPIX, FerroTERM, ...)
# sets `<PRODUCT>__USAGE_REPORT__ENABLED` to false or to an interpolation of
# FerroEHR's key; a `docker run` of a sibling image hands the container its
# key; a Helm install also passes `ferropix.usageReport.enabled=false` for the
# FerroPIX dependency chart; and a crate whose Rust source starts a sibling
# image (testcontainers) sets that image's key to "false". No openEHR spec
# governs this — our own design.
#
# Exempt: deploy/hosted/ (the hosted sandbox is a real deployment and reports
# like one) and scripts/checks/ (guards read these commands as data).
#
# Usage: scripts/checks/usage-report-off.sh [--self-test]
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

readonly OWN_KEY='FERROEHR__USAGE_REPORT__ENABLED'
readonly SWITCH='FERROEHR__USAGE_REPORT__ENABLED(=|:[[:space:]]*["'\'']?)false|usageReport\.enabled=false'
readonly COMPOSE_UP='(docker[ -]compose|(^|[;&|(`[:space:]])dc)([[:space:]].*)?[[:space:]]up([[:space:]]|$)|\[@\]\}"?[[:space:]]+up([[:space:]]|$)'
readonly HELM_INSTALL='(^|[;&|(`[:space:]])helm[[:space:]]+(upgrade|install)[[:space:]]'
readonly DOCKER_RUN='docker[[:space:]]+run[[:space:]].*(ghcr\.io/ferrohealth/ferroehr[:@]|[[:space:]"'\'']ferroehr[:@]|FERROEHR_IMAGE|APP_IMAGE)'
# The switch handed to a container on its own `docker run` command line.
readonly RUN_SWITCH='(-e|--env)[[:space:]=]+["'\'']?FERROEHR__USAGE_REPORT__ENABLED'
# A one-shot CLI subcommand reads its configuration and exits without serving.
# The FerroPIX dependency chart's own switch, or the dependency switched off.
readonly FERROPIX_CHART_SWITCH='ferropix\.(usageReport\.)?enabled=false'
readonly ONE_SHOT='[[:space:]](config[[:space:]]+check|db[[:space:]]+(migrate|verify)|healthcheck|usage-report)([[:space:]]|$)'

# Prints the file's content without comment lines, one logical line per
# backslash-continued command.
logical_lines() {
  local file="$1"
  awk '/^[[:space:]]*#/ { next }
       { l = $0; if (sub(/\\$/, "", l)) { buf = buf l " "; next } print buf l; buf = "" }
       END { if (buf != "") print buf }' "$file"
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

# Whether $1 starts the server with a `docker run` that does not hand the
# container the switch itself.
bare_docker_run() {
  local runs
  runs="$(logical_lines "$1" | grep -E "$DOCKER_RUN" | grep -vE "$ONE_SHOT" || true)"
  [[ -n "$runs" ]] && grep -qvE "$RUN_SWITCH" <<<"$runs"
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
        grep -qE '^#!.*(ba)?sh' <<<"$(head -n 1 "$f" 2>/dev/null)" && printf '%s\n' "$f"
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
    if bare_docker_run "$f"; then
      printf '%s\n' "$f"
      continue
    fi
    if grep -qE "$HELM_INSTALL" <<<"$(logical_lines "$f")" && ! grep -qE "$FERROPIX_CHART_SWITCH" <<<"$(logical_lines "$f")"; then
      printf '%s (ferropix.usageReport.enabled)\n' "$f"
    fi
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
  sibling_findings "$@"
}

# --- Sibling servers: the family products a launcher starts besides this one.
#
# A Compose file, a `docker run` or a test harness can start another family
# server (FerroEHR, FerroPIX, FerroTERM, FerroFED, FerroBRIDGE), and each one
# reports under its own key, `<PRODUCT>__USAGE_REPORT__ENABLED`. This repo's
# switch stops none of them on its own.
readonly FAMILY='ferroehr|ferropix|ferroterm|ferrofed|ferrobridge'
readonly FAMILY_IMAGE="ghcr\\.io/ferrohealth/($FAMILY)[:@]"

# Prints `file: service (KEY)` for each service of a Compose file that runs a
# family server image without that server's switch in its own `environment`:
# KEY set to false, or to an interpolation of OWN_KEY, which every Compose
# launcher exports as false. A service that merges an `x-` anchor
# (`<<: *name`) runs the anchor's image.
compose_findings() {
  awk -v own="$OWN_KEY" -v img="$FAMILY_IMAGE" -v file="$1" -v sq="'" '
    function close_block() {
      if (kind == "anchor" && product != "") anchor_image[name] = product
      if (kind == "service") { svc[++n] = name; svc_image[name] = product; svc_merge[name] = merge }
      kind = ""; name = ""; product = ""; merge = ""
    }
    /^[[:space:]]*#/ { next }
    /^[^[:space:]]/ {
      close_block(); in_services = ($0 ~ /^services:/)
      if (match($0, /&[A-Za-z0-9_-]+/)) { kind = "anchor"; name = substr($0, RSTART + 1, RLENGTH - 1) }
      next
    }
    in_services && /^  [A-Za-z0-9_.-]+:/ {
      close_block(); kind = "service"; name = $0; sub(/^  /, "", name); sub(/:.*/, "", name); next
    }
    kind == "" { next }
    /^[[:space:]]+image:/ && match($0, img) { product = toupper(substr($0, RSTART + 20, RLENGTH - 21)) }
    match($0, /<<:[[:space:]]*\*[A-Za-z0-9_-]+/) { merge = substr($0, RSTART, RLENGTH); sub(/.*\*/, "", merge) }
    match($0, /[A-Z]+__USAGE_REPORT__ENABLED[[:space:]]*[:=]/) {
      key = substr($0, RSTART, RLENGTH); sub(/[[:space:]]*[:=]$/, "", key)
      value = substr($0, RSTART + RLENGTH); gsub(sq, "", value); gsub(/^[[:space:]"]+|[[:space:]"]+$/, "", value)
      if (kind == "service" && (value == "false" || index(value, "${" own) > 0)) ok[name, key] = 1
    }
    END {
      close_block()
      for (i = 1; i <= n; i++) {
        s = svc[i]; p = svc_image[s]
        if (p == "" && svc_merge[s] != "") p = anchor_image[svc_merge[s]]
        if (p != "" && !((s, p "__USAGE_REPORT__ENABLED") in ok))
          printf "%s: %s (%s__USAGE_REPORT__ENABLED)\n", file, s, p
      }
    }
  ' "$1"
}

# Prints `file (KEY)` for each `docker run` in $1 that starts a sibling family
# server without handing the container that server's switch on its own
# command line. A one-shot job starts no server.
sibling_run_findings() {
  local line product key
  while IFS= read -r line; do
    product="$(grep -oE "$FAMILY_IMAGE" <<<"$line" | sed -nE '1{s|.*/||; s|[:@]$||; p;}' | tr '[:lower:]' '[:upper:]')"
    key="${product}__USAGE_REPORT__ENABLED"
    [[ -n "$product" && "$key" != "$OWN_KEY" ]] || continue
    grep -qE '[[:space:]](healthcheck|--version|--help|config[[:space:]]+check)([[:space:]]|$)' <<<"$line" && continue
    grep -qE "(-e|--env)[[:space:]=]+[\"']?${key}=false" <<<"$line" || printf '%s (%s)\n' "$1" "$key"
  done < <(logical_lines "$1" | grep -E "docker[[:space:]]+run[[:space:]].*$FAMILY_IMAGE" || true)
}

# Prints `crate (KEY)` for each crate whose Rust source names a sibling
# family server image (`"ghcr.io/ferrohealth/<product>"`) while no Rust
# source of that crate sets `"<PRODUCT>__USAGE_REPORT__ENABLED", "false"` on
# the container. $@ = the file-listing command.
rust_findings() {
  local files f crate product key
  files="$("$@" | grep -E '\.rs$' || true)"
  [[ -n "$files" ]] || return 0
  # shellcheck disable=SC2086 # one word per tracked path, none with spaces
  while IFS= read -r f; do
    crate="$(cut -d/ -f1-2 <<<"$f")"
    while IFS= read -r product; do
      key="$(tr '[:lower:]' '[:upper:]' <<<"$product")__USAGE_REPORT__ENABLED"
      [[ "$key" != "$OWN_KEY" ]] || continue
      # shellcheck disable=SC2046 # one word per tracked path, none with spaces
      grep -qE "\"${key}\",[[:space:]]*\"false\"" $(grep -E "^${crate}/" <<<"$files") ||
        printf '%s (%s)\n' "$crate" "$key"
    done < <(grep -oE "\"ghcr\\.io/ferrohealth/($FAMILY)\"" "$f" | sed -E 's|.*/||; s|"$||' | sort -u)
  done < <(grep -lE "\"ghcr\\.io/ferrohealth/($FAMILY)\"" $files 2>/dev/null || true)
}

# The Compose files among the listed ones: YAML with a top-level `services:`.
compose_files() {
  local f
  while IFS= read -r f; do
    case "$f" in
      .github/* | deploy/hosted/* | scripts/checks/*) continue ;;
      *.yml | *.yaml) grep -qE '^services:' "$f" 2>/dev/null && printf '%s\n' "$f" ;;
      *) ;;
    esac
  done < <("$@")
}

# Prints every sibling finding over the listed files.
sibling_findings() {
  local f
  while IFS= read -r f; do compose_findings "$f"; done < <(compose_files "$@")
  while IFS= read -r f; do [[ -n "$f" ]] && sibling_run_findings "$f"; done <<<"$(candidates "$@")"
  rust_findings "$@"
}

# Lists the self-test's fixture tree the way `git ls-files` lists the repo.
tracked_fixtures() { find . -type f | sed 's|^\./||'; }

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
  printf '#!/usr/bin/env bash\nhelm upgrade --install r chart \\\n  --set usageReport.enabled=false \\\n  --set ferropix.usageReport.enabled=false\n' >"$tmp/scripts/helm-off.sh"
  printf '#!/usr/bin/env bash\nhelm upgrade --install r chart \\\n  --set usageReport.enabled=false\n' >"$tmp/scripts/helm-pix-on.sh"
  # Sibling servers: Compose services, a docker run, and a Rust harness.
  mkdir -p "$tmp/app/bare/tests" "$tmp/app/held/src"
  # shellcheck disable=SC2016 # the fixture is literal Compose text, never expanded here
  printf 'x-node: &node\n  image: ghcr.io/ferrohealth/ferroehr:5.4.2\nservices:\n  ferroehr:\n    image: ghcr.io/ferrohealth/ferroehr:5.4.2\n    environment:\n      FERROEHR__USAGE_REPORT__ENABLED: ${FERROEHR__USAGE_REPORT__ENABLED:-true}\n  ferropix:\n    image: ghcr.io/ferrohealth/ferropix:0.0.7\n    environment:\n      FERROPIX_CONFIG: /etc/ferropix.toml\n  ferroterm:\n    image: ghcr.io/ferrohealth/ferroterm:0.1.6\n    environment:\n      FERROTERM__USAGE_REPORT__ENABLED: "false"\n  node-a:\n    <<: *node\n    environment:\n      FERROEHR__DB__URL: x\n  ferroehr-postgres:\n    image: ghcr.io/ferrohealth/ferroehr-postgres:5.4.2\n' >"$tmp/docker-compose.sib.yml"
  printf 'services:\n  ferropix:\n    image: ghcr.io/ferrohealth/ferropix:0.0.7\n' >"$tmp/deploy/hosted/docker-compose.yml"
  printf '#!/usr/bin/env bash\nexport FERROEHR__USAGE_REPORT__ENABLED=false\ndocker run -d ghcr.io/ferrohealth/ferropix:0.0.7\n' >"$tmp/scripts/pix-run-on.sh"
  printf '#!/usr/bin/env bash\ndocker run -d -e FERROPIX__USAGE_REPORT__ENABLED=false ghcr.io/ferrohealth/ferropix:0.0.7\n' >"$tmp/scripts/pix-run-off.sh"
  printf 'const IMAGE: &str = "ghcr.io/ferrohealth/ferroterm";\n' >"$tmp/app/bare/tests/term.rs"
  printf 'const IMAGE: &str = "ghcr.io/ferrohealth/ferroterm";\n' >"$tmp/app/held/src/images.rs"
  printf 'fn f(i: I) -> I {\n    i.with_env_var("FERROTERM__USAGE_REPORT__ENABLED", "false")\n}\n' >"$tmp/app/held/src/start.rs"
  printf '#!/usr/bin/env bash\n# docker compose up -d\necho done\n' >"$tmp/scripts/commented.sh"
  # shellcheck disable=SC2016 # the fixture is literal shell text, never expanded here
  printf 'dc() { docker compose "$@"; }\ndc -f docker-compose.yml up -d ferroehr\n' >"$tmp/scripts/fam/covered.sh"
  printf '#!/usr/bin/env bash\nexport FERROEHR__USAGE_REPORT__ENABLED=false\n. scripts/fam/covered.sh\n' >"$tmp/scripts/carrier.sh"
  printf 'dc -f docker-compose.yml up -d ferroehr\n' >"$tmp/scripts/fam/orphan.sh"
  printf '#!/usr/bin/env bash\n. scripts/fam/orphan.sh\n' >"$tmp/scripts/bare-carrier.sh"
  printf 'jobs:\n  a:\n    steps:\n      - run: |\n          docker run -d ghcr.io/ferrohealth/ferroehr:main\n' >"$tmp/.github/workflows/run-on.yml"
  # shellcheck disable=SC2016 # the fixture is literal shell text, never expanded here
  printf 'jobs:\n  a:\n    env:\n      FERROEHR__USAGE_REPORT__ENABLED: "false"\n    steps:\n      - run: docker run -d "$FERROEHR_IMAGE"\n' >"$tmp/.github/workflows/run-job-env.yml"
  # shellcheck disable=SC2016 # the fixture is literal shell text, never expanded here
  printf 'jobs:\n  a:\n    env:\n      FERROEHR__USAGE_REPORT__ENABLED: "false"\n    steps:\n      - run: docker run -d -e FERROEHR__USAGE_REPORT__ENABLED "$FERROEHR_IMAGE"\n' >"$tmp/.github/workflows/run-off.yml"
  printf '#!/usr/bin/env bash\ndocker compose up -d\n' >"$tmp/deploy/hosted/box.sh"
  printf '#!/usr/bin/env bash\ngrep "docker compose up" x\n' >"$tmp/scripts/checks/guard.sh"
  printf '#!/usr/bin/env bash\ndocker run --rm ferroehr-viewer:smoke\n' >"$tmp/docker/viewer.sh"
  printf '#!/usr/bin/env bash\ndocker run --rm ferroehr:main config check\n' >"$tmp/docker/one-shot.sh"
  # shellcheck disable=SC2016 # the fixture is literal shell text, never expanded here
  printf '#!/usr/bin/env bash\ndocker run --rm -v "$d:/etc/ferroehr:ro" "$IMAGE" x\n' >"$tmp/docker/mount.sh"

  found="$(cd "$tmp" && findings tracked_fixtures)"
  for name in scripts/on.sh scripts/array-on.sh scripts/long-on.sh scripts/helm-on.sh scripts/fam/orphan.sh \
    .github/workflows/run-on.yml .github/workflows/run-job-env.yml "scripts/helm-pix-on.sh (ferropix.usageReport.enabled)" \
    "docker-compose.sib.yml: ferropix (FERROPIX__USAGE_REPORT__ENABLED)" \
    "docker-compose.sib.yml: node-a (FERROEHR__USAGE_REPORT__ENABLED)" \
    "scripts/pix-run-on.sh (FERROPIX__USAGE_REPORT__ENABLED)" "app/bare (FERROTERM__USAGE_REPORT__ENABLED)"; do
    grep -qxF "$name" <<<"$found" || {
      echo "self-test: $name was not caught" >&2
      failures=1
    }
  done
  for name in scripts/off.sh scripts/helm-off.sh scripts/commented.sh scripts/fam/covered.sh scripts/carrier.sh \
    scripts/bare-carrier.sh .github/workflows/run-off.yml deploy/hosted/box.sh scripts/checks/guard.sh docker/viewer.sh \
    docker/one-shot.sh docker/mount.sh scripts/pix-run-off.sh; do
    if grep -qF "$name" <<<"$found"; then
      echo "self-test: $name was wrongly caught" >&2
      failures=1
    fi
  done
  # Only the two unswitched services of the Compose fixture, none in an exempt tree.
  if grep -E 'docker-compose\.sib\.yml: (ferroehr|ferroterm|ferroehr-postgres) |deploy/hosted|app/held' <<<"$found"; then
    echo "self-test: a switched, exempt or non-server service was wrongly caught" >&2
    failures=1
  fi
  [[ "$failures" -eq 0 ]] || return 1
  echo "usage-report-off: self-test OK (twelve unswitched launchers caught, sixteen clean or exempt cases allowed)."
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
  echo "to helm) in the launcher, or in every script that sources it; a docker run of the" >&2
  echo "server passes -e FERROEHR__USAGE_REPORT__ENABLED=false on its own command line." >&2
  echo "A sibling server named (KEY) takes its own key: false, or \${FERROEHR__USAGE_REPORT__ENABLED...}" >&2
  echo "in its Compose environment, -e KEY=false on a docker run, .with_env_var(\"KEY\", \"false\") in a" >&2
  echo "Rust harness, and --set ferropix.usageReport.enabled=false on a Helm install." >&2
  return 1
}

case "${1:-}" in
  --self-test) self_test ;;
  "") report "$(findings git ls-files)" ;;
  *) guard_usage "[--self-test]" ;;
esac
