#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# RFC 0201: an error carries its cause (#2034).
#
# `Error::source` is part of the `Error` contract, and `map_err(|e| V(e.to_string()))`
# destroys it at exactly the seam where it mattered. A stringified cause cannot
# be matched on, walked, or classified — the transport layer maps an error to a
# status code BY TYPE, so a `PoolTimedOut` that arrived as a string cannot become
# a 503 rather than a 500 without parsing prose.
#
# No clippy lint covers this (checked against the pinned 1.96.1 lint set), so
# the check is grep-shaped, and it is honest about that: it catches the common
# spelling, not every possible one. Its value is stopping the count from growing
# while the sweep runs.
#
# THE ALLOWLIST IS THE HONEST PART. Three shapes legitimately flatten:
#
#   1. a published `openehr-*` crate must not leak a private dependency's error
#      type into its own public API — that would make a patch bump of that
#      dependency a breaking change for us (RFC 1105);
#   2. a cause that is genuinely a MESSAGE rather than an error (a parser's
#      positional diagnostic that already IS the human-readable answer);
#   3. test code.
#
# An entry added without one of those reasons makes this gate decoration.
#
# The second check is the converse defect: a cause carried as the source AND
# interpolated into the variant's own message (`#[error("x: {0}")]` over a
# `#[from]`/`#[source]` field, or `{source}` in a struct variant). A chain
# printer then prints it twice; RFC 0201 keeps the cause out of the message, or
# the variant is `#[error(transparent)]`. Every tree is clean, so that check is
# a hard zero rather than a ratchet.
#
# Usage: scripts/checks/error-source-chain.sh [--self-test]
#
# The whole tree is always counted. A plain run proves the double-print
# detector first (`--self-test` runs only that), so CI exercises it too.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.." || exit 1

# Prints `file:line: attr` for every `#[error(...)]` whose format string
# interpolates a field that is also the source: `#[from]`, `#[source]`, or a
# struct field named `source`. Line-shaped like every guard here: it reads the
# attribute and the variant after it, skipping comment lines in the variant.
read -r -d '' DOUBLE_PRINT_AWK <<'AWK' || true
function reset_state() { in_attr = 0; pending = 0; collecting = 0; attr = ""; body = ""; kind = "" }
function balance(s,    i, c, n) {
  n = 0
  for (i = 1; i <= length(s); i++) {
    c = substr(s, i, 1)
    if (c == "(" || c == "[" || c == "{" || c == "<") n++
    else if (c == ")" || c == "]" || c == "}" || c == ">") n--
  }
  return n
}
function refs(name,    s, p, prev, nxt) {
  if (index(attr, "{" name "}") || index(attr, "{" name ":")) return 1
  s = attr
  while ((p = index(s, "." name)) > 0) {
    prev = (p > 1) ? substr(s, p - 1, 1) : " "
    nxt = substr(s, p + 1 + length(name), 1)
    if (prev !~ /[A-Za-z0-9_]/ && nxt !~ /[A-Za-z0-9_]/) return 1
    s = substr(s, p + 1)
  }
  return 0
}
function field_name(f,    t) {
  t = f
  gsub(/#\[[^]]*\]/, "", t)
  gsub(/[\n\t ]+/, " ", t)
  sub(/^ /, "", t)
  sub(/^pub(\([a-z]+\))? /, "", t)
  if (index(t, ":") == 0) return ""
  t = substr(t, 1, index(t, ":") - 1)
  gsub(/ /, "", t)
  return t
}
function judge(f, idx,    is_src, name) {
  is_src = (f ~ /#\[(from|source)\]/)
  name = (kind == "tuple") ? idx : field_name(f)
  if (kind == "struct" && name == "source") is_src = 1
  if (is_src && name != "" && refs(name)) {
    printf "%s:%d: %s\n", FILENAME, attr_line, attr
    return 1
  }
  return 0
}
function finish(    i, c, depth, field, idx) {
  collecting = 0
  depth = 0; field = ""; idx = 0
  for (i = 1; i <= length(body); i++) {
    c = substr(body, i, 1)
    if (c == "(" || c == "[" || c == "{" || c == "<") { depth++; if (depth == 1) continue }
    else if (c == ")" || c == "]" || c == "}" || c == ">") {
      depth--
      if (depth == 0) { if (judge(field, idx)) return; break }
    } else if (c == "," && depth == 1) {
      if (judge(field, idx)) return
      field = ""; idx++
      continue
    }
    field = field c
  }
}
FNR == 1 { reset_state() }
{
  line = $0
  if (collecting) {
    if (line ~ /^[ \t]*\/\//) next
    body = body "\n" line
    depth_total += balance(line)
    if (depth_total <= 0) finish()
    next
  }
  if (in_attr) {
    attr = attr " " line
    if (line ~ /\)\][ \t]*$/) { in_attr = 0; pending = 1 }
    next
  }
  if (line ~ /^[ \t]*#\[error\(/) {
    attr = line; attr_line = FNR
    gsub(/^[ \t]+/, "", attr)
    if (line ~ /\)\][ \t]*$/) pending = 1; else in_attr = 1
    next
  }
  if (pending) {
    if (line ~ /^[ \t]*(#\[|\/\/)/) next
    pending = 0
    if (attr ~ /#\[error\([ \t]*transparent/) next
    if (line ~ /^[ \t]*(pub(\([a-z]+\))?[ \t]+)?(struct[ \t]+)?[A-Z][A-Za-z0-9_]*[ \t]*(<[^>]*>)?[ \t]*\(/) {
      kind = "tuple"; body = substr(line, index(line, "("))
    } else if (line ~ /^[ \t]*(pub(\([a-z]+\))?[ \t]+)?(struct[ \t]+)?[A-Z][A-Za-z0-9_]*[ \t]*(<[^>]*>)?[ \t]*\{/) {
      kind = "struct"; body = substr(line, index(line, "{"))
    } else next
    depth_total = balance(body)
    if (depth_total <= 0) finish(); else collecting = 1
  }
}
AWK

double_prints() {
  find "$@" -name '*.rs' -not -path '*/target/*' -print0 \
    | xargs -0 awk "$DOUBLE_PRINT_AWK"
}

# Proves the detector rather than trusting it: every banned spelling is caught,
# and every legitimate one is not.
self_test() {
  local tmp found failures=0 expected
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" RETURN
  cat >"$tmp/bad.rs" <<'RS'
enum Bad {
    #[error("tuple from: {0}")]
    TupleFrom(#[from] std::io::Error),
    #[error("tuple source: {0:?}")]
    TupleSource(#[source] std::io::Error),
    #[error("dot arg: {}", .0)]
    DotArg(#[from] std::fmt::Error),
    #[error("named source: {source}")]
    NamedSource { source: std::io::Error },
    #[error(
        "multi-line attr naming {cause}"
    )]
    AttributedField {
        /// A doc line, with commas, (parens) and [brackets].
        path: String,
        #[source]
        cause: std::io::Error,
    },
}
RS
  cat >"$tmp/good.rs" <<'RS'
enum Good {
    #[error(transparent)]
    Forwarded(#[from] std::io::Error),
    #[error("context: {0}")]
    ContextThenSource(String, #[source] std::io::Error),
    #[error("reading {path}")]
    Struct {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{} items", .0.len())]
    Plain(Vec<u8>),
}
RS
  found="$(double_prints "$tmp")"
  for expected in 'tuple from' 'tuple source' 'dot arg' 'named source' 'multi-line attr'; do
    grep -q "$expected" <<<"$found" || {
      echo "self-test: \"$expected\" was not caught" >&2
      failures=1
    }
  done
  if grep -q 'good.rs' <<<"$found"; then
    echo "self-test: a legitimate shape was wrongly caught:" >&2
    grep 'good.rs' <<<"$found" >&2
    failures=1
  fi
  [[ "$failures" -eq 0 ]] || return 1
  echo "error-source-chain: self-test OK (five double prints caught, four clean shapes allowed)."
}

case "${1:-}" in
--self-test)
  self_test
  exit
  ;;
"")
  self_test
  ;;
*)
  guard_usage "[--self-test]"
  ;;
esac

# The sweep (#2034) is CLOSED: every remaining site below was judged and carries
# its `// NOTE:` reason at the site or on its error type. Each budget is a COUNT
# that may only go DOWN — a new flattened site fails the gate, and lowering a
# budget needs the same judgement the sweep applied.
#
# What survives, and why each one is structural rather than unfinished:
#
#   app/ferroehr           `prometheus::Error` has no source-bearing variant.
#   app/ferroehr-viewer  `ViewerError` crosses the server-fn boundary, so
#                          `FromServerFnError` requires Serialize/Deserialize;
#                          `LiftError` is held in an `RwSignal`, so it must be
#                          Clone + Eq. No underlying error is any of those.
#
# A line that ALSO calls `.with_source(e)` is not flattened — it carries the
# cause and stringifies only for the message — so the count excludes it.
#
# `tools/*` is NOT swept, and that is an adjudication rather than an omission —
# the tree falls under the shapes #2034 itself names as legitimate:
#   tools/openehr-codegen   every site is in testsupport.rs — test code, which
#                           #2034 exempts outright.
#
# Both stay out of the budget list so the gate measures the request path, where
# a status code is chosen BY TYPE and a flattened cause actually costs something.
declare -a BUDGETS=(
  "app/ferroehr:1"
  "app/ferroehr-ext:0"
  "app/ferroehr-viewer:3"
)

fail=0
for entry in "${BUDGETS[@]}"; do
  tree="${entry%%:*}"
  budget="${entry##*:}"
  count="$(grep -rn 'map_err(|e| .*to_string())' "$tree" 2>/dev/null \
           | grep -v '/tests/' | grep -vc 'with_source' || true)"
  count="${count:-0}"
  printf '%-26s %3s flattened (budget %s)\n' "$tree" "$count" "$budget"
  if [[ "$count" -gt "$budget" ]]; then
    echo "::error::$tree grew from $budget to $count flattened error sites — an error carries its cause (RFC 0201, #2034)"
    fail=1
  fi
done

# No error variant anywhere prints its own source.
found="$(double_prints app crates tools)"
if [[ -n "$found" ]]; then
  echo "::error::error variants print their source in their own message — keep the cause out of the message, or use #[error(transparent)] (RFC 0201)"
  printf '%s\n' "$found" >&2
  fail=1
else
  echo "double prints: none"
fi

[[ "$fail" -eq 0 ]] || exit 1
echo "error-source-chain: no tree grew"
