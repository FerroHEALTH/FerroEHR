# Vendored ITS-JSON schemas (canonical-JSON validation oracle)

Reproduced by: `scripts/vendor/codegen-inputs.sh` (`--check` verifies the committed files against the pin).

JSON Schemas from `openEHR/specifications-ITS-JSON`, vendored **verbatim** (full
upstream `components/` tree).

Source repo: https://github.com/openEHR/specifications-ITS-JSON
Pinned commit: `14387ce9191f228d768b6bade3308b79aaf87471` (master; ITS-JSON is
DEVELOPMENT status with no numbered release). The same commit as the ITS-JSON
spec text (`scripts/vendor/spec-docs.sh`); `codegen-inputs.sh` refuses to run
when the two differ. Moved from `5acae056248e917a4b4c56f7e712f4fcfeb616a6` on
2026-10-14 with no byte change to the `components/` tree.
License: Apache-2.0 (the upstream repo's `LICENSE`; root reference copy
`LICENSE-APACHE-2.0`)
Fetched: 2026-07-04.

## Layout (verbatim `components/`)

- `RM/Release-1.0.3/`, `RM/Release-1.0.4/`, `RM/Release-1.1.0/`
- `BASE/Release-1.1.0/`
- `AM/Release-1.4/`, `AM/Release-2.1.0/`, `AM/Release-2.2.0/`
- Per package: individual `<TYPE>.json` files + a package `main.json`.
- Per component/version: a consolidated `openehr_<component>_<version>_all.json`
  at the `components/` root (e.g. `openehr_rm_1.1.0_all.json`).

784 files total. Schemas use `if…then` for `_type` polymorphism and organize by
package (draft-07).

## Role

**Validation oracle only** — not a code source. The JSON *model* is the
BMM-generated RM types with the native `_type`
self-tagging; there are no JSON structs to generate. `openehr-its::json` reads
`openehr_rm_1.1.0_all.json` (via `include_str!`) to validate canonical-JSON
output in the fidelity gate (`tests/`).

That one file — and only that one — is also PACKAGED in the published
`openehr-its` crate, so its attribution has to travel with the package rather
than with this tree: the crate's `README.md` carries it, pinned to the same
commit as above, and `scripts/checks/packaged-attribution.sh` fails the build
if the two ever disagree. Re-vendoring moves both.

## Known version divergence (adjudicated, machine-pinned — #1697)

ITS-JSON tops out at RM 1.1.0 while our generated RM is 1.2.0 (from BMM), and
the all-schema is CLOSED (`additionalProperties: false` per class) — so the
first RM 1.2.0-only attribute on the wire (e.g. `EHR.tags`) FAILS the oracle
for a reason that is not a defect in our output. The full per-class attribute
delta between this schema and the generated RM 1.2.0 model is machine-derived
and pinned by `tests/it/its_json_delta.rs` (the XSD-gate precedent): the known
1.1.0↔1.2.0 divergence is adjudicated there, and any NEW divergence fails that
gate loudly instead of surfacing later as a spurious validation failure.
