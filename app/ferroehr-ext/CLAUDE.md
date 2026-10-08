# `ferroehr-ext` — the optional-integration crate (feature-gated)

The carve-out target for the platform's OPTIONAL integrations (#1890):
`fhir` (the typed FHIR surface — the ATNA `AuditEvent` renderer and the
terminology-response decoder; FHIR resource mapping is FerroBRIDGE's, not
this crate's), `events` (subscriptions + publisher
transports), `multimedia` (blob store/offload). One ADDITIVE cargo feature
per integration; the shipped binary builds all-on, slim deployments compile
integrations out.

- **Dependency arrows:** `ferroehr → ferroehr-ext` (optional,
  feature-forwarded) → `crates/openehr-*` as needed — never a cycle, never a
  dependency on `ferroehr`/`ferroehr-rest`. What a module needs from the
  service layer arrives as PARAMETERS/callbacks at the seam, decided per
  extraction child (#1892–#1894); service-coupled orchestration glue stays in
  `ferroehr` behind `cfg(feature)`.
- **No openEHR spec governs these surfaces** — flag every behaviour decision
  as our own design; vendor implementations are prior art only. FHIR wire
  facts still cite official HL7/docs.rs sources.
- **Two FHIR release identities, deliberately** (`src/fhir/mod.rs`): the
  `AuditEvent` renderer (`fhir::audit`) serves the ITI-81 retrieval under
  `/fhir/r4`, and the resource is unchanged between R4 and R4B; the TERMINOLOGY
  decoder (`fhir::terminology`) speaks **R4B** because the release belongs to
  the external server it reads, and cites `hl7.org/fhir/R4B/…`. Keep a new
  citation on the side its subsystem is on.
- Heavy external model dependencies (a generated FHIR model, brokers,
  codecs) land HERE, never in `ferroehr`. **`fhir-model` (fhir-sdk's `r4b`
  model generation) is named ONLY in this crate's `fhir` module** — no other
  crate may name a `fhir_model` type; the platform seams take the neutral
  descriptor structs (`fhir::audit::AuditRecord`) and get plain JSON / plain
  views back.
- Zero re-exports. The serde CONFIG sections stay in the `ferroehr` config
  tree (they carry `Secret`/`SecretUrl` and the tree's redaction semantics);
  this crate takes plain runtime parameter structs at construction
  (`BlobStoreParams`, the events/AMQP url) — the platform's
  gated glue maps config → params.
- Features are additive — no `compile_error!` pairs; the `--all-features`
  workspace lanes stay valid.
- **One integration-test binary, one module per integration**
  (`tests/it/main.rs` + `terminology_decode` / `events_contract` /
  `multimedia_engine`), each `#[cfg(feature)]`-gated so `--all-features` runs
  everything and the slim build compiles none of it. They drive this crate's
  PUBLIC seams with no broker and no network: the typed FHIR decoders over
  fixture bodies, the `EventPublisher` contract (a recording double plus the
  AMQP publisher's pre-broker behaviour), the multimedia engine over the
  in-memory `object_store`. The broker-backed and wiremock-backed journeys
  stay in `app/ferroehr/tests/it` where the platform wires the integrations
  in — never duplicated here.
- Gates: `cargo clippy -p ferroehr-ext --all-targets --all-features` +
  `cargo nextest run -p ferroehr-ext --all-features`, plus a
  `--no-default-features` check lane (the slim build).
