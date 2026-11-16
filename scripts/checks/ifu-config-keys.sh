#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The EHDS system documents a release attaches name only configuration keys
# this FerroEHR reads (#5534).
#
# EHDS Art. 30(1)(d) has each EHR system accompanied by "clear and complete
# instructions for use", and Art. 28(b) prohibits instructions that mislead
# the professional user. The pages live in the FerroHEALTH book and a release
# takes them at the commit docs/ehds/system-documents.env pins, so a key this
# release renames or removes can stay on the page an operator follows, and
# the server it describes refuses to start. This guard reads the FerroEHR
# parts of a page and refuses every key reference that does not resolve in
# app/ferroehr/assets/ferroehr.default.toml (scripts/lib/config-keys.sh).
#
# What it reads, in the sections whose heading names FerroEHR (a `##` or
# `###` heading containing "FerroEHR" opens one, the next such heading closes
# it, so FerroBRIDGE's settings are never read against FerroEHR's keys):
#   - a ```toml block: each `[table]` and `key = value` line;
#   - a code span `[table]`, `[table] key` or `[table] key = value`;
#   - a code span `FERROEHR__A__B`, the environment form of `a.b`;
#   - a code span `a.b.c` whose first segment is a top-level table of the
#     reference, so a JSON field such as `support.status` is not read as a key.
# A paragraph or list item that says "unknown key" names keys the server
# refuses on purpose, and each of its references must then NOT resolve.
# Bare key names in prose (`retention_days`) are not read: a bare word has no
# table, and the same word names values and fields elsewhere on the page.
#
# No specification governs this guard: our own design.
#
# Usage: scripts/checks/ifu-config-keys.sh <page.md>...
#        scripts/checks/ifu-config-keys.sh --self-test
# Exit codes: 0 every reference resolves, 1 a finding, 2 a usage error.
set -euo pipefail
# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
# shellcheck source=scripts/lib/config-keys.sh
. "$(dirname "$0")/../lib/config-keys.sh"
guard_known_flags "<page.md>... | --self-test" "--self-test" "$@"
ROOT="$(cd "$(dirname "$0")/../.." && pwd -P)"
readonly ROOT
readonly TOML="$ROOT/app/ferroehr/assets/ferroehr.default.toml"
WORK="$(mktemp -d)"
readonly WORK
trap 'rm -rf "$WORK"' EXIT

# references PAGE TOPS: one line per key reference in the FerroEHR sections
# of PAGE, `<want>\t<path>\t<line>`, where want is `known` or `refused` and
# TOPS is the space-separated top-level tables of the reference.
references() {
  local page="$1" tops="$2"
  awk -v tops=" $tops " '
    function flush(   i, n, span, rest, head, key, path, want) {
      if (block == "") return
      want = (tolower(block) ~ /unknown key/) ? "refused" : "known"
      rest = block
      while (match(rest, /`[^`]+`/)) {
        span = substr(rest, RSTART + 1, RLENGTH - 2)
        rest = substr(rest, RSTART + RLENGTH)
        path = ""
        if (span ~ /^\[[a-z0-9_.]+\]( [a-z0-9_]+)?( = .*)?$/) {
          head = span; sub(/\].*$/, "", head); sub(/^\[/, "", head)
          key = span; sub(/^\[[^]]*\] ?/, "", key); sub(/ = .*$/, "", key)
          path = (key == "" ? head : head "." key)
        } else if (span ~ /^FERROEHR__[A-Z0-9_]+$/) {
          path = tolower(substr(span, 11)); gsub(/__/, ".", path)
        } else if (span ~ /^[a-z_][a-z0-9_]*(\.[a-z0-9_]+)+$/) {
          head = span; sub(/\..*$/, "", head)
          if (index(tops, " " head " ") > 0) path = span
        }
        if (path != "") print want "\t" path "\t" start
      }
      block = ""
    }
    /^```/ {
      flush()
      if (fence) { fence = 0; toml = 0; next }
      fence = 1; toml = ($0 ~ /^```toml/); sec = ""; next
    }
    fence {
      if (!scope || !toml) next
      line = $0; sub(/#.*$/, "", line); gsub(/^[[:space:]]+|[[:space:]]+$/, "", line)
      if (line ~ /^\[\[?[a-z0-9_.]+\]\]?$/) {
        sec = line; gsub(/[][]/, "", sec); print "known\t" sec "\t" NR; next
      }
      if (line ~ /^[a-z0-9_]+[[:space:]]*=/) {
        k = line; sub(/[[:space:]]*=.*$/, "", k)
        print "known\t" (sec == "" ? k : sec "." k) "\t" NR
      }
      next
    }
    /^##+ / { flush(); scope = ($0 ~ /^###? / && $0 ~ /FerroEHR/) ? 1 : (($0 ~ /^###? /) ? 0 : scope); next }
    /^[[:space:]]*$/ { flush(); next }
    /^([-*]|[0-9]+\.|\|) / { flush() }
    { if (!scope) next; if (block == "") start = NR; block = block " " $0 }
    END { flush() }
  ' "$page"
}

# check PAGE...: every reference of every PAGE against the reference file.
check() {
  local work="$WORK/check" page tops findings=0 want path line
  mkdir -p "$work"
  config_keys "$TOML" >"$work/keys"
  # A key under an operator-chosen table name matches any one name.
  { echo '$^'; grep -F '*' "$work/keys" | sed -e 's/\./\\./g' -e 's/\*/[a-z0-9_]+/g' | sed 's/.*/^&$/'; } >"$work/named"
  tops="$(sed 's/\..*$//' "$work/keys" | sort -u | tr '\n' ' ')"
  for page in "$@"; do
    [[ -f "$page" ]] || { echo "ifu-config-keys: no page $page" >&2; exit 2; }
    while IFS=$'\t' read -r want path line; do
      if grep -qxF "$path" "$work/keys" || grep -qEf "$work/named" <<<"$path"; then
        [[ "$want" == known ]] && continue
        echo "$page:$line: names \`$path\` as an unknown key, and this FerroEHR reads it" >&2
      else
        [[ "$want" == refused ]] && continue
        echo "$page:$line: names \`$path\`, which this FerroEHR refuses (not a key of app/ferroehr/assets/ferroehr.default.toml)" >&2
      fi
      findings=$((findings + 1))
    done < <(references "$page" "$tops" | sort -u -t $'\t' -k2,2 -k3,3n)
  done
  if [[ "$findings" -gt 0 ]]; then
    echo "ifu-config-keys: $findings reference(s) the configuration of this release does not hold; correct the page in the FerroHEALTH book and move docs/ehds/system-documents.env to it" >&2
    return 1
  fi
}

self_test() {
  local tmp="$WORK/self-test"
  mkdir -p "$tmp"
  cat >"$tmp/good.md" <<'EOF'
# Instructions

Revised 2026-01-01.

### FerroEHR settings

```toml
[deployment]
profile = "production"   # a comment

[audit.store]
verify_interval = "1h"
```

- Set `[authz.rbac] first_admin = "x"` and read `support.status`, which is
  no key. `[audit]` alone is a table, and `FERROEHR__AUDIT__ENABLED` its key.
- A configuration that carries `[fhir]` is refused at boot as unknown keys.

### FerroBRIDGE settings

- `[cdr.credentials]` is FerroBRIDGE's, and `[auth] enabled` too.

```toml
[cdr]
url = "https://ehr.example.org"
```

## Maintenance

### FerroEHR maintenance

| Act | `[audit.store] verify_interval` |
EOF
  check "$tmp/good.md" || { echo "ifu-config-keys: self-test: a page naming only known keys was refused" >&2; exit 1; }
  local bad
  for bad in \
    'deployment_profile = "production"' \
    '[audit.store]\nretention_years = 5' \
    '[audit.syslog]\nhost = "arr.example.org"'; do
    # shellcheck disable=SC2016 # the backticks are Markdown, not an expansion
    printf '### FerroEHR settings\n\n```toml\n%b\n```\n' "$bad" >"$tmp/bad.md"
    if check "$tmp/bad.md" 2>/dev/null; then
      echo "ifu-config-keys: self-test: a refused key in a toml block passed: $bad" >&2
      exit 1
    fi
  done
  # shellcheck disable=SC2016 # the backticks are Markdown, not an expansion
  for bad in '`[audit.categories]`' '`[audit.store] verify_interval_seconds = 86400`' \
    '`authz.professional.claim`' '`FERROEHR__AUDIT__NO_SUCH_KEY`' \
    'a still accepted `[audit] enabled` is refused as an unknown key'; do
    printf '### FerroEHR: security\n\n- Declare %s.\n' "$bad" >"$tmp/bad.md"
    if check "$tmp/bad.md" 2>/dev/null; then
      echo "ifu-config-keys: self-test: a refused reference passed: $bad" >&2
      exit 1
    fi
  done
  echo "ifu-config-keys: self-test OK (toml blocks, table and key spans, environment forms, dotted keys, FerroBRIDGE sections skipped, unknown keys held refused)."
}

case "${1:-}" in
  --self-test)
    shift
    guard_no_args "$@"
    self_test
    ;;
  *)
    [[ "$#" -ge 1 ]] || guard_usage "<page.md>... | --self-test"
    check "$@"
    echo "ifu-config-keys: OK ($# page(s); every FerroEHR key reference resolves)."
    ;;
esac
