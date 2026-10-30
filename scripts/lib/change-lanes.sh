#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Which CI lanes a change needs: the path classifiers of the `changes` jobs in
# ci.yml, docs.yml and sonar.yml, in one place (#4208).
#
# It reads the changed-path list on stdin, whole, and prints one `lane=true`
# or `lane=false` line per lane of the named workflow, the form
# $GITHUB_OUTPUT takes. Every match runs against a here-string. The
# classifiers used to pipe the list into `grep -q` under pipefail: once the
# list topped the 64 KiB pipe buffer, grep exited at an early match, the echo
# died of SIGPIPE, the pipeline returned 141, and the `if` read the match as a
# miss, so a large pull request skipped the lanes it needed.
#
# An empty list answers false for every lane. The caller keeps its own
# fallback, every lane true, for a base it cannot diff against.
#
# Usage:
#   scripts/lib/change-lanes.sh <ci|docs|sonar> < paths
#   scripts/lib/change-lanes.sh --self-test
set -euo pipefail

# The lanes, one per line: workflow|lane|extended regex over one path.
# Each lane's reason, with the issue that set it:
#
# ci deploy: the deployment harness, the compose files and the server image.
# ci code: the Rust trees and manifests, the shared corpus (#2811), and the
# only scripts a code-gated job runs (the drift check and the e2e harness);
# any other script costs no Rust run.
# ci msrv (#3120): what can move the minimum Rust version, a manifest, the
# lockfile, the toolchain, or the recipe; a source edit cannot.
# ci viewer (#335, #3190): the viewer, its harness, the stack it drives, the
# inputs to the server it runs against and the boot-installed policy and
# access configuration.
# ci server_dockerfile: the Dockerfile and the build-context ignore list.
# ci viewer_dockerfile (#4222): the viewer Dockerfile and the same ignore list.
# ci helm (#2110): the chart and the recipe. ci chart_boot (#2159) is a
# superset: the overlays and the configuration they are written against.
# ci workflows (#2373): the recipe of the cargo lanes.
# ci vex (#2370): the OpenVEX documents and the checks the deny job runs.
# docs: the site sources and every input of the build's own gates.
# sonar: the scanned trees, their manifests, the scan scope and the recipe.
readonly LANES='ci|deploy|^(scripts/deploy-probe|docker-compose|docker/)
ci|code|^scripts/(checks/codegen-drift|ui-e2e)\.sh$|^(app/|crates/|tools/|corpus/|\.cargo/|\.config/|Cargo\.toml$|Cargo\.lock$|rust-toolchain\.toml$|rustfmt\.toml$|clippy\.toml$|deny\.toml$)
ci|msrv|(^|/)Cargo\.toml$|^Cargo\.lock$|^rust-toolchain\.toml$|^clippy\.toml$|^\.github/workflows/ci\.yml$
ci|viewer|^(app/ferroehr-viewer/|app/ferroehr-server/src/|app/ferroehr/src/privacy/|app/ferroehr/src/config/|app/ferroehr-rest/src/extensions/access/|app/ferroehr-rest/src/smart/|app/ferroehr-rest/src/config\.rs$|app/ferroehr-rest/src/router\.rs$|docker/ferroehr\.dev\.toml$|app/ferroehr/assets/ferroehr\.default\.toml$|scripts/ui-e2e\.sh$|docker-compose\.yml$|docker-compose\.override\.yml$|docker-compose\.dev\.yml$|docker-compose\.keycloak\.yml$|docker/Dockerfile$|docker/viewer/|docker/keycloak/|docker/postgres/|Cargo\.lock$|\.github/workflows/ci\.yml$)
ci|server_dockerfile|^(docker/Dockerfile|\.dockerignore)$
ci|viewer_dockerfile|^(docker/viewer/Dockerfile|\.dockerignore)$
ci|helm|^(deploy/helm/|\.github/workflows/ci\.yml$)
ci|chart_boot|^(deploy/helm/|\.github/workflows/ci\.yml$|docker/Dockerfile$|app/ferroehr/src/config/|app/ferroehr/assets/ferroehr\.default\.toml$|app/ferroehr-rest/src/config\.rs$|app/ferroehr/src/report\.rs$)
ci|workflows|^\.github/workflows/ci\.yml$
ci|vex|^(security/vex/|scripts/checks/(vex-reachability|advisory-exceptions|vex-linkage)\.sh$)
docs|docs|^(website/|docs/conformance/|scripts/(render|site)/|scripts/lib/veredictum\.sh$|scripts/checks/(docs-claims|conformance-numbers)\.sh$|\.github/workflows/docs\.yml$|\.github/actions/docs-toolchain/)
sonar|code|^(app/|crates/|tools/|Cargo\.(toml|lock)$|rust-toolchain\.toml$|clippy\.toml$|sonar-project\.properties$|\.github/workflows/sonar\.yml$)'

# classify <workflow> <paths>: print `lane=true|false` for each of its lanes.
classify() {
  local workflow="$1" paths="$2" set lane regex found=0
  while IFS='|' read -r set lane regex; do
    [[ "$set" == "$workflow" ]] || continue
    found=1
    if [[ -n "$paths" ]] && grep -qE -- "$regex" <<<"$paths"; then
      printf '%s=true\n' "$lane"
    else
      printf '%s=false\n' "$lane"
    fi
  done <<<"$LANES"
  [[ "$found" -eq 1 ]] || { echo "change-lanes: unknown workflow '$workflow' (ci, docs or sonar)" >&2; return 2; }
}

# filler <n>: n long paths no lane matches, so the list tops the pipe buffer.
filler() {
  local count="$1" i
  for ((i = 1; i <= count; i++)); do
    printf 'zz-filler/a/directory/deep/enough/that/two/thousand/paths/top/the/pipe/buffer/file-%04d.txt\n' "$i"
  done
}

self_test() {
  local failures=0 big name workflow first last want got
  big=$(filler 1999)
  if [[ "${#big}" -le 65536 ]]; then
    echo "self-test: the filler is ${#big} bytes, not past the 64 KiB pipe buffer" >&2
    return 1
  fi
  # The old form, run on the same list: the match comes first, grep -q exits
  # on it, and the producer is left writing into a closed pipe.
  if (set -o pipefail; printf '%s\n' "Cargo.toml" "$big" | grep -qx 'Cargo.toml'); then # pipe-early-exit: deliberate
    echo "self-test: the pipe form did not fail on a match first in a large list; the list does not exercise SIGPIPE" >&2
    failures=1
  fi
  # Each case: a name, the workflow, the path, and the lanes that must be true.
  while IFS='|' read -r name workflow first want; do
    for last in true false; do
      if [[ "$last" == true ]]; then
        got=$(classify "$workflow" "$big"$'\n'"$first")
      else
        got=$(classify "$workflow" "$first"$'\n'"$big")
      fi
      got=$(grep '=true$' <<<"$got" | sed 's/=true$//' | paste -sd, - || true)
      if [[ "$got" != "$want" ]]; then
        echo "self-test: $name (match $([[ "$last" == true ]] && echo last || echo first) of 2000): got '$got', want '$want'" >&2
        failures=1
      fi
    done
  done <<'CASES'
a crate source runs the cargo lanes|ci|crates/openehr-rm/src/lib.rs|code
a nested manifest moves the msrv|ci|crates/openehr-rm/Cargo.toml|code,msrv
the lockfile reaches the viewer and the msrv|ci|Cargo.lock|code,msrv,viewer
the e2e harness runs the cargo lanes and the viewer|ci|scripts/ui-e2e.sh|code,viewer
any other script runs no lane|ci|scripts/watch/eu-acts.sh|
the deploy probe runs the deploy lane|ci|scripts/deploy-probe.sh|deploy
the server Dockerfile reaches five lanes|ci|docker/Dockerfile|deploy,viewer,server_dockerfile,chart_boot
the viewer Dockerfile reaches its own image lane|ci|docker/viewer/Dockerfile|deploy,viewer,viewer_dockerfile
the chart runs helm and chart boot|ci|deploy/helm/ferroehr/values.yaml|helm,chart_boot
the configuration tree reboots the chart|ci|app/ferroehr/src/config/mod.rs|code,viewer,chart_boot
the ci recipe reaches its recipe lanes|ci|.github/workflows/ci.yml|msrv,viewer,helm,chart_boot,workflows
a VEX document runs the vex lane|ci|security/vex/rust.openvex.json|vex
a book page builds the site|docs|website/book/src/index.md|docs
a crate source does not build the site|docs|crates/openehr-rm/src/lib.rs|
a crate source runs the scan|sonar|crates/openehr-rm/src/lib.rs|code
a book page runs no scan|sonar|website/book/src/index.md|
CASES
  for workflow in ci docs sonar; do
    got=$(classify "$workflow" "" | grep -c '=true$' || true)
    if [[ "$got" != 0 ]]; then
      echo "self-test: an empty list set $got $workflow lanes true" >&2
      failures=1
    fi
  done
  if classify nope "x" >/dev/null 2>&1; then
    echo "self-test: an unknown workflow was accepted" >&2
    failures=1
  fi
  [[ "$failures" -eq 0 ]] || return 1
  echo "change-lanes: self-test OK (16 paths classified first and last in a 2000-path list; the pipe form fails on it)."
}

case "${1-}" in
--self-test)
  [[ "$#" -eq 1 ]] || { echo "usage: $0 <ci|docs|sonar> < paths | --self-test" >&2; exit 2; }
  self_test
  ;;
ci | docs | sonar)
  [[ "$#" -eq 1 ]] || { echo "usage: $0 <ci|docs|sonar> < paths | --self-test" >&2; exit 2; }
  paths=$(cat)
  classify "$1" "$paths"
  ;;
*)
  echo "usage: $0 <ci|docs|sonar> < paths | --self-test" >&2
  exit 2
  ;;
esac
