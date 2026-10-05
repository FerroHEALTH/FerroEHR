#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the real-world openEHR canonical-JSON corpus that grounds the
# `openehr-its` fidelity gates (read -> re-serialize -> equality +
# ITS-JSON schema validation).
#
# WHY THIS SOURCE: `ehrbase/openEHR_SDK` is the serialization library EHRbase
# itself uses, so its `canonical_json` test data is real interoperability
# ground truth rather than our own assumptions about the wire. It is prior
# art, never an oracle: where a fixture and the vendored openEHR spec text
# disagree, the spec text wins and the file is adjudicated in the ONE
# exclusion registry (`crates/openehr-its/tests/it/common.rs::excluded`), with
# a repo-authored valid twin under `crates/openehr-its/tests/fixtures/twins/`.
#
# The corpus is vendored VERBATIM and never hand-edited: to correct a fixture,
# bump the pin and re-run this script (`.claude/rules/vendored-corpora.md`).
#
# Usage:
#   scripts/vendor/openehr-sdk-json.sh            # vendor at the pin below
#   SDK_PIN=<sha> scripts/vendor/openehr-sdk-json.sh
#   scripts/vendor/openehr-sdk-json.sh --check    # report drift, write nothing
set -Eeuo pipefail

REPO="ehrbase/openEHR_SDK"
# The pin. Bump deliberately (a bump is a corpus change: re-run the
# openehr-its gates and update crates/openehr-its/tests/vendor/PROVENANCE.md
# in the same commit).
PIN="${SDK_PIN:-22b01e0c99b53669394e56da29c2410838b5cf7e}"

# Upstream root of the corpus, and the packages taken from it. Each package
# contributes exactly its `canonical_json/` directory.
SRC_ROOT="test-data/src/main/resources"
PACKAGES=(composition contribution ehr folder item_structure)
DEST="crates/openehr-its/tests/vendor/openehr_sdk"

CHECK=0
[[ "${1:-}" == "--check" ]] && CHECK=1

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

echo "==> fetching $REPO @ ${PIN:0:12}"
curl -fsSL --proto '=https' --proto-redir '=https' "https://codeload.github.com/$REPO/tar.gz/$PIN" -o "$WORK/repo.tar.gz"
mkdir -p "$WORK/src"
tar -xzf "$WORK/repo.tar.gz" -C "$WORK/src" --strip-components=1

drift=0
for package in "${PACKAGES[@]}"; do
  src="$WORK/src/$SRC_ROOT/$package/canonical_json"
  [[ -d "$src" ]] || {
    echo "::error::$SRC_ROOT/$package/canonical_json is absent at ${PIN:0:12} — the upstream layout moved" >&2
    exit 1
  }
  # Stage the package with the one adjudicated exclusion applied, so `--check`
  # and the write path judge the SAME tree. Upstream carries an editor backup
  # (`composition/canonical_json/compo_feeder_audit_details.json.bak`) beside
  # its fixtures; it is not a corpus document — the corpus is the `*.json`
  # canonical instances the fidelity gates read — and vendoring it would put a
  # file in the tree that no gate can classify.
  stage="$WORK/stage/$package"
  mkdir -p "$stage"
  rsync -a --exclude='*.bak' "$src/" "$stage/"
  dest="$DEST/$package/canonical_json"
  if [[ $CHECK == 1 ]]; then
    echo "==> check $dest"
    diff -rq "$stage" "$dest" || drift=1
    continue
  fi
  echo "==> vendor $dest"
  mkdir -p "$dest"
  # --delete so a file upstream removed disappears here too; the tree is
  # vendored verbatim and never hand-edited.
  rsync -a --delete "$stage/" "$dest/"
done

if [[ $CHECK == 1 ]]; then
  [[ $drift == 0 ]] && echo "==> no drift: the committed tree is what the pin produces"
  exit $drift
fi

count=$(find "$DEST" -type f -name '*.json' | wc -l | tr -d ' ')
echo "==> $count canonical-JSON fixtures → $DEST"
echo "    provenance record: crates/openehr-its/tests/vendor/PROVENANCE.md"
echo "    exclusion registry: crates/openehr-its/tests/it/common.rs::excluded"
