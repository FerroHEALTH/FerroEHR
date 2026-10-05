#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Regenerate every codegen output and fail if any drifted from the vendored
# specs (BMM → spec crates; the ITS XML/REST surfaces).
#
# The generated code is a pure function of the vendored specs + the emitter, so
# a clean checkout must regenerate byte-identically. This guards against someone
# hand-editing a `// @generated` file, or changing the emitter without
# regenerating. Run in CI and locally before committing generator changes.
#
# Usage: scripts/checks/codegen-drift.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

# Paths the emit targets own (and only those).
GENERATED=(
  crates/openehr-base/src
  crates/openehr-rm/src
  crates/openehr-am/src
  crates/openehr-term/src
  crates/openehr-lang/src
  crates/openehr-its/src/xml/generated
  crates/openehr-its/src/json_codec/generated
  crates/openehr-its/src/rest/generated
  crates/openehr-its/src/opt14
  crates/openehr-its/src/aom2
  crates/openehr-its/src/aom2_model
)

echo "regenerating spec crates (BMM → RM/BASE/AM/TERM/LANG)…"
cargo run -q -p openehr-codegen -- emit
echo "regenerating ITS-XML (ToXml/FromXml impls)…"
cargo run -q -p openehr-codegen -- emit-xml
echo "regenerating ITS-JSON (the emitted serde impls + the _type dispatch)…"
cargo run -q -p openehr-codegen -- emit-json
echo "regenerating ITS-REST (DTOs + traits + routes)…"
cargo run -q -p openehr-codegen -- emit-rest
echo "regenerating OPT 1.4 model (opt14 types + XML codec)…"
cargo run -q -p openehr-codegen -- emit-opt
echo "regenerating the AOM2 archetype models (aom2 + aom2_model types + XML codecs)…"
cargo run -q -p openehr-codegen -- emit-aom2
echo "regenerating the RM attribute/type model (openehr-rm/src/model)…"
cargo run -q -p openehr-codegen -- emit-rm-model
echo "regenerating the RM invariant cores (openehr-rm/src/validate/generated.rs)…"
cargo run -q -p openehr-codegen -- emit-validate

if ! git diff --quiet -- "${GENERATED[@]}"; then
  echo "::error::Generated code is out of sync with the vendored specs." >&2
  echo "Run: cargo run -p openehr-codegen -- emit && … emit-xml && … emit-json && … emit-rest && … emit-opt && … emit-aom2 && … emit-rm-model && … emit-validate, then commit." >&2
  git diff --stat -- "${GENERATED[@]}" >&2
  exit 1
fi

echo "✓ Generated code is in sync with the vendored specs."
