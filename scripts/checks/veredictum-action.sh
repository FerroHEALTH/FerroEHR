#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Veredictum pin guard: the conformance workflow runs the release
# scripts/lib/veredictum.sh pins, and nothing else (#4449).
#
# Three places name the instrument: VEREDICTUM_VERSION and VEREDICTUM_IMAGE in
# scripts/lib/veredictum.sh, and the Veredictum Action step of
# .github/workflows/conformance.yml, pinned by full commit SHA with the
# version in a trailing comment and passed the image by digest. The guard
# holds the three to one release:
#
#   offline  every `uses: Vernum-Projecten/Veredictum@<sha> # v<X.Y.Z>` line
#            names VEREDICTUM_VERSION, and every `image:` the workflow passes
#            is VEREDICTUM_IMAGE, a digest reference;
#   online   the tag v<VEREDICTUM_VERSION> resolves to that SHA in the
#            Veredictum repository, and the release image tag
#            <VEREDICTUM_VERSION> resolves to that digest on ghcr.io. A
#            registry or git failure fails the guard; it never passes on an
#            answer it could not read.
#
# Usage: scripts/checks/veredictum-action.sh               offline and online
#        scripts/checks/veredictum-action.sh --offline     the files alone
#        scripts/checks/veredictum-action.sh --self-test   the reader on fixtures
# Needs git, curl and jq for the online half.
# No specification governs this guard: our own design.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
GRAMMAR="[--offline | --self-test]"
mode=all
case "${1:-}" in
  "") ;;
  --offline) mode=offline ;;
  --self-test) mode=self-test ;;
  *) guard_usage "$GRAMMAR" ;;
esac
[[ "$#" -le 1 ]] || guard_usage "$GRAMMAR"

readonly WORKFLOW_PATH=".github/workflows/conformance.yml"
readonly LIB_PATH="scripts/lib/veredictum.sh"
readonly IMAGE_REPO="ghcr.io/vernum-projecten/veredictum"

# Reads one assignment `NAME="value"` from the pin file $1.
read_assignment() {
  sed -nE "s/^$2=\"([^\"]*)\"\$/\\1/p" "$1" | sed -n 1p
}

# Checks the tree under $1 without the network. Prints `<version> <sha>
# <digest>` on success; prints every disagreement and fails otherwise.
check_files() {
  local root="$1" lib workflow version image bad=0 line sha tag shas="" used
  lib="$root/$LIB_PATH"
  workflow="$root/$WORKFLOW_PATH"
  version="$(read_assignment "$lib" VEREDICTUM_VERSION)"
  image="$(read_assignment "$lib" VEREDICTUM_IMAGE)"
  if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "::error file=$LIB_PATH::VEREDICTUM_VERSION '$version' is not a release X.Y.Z" >&2
    return 1
  fi
  if [[ ! "$image" =~ ^${IMAGE_REPO//./\\.}@sha256:[0-9a-f]{64}$ ]]; then
    echo "::error file=$LIB_PATH::VEREDICTUM_IMAGE '$image' is not $IMAGE_REPO pinned by digest" >&2
    return 1
  fi
  [[ -f "$workflow" ]] || { echo "::error file=$WORKFLOW_PATH::the conformance workflow is missing" >&2; return 1; }

  used="$(grep -E '^[[:space:]]*(-[[:space:]]+)?uses:[[:space:]]+Vernum-Projecten/Veredictum' "$workflow" || true)"
  if [[ -z "$used" ]]; then
    echo "::error file=$WORKFLOW_PATH::no step uses the Veredictum Action" >&2
    return 1
  fi
  while IFS= read -r line; do
    if [[ "$line" =~ uses:[[:space:]]+Vernum-Projecten/Veredictum@([0-9a-f]{40})[[:space:]]+#[[:space:]]+v([0-9]+\.[0-9]+\.[0-9]+)[[:space:]]*$ ]]; then
      sha="${BASH_REMATCH[1]}"
      tag="${BASH_REMATCH[2]}"
      if [[ "$tag" != "$version" ]]; then
        echo "::error file=$WORKFLOW_PATH::the Veredictum Action is pinned at v$tag here and VEREDICTUM_VERSION is $version; move the SHA, its comment, the image digest and the version together" >&2
        bad=1
      fi
      shas+="$sha"$'\n'
    else
      echo "::error file=$WORKFLOW_PATH::the Veredictum Action must be pinned by full commit SHA with its version in a trailing comment (\`@<sha> # vX.Y.Z\`): $line" >&2
      bad=1
    fi
  done <<<"$used"
  if [[ "$(sort -u <<<"$shas" | grep -c . || true)" -gt 1 ]]; then
    echo "::error file=$WORKFLOW_PATH::the Veredictum Action steps name more than one commit" >&2
    bad=1
  fi

  local images
  images="$(sed -nE 's/^[[:space:]]+image:[[:space:]]+(.*[^[:space:]])[[:space:]]*$/\1/p' "$workflow" | grep -F "$IMAGE_REPO" || true)"
  if [[ -z "$images" ]]; then
    echo "::error file=$WORKFLOW_PATH::the Veredictum Action is passed no image by digest (\`image: $image\`)" >&2
    bad=1
  fi
  while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    if [[ "$line" != "$image" ]]; then
      echo "::error file=$WORKFLOW_PATH::the workflow runs $line and VEREDICTUM_IMAGE is $image" >&2
      bad=1
    fi
  done <<<"$images"
  [[ "$bad" -eq 0 ]] || return 1
  printf '%s %s %s\n' "$version" "$(sed -n 1p <<<"$shas")" "${image#*@}"
}

# The commit the tag v$1 names in the Veredictum repository: the peeled commit
# of an annotated tag, the tag's own object otherwise.
remote_commit() {
  local refs commit
  refs="$(git ls-remote https://github.com/Vernum-Projecten/Veredictum "refs/tags/v$1" "refs/tags/v$1^{}")"
  commit="$(awk -v r="refs/tags/v$1^{}" '$2 == r { print $1 }' <<<"$refs")"
  [[ -n "$commit" ]] || commit="$(awk -v r="refs/tags/v$1" '$2 == r { print $1 }' <<<"$refs")"
  printf '%s' "$commit"
}

# The digest ghcr.io serves for the release image tag $1.
remote_digest() {
  local token
  token="$(curl -fsS --retry 3 --retry-all-errors \
    "https://ghcr.io/token?scope=repository:vernum-projecten/veredictum:pull" | jq -er .token)"
  curl -fsSI --retry 3 --retry-all-errors \
    -H "Authorization: Bearer $token" \
    -H "Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json" \
    "https://ghcr.io/v2/vernum-projecten/veredictum/manifests/$1" |
    tr -d '\r' | sed -nE 's/^[Dd]ocker-[Cc]ontent-[Dd]igest:[[:space:]]*(sha256:[0-9a-f]{64})$/\1/p'
}

check_remote() {
  local version="$1" sha="$2" digest="$3" have bad=0
  have="$(remote_commit "$version")" || have=""
  if [[ -z "$have" ]]; then
    echo "::error file=$WORKFLOW_PATH::could not read the tag v$version from the Veredictum repository" >&2
    bad=1
  elif [[ "$have" != "$sha" ]]; then
    echo "::error file=$WORKFLOW_PATH::the Veredictum Action is pinned at $sha, and the tag v$version names $have" >&2
    bad=1
  fi
  have="$(remote_digest "$version")" || have=""
  if [[ -z "$have" ]]; then
    echo "::error file=$LIB_PATH::could not read the digest of $IMAGE_REPO:$version from ghcr.io" >&2
    bad=1
  elif [[ "$have" != "$digest" ]]; then
    echo "::error file=$LIB_PATH::VEREDICTUM_IMAGE names $digest, and $IMAGE_REPO:$version is $have" >&2
    bad=1
  fi
  return "$bad"
}

self_test() {
  local work fixture sha1 sha2 d1 d2 out
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand now: $work is local to this function
  trap "rm -rf '$work'" EXIT
  fixture="$work/tree"
  mkdir -p "$fixture/scripts/lib" "$fixture/.github/workflows"
  sha1="$(printf 'a%.0s' {1..40})"
  sha2="$(printf 'b%.0s' {1..40})"
  d1="$(printf 'c%.0s' {1..64})"
  d2="$(printf 'd%.0s' {1..64})"
  # write <lib version> <lib digest> <uses line> <image line>
  write() {
    printf 'VEREDICTUM_VERSION="%s"\nVEREDICTUM_IMAGE="%s@sha256:%s"\n' "$1" "$IMAGE_REPO" "$2" \
      > "$fixture/$LIB_PATH"
    printf 'jobs:\n  conformance:\n    steps:\n      - id: veredictum\n        %s\n        with:\n          %s\n' "$3" "$4" \
      > "$fixture/$WORKFLOW_PATH"
  }
  write 1.2.3 "$d1" "uses: Vernum-Projecten/Veredictum@$sha1 # v1.2.3" "image: $IMAGE_REPO@sha256:$d1"
  out="$(check_files "$fixture" 2>&1)" || { echo "self-test: agreeing pins were refused: $out" >&2; exit 1; }
  [[ "$out" == "1.2.3 $sha1 sha256:$d1" ]] || { echo "self-test: read '$out'" >&2; exit 1; }

  # refuse <name> <write args...>: the tree written from the args is refused.
  refuse() {
    local name="$1"
    shift
    write "$@"
    if out="$(check_files "$fixture" 2>&1)"; then
      echo "self-test: $name was accepted" >&2
      exit 1
    fi
  }
  local good_use="uses: Vernum-Projecten/Veredictum@$sha1 # v1.2.3" good_image="image: $IMAGE_REPO@sha256:$d1"
  refuse "a comment on another version" 1.2.3 "$d1" "uses: Vernum-Projecten/Veredictum@$sha1 # v1.2.4" "$good_image"
  refuse "a tag in place of the SHA" 1.2.3 "$d1" "uses: Vernum-Projecten/Veredictum@v1.2.3" "$good_image"
  refuse "a SHA with no version comment" 1.2.3 "$d1" "uses: Vernum-Projecten/Veredictum@$sha1" "$good_image"
  refuse "a short SHA" 1.2.3 "$d1" "uses: Vernum-Projecten/Veredictum@${sha1:0:12} # v1.2.3" "$good_image"
  refuse "the workflow on another digest" 1.2.3 "$d1" "$good_use" "image: $IMAGE_REPO@sha256:$d2"
  refuse "the workflow on a tag" 1.2.3 "$d1" "$good_use" "image: $IMAGE_REPO:1.2.3"
  refuse "no image passed" 1.2.3 "$d1" "$good_use" "out-dir: x"
  refuse "a pre-release version" 1.2.3-rc.1 "$d1" "uses: Vernum-Projecten/Veredictum@$sha1 # v1.2.3-rc.1" "$good_image"

  # Two Action steps on two commits.
  write 1.2.3 "$d1" "uses: Vernum-Projecten/Veredictum@$sha1 # v1.2.3" "image: $IMAGE_REPO@sha256:$d1"
  printf '      - uses: Vernum-Projecten/Veredictum@%s # v1.2.3\n' "$sha2" >> "$fixture/$WORKFLOW_PATH"
  if check_files "$fixture" >/dev/null 2>&1; then
    echo "self-test: two Action steps on two commits were accepted" >&2
    exit 1
  fi
  echo "veredictum-action: self-test OK (one agreeing tree accepted, nine disagreements refused)."
}

if [[ "$mode" == self-test ]]; then
  self_test
  exit 0
fi

root="$(cd "$(dirname "$0")/../.." && pwd)"
read -r version sha digest < <(check_files "$root") || exit 1
[[ -n "${version:-}" ]] || exit 1
echo "veredictum-action: the workflow runs the Action at $sha (v$version) on $IMAGE_REPO@$digest, as $LIB_PATH pins."
if [[ "$mode" == offline ]]; then
  exit 0
fi
check_remote "$version" "$sha" "$digest"
echo "veredictum-action: the tag v$version names $sha and $IMAGE_REPO:$version is $digest."
