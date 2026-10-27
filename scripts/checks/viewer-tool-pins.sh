#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Viewer build-tool pin guard (#4222).
#
# The wasm-bindgen CLI must be the exact version of the wasm-bindgen library
# the viewer links: the CLI refuses a module built against another
# version, so the bindings step of the viewer build fails. The
# library version is the one Cargo.lock resolves, so this guard fails when
# any wasm-bindgen CLI pin in a Dockerfile, compose file, workflow or script
# names another version, and when docker/viewer/Dockerfile stops reading the
# version from the bind-mounted Cargo.lock. It also fails when the
# cargo-leptos pins disagree with one another, since CI, the image lanes and
# the compose-built image must all build the viewer with one cargo-leptos.
# Dependency-free: grep + sed only.
#
# Usage: scripts/checks/viewer-tool-pins.sh [--self-test]
#   --self-test mutates a COPY of the scanned tree six ways and asserts the
#   guard rejects each, then that the unmutated copy passes.
set -Eeuo pipefail

if [[ "$#" -eq 1 && "$1" == "--self-test" ]]; then
  cd "$(dirname "$0")/../.."
  self_root="$(mktemp -d)"
  trap 'rm -rf "$self_root"' EXIT
  mkdir -p "$self_root/.github" "$self_root/deploy"
  cp -R scripts docker "$self_root/"
  cp -R .github/workflows "$self_root/.github/"
  cp -R deploy/hosted "$self_root/deploy/" 2>/dev/null || true
  cp Cargo.lock docker-compose*.yml "$self_root/"
  self_fail=0
  # <label>|<file>|<sed program>
  mutations=(
    "a stale wasm-bindgen CLI in ci.yml|${self_root}/.github/workflows/ci.yml|s|wasm-bindgen@[0-9][0-9.]*|wasm-bindgen@0.0.1|"
    "a lock that moves without the workflow pins|${self_root}/Cargo.lock|/^name = \"wasm-bindgen\"$/{n;s|^version = \".*\"$|version = \"0.0.1\"|;}"
    "a viewer Dockerfile that no longer reads the lock|${self_root}/docker/viewer/Dockerfile|s|source=Cargo.lock|source=Cargo.toml|"
    "a script pinning another wasm-bindgen CLI|${self_root}/scripts/cargo-leptos.sh|s|^set -Eeuo pipefail$|& # cargo install wasm-bindgen-cli --version 0.0.1|"
    "a stale cargo-leptos in release.yml|${self_root}/.github/workflows/release.yml|s|cargo-leptos@[0-9][0-9.]*|cargo-leptos@0.0.1|"
    "a stale cargo-leptos in the viewer Dockerfile|${self_root}/docker/viewer/Dockerfile|s|^ARG CARGO_LEPTOS_VERSION=.*|ARG CARGO_LEPTOS_VERSION=0.0.1|"
  )
  for mutation in "${mutations[@]}"; do
    label="${mutation%%|*}"
    rest="${mutation#*|}"
    file="${rest%%|*}"
    program="${rest#*|}"
    cp "$file" "${file}.orig"
    sed "$program" "${file}.orig" > "$file"
    if cmp -s "${file}.orig" "$file"; then
      echo "::error::--self-test could not apply the mutation '${label}': its anchor no longer matches, so this case proves nothing." >&2
      self_fail=1
    elif (cd "$self_root" && bash scripts/checks/viewer-tool-pins.sh >/dev/null 2>&1); then
      echo "::error::--self-test: the guard ACCEPTED ${label}." >&2
      self_fail=1
    else
      echo "  self-test: rejected ${label}"
    fi
    mv "${file}.orig" "$file"
  done
  # The unmutated copy must pass, or every rejection above is a broken fixture.
  if ! (cd "$self_root" && bash scripts/checks/viewer-tool-pins.sh >/dev/null 2>&1); then
    echo "::error::--self-test: the guard REJECTED the unmutated tree." >&2
    self_fail=1
  fi
  [[ "$self_fail" -eq 0 ]] || exit 1
  echo "viewer-tool-pins --self-test: every mutation rejected, the unmutated tree accepted."
  exit 0
fi

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
[[ "$#" -eq 0 ]] || guard_usage "[--self-test]"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SELF="$ROOT_DIR/scripts/checks/viewer-tool-pins.sh"
VIEWER_DOCKERFILE="$ROOT_DIR/docker/viewer/Dockerfile"
SEMVER='[0-9]+\.[0-9]+\.[0-9]+'

rc=0

lock_versions="$(sed -n '/^name = "wasm-bindgen"$/{n;s/^version = "\(.*\)"$/\1/p;}' "$ROOT_DIR/Cargo.lock")"
if [[ ! "$lock_versions" =~ ^${SEMVER}$ ]]; then
  echo "::error::Cargo.lock must resolve exactly one wasm-bindgen; found '${lock_versions//$'\n'/, }'." >&2
  exit 1
fi
lock="$lock_versions"
echo "Cargo.lock resolves wasm-bindgen $lock."

# Every file that can pin a build tool: Dockerfiles and compose files,
# workflows and composite actions, and scripts.
files=()
while IFS= read -r file; do
  [[ "$file" == "$SELF" ]] || files+=("$file")
done < <(
  {
    find "$ROOT_DIR" -maxdepth 1 -type f \( -name 'Dockerfile*' -o -name 'docker-compose*.yml' \)
    for dir in .github docker deploy scripts; do
      [[ -d "$ROOT_DIR/$dir" ]] || continue
      find "$ROOT_DIR/$dir" -type f \( -name '*.yml' -o -name '*.yaml' -o -name '*.sh' -o -name 'Dockerfile*' \)
    done
  } | LC_ALL=C sort
)

# Prints `<file>:<line>:<version>` for each pin that the extended regex $1
# matches, the version being the last X.Y.Z in the match.
pins() {
  local file match
  for file in "${files[@]}"; do
    while IFS=: read -r line match; do
      echo "${file#"$ROOT_DIR"/}:$line:$(grep -oE "$SEMVER" <<<"$match" | tail -n 1)"
    done < <(grep -noE "$1" "$file" || true)
  done
}

wasm_checked=0
while IFS=: read -r rel line version; do
  wasm_checked=$((wasm_checked + 1))
  if [[ "$version" != "$lock" ]]; then
    echo "::error::$rel:$line pins the wasm-bindgen CLI at $version but Cargo.lock resolves wasm-bindgen $lock; the CLI must match the library exactly." >&2
    rc=1
  else
    echo "$rel:$line wasm-bindgen CLI $version matches Cargo.lock."
  fi
done < <(pins "wasm-bindgen(-cli)?@${SEMVER}|WASM_BINDGEN_VERSION[=:][[:space:]]*\"?${SEMVER}|wasm-bindgen-cli[[:space:]]+--version[[:space:]]+${SEMVER}|wasm-bindgen/releases/download/${SEMVER}")
if [[ "$wasm_checked" -eq 0 ]]; then
  echo "::error::no workflow or script pins a wasm-bindgen CLI; the guard has nothing to check, so its scan is broken." >&2
  rc=1
fi

if ! grep -q 'source=Cargo.lock' "$VIEWER_DOCKERFILE"; then
  echo "::error::docker/viewer/Dockerfile no longer bind-mounts Cargo.lock to read the wasm-bindgen version from it (#4222)." >&2
  rc=1
else
  echo "docker/viewer/Dockerfile reads the wasm-bindgen version from Cargo.lock."
fi

leptos_first=""
leptos_checked=0
while IFS=: read -r rel line version; do
  leptos_checked=$((leptos_checked + 1))
  if [[ -z "$leptos_first" ]]; then
    leptos_first="$version"
    leptos_where="$rel:$line"
  fi
  if [[ "$version" != "$leptos_first" ]]; then
    echo "::error::$rel:$line pins cargo-leptos $version but $leptos_where pins $leptos_first; every viewer build uses one cargo-leptos." >&2
    rc=1
  else
    echo "$rel:$line cargo-leptos $version."
  fi
done < <(pins "cargo-leptos@${SEMVER}|CARGO_LEPTOS_VERSION[=:][[:space:]]*\"?${SEMVER}|cargo-leptos[[:space:]]+--version[[:space:]]+${SEMVER}")
if [[ "$leptos_checked" -eq 0 ]]; then
  echo "::error::no workflow, Dockerfile or script pins cargo-leptos; the guard has nothing to check, so its scan is broken." >&2
  rc=1
fi

exit "$rc"
