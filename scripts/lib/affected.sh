#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Which workspace packages a change reaches, for the cargo lanes of ci.yml
# (#6519). No specification governs this: our own design (docs/ci-cd.md).
#
# Usage:
#   scripts/lib/affected.sh <base-rev> <head-rev>
#   scripts/lib/affected.sh --all
#   scripts/lib/affected.sh --self-test
#
# Output, the form $GITHUB_OUTPUT reads:
#   all        true when the change reaches every package
#   packages   the reached packages, space-separated, in workspace order
#   rust       true when any package is reached
#   viewer     true when ferroehr-viewer is reached
#   server     true when a package other than ferroehr-viewer is reached
#   cargo_args the package arguments of the server lanes (viewer excluded):
#              `--workspace --exclude ferroehr-viewer`, or `-p a -p b ...`
#   all_args   the package arguments of the whole-workspace lanes (clippy,
#              rustdoc): `--workspace`, or `-p a -p b ...`
#
# A changed path seeds packages three ways:
#   * under a member's directory: that member, and every member whose Rust
#     source names that directory in a string literal (it reads the files);
#   * elsewhere: every member whose Rust source names the path's top-level
#     directory or file in a string literal (`"corpus/`, `/docs/`, `"perf"`,
#     `/CHANGELOG.md"`); a path no Rust source names reaches nothing;
#   * the build inputs (the toolchain, .cargo/, .config/, the lint and format
#     configurations, cargo-deny's, an orphan under app/, crates/ or tools/,
#     and this CI recipe) reach everything, and so do a third-party line of
#     the root manifest or the lockfile.
# A seed reaches its normal and build dependants transitively, and their
# dev-dependants (cargo metadata, https://doc.rust-lang.org/cargo/commands/cargo-metadata.html).
set -euo pipefail
cd "$(dirname "$0")/../.."

readonly VIEWER=ferroehr-viewer

die() {
  echo "affected: $*" >&2
  exit 1
}

has() {
  case " $1 " in *" $2 "*) return 0 ;; *) return 1 ;; esac
}

# {members: [{name, dir}], lib: [[from, to]], dev: [[from, to]]}: `to`
# depends on `from` as a normal or build dependency, or as a dev-dependency.
graph() {
  local metadata
  metadata="$(cargo metadata --no-deps --format-version 1 --locked)" || die "cargo metadata failed"
  jq -c '
    .workspace_root as $root
    | [.packages[] | {name, dir: (.manifest_path | ltrimstr($root + "/") | rtrimstr("/Cargo.toml"))}] as $members
    | [.packages[] as $p | $p.dependencies[] | select(.path != null)
        | (.path | ltrimstr($root + "/")) as $dir
        | ($members[] | select(.dir == $dir) | .name) as $from
        | {kind: (.kind // "normal"), edge: [$from, $p.name]}] as $deps
    | {members: $members,
       lib: ([$deps[] | select(.kind != "dev") | .edge] | unique),
       dev: ([$deps[] | select(.kind == "dev") | .edge] | unique)}
  ' <<<"$metadata"
}

# The members whose Rust sources name $2 (a top-level name, or a member's
# directory) in a string literal, space-separated.
readers() {
  local graph="$1" name="$2" leaf files
  leaf="${name##*/}"
  files="$(git grep -l -F -e "\"$name/" -e "/$name/" -e "\"$name\"" -e "/$name\"" -e "\"../$leaf" \
    -- 'app/*.rs' 'crates/*.rs' 'tools/*.rs' || true)"
  [[ -n "$files" ]] || return 0
  jq -rR --argjson g "$graph" -s '
    split("\n") | map(select(length > 0)) as $files
    | [$g.members[] | select(.dir as $d | $files | any(startswith($d + "/"))) | .name] | join(" ")
  ' <<<"$files"
}

# The member owning a path, or nothing.
owner() {
  jq -rn --argjson g "$graph" --arg path "$1" \
    '[$g.members[] | select(.dir as $d | $path | startswith($d + "/")) | .name][0] // ""'
}

# Every lockfile entry as `"name" "version" [source]`, sorted.
lock_entries() {
  awk '/^\[\[package\]\]/ { if (n != "") print n, v, s; n = v = s = "" }
       /^name = / { n = $3 } /^version = / { v = $3 } /^source = / { s = $3 }
       END { if (n != "") print n, v, s }' | sort
}
manifest_diff() { git diff -U0 "$base" "$head" -- Cargo.toml; }
lock_at() { if [[ "$1" = old ]]; then git show "$base:Cargo.lock"; else git show "$head:Cargo.lock"; fi; }

# The members a root-manifest change reaches, or ALL: a changed line that is a
# member's own `[workspace.dependencies]` path entry reaches that member, any
# other line everything.
manifest_effect() {
  local members line name names=""
  members="$(jq -r '[.members[].name] | join(" ")' <<<"$graph")"
  while IFS= read -r line; do
    case "$line" in
      '+++ '* | '--- '*) continue ;;
      '+'* | '-'*) line="${line#?}" ;;
      *) continue ;;
    esac
    name="${line%% = \{ path = \"*}"
    if [[ "$name" = "$line" ]] || ! has "$members" "$name"; then
      echo ALL
      return
    fi
    names+=" $name"
  done < <(manifest_diff)
  echo "$names"
}

# The members a lockfile change reaches, or ALL: a moved entry with no source
# is a member's version, one with a source a third-party crate.
lock_effect() {
  local members old new name version source names=""
  members="$(jq -r '[.members[].name] | join(" ")' <<<"$graph")"
  old="$(lock_at old | lock_entries)" || { echo ALL; return; }
  new="$(lock_at new | lock_entries)" || { echo ALL; return; }
  while read -r name version source; do
    [[ -n "$name" ]] || continue
    name="${name//\"/}"
    if [[ -n "$source" || -z "$version" ]] || ! has "$members" "$name"; then
      echo ALL
      return
    fi
    names+=" $name"
  done < <(comm -3 <(printf '%s\n' "$old") <(printf '%s\n' "$new"))
  echo "$names"
}

# The seeds of the changed paths on stdin, or ALL.
seeds_of() {
  local paths path top member effect seeds="" seen=" "
  paths="$(cat)"
  while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    case "$path" in
      Cargo.toml) effect="$(manifest_effect)" ;;
      Cargo.lock) effect="$(lock_effect)" ;;
      rust-toolchain.toml | clippy.toml | rustfmt.toml | deny.toml | .cargo/* | .config/* | .github/workflows/ci.yml | scripts/lib/affected.sh)
        effect=ALL
        ;;
      app/* | crates/* | tools/*)
        member="$(owner "$path")"
        if [[ -z "$member" ]]; then
          effect=ALL
        else
          top="$(jq -r --arg n "$member" '.members[] | select(.name == $n) | .dir' <<<"$graph")"
          effect="$member"
          if ! has "$seen" "$top"; then
            seen+="$top "
            effect+=" $(readers "$graph" "$top")"
          fi
        fi
        ;;
      *)
        top="${path%%/*}"
        effect=""
        if ! has "$seen" "$top"; then
          seen+="$top "
          effect="$(readers "$graph" "$top")"
        fi
        ;;
    esac
    if [[ "$effect" = ALL ]]; then
      echo "affected: $path reaches every package" >&2
      echo ALL
      return
    fi
    seeds+=" $effect"
  done <<<"$paths"
  echo "$seeds"
}

# The members the seeds reach, in workspace order.
reach() {
  jq -rn --argjson g "$graph" --arg seeds "$1" '
    def hop($edges; $set): [$edges[] | select(.[0] as $f | $set | any(. == $f)) | .[1]];
    ($seeds | split(" ") | map(select(length > 0)) | unique) as $start
    | {set: $start, grew: true}
    | until(.grew | not;
        .set as $s
        | ($s + hop($g.lib; $s) | unique) as $next
        | {set: $next, grew: (($next | length) > ($s | length))})
    | (.set + hop($g.dev; .set) | unique) as $s
    | [$g.members[].name | select(. as $n | $s | any(. == $n))]
    | join(" ")
  '
}

emit() {
  local all="$1" packages="$2" p args=""
  echo "all=$all"
  echo "packages=$packages"
  if [[ -n "$packages" ]]; then echo "rust=true"; else echo "rust=false"; fi
  if has "$packages" "$VIEWER"; then echo "viewer=true"; else echo "viewer=false"; fi
  local every=""
  if [[ "$all" = true ]]; then
    args="--workspace --exclude $VIEWER"
    every="--workspace"
  else
    for p in $packages; do
      every+="${every:+ }-p $p"
      [[ "$p" = "$VIEWER" ]] || args+="${args:+ }-p $p"
    done
  fi
  if [[ -n "$args" ]]; then echo "server=true"; else echo "server=false"; fi
  echo "cargo_args=$args"
  echo "all_args=$every"
}

everything() {
  emit true "$(jq -r '[.members[].name] | join(" ")' <<<"$graph")"
}

self_test() {
  local failures=0 got
  check() {
    local label="$1" paths="$2" want="$3" w
    got="$(printf '%s\n' "$paths" | seeds_of 2>/dev/null)"
    [[ "$got" = ALL ]] || got="$(reach "$got")"
    for w in $want; do
      case "$w" in
        ALL) [[ "$got" = ALL ]] && continue ;;
        NONE) [[ -z "$got" ]] && continue ;;
        +*) [[ "$got" != ALL ]] && has "$got" "${w#+}" && continue ;;
        -*) [[ "$got" != ALL ]] && ! has "$got" "${w#-}" && continue ;;
        *) ;;
      esac
      echo "self-test: $label: expected $w, got '$got'" >&2
      failures=$((failures + 1))
    done
  }
  local diff_text="" lock_old="" lock_new="" q='"'
  manifest_diff() { printf '%s\n' "$diff_text"; }
  lock_at() { if [[ "$1" = old ]]; then printf '%s\n' "$lock_old"; else printf '%s\n' "$lock_new"; fi; }
  check "a generated crate reaches the server and its dependants" "crates/openehr-rm/src/lib.rs" \
    "+openehr-rm +ferroehr +ferroehr-rest +ferroehr-server -openehr-base"
  check "the viewer reaches the viewer alone of the apps" "app/ferroehr-viewer/src/lib.rs" \
    "+ferroehr-viewer -ferroehr -ferroehr-rest -openehr-rm"
  check "a server source does not reach the spec crates" "app/ferroehr-rest/src/lib.rs" \
    "+ferroehr-rest +ferroehr-server -openehr-rm -ferroehr-viewer"
  check "the corpus reaches the crates that read it" "corpus/README.md" "+ferroehr-rest +openehr-its"
  check "a changelog fragment reaches nothing" "changelog.d/6519-x.changed.md" "NONE"
  check "another workflow reaches nothing" ".github/workflows/docs.yml" "NONE"
  check "the CI recipe reaches everything" ".github/workflows/ci.yml" "ALL"
  check "the toolchain reaches everything" "rust-toolchain.toml" "ALL"
  check "an orphan under crates/ reaches everything" "crates/no-such-member/x.rs" "ALL"
  check "a long list opening with a build input reaches everything" \
    "$(printf 'rust-toolchain.toml\n'; seq -f 'corpus/p/%g.json' 1 5000)" "ALL"
  diff_text="+openehr-rm = { path = ${q}crates/openehr-rm${q}, version = ${q}9.9.9${q} }"
  check "a member's manifest entry reaches that member" "Cargo.toml" "+openehr-rm +ferroehr -openehr-base"
  diff_text="+serde = { version = ${q}8.8.9${q} }"
  check "a third-party manifest line reaches everything" "Cargo.toml" "ALL"
  lock_old="[[package]]
name = ${q}openehr-rm${q}
version = ${q}9.9.8${q}
[[package]]
name = ${q}serde${q}
version = ${q}8.8.8${q}
source = ${q}registry+https://github.com/rust-lang/crates.io-index${q}"
  lock_new="${lock_old/9.9.8/9.9.9}"
  check "a member's lock move reaches that member" "Cargo.lock" "+openehr-rm -openehr-base"
  lock_new="${lock_old/8.8.8/8.8.9}"
  check "a third-party lock move reaches everything" "Cargo.lock" "ALL"
  # Every member is reachable from its own directory, so none is unmapped.
  local name dir
  while IFS=$'\t' read -r name dir; do
    got="$(reach "$(printf '%s\n' "$dir/src/lib.rs" | seeds_of 2>/dev/null)")"
    has "$got" "$name" || { echo "self-test: member $name is not reached from $dir" >&2; failures=$((failures + 1)); }
  done < <(jq -r '.members[] | [.name, .dir] | @tsv' <<<"$graph")
  [[ "$failures" -eq 0 ]] || die "self-test: $failures failure(s)"
  echo "affected: self-test OK"
}

command -v jq >/dev/null 2>&1 || die "jq is not installed"
graph="$(graph)"
case "${1:-}" in
  --self-test) self_test ;;
  --all) everything ;;
  "") echo "usage: $0 <base-rev> <head-rev> | --all | --self-test" >&2; exit 2 ;;
  *)
    base="$1"
    head="${2:?usage: $0 <base-rev> <head-rev>}"
    seeds="$(bash scripts/lib/changed-paths.sh "$base" "$head" | seeds_of)"
    if [[ "$seeds" = ALL ]]; then everything; else emit false "$(reach "$seeds")"; fi
    ;;
esac
