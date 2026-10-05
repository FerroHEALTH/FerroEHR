#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# One copyright holder, stated identically everywhere it is stated.
#
# `LICENSE` once said one holder while `REUSE.toml`, the codegen header constant
# and 2470 file headers said "FerroEHR contributors" (#2325). The Business
# Source License names Cadasto B.V. as the Licensor and copyright
# holder (#3435, #3567), so every source states that. Asserting two holders means a
# downstream redistributor reading a file
# header and a lawyer reading LICENSE come away with different answers about who
# holds the copyright — the exact ambiguity per-file licensing exists to remove.
#
# The divergence survived because nothing compared the sources. This compares
# seven: the three that state the holder in a machine-readable field (`LICENSE`,
# `REUSE.toml`, the emitter constant) and the four that state it in prose a
# reader is handed — the published licensing page, which states it twice, and
# the `README.md` of each hand-written engine crate, which ships inside the
# published package (#3438).
#
# NOT compared: `CITATION.cff` authors and `.zenodo.json` creators. Those record
# AUTHORSHIP for citation, which is a different datum from the copyright holder
# and is correctly a named person. Nor `LICENSES/MIT.txt`, which reproduces the
# grant the MIT-published versions carry and keeps the holder they were
# published under.
#
# Usage: scripts/checks/copyright-holder.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"

cd "$(dirname "$0")/../.."

readonly HOLDER='Cadasto B.V.'

fail=0
check() {
  local what="$1" found="$2"
  if [[ "$found" != "$HOLDER" ]]; then
    echo "error: $what states the copyright holder as:" >&2
    echo "         ${found:-<not found>}" >&2
    echo "       every source must state: $HOLDER" >&2
    fail=1
  fi
}

# `LICENSE` — the Business Source License's "The Licensed Work is (c) YYYY …"
# line, taken to end of line: a holder whose own name ends in a period closes
# the sentence with it rather than doubling it.
check "LICENSE" \
  "$(sed -n 's/^.*The Licensed Work is (c) [0-9]\{4\} \(.*\)$/\1/p' LICENSE | head -1)"

# `REUSE.toml` — the first-party annotation (the openEHR Foundation appears as a
# SECOND holder on vendored-derived files, which is a different statement).
check "REUSE.toml" \
  "$(sed -n 's/^SPDX-FileCopyrightText = "\(.*\)"$/\1/p' REUSE.toml | head -1)"

# The emitter constant every generated header is stamped from.
check "tools/openehr-codegen/src/render/spdx.rs" \
  "$(sed -n 's/^pub(crate) const PROJECT_COPYRIGHT: &str = "\(.*\)";$/\1/p' \
       tools/openehr-codegen/src/render/spdx.rs | head -1)"

# The prose sources: a file states the holder by name, a fixed number of times.
# Prose carries no field to parse, so the assertion is the COUNT — rewrite one
# of the licensing page's two statements and the count drops to one.
check_count() {
  local what="$1" file="$2" want="$3" got
  if [[ ! -f "$file" ]]; then
    echo "error: $what is missing: $file" >&2
    fail=1
    return
  fi
  got=$(grep -o -F "$HOLDER" "$file" | wc -l | tr -d ' ')
  if [[ "$got" != "$want" ]]; then
    echo "error: $what states '$HOLDER' $got time(s); it must state it $want time(s)" >&2
    fail=1
  fi
}

# The published licensing page names the Licensor and then states the holder.
check_count "website/book/src/licensing.md" website/book/src/licensing.md 2

# The three hand-written engine crates carry the BUSL paragraph by hand, and a
# crate README ships inside the published package.
for crate_readme in crates/openehr-adl/README.md crates/openehr-query/README.md \
  crates/openehr-sdt/README.md; do
  check_count "$crate_readme" "$crate_readme" 1
done

if [[ "$fail" -ne 0 ]]; then
  echo >&2
  echo "Changing the holder means changing ALL of them together, plus the file" >&2
  echo "headers (re-run the codegen emit set for the generated half)." >&2
  exit 1
fi

echo "ok: one copyright holder — $HOLDER"
