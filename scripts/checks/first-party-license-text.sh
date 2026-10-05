#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Refuses copyleft licence TEXT inside this project's own source.
#
# This project's own code is under the Business Source License 1.1
# (.claude/rules/reliability.md §C-PERMISSIVE).
# A GPL-family or SSPL grant appearing in a file WE wrote is a licence conflict
# that no reviewer reliably catches by eye, so it is checked here.
#
# WHY THIS IS LOCAL AND NOT `customLicenseSearch` IN .fossa.yml: that feature was
# tried and removed on 2026-08-12 (CLI 3.17.16). It could not upload at all
# (`archiveAndLicenseUpload is not enabled in this organization`, failing the
# lane after a clean analysis), and it did not honour the `paths.exclude` list —
# all seven of its hits were inside vendored third-party trees that list already
# excludes. A gate that reads the wrong files and cannot report is worse than no
# gate; this one shares that exclusion list and fails the build.
#
# Third-party material is EXCLUDED, not exempted: it is redistributed verbatim
# under terms recorded in each tree's PROVENANCE.md. Its licences are facts
# about somebody else's work, and flagging them here would say this project
# adopted them.
#
# Usage: scripts/checks/first-party-license-text.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"

cd "$(dirname "$0")/../.."

# The forbidden grants. Copyleft families incompatible with shipping BUSL-1.1 source.
readonly PATTERN='GNU (Affero |Lesser )?General Public License|Server Side Public License'

# Mirrors .fossa.yml `paths.exclude`, plus build/VCS output. Keep the two in
# step: a tree vendored into one and not the other is a silent coverage hole.
readonly -a EXCLUDED=(
  ':(exclude)docs/specs/openehr/**'
  ':(exclude)crates/openehr-term/assets/**'
  ':(exclude)crates/openehr-its/schemas/**'
  ':(exclude)crates/openehr-adl/vendor/**'
  ':(exclude)crates/openehr-lang/vendor/**'
  ':(exclude)tools/openehr-codegen/vendor/**'
  ':(exclude)crates/openehr-adl/tests/corpus/**'
  ':(exclude)crates/openehr-its/tests/vendor/**'
  ':(exclude)crates/openehr-its/tests/fixtures/**'
  ':(exclude)crates/openehr-sdt/tests/fixtures/**'
  ':(exclude)crates/openehr-lang/tests/vendor/**'
  ':(exclude)crates/openehr-term/tests/**'
  ':(exclude)corpus/**'
)

# Files that legitimately NAME these licences without granting them: the licence
# texts this project redistributes, the rules that adjudicate them, this gate,
# and the dependency policy that denies them. A match here is prose ABOUT a
# licence, which is the opposite of a violation.
readonly -a PROSE=(
  ':(exclude)LICENSE-APACHE-2.0'
  ':(exclude)LICENSE-CC-BY-SA-3.0'
  ':(exclude)LICENSE-CC-BY-SA-4.0'
  # The REUSE 3.3 licence directory is the same category as the root licence
  # texts above: quoted legal text this project REDISTRIBUTES so that a
  # downstream copier can read the terms of the file they took, never a grant
  # over anything written here. REUSE requires the full text of every licence
  # any file in the tree is offered under, and one vendored file carries an
  # upstream AGPL header — so an AGPL text has to be present for the tree to be
  # compliant at all.
  #
  # This does not open a hole where the gate used to be: a licence text that
  # belongs to no file now fails `scripts/checks/licensing-declarations.sh`,
  # which refuses any text in LICENSES/ that REUSE.toml does not use. The
  # control moved to the place that can judge it.
  ':(exclude)LICENSES/**'
  ':(exclude)REUSE.toml'
  ':(exclude)website/book/src/licensing.md'
  ':(exclude).claude/**'
  ':(exclude)scripts/checks/first-party-license-text.sh'
  ':(exclude)deny.toml'
  ':(exclude).fossa.yml'
  ':(exclude)CHANGELOG.md'
)

# `git grep` over TRACKED files only, so an untracked local scratch file can
# never fail somebody's build, and target/ is skipped without naming it.
if matches=$(git grep -InE "$PATTERN" -- . "${EXCLUDED[@]}" "${PROSE[@]}"); then
  echo "error: copyleft licence text in first-party source" >&2
  echo >&2
  echo "$matches" >&2
  echo >&2
  echo "This project's own code is BUSL-1.1. A GPL-family or SSPL grant here is a" >&2
  echo "licence conflict. If the file is vendored third-party material, add its" >&2
  echo "tree to BOTH the exclusion list in this script and paths.exclude in" >&2
  echo ".fossa.yml, and record its terms in that tree's PROVENANCE.md." >&2
  exit 1
fi

echo "ok: no copyleft licence text in first-party source"
