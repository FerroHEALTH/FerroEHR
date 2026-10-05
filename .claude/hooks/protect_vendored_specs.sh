#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# .claude/hooks/protect_vendored_specs.sh
#
# Claude Code PreToolUse hook (matcher: Write|Edit|NotebookEdit). Mechanical
# enforcement of the repo's "never hand-edit" hard rules (CLAUDE.md):
#
#   1. docs/specs/openehr/**            — vendored upstream openEHR spec text
#      (the conformance oracle); refreshed only by scripts/vendor/spec-docs.sh.
#      Exception: the top-level README.md (our own index).
#   2. tools/openehr-codegen/vendor/** — vendored BMM codegen inputs
#      crates/openehr-its/vendor/**     — vendored REST OAS
#      crates/openehr-its/schemas/**    — vendored XSD / ITS-JSON schemas
#      (re-vendor on a pin bump; never edit).
#   3. crates/openehr-its/src/**/generated/** — generator-owned output trees.
#   4. ANY existing file whose head carries an `@generated` marker — the
#      generated spec crates (openehr-base/rm/am, generated impls). Change
#      the emitter (tools/openehr-codegen) and regenerate (/regen-codegen);
#      never the output. (`openehr-codegen` itself writes via its own
#      process, not this tool, so it is unaffected.)
#
# Reads the tool-call JSON on stdin. Exit 2 blocks; exit 0 allows.
# Extended 2026-07-13 (was: docs/specs only). Filename kept stable so
# settings.json and concurrently-running sessions stay valid.

set -euo pipefail

payload="$(cat)"

if command -v jq >/dev/null 2>&1; then
  path="$(printf '%s' "$payload" | jq -r '.tool_input.file_path // .tool_input.notebook_path // empty' 2>/dev/null || true)"
else
  path="$payload"
fi
[ -n "${path:-}" ] || exit 0

block() {
  echo "BLOCKED: $1" >&2
  exit 2
}

case "$path" in
  */docs/specs/openehr/README.md | docs/specs/openehr/README.md)
    exit 0
    ;;
  */docs/specs/openehr/* | docs/specs/openehr/*)
    block "docs/specs/openehr/** is vendored upstream openEHR spec text (the conformance oracle) and must never be hand-edited. Re-vendor with scripts/vendor/spec-docs.sh; pins live in that script + docs/VERSIONS.md."
    ;;
  # PROVENANCE.md files inside vendored trees are REPO-AUTHORED records (the
  # very files a pin bump updates) — never upstream content; exempt them.
  */PROVENANCE.md)
    exit 0
    ;;
  */tools/openehr-codegen/vendor/* | tools/openehr-codegen/vendor/* | \
  */crates/openehr-its/vendor/*     | crates/openehr-its/vendor/*     | \
  */crates/openehr-its/schemas/*    | crates/openehr-its/schemas/*)
    block "vendored spec inputs (BMM / OAS / XSD / ITS-JSON) are upstream-verbatim and must never be hand-edited. Re-vendor on a pin bump (provenance files + docs/VERSIONS.md)."
    ;;
  */crates/openehr-its/src/xml/generated/*  | crates/openehr-its/src/xml/generated/*  | \
  */crates/openehr-its/src/rest/generated/* | crates/openehr-its/src/rest/generated/*)
    block "openehr-its generated/ trees are generator-owned (emit-xml / emit-rest). Edit the emitter in tools/openehr-codegen and run /regen-codegen; never the output."
    ;;
esac

# Content guard: block edits to any existing file carrying an @generated
# marker in its head (the generated spec crates and any future generated
# output, wherever it lives).
# Anchored to a real generated-file marker line (`// @generated` / `-- @generated`
# at line start) — prose that merely MENTIONS @generated must not trip this
# (a hand-written file describing the convention hit the substring form).
if [ -f "$path" ] && head -n 10 "$path" 2>/dev/null | grep -qE '^(//|--) @generated'; then
  block "'$path' carries an @generated marker — it is produced by openehr-codegen. Change the emitter (tools/openehr-codegen/src/render/emit.rs or the *_impl.rs sibling) and run /regen-codegen; never hand-edit generated output."
fi

exit 0
