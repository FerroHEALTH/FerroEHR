#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Migration-citation guard: a Rust source under app/ may only cite a migration
# file that exists in the tree.
#
# A comment naming `0001_baseline.sql` used to point at the relation it
# described. The storage rewrite deleted that file, and eight comments went on
# citing it (#3418), which sends the next reader to a path that is not there and
# tells them nothing about where the relation is now defined.
#
# The check: every `NNNN_<name>.sql` token in a scanned file must match some
# file under app/ferroehr/migrations/ by path suffix, so `0006_definitions.sql`,
# `clinical/0006_definitions.sql` and `migrations/clinical/0006_definitions.sql`
# all resolve, and a deleted or misspelled one does not.
#
# Scope, and its limit: tracked `*.rs` under `app/`. Markdown is deliberately
# out of scope — the plans, the agent memory and the root CLAUDE.md discuss the
# first generation's files as history, and a guard that refused those would be
# refusing accurate prose.
#
# Exemptions are per FILE, each with its reason, below.
#
# Usage: scripts/checks/migration-citations.sh [--all | --self-test | <file>...]
#   no args     → the changed .rs files under app/
#   --all       → every tracked .rs file under app/
#   --self-test → runs the guard over a fixture citing a migration that does not
#   exist and over one citing a migration that does, and asserts the first is
#   rejected and the second accepted. A text detector never shown to fail is a
#   green light nobody has tested (reliability.md §enforcement tiers).
set -euo pipefail

if [[ "${1:-}" == "--self-test" ]]; then
  cd "$(dirname "$0")/../.."
  self_root="$(mktemp -d)"
  trap 'rm -rf "$self_root"' EXIT
  mkdir -p "$self_root/scripts" "$self_root/app/ferroehr/src"
  cp -R scripts/checks scripts/lib "$self_root/scripts/"
  cp -R app/ferroehr/migrations "$self_root/app/ferroehr/migrations"
  self_fail=0
  # <label>|<cited file>|<expected exit: 0 accept, 1 reject>
  self_cases=(
    "a deleted migration|0001_baseline.sql|1"
    "a misspelled schema directory|clinical/0006_defintions.sql|1"
    "a migration that exists|clinical/0006_definitions.sql|0"
  )
  for self_case in "${self_cases[@]}"; do
    label="${self_case%%|*}"
    rest="${self_case#*|}"
    cited="${rest%%|*}"
    want="${rest#*|}"
    printf '// the relation this file writes: %s\n' "$cited" \
      > "$self_root/app/ferroehr/src/self_test.rs"
    got=0
    (cd "$self_root" && bash scripts/checks/migration-citations.sh app/ferroehr/src/self_test.rs \
      >/dev/null 2>&1) || got=1
    if [[ "$got" != "$want" ]]; then
      echo "::error::--self-test: ${label} — expected exit ${want}, got ${got}." >&2
      self_fail=1
    else
      echo "  self-test: ${label} → exit ${got} as expected"
    fi
  done
  [[ "$self_fail" -eq 0 ]] || exit 1
  echo "migration-citations --self-test: a missing migration is rejected, a present one accepted — OK."
  exit 0
fi

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--all | --self-test | <file>...]" "--all --self-test" "$@"
cd "$(dirname "$0")/../.."

MIGRATIONS=app/ferroehr/migrations

# Files whose historical citations are deliberate, each with its reason.
#
# `db/mod.rs` IS the boot refusal for a first-generation database: it names that
# generation's opening files because sqlx records the file name as the recorded
# description, so the names are the signature it matches on.
#
# `ext_functions.rs` cites the first generation's helper file at the `v4.3.0`
# tag, where it still exists; the same test asserts its embedded copy equals
# `git show v4.3.0:…` byte for byte, so the citation is machine-checked.
EXEMPT='app/ferroehr/src/db/mod.rs
app/ferroehr/tests/it/ext_functions.rs'

exempted() {
  local file=$1 entry
  while IFS= read -r entry; do
    [[ -n "$entry" ]] || continue
    [[ "$entry" = "$file" ]] && return 0
  done < <(printf '%s\n' "$EXEMPT")
  return 1
}

collect() {
  if [[ "${1:-}" = "--all" ]]; then
    git ls-files -- 'app/**/*.rs'
  elif [[ "$#" -gt 0 ]]; then
    printf '%s\n' "$@"
  else
    git diff --name-only origin/main...HEAD -- 'app/**/*.rs' 2>/dev/null ||
      git ls-files -- 'app/**/*.rs'
  fi
}

failures=0
files=$(collect "$@")
[[ -n "$files" ]] || {
  echo "migration-citations: no files in scope to check."
  exit 0
}

for f in $files; do
  [[ -f "$f" ]] || continue
  case $f in
  *.rs) ;;
  *) continue ;;
  esac
  exempted "$f" && continue

  while IFS=: read -r line cited; do
    [[ -n "${cited:-}" ]] || continue
    # The citation may be written relative to the citing file, so only its last
    # two components carry information: the schema directory and the file. A
    # bare file name matches on the file alone; a wrong schema directory still
    # fails.
    suffix=$(printf '%s\n' "$cited" | awk -F/ '{ if (NF >= 2) print $(NF-1) "/" $NF; else print $NF }')
    if ! find "$MIGRATIONS" -type f -path "*/$suffix" -print -quit | grep -q .; then
      printf '%s\n' "$f:$line: cites the migration \`$cited\`, which no file under \
$MIGRATIONS matches — name the file that defines the relation now \
(scripts/checks/migration-citations.sh)" >&2
      failures=$((failures + 1))
    fi
  done < <(grep -noE '([A-Za-z0-9_.-]+/)*[0-9]{4}_[A-Za-z0-9_]+\.sql' "$f" || true)
done

if [[ "$failures" -gt 0 ]]; then
  echo "migration-citations: $failures stale citation(s) — see above." >&2
  exit 1
fi
echo "migration-citations: OK."
