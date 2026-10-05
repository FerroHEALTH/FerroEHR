#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# A migration that has SHIPPED is never edited (owner rulings 2026-09-09 and
# 2026-09-15).
#
# sqlx records a checksum of every applied migration and refuses a database
# whose recorded checksum no longer matches the file on disk, so editing a
# migration an installation has applied does not change history: it locks that
# installation out of its own database at boot, with an error about a checksum
# rather than about the edit.
#
# SHIPPED means present at the latest release tag in the repository
# (`v<major>.<minor>.<patch>`, the tags the release procedure cuts), whether or
# not the branch descends from it: a history rewrite of `main` (2026-08-31,
# 2026-09-16) leaves every earlier tag on a lineage the branch no longer
# reaches, and the files those releases ship are applied by installations all
# the same. A
# file that is on the base branch but in no release yet has been applied by no
# installation: it is rewrite material, and it is fixed in the file that
# defines it rather than papered over by a follow-up file (owner ruling
# 2026-09-15: the rewrite is breaking changes only, no rename migrations, no
# placeholder roles, no compatibility paths). The rule therefore re-arms by
# itself at every release cut, and needs no escape-hatch label. For a shipped
# file a schema change is a NEW file; a shipped migration that turned out wrong
# is superseded by a later one, never rewritten.
#
# RETIRING A WHOLE SET IS THE ONE THING THAT IS NOT AN EDIT, and it is accepted
# only in the shape that keeps an installation told rather than locked out:
#
#   * every SHIPPED file of `app/ferroehr/migrations/<schema>/` is gone at head,
#     so no half-set survives for a database to apply against. New
#     files may appear in the same directory: a schema NAME can outlive the set
#     that used it, and re-authoring `ext`, `linkage` or `audit` is exactly that.
#     What may not survive is any file of the old set, edited or untouched.
#   * `<schema>` is named in `FIRST_GENERATION_SETS` in
#     `app/ferroehr/src/db/mod.rs` at head, which is the table the server's boot
#     refusal reads: a database carrying that set's bookkeeping is refused by
#     name, with the remedy, instead of failing on a checksum.
#
# Both halves are machine-checked here, so the acceptance cannot be claimed by
# a comment or a label. For shipped files a single-file edit, a rename, a
# partial turnover, or a whole-set turnover whose schema the boot refusal does
# not name stays refused. When the repository carries no release tag (a clone
# without tags), every base file counts as shipped: the conservative reading.
#
# Usage:
#   scripts/checks/migration-immutability.sh --diff <base> [head]
#   scripts/checks/migration-immutability.sh --self-test
set -euo pipefail
cd "$(dirname "$0")/../.." || exit 1

readonly MIGRATIONS='app/ferroehr/migrations/'
readonly RULE='.claude/rules/sqlx-conventions.md §Migrations'
# The file whose `FIRST_GENERATION_SETS` table the boot refusal reads.
readonly REFUSAL_FILE='app/ferroehr/src/db/mod.rs'

# The schemas the boot refusal names at `$1`, one per line.
#
# The table is a list of `FirstGenerationSet { schema: "…", … }` records, so the
# schema names are read out of it rather than restated here: a schema the guard
# accepted a retirement for is by construction one the server refuses a database
# for.
refused_schemas_at() {
  local rev="$1"
  git show "$rev:$REFUSAL_FILE" 2>/dev/null \
    | sed -n '/^const FIRST_GENERATION_SETS/,/^];$/p' \
    | grep -o 'schema: "[A-Za-z0-9_]*"' \
    | sed 's/schema: "\(.*\)"/\1/'
}

# The highest release tag in the repository by version, or nothing when it
# carries none (a clone fetched without tags, or a repository before its first
# release). Ancestry is deliberately not required: after a rewrite of `main`
# the release tags sit on the old lineage, and `git describe` would find none.
shipped_tag() {
  git tag --list --sort=-version:refname 'v[0-9]*.[0-9]*.[0-9]*' 2>/dev/null \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | head -1 || true
}

# Whether `$2` (a migration path) exists at `$1` (the release tag, or the merge
# base when the repository carries no tag): that is what makes a file SHIPPED.
shipped_at() {
  git cat-file -e "$1:$2" 2>/dev/null
}

# Whether every SHIPPED file under `$3` (a schema directory) is absent at `$2`
# (head), `$1` being the revision whose tree defines "shipped".
#
# Turnover, not emptiness: the directory may hold a whole new set at head, which
# is what re-authoring a surviving schema name looks like. What must be gone is
# every shipped file of the old one.
whole_set_turned_over() {
  local shipped="$1" head="$2" schema="$3" file
  while IFS= read -r file; do
    [[ -n "$file" ]] || continue
    if git cat-file -e "$head:$file" 2>/dev/null; then
      return 1
    fi
  done < <(git ls-tree -r --name-only "$shipped" -- "${MIGRATIONS}${schema}/")
  return 0
}

# Whether deleting a file of `$3` is part of a whole-set retirement the boot
# refusal covers.
retired_with_the_refusal() {
  local shipped="$1" head="$2" schema="$3"
  whole_set_turned_over "$shipped" "$head" "$schema" || return 1
  refused_schemas_at "$head" | grep -qx "$schema"
}

# The status letters git reports for a change that is not a pure addition.
# A = added (allowed); M = modified, D = deleted, R = renamed, C = copied,
# T = type change (all refused, except the whole-set retirement above).
#
# A RENAME is judged like a deletion of its old path, because that is what an
# installed database sees: the row it recorded for the old file has nothing left
# to match, checksum or not. Git reports the old path first, which is the one
# the retirement test reads.
refused_changes() {
  local base="$1" head="$2" fork shipped status path schema
  # The comparison is against the MERGE BASE, not the base branch's tip: a
  # branch that fell behind a main which added a migration must not be told
  # it deleted that file (#3358).
  fork="$(git merge-base "$base" "$head")"
  # What counts as shipped is the tree of the latest release tag; without one
  # every base file does.
  shipped="$(shipped_tag)"
  [[ -n "$shipped" ]] || shipped="$fork"
  while IFS=$'\t' read -r status path _; do
    [[ -n "${status:-}" ]] || continue
    # A file no release carries has been applied by no installation.
    shipped_at "$shipped" "$path" || continue
    case $status in
    D* | R* | C*)
      schema="${path#"$MIGRATIONS"}"
      schema="${schema%%/*}"
      retired_with_the_refusal "$shipped" "$head" "$schema" && continue
      ;;
    *) ;;
    esac
    printf '%s\t%s\n' "$status" "$path"
  done < <(git diff --name-status --diff-filter=MDRCT "$fork" "$head" -- "$MIGRATIONS")
}

report() {
  local found="$1"
  if [[ -z "$found" ]]; then
    echo "migration-immutability: OK (no migration file was modified, renamed or partially deleted)."
    return 0
  fi
  echo "error: a migration that has SHIPPED (is in a release) was changed" >&2
  echo >&2
  printf '%s\n' "$found" >&2
  echo >&2
  echo "sqlx checksums every applied migration, so changing one refuses the" >&2
  echo "database of every installation that already ran it. A file that is in" >&2
  echo "no release yet may be fixed in place; a shipped one carries the change" >&2
  echo "forward in a NEW migration instead:" >&2
  echo >&2
  echo "  sqlx migrate add --source ${MIGRATIONS}<schema> --sequential <desc>" >&2
  echo >&2
  echo "A whole SET is retired rather than edited, and only together with the" >&2
  echo "boot refusal that names its schema: every shipped file under" >&2
  echo "${MIGRATIONS}<schema>/ must be gone (a new set may take its place), and" >&2
  echo "<schema> must be named in FIRST_GENERATION_SETS in ${REFUSAL_FILE}, so an" >&2
  echo "existing database is told what happened instead of being locked out by a" >&2
  echo "checksum." >&2
  echo >&2
  echo "The rule is $RULE." >&2
  return 1
}

# Proves the detector in every direction rather than trusting it, over the four
# shapes the rule turns on: turnover WITH the refusal, turnover WITHOUT it,
# PARTIAL turnover, and a one-file edit.
self_test() {
  local tmp
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" RETURN
  (
    cd "$tmp" || exit 1
    # The base branch is NAMED, so the diff below does not guess at the
    # machine's init.defaultBranch (#3285: a main/master fallback chained with
    # `||` always ran the master diff and died on a checkout defaulting to main).
    git init -q --initial-branch=base .
    git config user.email t@example.invalid
    git config user.name t
    mkdir -p "$MIGRATIONS/ext" "$MIGRATIONS/demographic" "$MIGRATIONS/audit" \
             "$MIGRATIONS/keep" "$(dirname "$REFUSAL_FILE")"
    printf 'SELECT 1;\n' > "$MIGRATIONS/ext/0001_openehr_functions.sql"
    printf 'SELECT 2;\n' > "$MIGRATIONS/ext/0002_tenant_context.sql"
    printf 'SELECT 3;\n' > "$MIGRATIONS/demographic/0001_baseline.sql"
    printf 'SELECT 4;\n' > "$MIGRATIONS/audit/0001_baseline.sql"
    printf 'SELECT 5;\n' > "$MIGRATIONS/audit/0002_chain.sql"
    printf 'SELECT 6;\n' > "$MIGRATIONS/keep/0001_baseline.sql"
    printf 'SELECT 7;\n' > "$MIGRATIONS/keep/0002_second.sql"
    printf 'const FIRST_GENERATION_SETS: &[FirstGenerationSet] = &[\n];\n' > "$REFUSAL_FILE"
    git add -A && git commit -qm base
    # Everything above is SHIPPED: the release tag sits on it.
    git -c tag.gpgSign=false tag v0.1.0
    # A file added to the base branch AFTER the release is in no installation's
    # database: (f) editing it on the work branch is allowed.
    printf 'SELECT 14;\n' > "$MIGRATIONS/keep/0003_unshipped.sql"
    git add -A && git commit -qm "post-release, unshipped"
    # (g) A LATER release was cut on a lineage the base no longer descends from
    #     (a rewritten main). Its files are shipped all the same: editing one
    #     on the work branch is refused, while 0003 stays unshipped.
    git switch -qc old-lineage v0.1.0
    printf 'SELECT 15;\n' > "$MIGRATIONS/keep/0006_released_off_lineage.sql"
    git add -A && git commit -qm "released on the old lineage"
    git -c tag.gpgSign=false tag v0.2.0
    git switch -q base
    printf 'SELECT 15;\n' > "$MIGRATIONS/keep/0006_released_off_lineage.sql"
    git add -A && git commit -qm "the same file on the rewritten base"

    # (a) `ext` turns over WHOLE — every base file gone, a new set in its place —
    #     and the boot refusal names it: accepted, which is the shape this
    #     branch's own `ext`, `linkage` and `audit` sets have.
    # (b) `audit` turns over whole and the refusal does NOT name it: refused,
    #     because such a database is locked out rather than told.
    # (c) `demographic` turns over only PARTIALLY (one base file survives
    #     untouched): refused, even though the refusal names it.
    # (d) one file of `keep` is edited: refused, the plain rule.
    # (e) a file is added: allowed.
    git switch -qc work
    # A rename out of the old set is a deletion of the old path, so the
    # turnover still counts: git reports it as R, not D.
    git mv "$MIGRATIONS/ext/0001_openehr_functions.sql" "$MIGRATIONS/ext/0001_schema_and_roles.sql"
    printf 'SELECT 8;\n' > "$MIGRATIONS/ext/0001_schema_and_roles.sql"
    rm "${MIGRATIONS:?}/ext/0002_tenant_context.sql"
    rm "${MIGRATIONS:?}/audit/0001_baseline.sql" "${MIGRATIONS:?}/audit/0002_chain.sql"
    printf 'SELECT 9;\n' > "$MIGRATIONS/audit/0001_schema_and_roles.sql"
    printf 'SELECT 10; -- typo fix\n' > "$MIGRATIONS/keep/0002_second.sql"
    printf 'SELECT 11;\n' > "$MIGRATIONS/keep/0004_third.sql"
    printf 'SELECT 14; -- fixed in place, unshipped\n' > "$MIGRATIONS/keep/0003_unshipped.sql"
    printf 'SELECT 15; -- edited, but v0.2.0 ships it\n' > "$MIGRATIONS/keep/0006_released_off_lineage.sql"
    printf 'const FIRST_GENERATION_SETS: &[FirstGenerationSet] = &[\n    FirstGenerationSet { schema: "ext", first_description: Some("openehr functions") },\n    FirstGenerationSet { schema: "demographic", first_description: None },\n];\n' > "$REFUSAL_FILE"
    git add -A && git commit -qm work

    # Meanwhile the base branch gained a migration the work branch never saw:
    # a tip-to-head diff would report it as deleted (#3358).
    git switch -q base
    printf 'SELECT 12;\n' > "$MIGRATIONS/keep/0005_later.sql"
    git add -A && git commit -qm later
  )
  local found failures=0
  found="$(cd "$tmp" && refused_changes base work)"
  if grep -q '/ext/' <<<"$found"; then
    echo "self-test: a whole-set turnover the boot refusal NAMES was wrongly caught" >&2
    failures=1
  fi
  grep -q '/audit/' <<<"$found" \
    || { echo "self-test: a whole-set turnover the boot refusal does NOT name was not caught" >&2; failures=1; }
  grep -q '0002_second.sql' <<<"$found" \
    || { echo "self-test: an EDITED migration was not caught" >&2; failures=1; }
  if grep -q '0004_third.sql' <<<"$found"; then
    echo "self-test: an ADDED migration was wrongly caught" >&2
    failures=1
  fi
  if grep -q '0003_unshipped.sql' <<<"$found"; then
    echo "self-test: an UNSHIPPED migration (on base, in no release) edited in place was wrongly caught" >&2
    failures=1
  fi
  grep -q '0006_released_off_lineage.sql' <<<"$found" \
    || { echo "self-test: a migration shipped by a release tag OFF the base's lineage was not caught" >&2; failures=1; }
  if grep -q '0005_later.sql' <<<"$found"; then
    echo "self-test: a migration the BASE gained after the branch point was wrongly caught" >&2
    failures=1
  fi

  # PARTIAL turnover of a named set: one base file of `demographic` is deleted
  # while the other survives. Never acceptable — half a set is what a database
  # would apply against.
  (
    cd "$tmp" || exit 1
    git switch -q work
    printf 'SELECT 13;\n' > "$MIGRATIONS/demographic/0002_second.sql"
    git add -A && git commit -qm "a second demographic file, on the work branch"
    git switch -q base
    printf 'SELECT 13;\n' > "$MIGRATIONS/demographic/0002_second.sql"
    git add -A && git commit -qm "the same file on base, so it is a BASE file"
    git switch -q work
    git merge -q --no-edit base >/dev/null 2>&1 || true
    rm "${MIGRATIONS:?}/demographic/0001_baseline.sql"
    git add -A && git commit -qm "only half of the demographic set is gone"
  )
  local partial
  partial="$(cd "$tmp" && refused_changes base work)"
  grep -q '0001_baseline.sql' <<<"$partial" \
    || { echo "self-test: a PARTIAL turnover of a named set was not caught" >&2; failures=1; }
  if [[ "$failures" -ne 0 ]]; then
    return 1
  fi
  echo "migration-immutability: self-test OK (shipped edit, partial turnover and unannounced turnover caught; announced whole-set turnover, addition, an unshipped edit and behind-main allowed; a release tag off the lineage still governs)."
}

case "${1:-}" in
--self-test)
  self_test
  ;;
--diff)
  base="${2:?usage: --diff <base> [head]}"
  head="${3:-HEAD}"
  report "$(refused_changes "$base" "$head")"
  ;;
*)
  echo "usage: $0 [--diff <base> [head] | --self-test]" >&2
  exit 2
  ;;
esac
