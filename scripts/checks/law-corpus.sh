#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The vendored legal texts are the bytes their records say they are.
#
# `docs/law/` exists so a compliance claim resolves to a fixed referent instead
# of to whatever a publisher's page says today. That only holds while three
# things stay true, and none of them is self-enforcing:
#
#   1. every vendored file still hashes to the digest its `SHA256SUMS` records
#      — a hand edit, a partial re-fetch or a git-side accident is otherwise
#      invisible, and the act would quietly stop being the act;
#   2. every act directory carries a `PROVENANCE.md`, and one with vendored
#      files carries a `SHA256SUMS` — an unrecorded text has no identifier, no
#      consolidation date and no licence, which is worse than not having it;
#   3. the record and the directory agree in both directions: a file the record
#      names exists, and a file that exists is covered by the digests.
#
# It also checks that every act directory is linked from `docs/law/README.md`,
# because an index nobody is forced to update is an index that goes stale, and
# the index is where a reviewer starts.
#
# WHAT IT DOES NOT CHECK, labelled rather than implied: nothing here can judge
# whether a vendored consolidation is still the CURRENT one upstream, or
# whether a `PROVENANCE.md` describes its file truthfully. Re-pinning is a
# deliberate act (change the script, re-run it, review the diff), and the
# records are review-enforced prose.
#
# The NEN directories are the deliberate exception to rule 2's second half:
# they hold a record and no text, because the standard is sold under copyright
# and may not be vendored. A directory with no vendored files needs no digests.
#
# Usage: scripts/checks/law-corpus.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

readonly ROOT='docs/law'
readonly INDEX="$ROOT/README.md"

# A backticked filename in a record, in one of the formats a publisher actually
# serves. The backticks are what keeps a book page or a sibling directory
# mentioned in the same prose from being read as a missing file.
# shellcheck disable=SC2016
readonly NAMED_FILE='`[A-Za-z0-9._-]+\.(html|pdf|xml|json|txt|csv)`' # literal backticks in a regex, not a substitution

[[ -f "$INDEX" ]] || {
  echo "law-corpus: missing $INDEX — the corpus index is what a reviewer starts from" >&2
  exit 1
}

# `sha256sum` on Linux, `shasum -a 256` on macOS. Both read the same file
# format, which is why the vendor scripts write that format.
if command -v sha256sum >/dev/null 2>&1; then
  verify_sums() { sha256sum -c --quiet SHA256SUMS; }
elif command -v shasum >/dev/null 2>&1; then
  verify_sums() { shasum -a 256 -c SHA256SUMS >/dev/null; }
else
  echo "law-corpus: neither sha256sum nor shasum is available" >&2
  exit 1
fi

fail=0
dirs=0
files=0
note() {
  echo "law-corpus: $*" >&2
  fail=1
}

while IFS= read -r dir; do
  rel="${dir#"$ROOT"/}"
  dirs=$((dirs + 1))

  if [[ ! -f "$dir/PROVENANCE.md" ]]; then
    note "$dir has no PROVENANCE.md — a vendored text with no record has no identifier, no consolidation date and no licence"
    continue
  fi

  # The vendored bytes: everything that is not one of the two records.
  vendored=$(cd "$dir" && find . -type f ! -name SHA256SUMS ! -name PROVENANCE.md | sed 's|^\./||' | sort)
  count=0
  [[ -z "$vendored" ]] || count=$(printf '%s\n' "$vendored" | wc -l | tr -d ' ')
  files=$((files + count))

  if [[ -n "$vendored" ]]; then
    if [[ ! -f "$dir/SHA256SUMS" ]]; then
      note "$dir has $count vendored file(s) and no SHA256SUMS — nothing proves they are the bytes the script fetched"
    elif ! (cd "$dir" && verify_sums); then
      note "$dir: a vendored file does not match its recorded digest. Do not re-record it — find out what changed it, and re-run the vendor script if the pin genuinely moved"
    else
      # `-c` only checks what is listed, so a file added beside the record
      # would pass unnoticed. The reverse direction closes that.
      listed=$(awk '{ sub(/^[*]/, "", $2); print $2 }' "$dir/SHA256SUMS" | sort)
      while IFS= read -r extra; do
        [[ -n "$extra" ]] || continue
        note "$dir/$extra is present but absent from SHA256SUMS — re-run the vendor script rather than adding a file by hand"
      done < <(comm -23 <(printf '%s\n' "$vendored") <(printf '%s\n' "$listed"))
    fi
  elif [[ -f "$dir/SHA256SUMS" ]]; then
    note "$dir has a SHA256SUMS and no vendored file for it to cover"
  fi

  # A record that names a file must name one that is there. Only the vendored
  # formats are read as a filename: the records also mention book pages and
  # sibling directories, which live elsewhere by design.
  while IFS= read -r named; do
    [[ -n "$named" ]] || continue
    [[ -f "$dir/$named" ]] || note "$dir/PROVENANCE.md names \`$named\`, which is not in the directory"
  done < <(grep -oE "$NAMED_FILE" "$dir/PROVENANCE.md" | tr -d '`' | sort -u)

  grep -qF -- "]($rel/)" "$INDEX" || note "$dir is not linked from $INDEX — the index must list every act in the corpus"
done < <(find "$ROOT" -mindepth 2 -maxdepth 2 -type d | sort)

if [[ "$dirs" -eq 0 ]]; then
  note "$ROOT holds no act directories"
fi

[[ "$fail" -eq 0 ]] || {
  echo >&2
  echo "A vendored legal text is only worth carrying while its recorded digest" >&2
  echo "is true. Change the vendor script for its jurisdiction (scripts/vendor/" >&2
  echo "law-eu.sh, law-nl.sh, law-de.sh or law-ch.sh) and re-run it; never" >&2
  echo "hand-edit a vendored act or hand-write a digest." >&2
  exit 1
}

echo "law-corpus: OK ($dirs act directories, $files vendored files, digests verified)."
