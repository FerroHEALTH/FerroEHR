# `openehr-its` — canonical JSON/XML + the ITS-REST contract + the archetype XML codecs (MIXED)

The ITS wire layer, with a strict generated/hand-written split. Know which
part you are touching before editing anything. Licence `Apache-2.0` (holders
Cadasto B.V. and openEHR Foundation): the hand-written runtimes, entry
points and wire-validation dispatcher are here because the generated code
cannot ship without them. The hand-written Simplified Formats, RM-instance
validation and SMART scope grammar are `openehr-sdt`, which builds on this
crate (never the reverse).

**ITS-BMM is deliberately NOT here.** The vendored BMM meta-model that drives
code generation lives at `tools/openehr-codegen/vendor/bmm/` (read by the
generator's own loader), and the runtime BMM/P_BMM object model is
`openehr-lang`. This crate carries no BMM module — an empty one existed as a
placeholder and was removed, because a published module that promises future
surface is API nobody can use.


| Part | Status | To change it |
|---|---|---|
| `src/xml/generated/` (`ToXml`/`FromXml`) | **GENERATED** (`emit-xml`, from the XSDs + BMM) | edit the emitter, regenerate |
| `src/json_codec/generated/structural.rs` (the `_type` → decode dispatch + the declared-key table) | **GENERATED** (`emit-json`, from BMM) | edit the emitter, regenerate |
| `src/rest/generated/` (ITS-REST DTOs, params and response headers structs, the per-group `server` module — trait, success-answer enums, axum `router` — and `client` module, route tables) | **GENERATED** (`emit-rest`, from the vendored OAS) | edit the emitter, regenerate |
| `xml/runtime.rs`, `rest/runtime.rs` (`ApiError`; `Refusal` = an `ApiError` plus answer headers, what the server traits return; the `ResponseHeaders` seam the generated headers structs implement; the ITS-REST `Error` rendering), `rest/routes.rs` (the body-free operation matcher over every `ROUTES` table), `rest/server.rs` (public: the whole-API `router` and `with_fallbacks` — `404`/`405` with the `Error` body and `Allow`; crate-private: the runtime the generated routers share), `rest/client.rs` (the client runtime: `Transport`, `ReqwestTransport`, `Client` with `execute`/`forward`, `Credentials` over `secrecy::SecretString` and the `CredentialsProvider` trait, `CallOptions`, `RequestTimeout`, `RetryPolicy`, `ErrorBody`, `ClientError`), `json` + `wire_validate` entry points, validation, fidelity gates | hand-written | edit normally, with spec citations |

**Features (`default = ["full"]` = everything, so consumers are unaffected).**
The graph underneath is layered: `json` → `xml` → `opt14` are the openEHR
content surfaces and are all wasm-safe; `schema-validation` (`jsonschema` + the
embedded RM schema) and the three REST features sit outside that chain:
`rest` (DTOs, params, routes, the `routes` matcher, `ApiError` over serde + `http` +
`urlencoding`), `rest-server` (`rest` + the server traits and generated routers + `axum`) and `rest-client` (`rest` + the
generated clients + the `rest/client.rs` runtime over `reqwest`, with
`secrecy` for the credential secret). The generated contract carries, beyond
the OAS parameters, the request headers the ITS-REST docs text defines
(`openehr-version`, `openehr-audit-details`, `openehr-template-id` on the
commit operations — `plan::overrides::REST_DOCS_TEXT_HEADERS`), and every
`4xx` outcome variant carries an `ErrorBody`. A CI lane
clippies each REST feature alone and reads back from `cargo tree` that `rest`
pulls neither `axum` nor `reqwest` and `rest-client` pulls no `axum`. A
browser consumer takes `default-features = false, features = ["opt14"]`, and
CI clippies that selection on `wasm32-unknown-unknown`. There is no
dependency-free island: `default-features = false` alone compiles an empty
crate. When adding surface,
put a new dependency in the layer that uses it (never in `json`, which every
layer inherits).

**Canonical JSON is EMITTED `serde` impls on the spec types themselves, and
they do NOT live in this crate.** `serde::Serialize`/`Deserialize` and the spec
types are both foreign here, so an impl in `openehr-its` would break the orphan
rule: `emit-json` writes one `json_serde` module into EACH spec crate
(`openehr-base/rm/am/term/lang`), over the shared hand-written runtime
`openehr_base::serde_support`. They are MANUAL long-form impls (a field
identifier enum + a visitor, <https://serde.rs/deserialize-struct.html>) — never
a derive, because serde's four enum representations cannot express the
canonical `_type` discriminator (context-dependent presence, deep-descendant
dispatch, closed key set). What stays here is `json_codec::generated::structural`
— the `_type` → `Deserialize` dispatch and the declared-key table, which span
every spec crate at once.
`json::to_canonical_json`/`to_canonical_value`/`from_canonical_json`/
`from_canonical_value` ARE the entry points (reads wrapped once in
`serde_path_to_error`, so every refusal carries the JSON path);
`json::JsonParseError` is the refusal type. `wire_validate::validate_rm_value` is the wire-boundary
RM class-invariant DISPATCH LAYER — thin entry points that COMPOSE the tiers in
a fixed order. Both tiers live upstream in `openehr_rm::validate` (the fast path
`try_fast_validate`; the authoritative `_type` → concrete-type table
`typed_dispatch::dispatch_typed`); what stays here is the part that needs this
crate: the undeclared-key door over the generated `declared_fields` table, and
the generated `structural_check` fallthrough for every class the typed table
declines (`dispatch_typed` returns `false`) — which spans all five spec crates
at once, so the codec is the structural-conformance authority for EVERY emitted
class (a defective node of a class with no invariant is refused too). It only
ROUTES: every value-level decision (the fast path, the typed table, the
invariant cores, the mandatory-container bounds, the JSON-level per-node checks,
the terminology binding table) is defined in `openehr_rm::validate`. The
template-independent whole-instance passes (`rm_instance`) and the
template-driven `flat::validation` pass live in `openehr-sdt`;
`wire_validate` stays here. Proven by
`tests/it/json_codec_parity.rs` (byte hazards + reader tolerance) +
`tests/it/canonical_contract.rs` (the R0 determinism manifest).

- **NEVER hand-edit anything under a `generated/` directory** — the
  `codegen-drift` CI job regenerates and fails on any diff
  (`/regen-codegen` runs emit + emit-xml + emit-rest + the check). The three
  XSD-driven ARCHETYPE modules `opt14/`, `aom2/` and `aom2_model/` are generated
  wholesale (every file carries the `@generated` banner) and are drift-guarded
  the same way, even though they are not under a `generated/` dir.
- **Three XSD-driven archetype codecs, one pipeline** (`render/emit_opt.rs`,
  one curated closure each in `load/xsd.rs`):

  | module | subcommand | closure | root |
  |---|---|---|---|
  | `opt14` | `emit-opt` | `AM_FILES_V1` (`Template.xsd`) | `<template>` = `OPERATIONAL_TEMPLATE` |
  | `aom2` | `emit-aom2` | `AOM2_FILES` (`P_Archetype.xsd`) | `<archetype>` = `P_AUTHORED_ARCHETYPE` |
  | `aom2_model` | `emit-aom2` | `AOM2_MODEL_FILES` (`Archetype.xsd`) | `<archetype>` = `AUTHORED_ARCHETYPE` |

  Everything module-specific is an `emit_opt::ModelTarget` parameter (module
  path, `@generated` banner, doc labels), never a constant: a hardcoded value
  silently stamps one module's identity onto another's files.
  The two AOM2 serializations stay SEPARATE closures — both schemas declare the
  top-level element `archetype` with different root types and define same-named
  supporting types, so merging them resolves the abstract slots inconsistently.
  `aom2_model`'s entry points are typed to `AUTHORED_ARCHETYPE`, not to the
  `ARCHETYPE` its global element names: `ARCHETYPE` is `abstract="true"` with no
  derived type in the closure (`AUTHORED_ARCHETYPE` extends `AUTHORED_RESOURCE`
  and re-uses the body via `<xs:group ref="ARCHETYPE"/>`).
- `openehr-its` has NO model-form instance corpus and cannot get one (all 8
  vendored `AOM2/examples/*.xml` are persistent-form; upstream publishes ADL text
  only). `tests/it/aom2_model_xml.rs` is therefore a construct → serialize →
  parse self-consistency gate, and `tests/it/aom2_xml.rs` reads all 8 examples.
  Both assert the archetype BODY is non-empty, because that body sits behind
  `xs:group` references — a codec that dropped group refs would round-trip every
  document vacuously over an empty envelope.
- The vendored inputs are authoritative: XSDs at `schemas/xml/`, ITS-JSON
  schema at `schemas/json/` (validation oracle), REST OAS at
  `vendor/rest-oas/` — pinned to the same commit as the spec text under
  `docs/specs/openehr/ITS-REST/` (a reconciliation guard enforces this).
  Never edit vendored files; re-vendor on a pin bump.
- Canonical-JSON `_type` self-tagging comes from the emitted manual `serde`
  impls (`emit-json`, in each spec crate's `json_serde`) — no per-struct tag
  fields, and never a serde DERIVE on a spec type.
- XML: one impl set serves both namespaces (v1/v2 differ only by root
  `xmlns`); `xsi:type` emitted iff concrete type ≠ declared slot type.
- **The fidelity gates in `tests/` are the crate's acceptance instrument**
  (canonical-JSON corpus round-trips, C14N, schema validation, the R0
  determinism manifest) — never weaken or skip one to get green; a gate failure
  means the emitter or runtime is wrong.
- `tests/it/opt14_corpus.rs` carries TWO parse gates: (1) every vendored `.opt`
  under the shared corpus at `corpus/fixtures/service` (reached through
  `common::service_corpus_dir()`; a test in this crate never reads `app/`)
  parses + round-trips through the generated `opt14::OperationalTemplate`; (2) every official CNF robot
  VALID-template fixture (`docs/specs/openehr/CNF/tests/platform/robot/
  _resources/test_data_sets/valid_templates`) parses, with exactly two fixtures
  adjudicated XSD-invalid (a missing mandatory `OPERATIONAL_TEMPLATE.language`
  per Template.xsd; a missing mandatory `DV_PROPORTION.type` per BaseTypes.xsd)
  — each pinned as an EXPECTED rejection with its citation; a fixture that
  starts parsing must be re-adjudicated, never silently dropped.
- The shared canonical-JSON corpus `tests/vendor/openehr_sdk` and the
  `tests/fixtures/{sdk,twins,repeated_member}` fixtures are also read by
  `openehr-sdt`'s tests, at `../openehr-its/tests/…`. Moving or renaming one
  moves those paths too.
