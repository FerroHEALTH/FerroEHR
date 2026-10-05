#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# String-built-SQL guard for every layer that builds SQL at runtime.
#
# The AQL engine builds SQL dynamically from attacker-controlled query text, so
# the one property that keeps it safe is that the SQL SHAPE comes from our own
# code and every user-supplied value arrives as a bound parameter. The OWASP SQL
# Injection Prevention Cheat Sheet names the failure mode this guard blocks:
# "Validated data is not necessarily safe to insert into SQL queries via string
# building"
# (https://cheatsheetseries.owasp.org/cheatsheets/SQL_Injection_Prevention_Cheat_Sheet.html).
#
# Four shapes fail, over the three directories that build SQL at runtime —
# `app/ferroehr/src/aql/`, `app/ferroehr/src/storage/` and
# `app/ferroehr/src/system_log/` (the ITI-81 audit retrieval, the other
# `sea-query` site):
#   1. `format!`/`write!`/`writeln!` whose literal holds a SQL keyword AND an
#      interpolation — an interpolated clause is string-built SQL by definition.
#   2. `push_str("…<SQL keyword>…")` — assembling a statement fragment by
#      fragment onto a String, the shape step 1 usually grows into.
#   3. `push_str(<runtime value>)` — a value appended to a string being built.
#      Whether that string is SQL cannot be decided by grep, so the rule is
#      strict and legitimate sites are exempted by name below.
#   4. `AssertSqlSafe(format!(…))` — the sqlx injection guard waived over an
#      interpolated string, which is the assertion being false.
#
# What this guard CANNOT see, so that nobody mistakes a green run for the whole
# property: an identifier passed to `sea_query::Alias::new` is indistinguishable
# from our own generated alias at the grep level. That half is pinned by tests
# instead — `app/ferroehr/tests/it/sql_injection.rs` asserts that every quoted
# identifier in the emitted SQL comes from the closed set, over a corpus of
# queries carrying hostile text in every user-supplied position.
#
# Usage: scripts/checks/sql-string-building.sh [--all | --self-test | <file>...]
#   no args     → the changed files that fall inside the scanned directories
#   --all       → every tracked .rs file in those directories
#   --self-test → runs the guard over a fixture the guard flags, once with a
#   space-carrying exemption naming it and once with no exemption at all, and
#   asserts the first is accepted and the second rejected. The exemption list is
#   read line by line, and an iteration never shown to honour a needle with a
#   space is a green light nobody has tested (reliability.md §enforcement tiers).
set -euo pipefail

if [[ "${1:-}" == "--self-test" ]]; then
  cd "$(dirname "$0")/../.."
  self_root="$(mktemp -d)"
  trap 'rm -rf "$self_root"' EXIT
  mkdir -p "$self_root/scripts" "$self_root/app/ferroehr/src/storage"
  cp -R scripts/checks scripts/lib "$self_root/scripts/"
  self_file='app/ferroehr/src/storage/self_test.rs'
  # Rule 3 flags this line: a runtime value appended to a string under
  # construction. Its exemption needle carries two spaces.
  printf '    out.push_str(&render(a, b, c));\n' > "$self_root/$self_file"
  self_guard="$self_root/scripts/checks/sql-string-building.sh"
  self_fail=0
  # <label>|<exemption list>|<expected exit: 0 accept, 1 reject>
  self_cases=(
    "a needle carrying spaces|${self_file}:out.push_str(&render(a, b, c))|0"
    "the same line with no exemption||1"
  )
  for self_case in "${self_cases[@]}"; do
    label="${self_case%%|*}"
    rest="${self_case#*|}"
    exempt="${rest%%|*}"
    want="${rest#*|}"
    awk -v repl="EXEMPT='${exempt}'" '
      /^EXEMPT=/ { print repl; skip = 1; next }
      skip && /'"'"'$/ { skip = 0; next }
      !skip { print }
    ' "$self_guard" > "$self_guard.case"
    got=0
    (cd "$self_root" && bash scripts/checks/sql-string-building.sh.case "$self_file" >/dev/null 2>&1) || got=1
    if [[ "$got" != "$want" ]]; then
      echo "::error::--self-test: ${label} — expected exit ${want}, got ${got}." >&2
      self_fail=1
    else
      echo "  self-test: ${label} → exit ${got} as expected"
    fi
  done
  [[ "$self_fail" -eq 0 ]] || exit 1
  echo "sql-string-building --self-test: the exemption list honours a needle with spaces — OK."
  exit 0
fi

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--all | --self-test | <file>...]" "--all --self-test" "$@"
cd "$(dirname "$0")/../.."

# The directories whose SQL is built at runtime. Everything else in the tree
# uses `sqlx::query!`/`query_as!` over static SQL, which the compiler checks.
SCOPE='app/ferroehr/src/aql/ app/ferroehr/src/storage/ app/ferroehr/src/system_log/'

# Clause-introducing keywords. A literal holding one of these is a statement
# fragment, not prose.
KEYWORDS='SELECT|FROM|WHERE|ORDER[[:space:]]+BY|GROUP[[:space:]]+BY|HAVING|JOIN|INSERT[[:space:]]+INTO|UPDATE[[:space:]]|DELETE[[:space:]]+FROM|UNION|LIMIT|OFFSET'

# Narrow, named exemptions, one per line. Each is a `file:needle` pair: the
# needle must appear on the flagged line for the exemption to apply, so it cannot
# silently widen to a neighbouring statement, and it may carry spaces.
#
# `node_repo.rs` composes the promoted-column list of one INSERT header from
# `storage::promoted::PROMOTED_LEAVES`, whose `column` field is a variant of the
# schema catalog `crate::db::iden::Node` — a closed set of identifiers in our own
# source, with no request-time input, and the values themselves are bound through
# `QueryBuilder::push_bind`.
#
# `ddl_template.rs` renders MIGRATION FILES, not statements a server executes:
# its inputs are the two `Domain` constants in the same file and a committed
# template, and its only consumer is a test that compares the rendering with the
# committed migrations. Nothing here reaches a connection.
EXEMPT='app/ferroehr/src/storage/node_repo.rs:push_str(sea_query::Iden::unquoted(&leaf.column))
app/ferroehr/src/storage/ddl_template.rs:out.push_str(&line.replace(KINDS, &kinds))'

collect() {
  if [[ "${1:-}" = "--all" ]]; then
    # shellcheck disable=SC2086 # SCOPE is a deliberate list of pathspecs
    git ls-files -- $SCOPE
  elif [[ "$#" -gt 0 ]]; then
    printf '%s\n' "$@"
  else
    # shellcheck disable=SC2086
    git diff --name-only origin/main...HEAD -- $SCOPE 2>/dev/null ||
      git ls-files -- $SCOPE
  fi
}

failures=0
report() {
  printf '%s\n' "$1" >&2
  failures=$((failures + 1))
}

# Whether `$1:$2` (file, line body) is one of the named exemptions.
# The list is read line by line: a needle is arbitrary source text and may hold
# spaces, which word splitting would break into entries that never match.
exempted() {
  local file=$1 body=$2 entry needle
  while IFS= read -r entry; do
    [[ -n "$entry" ]] || continue
    [[ "${entry%%:*}" = "$file" ]] || continue
    needle=${entry#*:}
    case $body in
    *"$needle"*) return 0 ;;
    *) ;;
    esac
  done < <(printf '%s\n' "$EXEMPT")
  return 1
}

# The double-quoted substrings of a line, one per output line. Rust string
# literals in the scanned code carry no escaped double quotes, so the naive
# split is exact here.
quoted() { printf '%s\n' "$1" | grep -o '"[^"]*"' || true; }

files=$(collect "$@")
[[ -n "$files" ]] || {
  echo "sql-string-building: no files in scope to check."
  exit 0
}

for f in $files; do
  [[ -f "$f" ]] || continue
  case $f in
  *.rs) ;;
  *) continue ;;
  esac

  while IFS=: read -r line body; do
    [[ -n "${line:-}" ]] || continue
    exempted "$f" "$body" && continue

    # (1) an interpolated literal that carries a clause keyword
    if printf '%s' "$body" | grep -qE '(format!|write!|writeln!)[[:space:]]*\('; then
      while IFS= read -r lit; do
        [[ -n "$lit" ]] || continue
        printf '%s' "$lit" | grep -qE "$KEYWORDS" || continue
        printf '%s' "$lit" | grep -q '{' || continue
        report "$f:$line: interpolated SQL literal $lit — build the clause with \
\`sea-query\`'s typed API and bind every value (scripts/checks/sql-string-building.sh)"
      done < <(quoted "$body")
    fi

    # (2) a clause keyword pushed onto a String
    if printf '%s' "$body" | grep -qE 'push_str[[:space:]]*\([[:space:]]*"'; then
      while IFS= read -r lit; do
        [[ -n "$lit" ]] || continue
        printf '%s' "$lit" | grep -qE "$KEYWORDS" || continue
        report "$f:$line: SQL fragment $lit pushed onto a String — assemble the \
statement with \`sea-query\` instead (scripts/checks/sql-string-building.sh)"
      done < <(quoted "$body")
    fi

    # (3) a runtime value appended to a string under construction. Each
    # `push_str(` on the line is judged separately, so a line that also pushes a
    # literal cannot shield a value push.
    while IFS= read -r push; do
      [[ -n "$push" ]] || continue
      printf '%s' "$push" | grep -qE 'push_str[[:space:]]*\([[:space:]]*r?#*"' && continue
      report "$f:$line: a runtime value is appended to a string ($push) — if that \
string is SQL, bind the value; if it is not, add a named exemption with its \
reason (scripts/checks/sql-string-building.sh)"
    done < <(printf '%s\n' "$body" | grep -oE 'push_str[[:space:]]*\([^)]*' || true)

    # (4) the sqlx injection guard waived over an interpolated string
    if printf '%s' "$body" | grep -qE 'AssertSqlSafe[[:space:]]*\([[:space:]]*format!'; then
      report "$f:$line: \`AssertSqlSafe\` over a \`format!\` — the assertion is \
false for an interpolated statement (scripts/checks/sql-string-building.sh)"
    fi
  done < <(grep -nE 'format!|write!|writeln!|push_str|AssertSqlSafe' "$f" || true)
done

if [[ "$failures" -gt 0 ]]; then
  echo "sql-string-building: $failures violation(s) — see above." >&2
  exit 1
fi
echo "sql-string-building: OK."
