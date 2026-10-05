#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# scripts/checks/veredictum-pin.sh — a Veredictum pin bump carries its
# acceptance run, unless the bump moves neither the catalogue nor the runner
# (#2867, #3593).
#
# scripts/lib/veredictum.sh pins the conformance instrument. A PR that changes
# VEREDICTUM_VERSION between <base> and <head> passes when ONE of these holds:
#
#   1. the catalogue (`artifacts/`), the spec oracle (`specs/`), the schema
#      bundles (`schemas/`) and the runner source (`app/veredictum/src/`) git
#      trees are all identical at the old and the new tag, read from the GitHub
#      API for the repository each side's VEREDICTUM_REPO names (never a local
#      Veredictum checkout). Equal tree SHAs mean byte-identical content, so the
#      committed baseline already describes the new pin. The runner tree is the
#      smallest one holding every source file of the `veredictum` binary: the
#      published crate's `include` list is `src/**` plus its manifest, and the
#      manifest (like Cargo.lock) carries the release version, so it differs
#      at every tag and is not compared;
#   2. the PR commits something under docs/conformance/ferroehr/ (the refreshed
#      record of a full `bash scripts/conformance.sh` run);
#   3. the PR carries the `no-conformance-run` label (--no-conformance-run).
#
# The tree comparison fails closed: an API error, a tag that does not resolve,
# a missing directory or a VEREDICTUM_REPO that is not a github.com URL proves
# nothing, and the bump then needs the record or the label.
#
# Usage: scripts/checks/veredictum-pin.sh [--no-conformance-run] <base> <head>
#        scripts/checks/veredictum-pin.sh --self-test
#   <base>/<head> are commits of this repository; GH_TOKEN authorises `gh api`.
#   --self-test drives the guard over a throwaway git repository against a stub
#   gh: unchanged trees pass, each of the four changed trees fails without
#   record or label, a changed tree passes with the label or the record, an API
#   failure and a missing directory fail closed, and an unchanged pin never
#   calls gh.
# Caller: the `veredictum-pin` job in ci.yml.

set -euo pipefail

readonly TREES=(artifacts specs schemas app/veredictum/src)

self_test() {
  command -v jq >/dev/null 2>&1 || { echo "veredictum-pin: jq is not installed" >&2; exit 1; }
  local here fixture stub log
  here="$(cd "$(dirname "$0")/../.." && pwd)"
  # Global, not local: the EXIT trap runs after this function has returned.
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  fixture="$work/repo"
  stub="$work/bin"
  log="$work/gh.log"
  mkdir -p "$fixture/scripts/checks" "$fixture/scripts/lib" "$stub"
  cp "$here/scripts/checks/veredictum-pin.sh" "$fixture/scripts/checks/"
  cp "$here/scripts/lib/guard-args.sh" "$fixture/scripts/lib/"

  # The stub answers `gh api repos/<slug>/git/trees/refs/tags/<tag>` (the root
  # tree) and `gh api repos/<slug>/contents/app/veredictum?ref=refs/tags/<tag>`
  # (the runner's parent): v1.0.0 is the old tag, v1.0.1 the new one, and
  # GH_STUB_MODE picks what moved.
  cat > "$stub/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$GH_STUB_LOG"
[ "${1:-}" = api ] || { echo "stub gh: unexpected call: $*" >&2; exit 2; }
if [ "$GH_STUB_MODE" = fail ]; then
  echo "gh: Not Found (HTTP 404)" >&2
  exit 1
fi
artifacts=1111111111111111111111111111111111111111
specs=2222222222222222222222222222222222222222
schemas=3333333333333333333333333333333333333333
runner=4444444444444444444444444444444444444444
endpoint="${2:-}"
case "$endpoint" in
  */git/trees/refs/tags/v1.0.[01]) kind=root ;;
  */contents/app/veredictum"?ref=refs/tags/v1.0."[01]) kind=runner ;;
  *) echo "gh: Not Found (HTTP 404)" >&2; exit 1 ;;
esac
if [ "${endpoint##*/tags/}" = v1.0.1 ]; then
  case "$GH_STUB_MODE" in
    artifacts) artifacts=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ;;
    specs) specs=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb ;;
    schemas) schemas=cccccccccccccccccccccccccccccccccccccccc ;;
    runner) runner=dddddddddddddddddddddddddddddddddddddddd ;;
    missing) schemas="" ;;
    runner-missing) runner="" ;;
    same) ;;
    *) echo "stub gh: unknown GH_STUB_MODE $GH_STUB_MODE" >&2; exit 2 ;;
  esac
fi
if [ "$kind" = runner ]; then
  entries='{"name":"Cargo.toml","path":"app/veredictum/Cargo.toml","type":"file","sha":"8888888888888888888888888888888888888888"}'
  if [ -n "$runner" ]; then
    entries="$entries,{\"name\":\"src\",\"path\":\"app/veredictum/src\",\"type\":\"dir\",\"sha\":\"$runner\"}"
  fi
  printf '[%s]\n' "$entries"
  exit 0
fi
entries='{"path":"app","type":"tree","sha":"9999999999999999999999999999999999999999"}'
entries="$entries,{\"path\":\"artifacts\",\"type\":\"tree\",\"sha\":\"$artifacts\"}"
entries="$entries,{\"path\":\"specs\",\"type\":\"tree\",\"sha\":\"$specs\"}"
if [ -n "$schemas" ]; then
  entries="$entries,{\"path\":\"schemas\",\"type\":\"tree\",\"sha\":\"$schemas\"}"
fi
printf '{"sha":"0000000000000000000000000000000000000000","truncated":false,"tree":[%s]}\n' "$entries"
STUB
  chmod +x "$stub/gh"

  local g=(git -C "$fixture" -c user.name=self-test -c user.email=self-test@example.invalid
    -c commit.gpgsign=false -c core.hooksPath=/dev/null)
  pin() {
    printf 'VEREDICTUM_VERSION="%s"\nVEREDICTUM_REPO="https://github.com/Example-Org/Instrument"\n' "$1" \
      > "$fixture/scripts/lib/veredictum.sh"
  }
  "${g[@]}" init -q -b main
  pin 1.0.0
  "${g[@]}" add -A
  "${g[@]}" commit -q -m base
  local base bumped recorded same_pin
  base="$("${g[@]}" rev-parse HEAD)"
  pin 1.0.1
  "${g[@]}" commit -q -am bump
  bumped="$("${g[@]}" rev-parse HEAD)"
  mkdir -p "$fixture/docs/conformance/ferroehr"
  echo record > "$fixture/docs/conformance/ferroehr/record.json"
  "${g[@]}" add -A
  "${g[@]}" commit -q -m record
  recorded="$("${g[@]}" rev-parse HEAD)"
  "${g[@]}" checkout -q -b same "$base"
  echo unrelated > "$fixture/unrelated.txt"
  "${g[@]}" add -A
  "${g[@]}" commit -q -m unrelated
  same_pin="$("${g[@]}" rev-parse HEAD)"

  local failed=0
  # expect LABEL WANT-EXIT MODE NEEDLE ARGS...: run the guard, check its exit
  # code, and that its output carries NEEDLE.
  expect() {
    local label=$1 want=$2 mode=$3 needle=$4 rc=0
    shift 4
    : > "$log"
    (cd "$fixture" && PATH="$stub:$PATH" GH_STUB_MODE="$mode" GH_STUB_LOG="$log" \
      bash scripts/checks/veredictum-pin.sh "$@") > "$work/out" 2>&1 || rc=$?
    if [[ "$rc" -ne "$want" ]]; then
      echo "::error::--self-test: ${label}: exit ${rc}, wanted ${want}." >&2
      cat "$work/out" >&2
      failed=1
    elif ! grep -qF -- "$needle" "$work/out"; then
      echo "::error::--self-test: ${label}: the output does not say '${needle}'." >&2
      cat "$work/out" >&2
      failed=1
    else
      echo "  self-test: ${label} (exit ${rc})"
    fi
  }

  expect "unchanged trees pass without record or label" 0 same \
    "app/veredictum/src  4444444444444444444444444444444444444444" "$base" "$bumped"
  local call
  for call in "git/trees/refs/tags/v1.0.0" "git/trees/refs/tags/v1.0.1" \
    "contents/app/veredictum?ref=refs/tags/v1.0.0" "contents/app/veredictum?ref=refs/tags/v1.0.1"; do
    if ! grep -qxF "api repos/Example-Org/Instrument/${call}" "$log"; then
      echo "::error::--self-test: the guard did not read ${call} from the VEREDICTUM_REPO repository:" >&2
      cat "$log" >&2
      failed=1
    fi
  done
  local mode
  for mode in artifacts specs schemas runner; do
    expect "a changed ${mode} tree fails without record or label" 1 "$mode" \
      "commits nothing under docs/conformance/ferroehr/" "$base" "$bumped"
  done
  expect "a changed artifacts tree passes with the label" 0 artifacts \
    "no-conformance-run" --no-conformance-run "$base" "$bumped"
  expect "a changed artifacts tree passes with a refreshed record" 0 artifacts \
    "refreshed docs/conformance/ferroehr/ record" "$base" "$recorded"
  expect "an API failure fails closed" 1 fail \
    "commits nothing under docs/conformance/ferroehr/" "$base" "$bumped"
  expect "a directory missing at one tag fails closed" 1 missing \
    "commits nothing under docs/conformance/ferroehr/" "$base" "$bumped"
  expect "a runner tree missing at one tag fails closed" 1 runner-missing \
    "commits nothing under docs/conformance/ferroehr/" "$base" "$bumped"
  expect "an unchanged pin passes" 0 fail "unchanged" "$base" "$same_pin"
  if [[ -s "$log" ]]; then
    echo "::error::--self-test: an unchanged pin still called gh:" >&2
    cat "$log" >&2
    failed=1
  fi
  expect "an unknown flag is a usage error" 2 same "usage:" --bogus "$base" "$bumped"

  [[ "$failed" -eq 0 ]] || exit 1
  echo "veredictum-pin --self-test: every case behaved — OK."
}

if [[ "${1:-}" == "--self-test" && "$#" -eq 1 ]]; then
  self_test
  exit 0
fi

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
readonly GRAMMAR="[--no-conformance-run] <base> <head> | --self-test"
guard_known_flags "$GRAMMAR" "--no-conformance-run" "$@"
has_label=false
if [[ "${1:-}" == "--no-conformance-run" ]]; then
  has_label=true
  shift
fi
[[ "$#" -eq 2 ]] || guard_usage "$GRAMMAR"
base=$1
head=$2

cd "$(dirname "$0")/../.."

# pin_value REV NAME: the quoted value of NAME in scripts/lib/veredictum.sh at
# REV, or nothing when the file or the assignment is absent.
pin_value() {
  { git show "$1:scripts/lib/veredictum.sh" 2>/dev/null || true; } |
    sed -nE "s/^$2=\"([^\"]*)\"\$/\\1/p" | head -n 1
}

old_version=$(pin_value "$base" VEREDICTUM_VERSION)
new_version=$(pin_value "$head" VEREDICTUM_VERSION)
if [[ "$old_version" == "$new_version" ]]; then
  echo "VEREDICTUM_VERSION unchanged (${old_version:-<absent>}) — guard not required."
  exit 0
fi

# tree_shas REPO VERSION: one `<path> <sha>` line per path in TREES, read from
# the tag's root tree or, for a nested path, its parent's contents listing;
# fails on anything that is not a complete answer.
tree_shas() {
  local repo=$1 version=$2 slug root json path parent sha
  slug="${repo#https://github.com/}"
  slug="${slug%/}"
  slug="${slug%.git}"
  if [[ -z "$version" || "$slug" == "$repo" || ! "$slug" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
    echo "::warning::cannot read the Veredictum trees for version '${version}' from '${repo}' — not a github.com repository URL and a version." >&2
    return 1
  fi
  if ! root=$(gh api "repos/${slug}/git/trees/refs/tags/v${version}"); then
    echo "::warning::the GitHub API did not answer for ${slug} tag v${version}." >&2
    return 1
  fi
  for path in "${TREES[@]}"; do
    parent="${path%/*}"
    if [[ "$parent" == "$path" ]]; then
      json=$root
    elif ! json=$(gh api "repos/${slug}/contents/${parent}?ref=refs/tags/v${version}"); then
      echo "::warning::the GitHub API did not answer for ${slug} ${parent}/ at tag v${version}." >&2
      return 1
    fi
    # A root tree answers {"tree": [...]} with type "tree"; a contents listing
    # answers [...] with type "dir". Both carry the full path.
    sha=$(jq -r --arg p "$path" '[(if type == "array" then . else .tree end)[]?
      | select(.path == $p and (.type == "tree" or .type == "dir")) | .sha] | first // empty' \
      <<< "$json") || sha=""
    if [[ ! "$sha" =~ ^[0-9a-f]{40}$ ]]; then
      echo "::warning::${slug} tag v${version} has no '${path}' tree." >&2
      return 1
    fi
    printf '%s %s\n' "$path" "$sha"
  done
}

echo "VEREDICTUM_VERSION ${old_version:-<absent>} → ${new_version:-<absent>}"
unchanged=false
if old_trees=$(tree_shas "$(pin_value "$base" VEREDICTUM_REPO)" "$old_version") &&
  new_trees=$(tree_shas "$(pin_value "$head" VEREDICTUM_REPO)" "$new_version"); then
  unchanged=true
  for path in "${TREES[@]}"; do
    old_sha=$(awk -v p="$path" '$1 == p { print $2 }' <<< "$old_trees")
    new_sha=$(awk -v p="$path" '$1 == p { print $2 }' <<< "$new_trees")
    if [[ "$old_sha" == "$new_sha" ]]; then
      verdict=equal
    else
      verdict=changed
      unchanged=false
    fi
    printf '  %-18s  %s  %s  %s\n' "$path" "$old_sha" "$new_sha" "$verdict"
  done
fi

if [[ "$unchanged" == true ]]; then
  echo "the catalogue, specs, schemas and runner trees are identical at both tags — the committed baseline already covers this pin, no record or label needed — OK."
  exit 0
fi
if git diff --name-only "$base" "$head" | grep -q '^docs/conformance/ferroehr/'; then
  echo "pin bump carries a refreshed docs/conformance/ferroehr/ record — OK."
  exit 0
fi
if [[ "$has_label" == true ]]; then
  echo "no-conformance-run label set — the deferral is recorded on the PR — OK."
  exit 0
fi
echo "::error::This PR bumps VEREDICTUM_VERSION (${old_version:-<absent>} → ${new_version:-<absent>}), the catalogue, specs, schemas and runner trees are not proven identical at both tags, and it commits nothing under docs/conformance/ferroehr/ — re-prove the pin with a full 'bash scripts/conformance.sh' acceptance run whose refreshed record lands in the same PR (scripts/lib/veredictum.sh), or apply the 'no-conformance-run' label to defer the run deliberately." >&2
exit 1
