#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Whether an image built from one commit holds what another commit would
# build (#4547): the server or postgres image's inputs at the image's
# org.opencontainers.image.revision against those of the tree under test.
#
# Conformance on a push to main that rebuilt no image judges the `:main`
# tag, which scan-and-tag.yml moves only when its scan passes. After a
# refused scan, `:main` was built from an older commit, and the record would
# describe that commit's server. conformance.yml asks this script before it
# judges an image and refuses one whose inputs differ from the tree's.
#
# The server inputs are the paths the app filter of containers.yml rebuilds
# the image on, less the ones no image holds (the self-test holds the list to
# that filter), and the files docker/Dockerfile copies into the runtime stage,
# read from the tree's Dockerfile.
#
# Usage:
#   scripts/lib/image-inputs.sh paths <server|postgres>
#       print the pathspecs of the image's inputs
#   scripts/lib/image-inputs.sh same <server|postgres> <revision> <tree>
#       exit 0 when the inputs at <revision> equal those at <tree>, 1 when
#       they differ (the differing paths on stdout), 2 when it cannot tell;
#       a <revision> the checkout lacks is fetched from origin, trees only
#   scripts/lib/image-inputs.sh --self-test
#
# No specification governs this: our own design.
set -Eeuo pipefail

readonly SERVER_INPUTS=(
  crates app/ferroehr app/ferroehr-rest app/ferroehr-server tools
  Cargo.toml Cargo.lock rust-toolchain.toml .cargo docker/Dockerfile
)
readonly POSTGRES_INPUTS=(docker/postgres)
# The app filter entries of containers.yml that rebuild the image and that
# no image holds: the smoke lane's script and compose file, and the workflow.
readonly NOT_IN_IMAGE=(docker/smoke-test.sh docker-compose.yml .github/workflows/containers.yml)

# The pathspecs of the image kind $1, read in the checkout the caller stands in.
inputs_of() {
  case "$1" in
    server)
      if [[ ! -f docker/Dockerfile ]]; then
        echo "image-inputs: no docker/Dockerfile in $PWD" >&2
        return 2
      fi
      printf '%s\n' "${SERVER_INPUTS[@]}"
      dockerfile_sources docker/Dockerfile
      ;;
    postgres) printf '%s\n' "${POSTGRES_INPUTS[@]}" ;;
    *)
      echo "image-inputs: '$1' is no image kind (server or postgres)" >&2
      return 2
      ;;
  esac
}

same() {
  local kind="$1" revision="$2" tree="$3" paths=() line changed listed
  listed="$(inputs_of "$kind")" || return 2
  while IFS= read -r line; do paths+=("$line"); done <<<"$listed"
  if [[ ! "$revision" =~ ^[0-9a-f]{40}$ ]]; then
    echo "image-inputs: '$revision' is no commit id" >&2
    return 2
  fi
  [[ "$revision" == "$tree" ]] && return 0
  if ! git cat-file -e "$revision^{commit}" 2>/dev/null &&
    ! git fetch -q --no-tags --depth=1 --filter=blob:none origin "$revision"; then
    echo "image-inputs: origin serves no commit $revision" >&2
    return 2
  fi
  if ! changed="$(git diff-tree -r --no-renames --name-only "$revision" "$tree" -- "${paths[@]}")"; then
    echo "image-inputs: cannot compare $revision with $tree" >&2
    return 2
  fi
  [[ -z "$changed" ]] && return 0
  printf '%s\n' "$changed"
  return 1
}

# The entries of the named filter in containers.yml, `/**` dropped.
filter_entries() {
  awk -v name="$1" '
    /^ *filters: \|/ { in_filters = 1; next }
    in_filters && $0 ~ "^ *" name ":$" { on = 1; next }
    in_filters && on && /^ *[a-z]+:$/ { on = 0 }
    on && /^ *- / { sub(/^ *- */, ""); gsub(/\047/, ""); sub(/\/\*\*$/, ""); print }
  ' "$2"
}

# The sources docker/Dockerfile copies into the runtime stages from the
# build context: every word of a COPY instruction after `AS runtime-base`
# but its flags, its destination and the binary the context provides, and no
# COPY --from of another stage.
dockerfile_sources() {
  awk '
    / AS runtime-base$/ { on = 1 }
    !on { next }
    /^COPY / { copy = 1; n = 0; staged = ($0 ~ /--from=/) }
    copy {
      line = $0
      sub(/\\$/, "", line)
      m = split(line, w, /[ \t]+/)
      for (i = 1; i <= m; i++) if (w[i] != "" && w[i] != "COPY" && w[i] !~ /^--/) words[++n] = w[i]
      if ($0 !~ /\\$/) {
        copy = 0
        if (!staged) for (i = 1; i < n; i++) if (words[i] !~ /^bin\//) { sub(/\/$/, "", words[i]); print words[i] }
      }
    }
  ' "$1"
}

# True when the path $1 lies under one of the inputs on stdin.
covered() {
  local input
  while IFS= read -r input; do
    [[ "$1" == "$input" || "$1" == "$input"/* ]] && return 0
  done
  return 1
}

self_test() {
  local repo tmp entry failures=0 got
  repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" EXIT
  fail() {
    echo "image-inputs: SELF-TEST FAILED: $1" >&2
    failures=1
  }

  # The lists against the filters of containers.yml, and the sources read
  # from docker/Dockerfile.
  while IFS= read -r entry; do
    printf '%s\n' "${NOT_IN_IMAGE[@]}" | covered "$entry" && continue
    printf '%s\n' "${SERVER_INPUTS[@]}" | covered "$entry" || fail "containers.yml rebuilds the app image on $entry, which the server inputs miss"
  done < <(filter_entries app "$repo/.github/workflows/containers.yml")
  while IFS= read -r entry; do
    [[ "$entry" == .github/workflows/containers.yml ]] && continue
    printf '%s\n' "${POSTGRES_INPUTS[@]}" | covered "$entry" || fail "containers.yml rebuilds the postgres image on $entry, which the postgres inputs miss"
  done < <(filter_entries postgres "$repo/.github/workflows/containers.yml")
  got="$(dockerfile_sources "$repo/docker/Dockerfile")"
  grep -qxF docker/terminology/seed <<<"$got" || fail "the COPY sources of docker/Dockerfile were misread: $got"
  if grep -qE '^(bin/|/)' <<<"$got"; then fail "a binary or a stage path was read as a source: $got"; fi

  # A miniature origin and a depth-1 checkout of its head, as CI checks out.
  (
    set -e
    commit() {
      mkdir -p "$(dirname "$1")"
      printf '%s\n' "$2" >"$1"
      git add -- "$1"
      git -c user.name=self-test -c user.email=self-test@invalid -c commit.gpgsign=false commit -qm "$1"
      git rev-parse HEAD >"$tmp/$3"
    }
    git init -q -b main "$tmp/origin"
    cd "$tmp/origin"
    git config uploadpack.allowFilter true
    git config uploadpack.allowAnySHA1InWant true
    printf 'FROM scratch AS builder\nFROM scratch AS runtime-base\nCOPY --chmod=0644 \\\n  docker/terminology/seed/ /seed/\nFROM runtime-base AS runtime-prebuilt\nCOPY bin/amd64/ferroehr /ferroehr\n' >Dockerfile.tmp
    mkdir -p docker && mv Dockerfile.tmp docker/Dockerfile && git add docker/Dockerfile
    commit crates/a/src/lib.rs one base
    commit docs/readme.md two docs
    commit crates/a/src/lib.rs three crates
    commit docker/postgres/initdb/1.sql four postgres
    commit docker/terminology/seed/codes.json five seed
    git clone -q --depth=1 "file://$tmp/origin" "$tmp/checkout"
  ) >/dev/null 2>&1 || {
    echo "image-inputs: SELF-TEST FAILED: could not build the miniature repository" >&2
    exit 1
  }
  local base docs crates postgres seed status
  base="$(cat "$tmp/base")" docs="$(cat "$tmp/docs")" crates="$(cat "$tmp/crates")"
  postgres="$(cat "$tmp/postgres")" seed="$(cat "$tmp/seed")"
  run() {
    status=0
    (cd "$tmp/checkout" && same "$@") >"$tmp/answer" 2>&1 || status=$?
    printf '%s\n' "$status"
  }
  [[ "$(run server "$seed" "$seed")" == 0 ]] || fail "an image of the tree itself was refused"
  [[ "$(run server "$docs" "$seed")" == 1 ]] || fail "a server image older than a crate change passed"
  grep -qxF crates/a/src/lib.rs "$tmp/answer" || fail "the changed crate was not named: $(cat "$tmp/answer")"
  grep -qxF docker/terminology/seed/codes.json "$tmp/answer" || fail "the changed seed was not named: $(cat "$tmp/answer")"
  [[ "$(run server "$base" "$crates")" == 1 ]] || fail "a fetched older revision with a crate change passed"
  [[ "$(run server "$crates" "$postgres")" == 0 ]] || fail "a server image was refused for a postgres change: $(cat "$tmp/answer")"
  [[ "$(run postgres "$crates" "$postgres")" == 1 ]] || fail "a postgres image older than a postgres change passed"
  [[ "$(run postgres "$base" "$crates")" == 0 ]] || fail "a postgres image was refused for a crate change: $(cat "$tmp/answer")"
  [[ "$(run server "$(printf '%040d' 0)" "$seed")" == 2 ]] || fail "a revision origin does not serve was not a failure"
  [[ "$(run server unknown "$seed")" == 2 ]] || fail "a label that is no commit id was not a failure"
  [[ "$(run viewer "$seed" "$seed")" == 2 ]] || fail "an unknown image kind was not a failure"

  [[ "$failures" -eq 0 ]] || exit 1
  echo "image-inputs: self-test OK (the lists against containers.yml and docker/Dockerfile, same and differing inputs per image, a fetched revision, and the failures)."
}

case "${1:-}" in
  paths)
    [[ "$#" -eq 2 ]] || { echo "usage: $0 paths <server|postgres>" >&2; exit 2; }
    inputs_of "$2"
    ;;
  same)
    [[ "$#" -eq 4 ]] || { echo "usage: $0 same <server|postgres> <revision> <tree>" >&2; exit 2; }
    same "$2" "$3" "$4"
    ;;
  --self-test) self_test ;;
  *)
    echo "usage: $0 paths <kind> | same <kind> <revision> <tree> | --self-test" >&2
    exit 2
    ;;
esac
