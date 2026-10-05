#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Every first-party shell, SQL and YAML file states its licensing INSIDE itself.
#
# The sibling gate `spdx-headers.sh` does this for Rust. This one exists rather
# than extending it because the two populations differ in three ways at once:
# the comment syntax (`#` and `--`, not `//`), the placement (a shebang must
# stay on line 1), and the licensing (these files are plain BUSL-1.1 — none carries
# openEHR specification text, so the dual-licence rule the Rust gate applies to
# the published spec crates has nothing to act on here). One gate with a mode
# matrix would be harder to read than two that each do one thing.
#
# WHY HEADERS AT ALL, when `REUSE.toml` already covers the tree by glob: a glob
# declaration does not travel with a copied file. REUSE 3.3 exists for exactly
# that property. The sharpest case here is `app/ferroehr/migrations/*.sql` — a
# migration lifted into another project is precisely the kind of file that gets
# copied, and it would arrive with no licence statement at all.
#
# SCOPE: tracked `.sh`, `.sql`, `.yml`. Vendored trees and third-party corpora
# are excluded, not exempted — a header there would assert something about
# somebody else's file. `.yaml` is deliberately NOT covered: that population is
# the CNF catalogue (data, thousands of files), and the Helm templates, where a
# `#` comment renders into `helm template` output and would change what users
# see.
#
# Usage:
#   scripts/checks/spdx-headers-text.sh          # verify
#   scripts/checks/spdx-headers-text.sh --fix    # insert missing headers
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--fix]" "--fix" "$@"

cd "$(dirname "$0")/../.."

mode="${1:-}"
case "$mode" in
'' | --fix) ;;
*)
  echo "usage: $0 [--fix]" >&2
  exit 2
  ;;
esac

# The tags below are DATA this gate compares against, not this file's own
# licensing, and `reuse lint` reads a tag wherever it appears — the
# specification's own remedy for a file that quotes the syntax it checks.
# REUSE-IgnoreStart
readonly COPYRIGHT='SPDX-FileCopyrightText: Cadasto B.V.'
readonly LICENSE='SPDX-License-Identifier: BUSL-1.1'
# REUSE-IgnoreEnd

# Vendored and third-party trees: their licensing is recorded in each tree's
# PROVENANCE.md and must not be overwritten by an assertion of ours. Kept in
# step with .fossa.yml and scripts/checks/first-party-license-text.sh.
readonly EXCLUDED='^(docs/specs|crates/[^/]+/vendor|crates/[^/]+/tests/vendor|crates/openehr-adl/tests/corpus|crates/openehr-its/tests/fixtures|crates/openehr-term/tests|crates/openehr-term/assets|crates/openehr-its/schemas|tools/openehr-codegen/vendor|corpus|LICENSES)/'

# The comment leader for a path's file type.
comment_prefix() {
  case "$1" in
  *.sql) printf -- '--' ;;
  *) printf '#' ;;
  esac
}

fail=0
fixed=0
checked=0

while IFS= read -r f; do
  [[ -f "$f" ]] || continue
  checked=$((checked + 1))
  prefix=$(comment_prefix "$f")

  # Read a window, not line 1: a shebang legitimately precedes the header.
  if head -n 8 "$f" | grep -qF "$LICENSE"; then
    continue
  fi

  if [[ "$mode" != --fix ]]; then
    echo "$f: no $LICENSE in the first 8 lines" >&2
    fail=1
    continue
  fi

  # A shebang must stay on line 1, or the file stops being executable by the
  # interpreter it names.
  if head -n 1 "$f" | grep -q '^#!'; then
    {
      head -n 1 "$f"
      printf '%s %s\n%s %s\n' "$prefix" "$COPYRIGHT" "$prefix" "$LICENSE"
      tail -n +2 "$f"
    } >"$f.spdx.tmp"
  else
    {
      printf '%s %s\n%s %s\n\n' "$prefix" "$COPYRIGHT" "$prefix" "$LICENSE"
      cat "$f"
    } >"$f.spdx.tmp"
  fi
  # Preserve the executable bit: a stamped script that stops being runnable is
  # a worse defect than a missing header.
  chmod --reference="$f" "$f.spdx.tmp" 2>/dev/null || chmod "$(stat -f '%A' "$f")" "$f.spdx.tmp"
  mv "$f.spdx.tmp" "$f"
  fixed=$((fixed + 1))
done < <(git ls-files '*.sh' '*.sql' '*.yml' | grep -vE "$EXCLUDED")

if [[ "$mode" = --fix ]]; then
  echo "spdx-headers-text: $fixed header(s) inserted across $checked file(s)."
  exit 0
fi

if [[ "$fail" -ne 0 ]]; then
  echo "spdx-headers-text: files without a licence header (run --fix)." >&2
  exit 1
fi
echo "spdx-headers-text: OK ($checked files)."
