#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendors the openEHR specification *documentation* (the normative spec text)
# into docs/specs/openehr/, pinned per docs/VERSIONS.md. Text formats only
# (adoc/md/txt/csv/json/yaml) — bitmap images, UML .xmi, XSDs, and other
# binaries are excluded (fetch from the upstream repo at the pinned ref if
# needed). .robot/.xml/.opt are included for the executable CNF suite +
# canonical examples.
#
# ONE exception, and it is pinned like everything else: the FIGURES the
# vendored chapters actually reference. openEHR/specifications-AA_GLOBAL
# docs/boilerplate/global_vars.adoc defines three figure attributes, all
# resolved relative to a component's docs/ root:
#
#   :uml_diagrams_uri: UML/diagrams      -> docs/UML/diagrams/<file>
#   :diagrams_uri: {doc_name}/diagrams   -> docs/<doc_name>/diagrams/<file>
#   :images_uri:   {doc_name}/images     -> docs/<doc_name>/images/<file>
#
# `{doc_name}` is the document directory the referencing chapter lives in
# (docs/common, docs/AOM2, docs/bmm, …), which is why the per-document sets are
# derived per directory rather than globally. Every referenced file exists at
# the PINNED COMMIT of its component, so the vendored mirror keeps the upstream
# layout and the references resolve as published. Exactly the referenced files
# are copied (never a whole figure directory), from the same pinned checkout
# the text comes from; a reference with no file at the pin fails the run.
# Figures are copied byte-for-byte — never re-encoded, never optimized.
#
# This is REFERENCE DOCUMENTATION for spec-adherence checks. It is NOT a build
# input: codegen consumes tools/openehr-codegen/vendor/** (BMM/XSD/OAS) and
# openehr-its/schemas/**; those vendor dirs stay authoritative for generation.
#
# Idempotent: wipes and re-vendors each component dir. Re-run after bumping a
# ref below (keep docs/VERSIONS.md in sync).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEST="$REPO_ROOT/docs/specs/openehr"
INCLUDE_EXT=(adoc md txt csv json yaml yml robot xml opt g4)

# component | upstream repo | human ref | pinned commit
# Master pins: the latest published spec versions (RM 1.2.0, BASE 1.3.0,
# TERM 3.1.0, AM 2.4.0, LANG 1.1.0) have no GitHub release tags yet — they
# live on master. SHAs chosen 2026-07-06 to match the ITS-BMM/ITS-JSON pins
# already vendored for codegen.
COMPONENTS=(
  "BASE|specifications-BASE|master (BASE 1.3.0)|e48795762a0648cbe5701be58d42ec5df0c701a7"
  "RM|specifications-RM|master (RM 1.2.0)|66d3ac45587e4532a94d5fd27ca24bcf049f5bf3"
  "AM|specifications-AM|master (AM 2.4.0 + ADL/AOM/OPT 1.4)|da06d63297e8549a351c854d8b1c45cd9f1d577c"
  "TERM|specifications-TERM|master (TERM 3.1.0)|78edd7f59600c40da00fa4e8a1282d563b8942cc"
  "LANG|specifications-LANG|master (LANG 1.1.0)|201b647034f7b1ddfe207e4c3c6f52f6878869b8"
  "QUERY|specifications-QUERY|Release-1.1.0|a87bb51fa1c515b863c9610a9444a2d5570dc05a"
  "SM|specifications-SM|master|23ffc4711c10bae2ae43724b1948fe3b24a0964e"
  "CNF|specifications-CNF|master|33251d2abe5a75c042e11c9385d2e9a79aa15904"
  "ITS-REST|specifications-ITS-REST|Release-1.1.0 (released 19-Jul-2026; matches the vendored OAS identity)|24058992d5fa96e8dfbd855d9c133f328387fc09"
  "ITS-XML|specifications-ITS-XML|master (1.0.2 target, 2.0.0 TRIAL)|de8b37ba6c9a5e126623a063cafba3b58ebf1107"
  "ITS-JSON|specifications-ITS-JSON|master (development pin)|5acae056248e917a4b4c56f7e712f4fcfeb616a6"
)
# ITS-BMM is deliberately absent: it is already vendored verbatim (all
# serializations) at tools/openehr-codegen/vendor/bmm/.

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Upstream repo tooling is not spec text: exclude agent configs and CI.
# `tmp-*` is upstream's own scratch-note prefix (e.g. the CNF repo's
# tmp-docs-how-to-get-code-coverage-from-all-tests.md, a JaCoCo/Java how-to
# with no conformance content) — noise in a tree we treat as an oracle.
#
# DEPENDENCY MANIFESTS of upstream's own build/test tooling are excluded by
# name, for a reason beyond noise: a vendored manifest makes this repository's
# dependency graph claim an ecosystem the workspace does not use (upstream's
# PHP artifact tooling, its stalled Python Robot harness), and the scanners
# then raise advisories against pinned third-party versions nothing here ever
# installs. This workspace's only dependency manifests are its own Cargo
# files. The list covers the common ecosystems so a future component's
# re-vendor cannot reintroduce the class.
MANIFESTS=(
  'requirements*.txt' 'Pipfile*' 'pyproject.toml' 'setup.py' 'setup.cfg'
  'composer.json' 'composer.lock'
  'package.json' 'package-lock.json' 'yarn.lock' 'pnpm-lock.yaml'
  'Gemfile*' '*.gemspec'
  'pom.xml' 'build.gradle*' 'go.mod' 'go.sum'
)
rsync_args=(--exclude='.git' --exclude='.claude' --exclude='.junie' --exclude='.github' --exclude='AGENTS.md' --exclude='tmp-*')
# CNF's abstract pseudo-code scripts are not vendored. Nothing in this
# repository reads them — the CNF material we use is the test schedule
# (docs/) and the Robot suites (tests/) — and every one of the 34 files
# carries a licence header naming CC-BY-SA 3.0 while linking
# creativecommons.org/licenses/by-nd/3.0 (NoDerivatives). Carrying a
# self-contradicting licence statement for material we do not consume buys
# nothing; the contradiction is reported upstream.
rsync_args+=(--exclude='scripts/openehr_platform/')
for manifest in "${MANIFESTS[@]}"; do
  rsync_args+=(--exclude="$manifest")
done
for ext in "${INCLUDE_EXT[@]}"; do
  rsync_args+=(--include="*.$ext")
done
rsync_args+=(--include='*/' --exclude='*')

mkdir -p "$DEST"
for entry in "${COMPONENTS[@]}"; do
  IFS='|' read -r name repo ref sha <<<"$entry"
  echo "==> $name ($repo @ $ref, $sha)"
  src="$TMP/$name"
  git init -q "$src"
  git -C "$src" remote add origin "https://github.com/openEHR/$repo.git"
  git -C "$src" fetch -q --depth 1 origin "$sha"
  git -C "$src" checkout -q FETCH_HEAD

  out="$DEST/$name"
  rm -rf "$out"
  mkdir -p "$out"
  rsync -a --prune-empty-dirs "${rsync_args[@]}" "$src/" "$out/"

  # The upstream LICENSE rides along verbatim (redistribution keeps the
  # source's own terms): the spec-docs repos carry CC-BY-SA 3.0, the ITS
  # artifact repos Apache-2.0. Anything else is unadjudicated — fail loud.
  if [[ ! -f "$src/LICENSE" ]]; then
    echo "ERROR: $name has no LICENSE at $sha — adjudicate before vendoring" >&2
    exit 1
  fi
  cp "$src/LICENSE" "$out/LICENSE"
  if grep -q "Attribution-ShareAlike 3.0" "$src/LICENSE"; then
    license="CC-BY-SA 3.0 Unported"
  elif grep -q "Apache License" "$src/LICENSE"; then
    license="Apache-2.0"
  else
    echo "ERROR: $name LICENSE at $sha is neither CC-BY-SA 3.0 nor Apache-2.0 — adjudicate before vendoring" >&2
    exit 1
  fi

  # The openehr-term crate embeds the TERM computable files, and its
  # asset_identity test byte-compares them against this vendored copy, so both
  # move from the same checkout. The XSDs ride along here although the spec
  # text above excludes them.
  if [[ "$name" == "TERM" ]]; then
    assets="$REPO_ROOT/crates/openehr-term/assets"
    for lang in en es ja pt zh; do
      cp "$src/computable/XML/$lang/openehr_terminology.xml" "$assets/$lang/openehr_terminology.xml"
    done
    cp "$src/computable/XML/openehr_external_terminologies.xml" "$assets/openehr_external_terminologies.xml"
    cp "$src/computable/XML/PropertyUnitData.xml" "$assets/PropertyUnitData.xml"
    cp "$src"/computable/XML/schema/*.xsd "$assets/schema/"
    sed -i.bak -E "s/^- Commit: \`[0-9a-f]{40}\`/- Commit: \`$sha\`/" "$assets/PROVENANCE.md"
    rm -f "$assets/PROVENANCE.md.bak"
  fi

  # The UML class diagrams the vendored chapters reference (see the header
  # note): derive the file list from the vendored text itself, then take
  # exactly those out of the same pinned checkout.
  refs="$(grep -rhoE '\{uml_diagrams_uri\}/[A-Za-z0-9._-]+' "$out" | sed 's|.*/||' | sort -u || true)"
  diagrams=0
  if [[ -n "$refs" ]]; then
    mkdir -p "$out/docs/UML/diagrams"
    while IFS= read -r svg; do
      [[ -n "$svg" ]] || continue
      if [[ ! -f "$src/docs/UML/diagrams/$svg" ]]; then
        echo "ERROR: $name references UML/diagrams/$svg, which does not exist at $sha" >&2
        exit 1
      fi
      cp "$src/docs/UML/diagrams/$svg" "$out/docs/UML/diagrams/$svg"
      diagrams=$((diagrams + 1))
    done <<<"$refs"
  fi

  # The per-document figure sets (see the header note): `{diagrams_uri}` and
  # `{images_uri}` expand to `<doc_name>/diagrams` and `<doc_name>/images`, so
  # the file list is derived per document directory from that directory's own
  # chapter text, then taken out of the same pinned checkout.
  figures=0
  for docdir in "$out"/docs/*/; do
    [[ -d "$docdir" ]] || continue
    doc="$(basename "$docdir")"
    for kind in diagrams images; do
      frefs="$(grep -rhoE --include='*.adoc' "\{${kind}_uri\}/[A-Za-z0-9._-]+" "$docdir" | sed 's|.*/||' | sort -u || true)"
      [[ -n "$frefs" ]] || continue
      mkdir -p "$out/docs/$doc/$kind"
      while IFS= read -r fig; do
        [[ -n "$fig" ]] || continue
        if [[ ! -f "$src/docs/$doc/$kind/$fig" ]]; then
          echo "ERROR: $name references $doc/$kind/$fig, which does not exist at $sha" >&2
          exit 1
        fi
        cp "$src/docs/$doc/$kind/$fig" "$out/docs/$doc/$kind/$fig"
        figures=$((figures + 1))
      done <<<"$frefs"
    done
  done

  if [[ "$diagrams" -gt 0 ]]; then
    diagram_note="- Plus the $diagrams UML class-diagram SVG(s) under \`docs/UML/diagrams/\` that
  the vendored chapters reference as \`image::{uml_diagrams_uri}/<name>.svg\`,
  taken from the same pinned commit. Referenced files only — the upstream
  diagram directory is not mirrored wholesale."
  else
    diagram_note="- No UML class-diagram SVGs: the vendored chapters of this component
  reference none."
  fi

  if [[ "$figures" -gt 0 ]]; then
    figure_note="- Plus the $figures per-document figure(s) under
  \`docs/<doc_name>/diagrams/\` and \`docs/<doc_name>/images/\` that the vendored
  chapters reference as \`image::{diagrams_uri}/<name>\` /
  \`image::{images_uri}/<name>\`, taken byte-for-byte from the same pinned
  commit. Referenced files only — the upstream figure directories are not
  mirrored wholesale."
  else
    figure_note="- No per-document figures: the vendored chapters of this component
  reference no \`{diagrams_uri}\`/\`{images_uri}\` file."
  fi
  cat >"$out/PROVENANCE.md" <<EOF
# Vendored openEHR spec docs: $name

- Source: https://github.com/openEHR/$repo
- Ref: $ref
- Commit: \`$sha\`
- License: $license — the upstream \`LICENSE\` is vendored verbatim alongside
  this file, from the same pinned commit. Root reference copies:
  \`LICENSE-CC-BY-SA-3.0\` / \`LICENSE-APACHE-2.0\`.
- Vendored by: \`scripts/vendor/spec-docs.sh\` (text formats only: ${INCLUDE_EXT[*]})
$diagram_note
$figure_note
- Unreferenced figures, UML \`.xmi\`/\`.mdzip\`, XSDs and other binaries
  excluded — fetch from the repo at the pinned commit.

Do not hand-edit files under this directory; re-run the script instead.
EOF
  echo "    $(find "$out" -type f | wc -l | tr -d ' ') files ($diagrams UML diagram(s), $figures document figure(s))"
done

# Requirements-level reference documents published outside the git spec repos
# (specifications.openehr.org release artifacts). PDF is allowed here — these
# are read-only reference statements, not build inputs.
REQ_OUT="$DEST/REQUIREMENTS"
mkdir -p "$REQ_OUT"
echo "==> REQUIREMENTS (release artifacts)"
curl -fsSL --proto '=https' --proto-redir '=https' -o "$REQ_OUT/iso18308_conformance.pdf" \
  "https://specifications.openehr.org/releases/1.0.2/requirements/iso18308_conformance.pdf"
cat >"$REQ_OUT/PROVENANCE.md" <<'EOF'
# Vendored openEHR requirements-conformance documents

- `iso18308_conformance.pdf` — openEHR ISO 18308 Conformance Statement
  (T. Beale, Rev 1.5.1, 2006-09-09; published release artifact at
  https://specifications.openehr.org/releases/1.0.2/requirements/iso18308_conformance.pdf).
  Maps the ISO 18308 EHR-architecture requirements (Structure, Process,
  Communication, Privacy & Security, Medico-legal, Ethical, Consumer/Cultural,
  Evolution) to openEHR features. A requirements-level reference statement;
  it is not a conformance oracle (the released openEHR components are).

## Licensing

The PDF predates the spec repos' CC-BY-SA licensing and carries its own
2006-era copyright notice (© Copyright openEHR Foundation 2001-2006, all
rights reserved): reading/printing for private non-commercial use and use
for non-commercial presentations and education that inform third parties
about openEHR are permitted, modification is not, and any use must include
the acknowledgement below. It is redistributed here unmodified, on that
non-commercial reference basis, with the required acknowledgement:

> © Copyright openEHR Foundation 2001-2006. All rights reserved.
> www.openEHR.org

Do not hand-edit files under this directory; re-run scripts/vendor/spec-docs.sh.
EOF
echo "    $(find "$REQ_OUT" -type f | wc -l | tr -d ' ') files"

echo "Done. Vendored into $DEST"
