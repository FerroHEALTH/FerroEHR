# Vendored openEHR ITS material (all four `specifications-ITS-*` repos)

Fetched directly from each `specifications-ITS-*` repo (NOT the
`specifications-ITS` umbrella submodules, which can lag). Every surface is
vendored **verbatim** (full upstream tree) — "literally everything
machine-readable" — with the prose `.adoc`, diagrams, and PHP/Makefile build
tooling deliberately excluded. All four `specifications-ITS-*` repos are
**Apache-2.0** (root reference copy `LICENSE-APACHE-2.0`).

| Surface | Repo | Pinned ref | Vendored to |
|---|---|---|---|
| ITS-JSON schemas | specifications-ITS-JSON | `5acae056…` (master) | `../schemas/json/` (verbatim `components/`, 784 files) |
| ITS-XML XSDs (v2/nsv2) | specifications-ITS-XML | `de8b37ba…` (tag `Release-2.0.0v2`) | `../schemas/xml/its-xml-2.0.0-nsv2/` (70 XSDs) + its XSD-only subset `../schemas/xml/components/` |
| ITS-XML XSDs (v1/nsv1) | specifications-ITS-XML | `f7a93777…` (tag `Release-1.0.2v2`) | `../schemas/xml/its-xml-1.0.2-nsv1/` (17 XSDs — parity target) |
| ITS-REST OAS | specifications-ITS-REST | `24058992…` (tag `Release-1.1.0`) | `rest-oas/*.openapi.yaml` (all 21: 7 groups × codegen/html/validation) |
| ITS-BMM meta-model | specifications-ITS-BMM | `37b31739…` (master), plus three json overlays from the component repos | `../../openehr-codegen/vendor/bmm/components/` (json+odin+yaml, 54 files) |

Every surface above is fetched and reproduced by
`scripts/vendor/codegen-inputs.sh`; `--check` diffs a fresh fetch at the
pins against the committed trees and writes nothing.

Per-surface detail + policy live beside each set:
- `../schemas/json/PROVENANCE.md`
- `../schemas/xml/PROVENANCE.md`
- `rest-oas/PROVENANCE.md`
- `../../openehr-codegen/vendor/bmm/PROVENANCE.md`

## How each surface is consumed

- **BMM** -> `openehr-codegen emit` generates the RM/BASE/AM/TERM/LANG spec crates.
- **XSDs** -> `openehr-codegen emit-xml` generates canonical-XML (de)ser onto the
  RM types (both v1 default + v2 flagged).
- **OAS** -> `openehr-codegen emit-rest` generates the REST contract (DTOs +
  params + server trait + router) into `openehr-its`; `ferroehr-rest` implements it.
- **JSON schemas** -> validation oracle for the canonical-JSON fidelity gate.

Related (verified at latest master the same day, vendored elsewhere):
- QUERY grammar + examples -> `../../openehr-query/vendor/`
- TERM terminology XML -> `../../openehr-term/assets/`
