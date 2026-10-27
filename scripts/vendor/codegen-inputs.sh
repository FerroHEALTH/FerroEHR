#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendors the machine-readable openEHR inputs of `openehr-codegen` (and of the
# openehr-its validation gates) verbatim from their pinned upstream commits:
#
#   BMM          specifications-ITS-BMM  components/   -> tools/openehr-codegen/vendor/bmm/components/
#                (json + odin; the yaml serialization is read by nothing)
#   ITS-JSON     specifications-ITS-JSON components/   -> crates/openehr-its/schemas/json/
#   ITS-XML v1   specifications-ITS-XML  components/   -> crates/openehr-its/schemas/xml/its-xml-1.0.2-nsv1/
#   ITS-XML v2   specifications-ITS-XML  components/   -> crates/openehr-its/schemas/xml/its-xml-2.0.0-nsv2/
#   ITS-REST OAS specifications-ITS-REST computable/OAS/ -> crates/openehr-its/vendor/rest-oas/
#
# plus single-file OVERLAYS: a BMM file taken from its component repo where
# ITS-BMM had not republished the version at our ITS-BMM pin. The overlay's
# docs/specs/openehr twin (vendored by spec-docs.sh) must exist and stay
# byte-identical; the script fails otherwise.
#
# Each destination's PROVENANCE.md is the human record and is never touched
# by a sync; the script refuses to run when a record does not name every pin
# that feeds its tree, so a pin bump and its record move in one change.
#
# Line endings: the repository stores text with LF (`.gitattributes`,
# `* text=auto`), and some upstream XSDs are committed with CRLF, so text files
# are normalized CRLF -> LF exactly once, at staging. Binary files (any NUL
# byte) pass through untouched.
#
# Usage:
#   scripts/vendor/codegen-inputs.sh           # vendor every set at its pin
#   scripts/vendor/codegen-inputs.sh --check   # reproduce into a temp dir, diff, write nothing
#
# After a pin bump: regenerate (`/regen-codegen`, scripts/checks/codegen-drift.sh)
# and review the generated diff in the same change.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

CHECK=0
case "${1:-}" in
  "") ;;
  --check) CHECK=1 ;;
  *)
    echo "usage: $0 [--check]" >&2
    exit 2
    ;;
esac

# name | upstream repo | pinned commit | upstream subdir | file filter | destination | provenance record
SETS=(
  "bmm|specifications-ITS-BMM|37b317396eb9b5f03c4c422296a15bdd665028dd|components|no-yaml|tools/openehr-codegen/vendor/bmm/components|tools/openehr-codegen/vendor/bmm/PROVENANCE.md"
  "its-json|specifications-ITS-JSON|14387ce9191f228d768b6bade3308b79aaf87471|components|all|crates/openehr-its/schemas/json|crates/openehr-its/schemas/json/PROVENANCE.md"
  "its-xml-nsv1|specifications-ITS-XML|f7a937778bf9ea43b01b0f9d8a616e47f35017c1|components|all|crates/openehr-its/schemas/xml/its-xml-1.0.2-nsv1|crates/openehr-its/schemas/xml/PROVENANCE.md"
  "its-xml-nsv2|specifications-ITS-XML|de8b37ba6c9a5e126623a063cafba3b58ebf1107|components|all|crates/openehr-its/schemas/xml/its-xml-2.0.0-nsv2|crates/openehr-its/schemas/xml/PROVENANCE.md"
  "rest-oas|specifications-ITS-REST|24058992d5fa96e8dfbd855d9c133f328387fc09|computable/OAS|all|crates/openehr-its/vendor/rest-oas|crates/openehr-its/vendor/rest-oas/PROVENANCE.md"
)

# destination file | upstream repo | pinned commit | upstream path | provenance record
OVERLAYS=(
  "tools/openehr-codegen/vendor/bmm/components/BASE/json/openehr_base_1.3.0.bmm.json|specifications-BASE|93abaf9f656b7bc87019458303c9d88f3f688ef8|computable/BMM/openehr_base_1.3.0.bmm.json|tools/openehr-codegen/vendor/bmm/PROVENANCE.md"
  "tools/openehr-codegen/vendor/bmm/components/RM/json/openehr_rm_1.2.0.bmm.json|specifications-RM|654296bb027b03b4b076101e7171f3359f44cebc|computable/BMM/openehr_rm_1.2.0.bmm.json|tools/openehr-codegen/vendor/bmm/PROVENANCE.md"
  "tools/openehr-codegen/vendor/bmm/components/TERM/json/openehr_term_3.1.0.bmm.json|specifications-TERM|b2dd077d268dff881e81899db58c22b1ed015fad|computable/BMM/openehr_term_3.1.0.bmm.json|tools/openehr-codegen/vendor/bmm/PROVENANCE.md"
)

# The provenance records must name every pin before anything is fetched.
missing=0
require_record() {
  local record=$1 sha=$2 what=$3
  if [[ ! -f "$record" ]]; then
    echo "ERROR: $record is missing ($what)" >&2
    missing=1
  elif ! grep -qF "$sha" "$record"; then
    echo "ERROR: $record does not name the $what pin $sha" >&2
    missing=1
  fi
}
for entry in "${SETS[@]}"; do
  IFS='|' read -r name _repo sha _sub _filter _dest record <<<"$entry"
  require_record "$record" "$sha" "$name"
done
for entry in "${OVERLAYS[@]}"; do
  IFS='|' read -r dest _repo sha _path record <<<"$entry"
  require_record "$record" "$sha" "$(basename "$dest")"
done
if [[ "$missing" -ne 0 ]]; then
  echo "Update the provenance record(s) in the same change as the pin." >&2
  exit 1
fi

# A set whose spec text spec-docs.sh also vendors pins the SAME commit, so the
# machine-readable artifact and its prose cannot drift apart.
# codegen set | spec-docs.sh component
TWIN_PINS=(
  "its-json|ITS-JSON"
  "its-xml-nsv2|ITS-XML"
  "rest-oas|ITS-REST"
)
pin_drift=0
for pair in "${TWIN_PINS[@]}"; do
  IFS='|' read -r set_name component <<<"$pair"
  set_sha=""
  for entry in "${SETS[@]}"; do
    IFS='|' read -r name _repo sha _sub _filter _dest _record <<<"$entry"
    [[ "$name" = "$set_name" ]] && set_sha=$sha
  done
  docs_sha=$(grep -E "^  \"${component}\|" scripts/vendor/spec-docs.sh | sed -n 1p | tr -d '"' | awk -F'|' '{print $4}')
  if [[ -z "$set_sha" || -z "$docs_sha" ]]; then
    echo "ERROR: no pin for $set_name here or for $component in scripts/vendor/spec-docs.sh" >&2
    pin_drift=1
  elif [[ "$set_sha" != "$docs_sha" ]]; then
    echo "ERROR: $set_name is pinned at $set_sha but spec-docs.sh pins $component at $docs_sha" >&2
    pin_drift=1
  fi
done
if [[ "$pin_drift" -ne 0 ]]; then
  echo "Pin the codegen input and its spec text to one commit." >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# One tarball per (repo, commit), fetched once.
checkout() {
  local repo=$1 sha=$2 dir="$WORK/src/$1@$2"
  if [[ ! -d "$dir" ]]; then
    mkdir -p "$dir"
    curl -fsSL --proto '=https' --proto-redir '=https' \
      "https://codeload.github.com/openEHR/$repo/tar.gz/$sha" |
      tar -xz -C "$dir" --strip-components=1
  fi
  printf '%s\n' "$dir"
}

# CRLF -> LF in every text file under a staged tree (see the header note).
normalize() {
  local root=$1 file
  while IFS= read -r -d '' file; do
    perl -pi -e 's/\r\n/\n/g' "$file"
  done < <(grep -rlI --null $'\r' "$root" || true)
}

STAGE="$WORK/stage"
for entry in "${SETS[@]}"; do
  IFS='|' read -r name repo sha sub filter dest _record <<<"$entry"
  echo "==> $name ($repo @ ${sha:0:12}, $sub/)"
  src="$(checkout "$repo" "$sha")"
  if [[ ! -d "$src/$sub" ]]; then
    echo "ERROR: $sub/ is absent from $repo at $sha" >&2
    exit 1
  fi
  mkdir -p "$STAGE/$dest"
  case "$filter" in
    all) rsync -a "$src/$sub/" "$STAGE/$dest/" ;;
    no-yaml) rsync -a --exclude='yaml/' "$src/$sub/" "$STAGE/$dest/" ;;
    *)
      echo "ERROR: unknown file filter '$filter' for $name" >&2
      exit 1
      ;;
  esac
done

# A BMM overlay replaces one release's json with the component repo's file.
# The set's ODIN serialisation of that release predates the overlay, so it is
# dropped rather than vendored as a second, different text of the release.
for entry in "${OVERLAYS[@]}"; do
  IFS='|' read -r dest _repo _sha _path _record <<<"$entry"
  case "$dest" in
    */json/*.bmm.json)
      stale="$STAGE/${dest%/json/*}/odin/$(basename "$dest" .json)"
      if [[ ! -f "$stale" ]]; then
        echo "ERROR: the overlay $dest has no ODIN twin in its set to drop ($stale)" >&2
        exit 1
      fi
      rm "$stale"
      ;;
    *) ;;
  esac
done

for entry in "${OVERLAYS[@]}"; do
  IFS='|' read -r dest repo sha path _record <<<"$entry"
  echo "==> overlay $(basename "$dest") ($repo @ ${sha:0:12})"
  mkdir -p "$(dirname "$STAGE/$dest")"
  curl -fsSL --proto '=https' --proto-redir '=https' -o "$STAGE/$dest" \
    "https://raw.githubusercontent.com/openEHR/$repo/$sha/$path"
done

normalize "$STAGE"

# A BMM overlay is the same artifact spec-docs.sh vendors as reference text:
# a missing or differing twin means the two scripts pin different commits.
twin_mismatch=0
for entry in "${OVERLAYS[@]}"; do
  IFS='|' read -r dest repo _sha path _record <<<"$entry"
  component="${repo#specifications-}"
  twin="docs/specs/openehr/$component/$path"
  if [[ ! -f "$twin" ]]; then
    echo "ERROR: the codegen overlay $dest has no reference twin $twin (re-run spec-docs.sh)" >&2
    twin_mismatch=1
  elif ! cmp -s "$STAGE/$dest" "$twin"; then
    echo "ERROR: $twin differs from the codegen overlay $dest (different pins in spec-docs.sh and here)" >&2
    twin_mismatch=1
  fi
done
if [[ "$twin_mismatch" -ne 0 ]]; then
  echo "Align each overlay pin here with its component pin in scripts/vendor/spec-docs.sh." >&2
  exit 1
fi

drift=0
for entry in "${SETS[@]}"; do
  IFS='|' read -r name _repo _sha _sub _filter dest _record <<<"$entry"
  if [[ "$CHECK" -eq 1 ]]; then
    if ! diff -r --exclude='PROVENANCE.md' "$STAGE/$dest" "$dest" >"$WORK/diff" 2>&1; then
      echo "DRIFT in $dest:" >&2
      head -20 "$WORK/diff" >&2
      drift=1
    else
      echo "    $dest reproduces byte-identically ($(find "$STAGE/$dest" -type f | wc -l | tr -d ' ') files)"
    fi
  else
    mkdir -p "$dest"
    # --delete drops a file upstream removed; the excluded record is kept.
    rsync -a --delete --exclude='PROVENANCE.md' "$STAGE/$dest/" "$dest/"
    echo "    $(find "$dest" -type f -not -name PROVENANCE.md | wc -l | tr -d ' ') files -> $dest"
  fi
done

if [[ "$drift" -ne 0 ]]; then
  echo "The committed codegen inputs do not match their pins; re-run without --check." >&2
  exit 1
fi
echo "Done."
