# `openehr-sdt` — Simplified Formats, RM-instance validation, SMART scope grammar (HAND-WRITTEN)

Three hand-written ITS-REST surfaces over the `openehr-its` wire layer. Nothing
here is generated: the specifications are prose sub-specifications of ITS-REST
with no machine-readable model, and the BMM has no simplified-format model.
Licence `BUSL-1.1` (single holder Cadasto B.V.); `openehr-its` below
it is Apache-2.0. Dependency arrow: `openehr-sdt → openehr-its`, never the
reverse, zero re-exports.

| Part | Status | To change it |
|---|---|---|
| `src/flat/` — Simplified Formats (FLAT / STRUCTURED / Web Template / TDD) | hand-written | edit normally, with spec citations |
| `src/rm_instance/` — template-independent RM-instance validation (`ValidationMessage`, `validate_rm_and_terminology{,_as}`, the composed `validate_composition`) | hand-written | edit normally, with spec citations |
| `src/smart_scopes.rs` — the SMART on openEHR scope grammar (master08 resource scopes + master07/09 launch contexts) | hand-written (std-only, always compiled) | edit normally, with spec citations. It is the ONE grammar the CDR's scope gate and scope-previewing REST clients (the viewer) parse with |

**Features (`default = ["full"]`, `full = ["flat", "cache"]`).** `flat` pulls
`openehr-its` with `opt14` (canonical JSON, canonical XML, the OPT 1.4
reader) plus the five generated spec crates and the regex/serde/quick-xml
stack; `cache` adds `moka` for `flat::cache`. `smart_scopes` sits under no
feature: `default-features = false` alone must compile it to std only, so
nothing under it may reach for serde, a spec crate, or any other dependency.
A new dependency is optional and joins the `flat` layer or above. CI checks
`--no-default-features` and `--no-default-features --features flat` on
`wasm32-unknown-unknown`.

**`rm_instance` and `flat::validation` are one layer.** `rm_instance` holds the
template-independent whole-instance passes and composes the template pass of
`flat::validation` over them; `flat::validation` holds ONLY the
template-driven archetype-conformance pass and reads `rm_instance`'s report
shape back. The wire-boundary class-invariant dispatcher is NOT here: it is
`openehr_its::wire_validate`.

## The `flat` module — Simplified Formats (`openehr_sdt::flat`)

FLAT + STRUCTURED data instances, the Web Template model, and the
TDD → COMPOSITION converter (`flat::tdd::from_tdd`, corpus-verified). This is
the ITS-REST **Formats** sub-specification (STABLE).

- **The wire oracle is the ITS-REST Simplified Formats specification**
  (`docs/specs/openehr/ITS-REST/docs/simplified_formats/`, STABLE):
  `master04` (field identifiers, node-id algorithm, level removal, `|raw`,
  `|other`, FLAT⇄STRUCTURED algorithms), `master05` (per-RM-type mapping
  tables), `master06` (the `ctx/` vocabulary). SM SIM-B / SDF are
  DEVELOPMENT-state model documents: never implement their terse string
  encodings. SDT (`simplified_data_template/`, the crate's namesake) carries
  upstream `spec_status: RETIRED` and is never implemented. No vendor
  implementation is an oracle.
- **Architecture: one internal tree** (`flat::sim::SimNode`). FLAT
  (`flat::sim::flat`) and STRUCTURED (`flat::sim::structured`) are pure codecs
  over it; the template-driven RM conversion is written once (`flat::flatten`
  RM→sim, `flat::build` sim→RM, entry points in `flat::convert`). Datum codecs
  from the `master05` tables live in `flat::map`; the `ctx/` vocabulary in
  `flat::ctx`; the Web Template model/builder in `flat::webtemplate` (node ids
  per `master04 §Node ID Generation Rules`; the document shape serves
  `application/openehr.wt+json`).
- Path/key encoding (`a/b:0/c|unit`) is load-bearing wire surface: no ad-hoc
  changes; every accepted/emitted form needs a spec citation and a round-trip
  test. Spec-example JSON blocks are the primary test vectors; the OPT corpus
  is regression.
- Consumes `openehr-rm`/`openehr-am` types and the `openehr_its::json` entry
  points directly (canonical JSON with `_type` tagging); never re-models the
  RM. `openehr_sdt::SPEC_VERSION` is the ITS-REST release (1.1.0) the
  Simplified Formats and SMART App Launch specifications belong to; no
  separate pin.

## Gates

```bash
cargo clippy -p openehr-sdt --all-targets
cargo nextest run -p openehr-sdt
cargo clippy -p openehr-sdt --target wasm32-unknown-unknown --no-default-features --features flat -- -D warnings
cargo clippy -p openehr-sdt --target wasm32-unknown-unknown --no-default-features -- -D warnings
```

- **The fidelity gates in `tests/it/` are the crate's acceptance instrument**:
  never weaken or skip one. Two complementary ones: `spec_vectors.rs` replays
  every `simplified_formats` example block for **syntax** + FLAT⇄STRUCTURED
  stability, and `master05_tables.rs` is the **semantic** battery (one test
  per `master05` section, one assertion per mapping-table row: Flat Path +
  Flat type against a minimal RM value through `composition_to_flat`), with
  every row the implementation relocates or does not emit recorded explicitly
  rather than skipped. Plus the `insta` goldens and the OPT-corpus
  round-trips. A new/changed `master05` row lands with its battery row.
- **Fixture paths.** This crate's own fixtures are `tests/fixtures/better` and
  `tests/fixtures/named_event`. The shared canonical-JSON corpus
  (`tests/vendor/openehr_sdk`) and the `sdk`/`twins`/`repeated_member`
  fixtures stay in `openehr-its`; tests reach them by the relative path
  `../openehr-its/tests/…` from the manifest dir. A test in this crate never
  reads `app/`.
- `openehr-adl` is a path-only dev-dependency (the v2_4 Web Template builder
  tests turn ADL2 corpus sources into OPTs); it is stripped at packaging.
