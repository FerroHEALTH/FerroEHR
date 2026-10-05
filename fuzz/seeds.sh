#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Populate fuzz/seeds/<target>/ from the corpora committed in this repository,
# as SYMLINKS — the archetype and template packs are ~100 MB, are
# provenance-stamped where they live, and are never copied. The one seeder that
# reads outside the tree is the AQL catalogue-query extractor; it says so.
#
# Selection is size-bounded and deterministic (sorted, then capped): libFuzzer
# derives its default input length from the largest seed and re-reads every seed
# on each run, so a handful of multi-megabyte templates would sink the execution
# rate for no extra coverage.
#
# The one input that is not a corpus is fuzz/regressions/<target>/ — the tracked
# artifacts of past findings, wired in after each target's wipe.
#
# Usage: fuzz/seeds.sh            (all targets)
#        fuzz/seeds.sh <target>…  (named targets only)
#
# Reference: the cargo-fuzz book, "Corpora"
# <https://rust-fuzz.github.io/book/cargo-fuzz/tutorial.html>.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
seeds_root="$repo_root/fuzz/seeds"
regressions_root="$repo_root/fuzz/regressions"

# Every seed source must exist: a renamed corpus directory silently producing an
# empty seed set is the failure mode this guards.
require_dir() {
  if [[ ! -d "$repo_root/$1" ]]; then
    echo "seeds.sh: missing corpus directory: $1" >&2
    exit 1
  fi
}

# link_from <target> <source-dir> <max-size> <cap> <find-name-pattern>…
#
# Symlinks up to <cap> files under <source-dir> smaller than <max-size>
# (a find -size argument, e.g. 32k) into fuzz/seeds/<target>/, naming each after
# its path so two corpora cannot collide.
link_from() {
  local target="$1" source="$2" max_size="$3" cap="$4"
  shift 4
  require_dir "$source"

  local find_args=()
  local pattern
  for pattern in "$@"; do
    find_args+=(-o -name "$pattern")
  done

  local dest="$seeds_root/$target"
  mkdir -p "$dest"

  local linked=0 path relative name
  while IFS= read -r path; do
    relative="${path#"$repo_root"/}"
    name="${relative//\//_}"
    ln -sf "../../../$relative" "$dest/$name"
    linked=$((linked + 1))
  done < <(
    find "$repo_root/$source" -type f \( -false "${find_args[@]}" \) \
      -size "-$max_size" | LC_ALL=C sort | head -n "$cap"
  )
  echo "  $target <- $source ($linked files)"
}

# The AQL seeds are the only ones that are not already files on disk. Two
# sources: the official worked-example corpus, which lives inside AsciiDoc
# `----` listing blocks in the vendored QUERY spec examples (the same extraction
# the `openehr-query` corpus test performs), and the query text of the CNF
# catalogue's own cases, read from the pinned instrument.
extract_aql_examples() {
  local dest="$seeds_root/aql_query"
  mkdir -p "$dest"
  require_dir "crates/openehr-query/vendor/examples"
  local written
  written=$(
    awk -v dest="$dest" '
      /^----$/ { inside = !inside; if (!inside && block != "") {
                   file = sprintf("%s/adoc_%04d.aql", dest, ++n)
                   printf "%s", block > file
                   close(file)
                 }
                 block = ""; next }
      inside   { block = block $0 "\n" }
      END      { print n + 0 }
    ' "$repo_root"/crates/openehr-query/vendor/examples/*.adoc
  )
  echo "  aql_query <- crates/openehr-query/vendor/examples ($written listing blocks)"
}

# The catalogue's query text lives with the instrument, not in this repository:
# it is Veredictum's, obtained at the pin scripts/lib/veredictum.sh records. The
# seeder therefore resolves an ABSOLUTE schedule path through that helper and
# checks it itself — a missing catalogue fails loud here, the same way a renamed
# corpus directory does in require_dir, rather than quietly producing a smaller
# seed set than the last run.
extract_aql_catalogue_queries() {
  local dest="$seeds_root/aql_query"
  mkdir -p "$dest"
  # shellcheck source=scripts/lib/veredictum.sh
  source "$repo_root/scripts/lib/veredictum.sh"
  local schedule
  schedule="$(veredictum_artifacts)/schedule"
  if [[ ! -d "$schedule" ]]; then
    echo "seeds.sh: missing catalogue schedule: $schedule" >&2
    exit 1
  fi
  local written=0 query index=0
  while IFS= read -r query; do
    index=$((index + 1))
    printf '%s\n' "$query" > "$(printf '%s/cnf_%04d.aql' "$dest" "$index")"
    written=$((written + 1))
  done < <(
    grep -rhE '^[[:space:]]*(q|query|aql):[[:space:]]*"' \
      "$schedule" --include='*.yaml' |
      sed -E 's/^[[:space:]]*(q|query|aql):[[:space:]]*"(.*)"[[:space:]]*$/\2/' |
      LC_ALL=C sort -u
  )
  echo "  aql_query <- veredictum $VEREDICTUM_VERSION catalogue ($written queries)"
}

corpus=corpus

seed_canonical_json() {
  link_from canonical_json crates/openehr-its/tests/vendor 512k 400 '*.json'
  link_from canonical_json crates/openehr-its/tests/fixtures 512k 400 '*.json'
  link_from canonical_json "$corpus/fixtures" 512k 400 '*.json'
}

seed_canonical_xml() {
  link_from canonical_xml "$corpus/fixtures" 512k 400 '*.xml'
  link_from canonical_xml "$corpus/archetypes/ckm/xml" 32k 250 '*.xml'
}

seed_aql_query() {
  link_from aql_query "$corpus/fixtures/query" 64k 100 '*.aql'
  # NOTE: the samples directory holds exactly one `.aql`,
  # `191_where_parenthesis.aql`, and this glob is its only consumer — a
  # realistic parenthesised-WHERE query kept for its seed shape.
  link_from aql_query "$corpus/fixtures/service/samples" 64k 200 '*.aql'
  extract_aql_examples
  extract_aql_catalogue_queries
}

seed_simplified_formats() {
  link_from simplified_formats "$corpus/fixtures/sf" 512k 300 '*.json'
  link_from simplified_formats "$corpus/fixtures/flat" 512k 100 '*.json'
}

seed_adl2_source() {
  link_from adl2_source crates/openehr-adl/tests/corpus 64k 400 '*.adls' '*.adl' '*.adlf'
  link_from adl2_source "$corpus/archetypes/adl2" 32k 250 '*.adls' '*.adl'
  link_from adl2_source "$corpus/archetypes/ckm/adl14" 16k 250 '*.adl'
}

seed_opt14_template() {
  link_from opt14_template "$corpus/fixtures/opt" 512k 200 '*.opt' '*.xml'
  link_from opt14_template "$corpus/templates" 64k 250 '*.opt'
}

# Identifiers have no vendored corpus to link: they are short strings, not
# documents. So this target's seeds are WRITTEN — a handful of literal forms
# from the BASE grammar, which is a different thing from downloading a corpus
# and does not touch the provenance rules for vendored material.
#
# The point of a seed here is to hand libFuzzer the SHAPE (two separator kinds,
# a dotted third part) so its mutations land inside the grammar instead of
# spending the budget discovering that `::` matters.
seed_identifiers() {
  local dir="$seeds_root/identifiers"
  mkdir -p "$dir"
  # A plain trunk version, a branch, a non-UUID object id, an archetype id, a
  # bare version tree id, and the two degenerate separator cases.
  printf '%s' '8849182c-82ad-4088-a07f-48ead4180515::ferroehr.example.org::1' > "$dir/trunk"
  printf '%s' '8849182c-82ad-4088-a07f-48ead4180515::ferroehr.example.org::1.2.3' > "$dir/branch"
  printf '%s' '1.2.840.113554.1.2.2::ferroehr.example.org::1' > "$dir/oid-object-id"
  printf '%s' 'openEHR-EHR-COMPOSITION.encounter.v1' > "$dir/archetype-id"
  printf '%s' '1.1.1' > "$dir/version-tree-id"
  printf '%s' '::::' > "$dir/only-separators"
  printf '%s' 'a::b::' > "$dir/empty-third-part"
  echo "  identifiers: 7 written"
}

# Recorded finding artifacts: the exact inputs that reproduced a crash, leak or
# timeout. They live in fuzz/regressions/<target>/, which is TRACKED — unlike
# fuzz/seeds, which is generated and ignored — so every run re-checks every
# finding this lane has ever had. The directory is optional per target, and it is
# wired AFTER the wipe and the corpus links so a recorded artifact wins a name
# clash rather than losing one.
link_regressions() {
  local target="$1"
  local dir="$regressions_root/$target"
  [[ -d "$dir" ]] || return 0

  local dest="$seeds_root/$target"
  mkdir -p "$dest"

  local linked=0 path name
  while IFS= read -r path; do
    name="$(basename "$path")"
    ln -sf "../../../fuzz/regressions/$target/$name" "$dest/$name"
    linked=$((linked + 1))
  done < <(find "$dir" -type f | LC_ALL=C sort)
  echo "  $target <- fuzz/regressions/$target ($linked recorded artifacts)"
}

targets=("$@")
if [[ ${#targets[@]} -eq 0 ]]; then
  targets=(canonical_json canonical_xml aql_query simplified_formats adl2_source opt14_template identifiers)
fi

for target in "${targets[@]}"; do
  if [[ "$(type -t "seed_$target")" != function ]]; then
    echo "seeds.sh: unknown target: $target" >&2
    exit 1
  fi
  rm -rf "${seeds_root:?}/${target:?}"
  # libFuzzer refuses to start when a corpus directory it was given does not
  # exist, and the writable corpus is empty on a first run (or a CI cache miss).
  mkdir -p "$repo_root/fuzz/corpus/$target"
  echo "$target:"
  "seed_$target"
  link_regressions "$target"
  echo "  total: $(find "$seeds_root/$target" \( -type l -o -type f \) | wc -l | tr -d ' ') seeds"
done
