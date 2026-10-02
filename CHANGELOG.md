# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Maintenance rules: every pull request that changes user-visible behaviour —
the REST surface, AQL, validation, storage/migrations, configuration, CLI,
container/Helm artifacts — adds an entry under **[Unreleased]** in the same
PR (a CI guard enforces this). Cutting a release renames [Unreleased] to the
version + date, adds fresh link references, and tags `vX.Y.Z`; the release
workflow refuses a tag that has no matching section here.

## [Unreleased]

### Changed

- **`openehr-its` exposes the `Authorization` value it sends** (#3535).
  `Credentials::header_value` is public, so a consumer can validate a
  credential at configuration load with exactly the value the client sends.
  Basic credentials are checked against RFC 7617 §2 (no colon in the user-id,
  no control character in either part) and bearer tokens against the
  `b64token` syntax of RFC 6750 §2.1. A refusal is a typed
  `InvalidCredentials`, which never carries the secret; a request whose
  credentials break these rules fails with `ClientError::InvalidCredentials`
  before anything is sent.
- **The `openehr-*` crates step to 0.0.78.**
- **`openehr-query` classifies AQL function calls** (#3529). A call naming
  one of the built-in functions in QUERY master03 §Functions parses to
  `FunctionCall::Builtin` with a typed `BuiltinFunction` (`StringFunction`,
  `NumericFunction`, `DateTimeFunction`); any other name parses to
  `FunctionCall::Other`. Both keep the name as written, so `printer::to_aql`
  returns it unchanged, `CONTAINS` included. `FunctionCall::Named` is gone, and
  the `Token::Contains` lexer token now carries its spelling. The CDR's AQL
  engine uses this classification instead of a function list of its own.
- **`openehr-its` names every operation's declared parameters** (#3530).
  Each generated `*Params` struct carries a `PARAMS` table of the path, query
  and header parameters it holds (name as the OAS spells it, location,
  required, explode, and the value kind its schema states), written in the
  same pass as the struct. Each API group carries `ROUTE_PARAMS`, aligned with
  `ROUTES`, and `routes::lookup` returns the matched operation's parameters
  with `query_param`, `query_key` and `header_param` lookups, so an
  intermediary can pass on exactly what an operation declares.
- **`openehr-its` `ReqwestTransport` constructors take a
  `reqwest::ClientBuilder`** (#3531). `ReqwestTransport::new` and the new
  `with_builder_timeout` (replacing `with_client_timeout`) build the client
  themselves, with redirects switched off.
- **The `openehr-*` crates step to 0.0.77.**
- **FerroTERM moves to 0.1.6 at `ghcr.io/ferrohealth/ferroterm`** (#3521).
  FerroTERM moved to the FerroHEALTH organization, and its releases from 0.1.6
  on publish there. The terminology compose overlay, the hosted sandbox, the
  Helm chart (`terminology.image`, chart 10.1.6) and the FerroTERM integration
  test pin `0.1.6@sha256:7ec4c962…`, and the book links
  <https://github.com/FerroHEALTH/FerroTERM>.

### Fixed

- **`Client::forward` returns a redirect as received** (#3531). The client
  built by `ReqwestTransport::with_timeout` used reqwest's default redirect
  policy and followed up to ten redirects, re-sending the request and its
  credentials to whatever host a `Location` named. Every `ReqwestTransport`
  constructor now switches redirects off, so a `3xx` reaches the caller and
  nothing is sent further. No ITS-REST 1.1.0 operation declares a `3xx`, so a
  generated client call that receives one refuses it as an undocumented
  status.
- **A FerroTERM pin the sandbox's index cannot serve is refused on the pull
  request** (#3520). FerroTERM 0.1.5 moved its artifact layout to 7, and the
  v4.3.2 sandbox leg failed on the layout-6 index the box carried. A
  `ferroterm-index-layout` CI job reads the layout at the pinned FerroTERM tag
  and refuses a pin that differs from `deploy/hosted/ferroterm-index.env`, the
  record of the box's index. The box now runs FerroTERM 0.1.6 on a rebuilt
  layout-7 index, and `deploy/hosted/README.md` documents the rebuild.
- **The generated ITS-REST types keep the members an open schema allows**
  (#3526). OpenAPI 3.0.3 makes `additionalProperties` default to `true`, so
  every ITS-REST object schema that does not set it to `false` admits members
  beyond its declared properties. The `openehr-its` REST types for those
  schemas now carry them in an `additional_properties` map, written back at
  the object's own level, where they used to be dropped on decode. `Error` is
  one of them, and the server now builds its error body from the generated
  type with the `error` member in that map. The body carries the same three
  members, with `error` now written after `message` and `validationErrors`.

## [4.3.2] - 2026-10-01

### Added

- **`openehr-query` can rewrite, locate and bind a query** (#3505, #3506,
  #3507, #3508). `visit::Visit` and `visit::VisitMut` walk every node of the
  AST, so a consumer finds, replaces or removes a `WHERE` condition without
  its own recursive walk. Every `IdentifiedPath` and every `WHERE` condition
  carries an `ast::Span`, the byte range it was written at, so a diagnostic
  can point at a predicate without repeating its literal; spans take no part
  in equality. `bind::bind` substitutes ITS-REST `query_parameters` into the
  AST as typed literals the printer escapes, at every position the grammar
  admits a parameter, and reports an unbound, unknown, duplicate or
  unbindable parameter by name, never by value. The opt-in `federation`
  feature adds `federation::parse_federated`, which lifts a
  `FROM ENDPOINT p [ … ]` or `ORGANISATION [ … ]` directive out of a query
  and parses the rest as strict AQL. Breaking: `WhereExpr::Identified` and
  `IdentifiedPath` gained the span (`WhereExpr::identified` and
  `IdentifiedPath::new` build them without one).
- **`openehr-its` publishes the server router and the intermediary seams**
  (#3509, #3510, #3511, #3512, #3514). Under `rest-server`, each API group's
  `server` module carries the trait, one success enum per operation (the
  documented `2xx` answers with their `Location`, `ETag` and other headers)
  and `router(api)`, an axum router binding every `ROUTES` entry to its trait
  method. A method refuses with `rest::runtime::Refusal`, an `ApiError` plus
  the headers its answer needs: the current `ETag` on a `412`, or the
  OpenAPI-declared headers of a demographic `409`/`412` through the generated
  headers structs (`Refusal::with_headers`). Either renders as the ITS-REST
  `Error` JSON body. `rest::server::router(api)` mounts all six groups at
  once, and `rest::server::with_fallbacks` makes an unknown path answer `404`
  and an undeclared method `405` with `Allow`, both with the `Error` body.
  `rest::routes::lookup` matches a method and path to its operation and path
  parameters without reading the body, and `Client::forward` sends a request
  once and returns every answer as received, so an intermediary passes bodies,
  `Location` and `ETag` through byte for byte. `CredentialsProvider` resolves
  the credential before every attempt and hears about a `401`, for tokens that
  refresh. `CallOptions` gives a generated client a deadline and extra headers
  per call (`with_options`); the remaining budget reaches the engine as
  `RequestTimeout`, and an elapsed deadline is `ClientError::DeadlineElapsed`.
  Breaking: the server traits moved into `<group>::server` and return
  `Result<success enum, Refusal>`, and the response headers structs moved
  from `client` to the group module. The nine `openehr-*` crates step to 0.0.74.

- **The `openehr-its` ITS-REST client carries what a consumer meets on first
  adoption** (#3487). The commit operations (COMPOSITION, EHR_STATUS, the
  directory, EHR creation, the demographic parties and relationships) take
  the committal-metadata headers the ITS-REST overview defines and the
  OpenAPI declares no parameter for: `openehr-version`,
  `openehr-audit-details` (one field line per value) and, on the COMPOSITION
  and CONTRIBUTION commits, `openehr-template-id`. Every documented `4xx`
  outcome carries the answer body as an `ErrorBody`: the bytes as received,
  decoded as the ITS-REST `Error` when they are one, so a `422`'s
  `validationErrors` reach the caller; `ClientError::Unauthorized`,
  `Forbidden`, `ServiceFailure` and `UndocumentedStatus` carry the same body.
  A `400` whose body carries `message` alone decodes as the `Error` with an
  empty `validationErrors`. A path parameter keeps `:` literal, so a version
  uid is sent as the specification writes it. `Credentials` hold their secret
  in a `secrecy::SecretString` (redacted `Debug`, zeroed on drop), with
  `Credentials::basic` and `Credentials::bearer` constructors. The nine
  `openehr-*` crates step to 0.0.72.
- **`openehr-its` ships an ITS-REST client** (#3485). The new `rest-client`
  feature adds a generated client per ITS-REST 1.1.0 API group
  (`rest::generated::<group>::client`) over a hand-written runtime
  (`rest::client`: the `Transport` engine trait, the shipped `reqwest` engine,
  Basic and Bearer credentials, the retry budget and `ClientError`). Each
  operation takes its parameter struct, with `Prefer`, `If-Match` and `Accept`
  as fields, and answers an outcome with one variant per status the OpenAPI
  documents for it; declared response headers such as `ETag` and `Location`
  come back in a typed struct. Any other status is
  `ClientError::UndocumentedStatus`, and `401`, `403` and `5xx` are their own
  errors. Idempotent requests are retried after a transport failure or a
  `5xx`; a `POST` is sent once. A create operation also answers `NoContent`,
  the `204` the ITS-REST overview prescribes under `Prefer: return=minimal`
  although the OpenAPI lists `201` alone. FerroBRIDGE keeps its hand-written
  client until it moves to this one (its issue #276). The DTOs, parameter
  structs and route tables move to a
  new `rest` feature over serde and `http` alone, which both `rest-server` and
  `rest-client` build on, so a client never compiles `axum`. `full` now
  includes `rest-client`. The nine `openehr-*` crates step to 0.0.71.
- **A CI guard keeps the Apache-2.0 crates free of BUSL-1.1 code:**
  `scripts/checks/licence-boundary.sh` refuses a normal or build dependency,
  or a feature, from `openehr-base`, `openehr-lang`, `openehr-term`,
  `openehr-rm`, `openehr-am` or `openehr-its` on `openehr-query`,
  `openehr-adl` or `openehr-sdt`, and a `license` field that changes side.

### Changed

- **The repository moved to the FerroHEALTH organization**
  (<https://github.com/FerroHEALTH/FerroEHR>, #3516); the old
  `rubentalstra/FerroEHR` URLs redirect. Images and the Helm chart now publish
  to `ghcr.io/ferrohealth/ferroehr`, `ghcr.io/ferrohealth/ferroehr-viewer`,
  `ghcr.io/ferrohealth/ferroehr-postgres` and
  `oci://ghcr.io/ferrohealth/charts/ferroehr`, and the compose files, the chart
  and the book name those references. The v4.3.1 images and chart are copied
  there by digest. Everything published up to v4.3.1 stays available under
  `ghcr.io/rubentalstra`, and its signatures name `rubentalstra/FerroEHR` as the
  repository that built it. The roadmap board is now
  <https://github.com/orgs/FerroHEALTH/projects/2>. The nine `openehr-*` crates
  step to 0.0.75 for their new `repository` field.

- **The dependency set moves forward in one step.** OpenTelemetry goes to
  0.33 (`opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`,
  `opentelemetry-prometheus`) with `tracing-opentelemetry` 0.34; the OpenAPI
  stack to `utoipa` 6, `utoipa-axum` 0.3 and `utoipa-swagger-ui` 10;
  `jsonschema` to 0.58, `sevenz-rust2` to 0.23, `lapin` to 4.12 and `config`
  to 0.15.27. The bundled PostgreSQL image is rebuilt on the current
  `postgres:18.6` upstream digest (`sha256:5a5a84b1…`), a same-version respin
  with rebuilt base packages. The nine `openehr-*` crates step to 0.0.73.

- **FerroTERM moves to 0.1.5 everywhere the product pins it.** The quickstart
  overlay `docker-compose.terminology.yml`, the hosted sandbox compose file, the
  Helm chart (`terminology.image.digest` and the pinned version its tag default
  and Artifact Hub annotation read) and the integration test that starts the
  real server all name
  `ghcr.io/rubentalstra/ferroterm:0.1.5@sha256:61a2d4aefc4a48e9e90b9a698a64bdfeb7ef6443744a1fe50972099456891fe9`,
  the index digest read from GHCR. That release adds the history interaction,
  `_summary` and `_elements` on read, a 406 on an unacceptable `Accept`, refuses
  a write that would duplicate a canonical, and states the position of a
  refused expression constraint on the `OperationOutcome`. The chart version
  becomes 10.1.3; an install that pins the chart keeps running the image it
  pinned.

- **`openehr-its` is split in two, and the wire layer is Apache-2.0 again**
  (#3482). The new crate `openehr-sdt` (BUSL-1.1) holds the hand-written
  Simplified Formats (FLAT, STRUCTURED, Web Template, TDD import), the
  template-independent RM-instance validation and the SMART on openEHR scope
  grammar. `openehr-its` keeps the ITS wire layer: the generated canonical JSON
  dispatch, the canonical XML codecs, the ITS-REST contract, the OPT 1.4 and
  AOM2 archetype XML codecs, and the hand-written runtimes, canonical JSON entry
  points and wire-validation dispatcher they need. From 0.0.69 it is published
  under Apache-2.0, so a consumer of canonical JSON takes on no BUSL-1.1 code.
  Rust callers import `openehr_sdt::flat`, `openehr_sdt::rm_instance` and
  `openehr_sdt::smart_scopes` in place of `openehr_its::flat`,
  `openehr_its::rm_instance` and `openehr_its::rest::smart_scopes`. The `flat`
  and `cache` features left `openehr-its` for `openehr-sdt`; a
  `wasm32-unknown-unknown` consumer of `openehr-its` now selects
  `default-features = false, features = ["opt14"]`, and `openehr-sdt` with
  `default-features = false` compiles to the scope grammar alone, with no
  dependency. Published versions keep their licence: `openehr-its` 0.0.60 to
  0.0.67 stay `BUSL-1.1 AND Apache-2.0`.
- **The published `openehr-*` crates are nine, and all step to 0.0.69** (#3482).
  `openehr-sdt` was first published alone at 0.0.68, built against the 0.0.67
  siblings; it joins the lockstep line at 0.0.69 and publishes after
  `openehr-its` and before `openehr-adl`.
- **FerroTERM moves to 0.1.4 everywhere the product pins it** (#3474). The
  quickstart overlay `docker-compose.terminology.yml`, the hosted sandbox
  compose file, the Helm chart (`terminology.image.digest` and the pinned
  version its tag default and Artifact Hub annotation read) and the
  integration test that starts the real server all name
  `ghcr.io/rubentalstra/ferroterm:0.1.4@sha256:f3b5f35a60d99d5c837125e5f8cf39d8eeecf9f69dbc79b9a526cc45aa706470`,
  the index digest read from GHCR. That release reloads its served set without
  a restart, treats an index root's child directories as releases, ships the
  `ferroterm-sync` service, and carries the standard concept status properties
  on locally authored code systems. The chart version becomes 10.1.2; an
  install that pins the chart keeps running the image it pinned.

### Fixed

- **Two `500` answers from the `openehr-its` REST server runtime carry a fixed
  message** (#3518). An illegal response header and an answer body that does
  not serialize put the header name or the serde error into the body the client
  received; both now answer with a fixed text.

- **The app and viewer images rebase onto a distroless base with the fixed
  OpenSSL** (#3504). `gcr.io/distroless/cc-debian13:nonroot` moves to
  `sha256:e792ab3d…`, which carries the `libssl3t64` update from DSA-6531-1
  (CVE-2026-75804, CVE-2026-84782). The image scan refused the previous base
  with both findings at HIGH, so the `main` images went untagged until this
  change.

- **An AQL path predicate whose node boolean opens with a parameter or an
  archetype id parses** (#3513): `CLUSTER k[$node and name/value='x']` and
  `[openEHR-EHR-CLUSTER.device.v1 or at0001]` were refused, though the
  grammar in `QUERY/docs/AQL/master07-grammar.adoc` admits both.
- The `no attribution in commits` CI job no longer refuses the
  `Co-authored-by: dependabot[bot]` trailer GitHub writes when it squash-merges
  a Dependabot pull request, which had failed the last three pushes to `main`.
  The commit-msg hook keeps that line too.

## [4.3.1] - 2026-09-16

### Added

- **A fourth backup CronJob for the audit domain** (#3399). The Helm chart
  renders `backup.audit` as soon as `database.audit.existingSecret` gives the
  audit trail a database of its own, with its own claim, credential and
  schedule, and drops `--schema=audit` from the clinical dump in the same
  render. Until then nothing changes: the trail shares the clinical database
  and travels with that dump. Before this, an audit database placed elsewhere
  was backed up by nothing and the operations page told the operator to dump it
  by hand.
- **An erasure tombstone on the party change-event stream** (#3417). A
  physically deleted party now leaves one contribution-less row on the party
  domain's own outbox before the delete commits, naming the party and every
  `PARTY_RELATIONSHIP` erased with it, so a consumer of the party stream is
  told to delete what it derived. GDPR Art. 19 makes the controller
  communicate an erasure to each recipient the data reached, and the clinical
  outbox has carried the same row since #3416.
- **A dispatch-only storage benchmark lane** (#3370). It runs the storage
  benchmark against a PostgreSQL 18 service with durability on, compares every
  operation with the record committed for the same schema generation, and fails
  the run when a p50 or p99 rose further above it than the dispatched tolerance.
  The summary lists each operation's baseline, measurement, delta and verdict
  and names both machines; the fresh record leaves as a run artifact and the
  committed baseline changes only in a pull request. Exploration, never a
  conformance record: it earns no performance class.
- **Restriction of processing, at whole-EHR or single-object grain** (#3324).
  A restriction register records who asked, on what ground and when, and the
  mark it drives stops every path that would process the object: a point read,
  a versioned read and a revision history answer `403` with a body naming the
  restriction, AQL omits it at every scope, an EHR Extract skips it and refuses
  outright for a restricted EHR, the change-event stream neither emits nor
  delivers an event about it, and a write to it is refused. The stored rows are
  untouched — storage is the only processing GDPR Art. 18(2) leaves. Lifting
  stamps the register rather than erasing it, so the sequence Art. 18(3) turns
  on survives, and a whole-record lift leaves an object-scoped restriction
  standing. `POST`/`GET {base}/admin/restriction` and
  `POST {base}/admin/restriction/lift` drive it, each act an access record of
  its own.
- **A per-EHR objection to research processing** (#3325). While it stands, the
  record leaves every full-population AQL query, every export and the event
  stream, and a query naming the `ehr_id` or a read for care is untouched: GDPR
  Art. 21(6) reaches research processing, not the care record. The
  public-interest override the article admits is recorded beside the objection
  and says on whose authority. `POST {base}/admin/research-objection`.
- **A retention register, and a list of what has fallen due** (#3346). The
  period per content category and jurisdiction with the legal citation it rests
  on, a per-EHR anchor, a whole-record hold and a per-object hold, and a view
  that lists the EHRs whose period has run with the counts of objects due and
  objects exempted. Nothing is deleted on a timer: an openEHR record is
  indelible, so acting on the list is the controller's decision, taken through
  the physical delete. Five routes under `{base}/admin/retention/`, and a new
  book page, [Retention, restriction and
  objection](https://ferroehr.eu/book/compliance/retention.html).
- **An access-log retention ceiling beside the floor** (#3346). Set
  `[audit.store] sgb_v_309_controller = true` where the deploying organisation
  is, or acts for, one of the controllers SGB V § 307 names for a German
  telematics-infrastructure application, and boot refuses any
  `retention_days` above the three-year period § 309 Abs. 1 sets — including
  `0`, keep forever. A floor and a ceiling that contradict each other are
  refused outright rather than silently resolved in favour of one.

### Fixed

- **A date with text after it compares as no date** (#3436). `openehr_date_days`
  read the leading four, six or eight characters of a value and ignored the
  rest, so `20210102XYZ`, `2021-01-02 BC`, `2021-01-02 Europe/Amsterdam` and
  `2021a` compared and ordered as dates in AQL. BASE `foundation_types`
  master06-time_types.adoc §Iso8601_date admits none of them, and each now reads
  as NULL, which is a comparison miss rather than an error. `2021-01-02T10:30:45`
  and its space-separated and lowercase-`t` variants are unchanged.

### Changed

- **The release profile links with thin LTO** (#3098). `[profile.release]`
  gains `lto = "thin"`, the whole-binary optimisation the Cargo book describes
  for a CPU-bound path; the overflow-checks, `panic = "unwind"` and line-table
  posture is unchanged. `codegen-units = 1` was tried in the same cycle and
  withdrawn: the arm64 release build exhausted its runner twice. A release
  build takes longer; the throughput before/after is measured under #3452 once
  the box can be idle.
- **`LATEST_VERSION` under `CONTAINS` is one key probe per object** (#3453).
  The AQL emitter used to express "this row is the current trunk version" as
  two correlated `EXISTS` subqueries over the head table, which let the planner
  reach the version row by `vo_id` alone through the commit-time index and walk
  every trunk version of the object for every node row: 75 601 of the 93 255
  buffers a class-`s` population `CONTAINS` read. The head row is now joined on
  its primary key and the version row reached through its own, `sys_version`
  bound; the plan-shape tests pin the index and a ceiling of eight buffers per
  probe. Results are unchanged.
- **The server needs only `btree_gist`, which it installs itself** (#3433). The
  second-generation schema calls nothing from `uuid-ossp`, `pgcrypto` or
  `pg_trgm`, so the PostgreSQL image no longer installs them and the CI step no
  longer creates them. `btree_gist` backs the temporal key on
  `linkage.subject_ehr` and the server creates it at boot, so a plain
  PostgreSQL 18 database with a migration credential that may create it serves
  with nothing preinstalled.
- **The conformance statement is signed by a named person for the company**
  (#3438). A company signs through someone: the party statement's signatory is
  now "Ruben Talstra, for Vernum Projecten B.V." with the date of the change,
  and the rendered statement follows. `AI_STATEMENT.md` names Ruben Talstra as
  author and maintainer, since the owner of the work is the company.
- **The Licensor and copyright holder is Vernum Projecten B.V.** (#3435). Every
  statement of the holder names the company: `LICENSE` and the per-crate
  licence texts, `REUSE.toml`, the SPDX header of every first-party file
  (emitted for the generated crates, so they were regenerated), the licensing
  page, the landing footer and the conformance party statement, whose rendered
  statement and certificate were regenerated with it. The licences themselves
  are unchanged — the application and the three hand-written engines stay
  Business Source License 1.1 with the same Additional Use Grant, Change Date
  and Change License, the five generated model crates stay Apache-2.0, and
  releases up to v4.0.17 and crate versions up to 0.0.56 stay MIT as published,
  under the holder they were published with. Ruben Talstra remains the
  maintainer credit the banner, `MAINTAINERS.md`, the OCI `authors` label and
  the chart maintainers carry, and the `urn:rubentalstra:ferroehr` conformance
  identifier is an identifier rather than a holder statement and stays.
- **The `production` deployment profile refuses the two compatibility
  defaults** (#3323). The per-EHR `EHR_ACCESS` default is `open` and the audit
  trail fails open, both chosen so the conformance instrument and the
  development stacks work out of the box. Neither default changes; what changes
  is that `deployment_profile = "production"` now refuses to start while either
  stands, naming the key, the value and the remedy, unless the deployment
  accepts it by name in `deployment_accepts` — the mechanism the profile
  already uses. The two new gap tokens are `open_ehr_access_default` and
  `audit_fails_open`. GDPR Art. 25(2) asks that by default personal data not be
  made accessible without the individual's intervention to an indefinite number
  of natural persons; the obligation binds the controller's production posture,
  which is what the profile expresses.
- **The deployment posture reads the audit pool and every domain database**
  (#3398). `shared_cluster` now includes the audit pool's own cluster identity,
  so an audit database co-located with a pseudonymisation domain is reported
  instead of being invisible (Swiss DSV Art. 4 Abs. 5 keeps the log on a system
  separate from the one processing the data). `migrate_on_runtime_credential`
  is evaluated per domain database rather than only from `[db] migrate_url`: a
  domain relocated to a database of its own is prepared on that domain's
  runtime DSN, which is the same gap under a different key.
- **`ext.openehr_timestamp` refuses a zone name and a `BC` era** (#3427).
  PostgreSQL's own date/time input reads `2021-01-02T10:30:45
  Europe/Amsterdam` and `2021-01-02T10:30:45 BC`, and the separator-position
  gate in front of it let both through. BASE `foundation_types`
  master06-time_types.adoc §Iso8601_date_time ends the value at the offset, so
  both now read as NULL: an AQL comparison against such a stored value misses
  rather than matching a date the record does not carry. The check costs the
  helper 93 ms per 50 000 readings against 65 ms without it, measured on the
  same corpus where the first generation took 117 ms.
- **The archive export lists multimedia blobs from the reference index**
  (#3420). It used to walk every exported body in process to find the blob
  keys; it now reads `blob_ref`, the index the node write path maintains in
  the same transaction as the nodes it describes, once per domain. The archive
  it writes is unchanged.
- **The `ext` value helpers are each in the form that measured fastest, and read
  fewer things than PostgreSQL's own date parser did** (#3351). Five of the
  seven no longer open a subtransaction per row: they guard their casts ahead of
  them instead of trapping an error. The two whose whole body is parsing a date
  or a time keep the trap, because measurement makes it the cheapest validator
  available — over 50 000 readings they cost 58 ms and 70 ms, where the same
  bodies validating by hand so the cast can run untrapped cost 65 ms and
  235 ms — and each carries that measurement in its function comment. Every
  helper reads faster than the one it replaces, and a value any of them refuses
  reads as NULL. Three readings tighten as a result, all of them
  values openEHR does not define: a date whose year carried leading whitespace
  or a sign is now unreadable, a date handed to the time parser no longer
  yields a number, and `ext.openehr_timestamp` refuses a date that is not
  zero-padded (`2021-1-2`), an ordinal date (`2021-002`), and the words
  PostgreSQL accepts (`now`, `today`, `yesterday`, `epoch`, `infinity`). The
  first two made a stored value read as the current time. In the
  other direction it now reads the lowercase `t` and space separators on a
  reduced-precision date, which it used to read only at full precision.
- **A physical EHR delete reaches everything the erasure has to reach**
  (#3347, #3403). `DELETE {base}/admin/ehr/{ehr_id}` runs one clinical
  transaction that removes the EHR and, through the foreign-key graph, its
  versions over both storage tiers with their nodes, attestations,
  contributions, commit audits, item tags, folder memberships, restriction and
  retention marks, multimedia blob references and pending change events; the
  subject proxies of a subject whose only record this was go with it, and an
  erasure tombstone is appended to the change-event stream, which each
  registered reader receives and applies by deleting what it derived (GDPR
  Art. 19). The cross-reference row naming the subject is then erased in the
  linkage domain, on the bulk route as well as the single one, and the
  externalized blobs no surviving version references leave the object store.
  The audit records naming the `ehr_id` stay, because Art. 17(3)(b) withholds
  erasure where a legal obligation requires the processing and the
  access-logging periods are that obligation.
- **Multimedia blob collection is an index lookup rather than a scan**
  (#3347). `clinical.blob_ref` and its party twin record which stored version
  references which externalized blob, written by the node write path, carried
  across the archival tier move by their foreign key and removed with the
  version. The collector reads that index instead of pulling every node body of
  the deleted record into the process and substring-matching it against every
  node of both domains.
- **An OPT 1.4 template whose ordinal symbol has no `<value>` element is
  refused with the fix in the message** (#3395). The ITS-XML profile schema
  types `C_DV_ORDINAL.list` as the RM `DV_ORDINAL`, whose `symbol` is a
  `DV_CODED_TEXT` carrying a mandatory `value`, so such a template is not a
  document the format admits and the refusal stands. Beside the element and
  line it already reported, the `422` now says to add `<value/>`, that an empty
  element is valid, and that the rubric is resolved from `term_definitions`.
  CKM's exporter writes the element; Archetype Designer's omits it.
- **The migration-immutability rule binds SHIPPED files only** (owner ruling
  2026-09-15). A migration is immutable once it is in a release (present at
  the latest `vX.Y.Z` tag); a file on `main` that no release carries has been
  applied by no installation and is fixed in place. The guard
  (`scripts/checks/migration-immutability.sh`) judges the pull request's diff
  against the release tag's tree and re-arms at every cut; nothing changes for
  an installation on a released version.
- **BREAKING — the storage schema is rewritten, and a database created by an
  earlier release must be recreated** (#3342, #3344). The rewrite is greenfield
  (owner ruling 2026-09-14): the new migration sets replace the old ones
  outright, nothing is upgraded in place, and the server refuses at boot a
  database whose migration record predates the rewrite. Take a dump of anything
  worth keeping before upgrading, recreate the database, and let the server
  migrate it. What changed, and why:
  - The `ehr` schema becomes `clinical` and `demographic` becomes `party`, each
    laid out as a sequence of natural migration files with one concern apiece
    rather than one squashed baseline.
  - `version` is append-only: it carries no validity interval, so a
    supersession writes no close-out statement, and the validity of a version
    is derived from its `committed_at`. One mutable `vo_head` row per versioned
    object answers "what is current", and none of the columns a commit updates
    appears in an index, so that update is heap-only.
  - `version`, `node` and `vo_attestation` are partitioned by storage tier, so
    archiving an EHR is row movement inside one relation set rather than a copy
    into mirror relations: the `cold` and `cold_demographic` schemas, every
    `*_all` union view and the `vo_archive` marker table are gone, the archive
    marker is two columns of `vo_head`, and a foreign key holds across the tier.
  - `node` gains the promoted `name_code` and `name_terminology` the AQL node
    predicate needs, and drops the unread `citem_num`.
  - The clinical and party change-control and node relations are rendered from
    one shared DDL template, so the two domains cannot drift.
  - The party domain gains `party_relationship_target`, the target-side index
    behind `PARTY.reverse_relationships`.
  - The archive file format carries a version's `committed_at` instead of a
    validity interval. An archive written by an earlier release does not load.

- **BREAKING — the EHR id / subject cross-reference moves to the linkage
  domain, and erasure reaches it** (#3345). `linkage.party_ehr` becomes
  `linkage.subject_ehr` and absorbs `ehr.ehr_index`: one temporal relation
  holding both the party-to-EHR map and the openEHR Service Model's EHR Index
  associations (`I_EHR_INDEX`), which is what that service is for — an EHR
  persisted with only an EHR id, and a separate index to obtain the subject
  identifier. The `EHR_INDEX` operations and the subject-proxy subject
  resolution now run on the linkage pool; the clinical schema keeps only the
  promoted `ehr.subject_id`/`subject_namespace` pair, under the pseudonym
  guard, because the wire binds `ehr_get_by_subject` and the one-EHR-per-subject
  `409` to EHR_STATUS content. The subject-proxy relations (`sp_subject`,
  `sp_variable`, `sp_data_set`, `sp_sample`) key on a derived opaque
  `subject_key` instead of the caller's own subject identifier, so no
  unguarded subject identifier remains in the clinical domain (GDPR Art. 4(5)),
  and a catalog test asserts it. Deleting an EHR now removes the
  cross-reference rows naming it, through `linkage.erase_ehr` — a
  `SECURITY DEFINER` function the `ferroehr_linkage` role may execute while
  still holding no `DELETE` on the table — so an erased record leaves no row
  behind saying whose it was (Art. 17(1)). Subject-proxy registrations and
  index associations made under an earlier release do not carry over; the
  storage rewrite above requires a fresh database in any case.

- **BREAKING — one pool and one DSN per storage domain, and the runtime roles
  are named after them** (#3343). Each of the four storage domains — `clinical`,
  `party`, `linkage`, `audit` — now has its own configuration table,
  `[storage.<domain>]` with `url`/`url_file`, and its own connection pool
  carrying only that domain's schema on its `search_path`. Every domain defaults
  to the shared `[db].url`, so a configuration file that names no `[storage]`
  table at all, and the compose quickstart, behave exactly as before. What
  changed for a deployment that had already separated its credentials:
  - `[db].demographic_url`, `[db].demographic_url_file`, `[db].linkage_url` and
    `[db].linkage_url_file` are gone. Move them to `[storage.party]` and
    `[storage.linkage]`; the old keys are refused at boot by name rather than
    ignored.
  - The chart's `database.demographicExistingSecret`,
    `database.linkageExistingSecret` and their `*Key`/`*Url` siblings become
    `database.<domain>.{existingSecret,existingSecretKey,url}` for `clinical`,
    `party`, `linkage` and `audit`. The mounted files are
    `storage.<domain>.url` and the environment names are
    `FERROEHR__STORAGE__<DOMAIN>__URL_FILE`.
  - The runtime roles are the domain roles and nothing else:
    `ferroehr_clinical`, `ferroehr_clinical_reader`, `ferroehr_party`,
    `ferroehr_party_reader` and `ferroehr_linkage`, beside the DDL-only
    `ferroehr_migrator`. The first generation's `ferroehr_ehr`,
    `ferroehr_ehr_reader`, `ferroehr_demographic`, `ferroehr_demographic_reader`
    and the generic `ferroehr_app`/`ferroehr_reader` pair are gone: no
    migration creates or grants to them, the compose database image no longer
    provisions them, and a DSN that authenticates as one of them holds nothing.
    A login role is granted the domain role of the pool it serves
    (`GRANT ferroehr_clinical TO <login>`); the clinical role also writes the
    local Audit Record Repository.
  - The compose backup service `ferroehr-backup-demographic` is
    `ferroehr-backup-party`, its directory variable
    `FERROEHR_BACKUP_DEMOGRAPHIC_DIR` is `FERROEHR_BACKUP_PARTY_DIR`, and the
    chart's `backup.demographic.*` values are `backup.party.*`.

  A DSN naming another host moves that domain to a database or cluster of its
  own, which is the separation a schema inside one database cannot make: a base
  backup, WAL archiving and physical replication carry every schema of a
  database together (PostgreSQL 18, Backup and Restore). Schema preparation
  follows: the `ext` set is applied per database by whichever domain reaches it
  first, so two co-located domains share one copy and a relocated domain carries
  its own. One layout is refused rather than half-applied — the `linkage`
  migration set revokes a function the `party` set creates, so those two are
  prepared in the same database.

  The boot gate grew two refusals. Two domains configured on DIFFERENT DSNs that
  authenticate as the SAME database role are refused, because a separation that
  exists only in the configuration reads as one that holds. And a domain role
  that does not exist is a warning under `deployment_profile = "sandbox"` and a
  refusal under `production`, where absent roles mean absent grants and the
  check would otherwise pass by having nothing to measure. `ferroehr config
  check` prints the resolved domain layout, naming no DSN.

- **The Helm chart is 10.1.1, a major bump** (#3342, #3343, #3344, #3378). The
  values contract changed in three ways a values file can notice: the per-domain
  `database.<domain>` blocks above replace the flat `demographicExistingSecret`
  and `linkageExistingSecret` keys, `config.tenancy` no longer exists and is
  refused at render with the reason, and the per-domain backup CronJobs dump the
  generation-2 schemas (`clinical, ext, audit` and `party`) instead of the
  retired mirror schemas. The docker-compose backup
  services dump the same sets, and all six jobs now pass `--strict-names`, so a
  `--schema` pattern matching nothing fails the dump instead of writing an
  artefact that silently lacks that half (PostgreSQL 18, pg_dump §Options).
- **A newer runtime base image** (#3439). The server and the viewer image build
  on `gcr.io/distroless/cc-debian13:nonroot` at digest `sha256:54df941e`, the
  digest that tag resolves to today. The
  `org.opencontainers.image.base.digest` label moves with it in both
  Dockerfiles and in the two publishing lanes, which is what the image-labels
  guard checks.
- **Four runtime dependencies move to their current patch** (#3442). `lapin`
  4.11.0 retries the initial AMQP connection properly when recovery is on,
  which is the mode the events transport runs in; `reqwest` 0.13.5 fixes proxy
  authentication picking the wrong credentials when several proxies match a
  request, and a blocking-client timeout panic; `tokio-rustls` 0.26.5 returns
  more data per `poll_read` on a TLS stream; `fancy-regex` 0.19.2 cuts pattern
  build time. No API a caller here uses changed.
- **`jsonschema` 0.55.1** (#3443). It compiles the vendored ITS-JSON schema
  behind `openehr-its`'s `schema-validation` feature. The releases since 0.53.0
  correct the keyword location an error names and several schema
  canonicalisation defects; the 626-test `openehr-its` suite, the ITS-JSON
  corpus gate among it, passes unchanged. The eight published `openehr-*`
  crates step to 0.0.67 with it, since the requirement is part of their
  packaged manifest.

### Removed

- **The Subject Proxy service** (#3431). It realised SM
  `I_SUBJECT_PROXY_SERVICE` and `I_DATA_BINDING` over the `sp_*` tables, had no
  REST route and no consumer outside its own configuration. Gone with it: the
  `[subject_proxy]` configuration section and its FHIR-frame executor, the
  `sp_*` tables, the erasure reach into them, the Helm `config.subject_proxy`
  values and the Subject Proxy book page. No openEHR specification obliges a
  CDR to realise every SM component; SM `master02` presents the platform as a
  set of services a deployment composes. `docs/architecture.md` lists the
  component as not realised.
- **Multi-tenancy** (#3378). Multi-tenancy is achieved by running separate
  instances: one instance, one database, one set of domain roles per
  organisation. That is where openEHR puts it. An openEHR *system* is "a
  distinct logical repository corresponding to an organisational entity that is
  legally responsible" for the data, and it is "distinct from any underlying
  virtualisation infrastructure or cloud computing facility, which may house
  multiple logical EHR systems in a multi-tenant fashion" (BASE
  `architecture_overview/master06-design_of_the_ehr.adoc` §The EHR System) — so
  tenancy belongs to the layer that hosts several systems, not inside one. The
  same chapter's §System Identity is why housing several organisations in one
  system was wrong rather than merely unnecessary: `system_id` names that one
  legally responsible system, it "becomes embedded in the version identifiers of
  committed -- and possibly signed -- content", and it "cannot easily be changed
  afterwards". No relation carries a tenant column, no row policy or session GUC
  scopes a read, and the `[tenancy]` configuration, the tenant middleware, the
  `/admin/tenant` routes and the Helm and compose tenancy keys are gone. The
  pseudonymisation domains and their role barriers are unrelated to tenancy and
  stay.
  
### Security

- **rustls 0.23.45** (RUSTSEC-2026-0285): the TLS stack no longer accepts TLS 1.3 handshake messages sent at the wrong encryption level after a key change (RFC 8446 §5.1). Lockfile bump only; no configuration changes.


## [4.3.0] - 2026-09-14

### Added

- **The pre-rewrite storage baseline is committed** (#3368). The storage
  benchmark's class-`s` record for the first-generation schema lives at
  `docs/benchmarks/storage/generation-1/record.json` and renders on the
  performance page, so the storage rewrite (#3337) is compared against a
  committed record rather than a claim. The harness's end-of-run relation table
  gains `dead_tuple_percent`, the exact heap figure `pgstattuple` reads (null
  where the extension cannot be created), beside the planner's estimates.
- **A storage benchmark harness** (#3367). `cargo bench -p ferroehr --bench
  storage` seeds a corpus through the ordinary write path, then times the
  storage layer's hot paths one at a time: a composition commit, a supersession,
  the `If-Match` supersession, point reads by version uid and by
  versioned-object uid, the version at an instant, the revision history, an AQL
  CONTAINS chain over one EHR and over the population, archive and restore of an
  EHR, and one retention prune. Beside each operation's wall-clock it records
  what the database did for it: the per-relation tuple counters and relation
  sizes from `pg_stat_user_tables`, buffer hits and misses from
  `pg_stat_database`, the write-ahead-log bytes either side of the measurement,
  one representative commit probed on its own, and
  `EXPLAIN (ANALYZE, BUFFERS, WAL)` of the population statement inside a
  rolled-back transaction. Everything runs through the service and the public
  storage API, and the relations are discovered from the catalogue, so the same
  harness measures a rewritten schema. The JSON record lands under
  `docs/benchmarks/storage/<schema-generation>/` and reaches the performance
  page through `scripts/render/storage-bench.sh`. It is a benchmark, never a
  conformance record: conformance is the CNF suite Veredictum runs.
- **The Kubernetes deployment probe observes the FerroTERM workload** (#3328).
  `scripts/deploy-probe-k8s.sh` gains a terminology stage (`P-K8S-TERM-*`) that
  reads the chart's `terminology.enabled` claims off a running cluster: the
  absence of every terminology object while the gate is off, the rollout, the
  applied runtime posture from the container runtime's own spec, the shaped seed
  reaching the process through the ConfigMap volume, the NetworkPolicy refusing a
  pod the CDR's selector does not name in this namespace and in another while the
  CDR itself is admitted, and a value-set binding resolved through the Service. A
  mounted index is probed when `K8S_TERM_INDEX_CLAIM` names an existing claim,
  and is declared not exercised by probe id when it does not.
- **Identifier-scanner rules for Germany and Switzerland, and the Swiss access-log
  retention floor** (#3318, #3319). `de-kvnr` refuses the unchangeable part of a
  Krankenversichertennummer (a letter, eight digits and the GKV-Spitzenverband's
  check digit), `ch-ahvn13` the AHV number in its plain and dotted display forms
  (AHVV Art. 133; the BSV's Wegleitung check digit) and `ch-epd-pid` the EPD
  patient identification number (EPDV-EDI Anhang 1, the fixed `76133761` prefix
  and its check digit), each transcribed from the issuing authority's own
  publication and tested against its worked examples. With a Swiss rule active,
  the server refuses to boot with an audit `retention_days` under the one year
  DSV Art. 4 Abs. 5 requires. The AHVV joins the vendored corpus at
  `docs/law/ch/ahvv/`.
- **The regulation corpus gains the German and Swiss layers** (#3293,
  #3294). `docs/law/de/` carries the BDSG, SGB V (the ePA and telematics
  provisions, §§ 341–355 and 360–363), the GDNG and the StGB (§ 203) as the
  XML gesetze-im-internet.de publishes, pinned by the publisher's build stamp,
  plus the Digital-Gesetz as its Bundesgesetzblatt issue; `docs/law/ch/`
  carries the DSG/FADP and DSV/DPO in German and the non-binding English, and
  the EPDG, EPDV and EPDV-EDI in German, each at its current Fedlex
  consolidation. Two vendor scripts, provenance and digests per act, the
  publishers' no-copyright articles quoted in `LICENSES/`, and the compliance
  overview gains a Germany and a Switzerland section stating, provision by
  provision, what the software ships and what the deploying organisation must
  do.
- **The Helm chart runs FerroTERM beside the CDR** (#3305, chart 8.3.0).
  `terminology.enabled` renders the terminology server as its own workload —
  Deployment, ClusterIP Service, ServiceAccount, NetworkPolicy — under the same
  Restricted profile as the CDR, and points the CDR's
  `config.terminology.external` at its Service, so archetype bindings resolve at
  commit and AQL `TERMINOLOGY()` expands through it (the CDR's own
  `/terminology/*` extension routes stay a separate switch,
  `config.terminology.api_enabled`, and are off by default). Every values file
  that contradicts that injection is refused at render rather than silently
  overwritten: a `default` provider of your own, an `external.enabled: false`,
  a `fail_on_error` disagreeing with `terminology.failOnError`, and a provider
  with no `external.routes` entry naming it. The licence-free shaped seed ships with the chart and
  is mounted as a ConfigMap, so an install from the registry serves terminology
  with no checkout; a release you hold a licence for is an index built
  off-cluster and mounted from an existing claim named in
  `terminology.index.persistentVolumeClaim` — which also switches the workload
  to `strategy: Recreate`, because a rolling update's surge pod cannot attach a
  ReadWriteOnce volume the outgoing pod still holds. No Ingress is rendered for
  it and no value renders one: the CDR is its only caller, and its NetworkPolicy
  admits the CDR's pods alone. No boolean opens that port;
  `terminology.networkPolicy.extraIngressFrom` admits a peer you write, and the
  values schema refuses an empty one (a bare `namespaceSelector: {}` selects
  every namespace). Above one replica, the pods spread one per node and
  `terminology.podDisruptionBudget` guards them through a drain.

### Fixed

- Under multi-tenancy an unset `ferroehr.tenant_id` no longer resolves to the
  reserved default tenant: the server stamps the tenancy posture at boot, every
  connection it opens declares its tenant explicitly, and the tenant reader
  refuses a connection that declares none. Single-tenant deployments are
  unchanged (#3341).
- Under multi-tenancy the AMQP outbox drainer and the FHIR outbound emitter
  drain every registered tenant's rows in turn, with a delivery cursor per
  tenant, instead of the default tenant's alone (#3355).
- The event-outbox retention prune stops at the lowest cursor of every active
  reader instead of deleting on age alone: the FHIR outbound emitter (and any
  future cursor reader) now owns a row in the new `event_outbox_reader`
  registry, reconciled from the configuration at boot, and a published row it
  has not reached survives the retention window. The singleton
  `fhir_outbound_cursor` table moves into the registry with its watermark
  (#3330).

## [4.2.5] - 2026-09-12

### Changed

- **The compliance overview's GDPR section covers the data-subject rights,
  the processor terms, breach notification and research** (#3326). From the
  audit recorded on #3322: rows for Art. 12(3), 15, 16, 17, 18, 19, 20, 21,
  28(3)(e) to (h), 33(3)(a) with 34(3)(a), and 89(1), each naming what the
  product ships, the tracker status and what the deploying organisation must
  still do, with the same split mirrored on the shared-responsibility page.
  Where the text is silent, on encryption at rest and on audit-log retention,
  the rows say so instead of implying a requirement.

- **The sandbox dataset pins 231 accepted / 91 refused ADL 2 archetypes**
  (#3315). Beside a terminology server that serves LOINC, two archetypes of the
  2013 CKM corpus are refused by the VETDF check for LOINC answer codes that do
  not exist: `apgar` binds fifteen codes without their check digit
  (`LA6713` … `LA6727`) and `braden_scale` binds the mistyped `LA9605-4`. The
  sources stay verbatim, the manifest records the adjudication, and the
  getting-started page counts 233 archetypes.

## [4.2.4] - 2026-09-12

### Fixed

- **ADL 2 archetypes with SNOMED CT or LOINC term bindings upload against a
  terminology server that does not serve those code systems** (#3311). The
  VETDF check asked the server about `system=SNOMED-CT&code=<the binding URI>`
  and read every miss as "the term does not exist", so with FerroTERM beside
  the CDR six archetypes of the sandbox dataset were refused and the seed
  failed. A binding target in the IHTSDO URI model (`http://snomed.info/id/…`,
  `http://loinc.org/id/…`) is now taken apart into the FHIR system and code the
  server is asked about, and the server's `OperationOutcome` decides the
  outcome: a code system it does not serve leaves the binding unverified and
  the archetype accepted with a warning, as AOM2's VETDF prescribes; a code
  absent from a served system is still refused with `422`. Verified against a
  real FerroTERM in the test suite.

## [4.2.3] - 2026-09-12

### Added

- **FerroTERM ships with the quickstart and runs beside the sandbox CDR**
  (#3304). `docker-compose.terminology.yml` starts the Ferro family's FHIR
  terminology server next to the CDR and switches `[terminology.external]` on,
  serving the licence-free shaped seed out of the `ferroehr` image by default and
  any edition you build into its index volume (`ferroterm-build`, the
  `terminology-build` profile). The hosted sandbox runs the same pair on one
  machine with the terminology server reachable only from inside the stack,
  which is the posture the SNOMED CT Affiliate Licence asks of a public system,
  and its dataset gains a composition whose coded text resolves through it. The
  deployment probe observes the overlay off, on and resolving, and the server
  down under both fail postures. FerroTERM is pinned at 0.1.3, whose image
  carries its own `HEALTHCHECK` (`ferroterm healthcheck`), so both compose files
  start the CDR only once the terminology server reports healthy (#3307).

## [4.2.2] - 2026-09-12

### Fixed

- **A wiped clinical schema comes back with its cold-tier alias views**
  (#3298). The three `ehr.cold_*` views the storage layer reads through were
  created by the demographic migration set, so a database whose `ehr` schema
  was dropped and rebuilt (the hosted sandbox after every nightly reset since
  4.2.0) came back without them, and every composition or `EHR_STATUS` update
  answered `500` while creates and reads worked. A new clinical migration
  recreates the views whenever the demographic set has run, and widens a view
  created before `vo_version.origins` existed. The sandbox reset now drops all
  seven schemas, so a reset is a fresh start rather than a partial one.
- **The compose quickstart role can reach the linkage domain.** The database
  init script granted the application login the clinical and demographic
  domain roles but not `ferroehr_linkage`; it worked only because that login
  owns the database.

## [4.2.1] - 2026-09-12

### Fixed

- **Identifiers carry the licence stamp the design specifies** (#3282). The
  stamp keyed its HMAC with a hash of the key material and used a fail-safe
  constant of its own, so no identifier a 4.2.0 server minted could be
  attributed to its licence by the attestation tooling; the key is now the raw
  material and the unlicensed constant is the design's. Identifiers minted by
  4.2.0 stay valid and simply carry no recognisable stamp.

## [4.2.0] - 2026-09-12

### Added

- **The storage-parity sweep reports rows an older release decomposed**
  (#3273). A fifth `defect` value, `stale_decomposition`, on
  `POST {base}/admin/integrity/verify` and its NDJSON stream: the rows
  reassemble into exactly the stored document, so nothing is damaged, but they
  are not the rows this version writes for it. The sweep now decomposes each
  version's document a second time and compares, which is the only way that
  case is visible; `POST {base}/admin/integrity/rebuild-nodes` repairs it like
  any other finding. A release that changes how content is decomposed is
  therefore an upgrade step an operator runs and verifies, instead of a shape
  that waits for each object's next commit.

- **Cohort queries across the pseudonymisation boundary** (#3159). A new
  `POST /query/cohort` runs an AQL query over a population selected in the
  demographic domain: a predicate from a deployment-declared allow-list
  (`[cohort.predicates]` — `city`, `postcode_area`, `sex`, `age_band`,
  `organisation`, each naming an archetype, an at-code and how a value is
  matched) selects parties, the linkage domain resolves those parties to the
  EHRs they are the subject of, and the caller's AQL runs scoped to exactly
  those EHRs. The three steps run on three separated database credentials and
  carry identifiers only — no name, address or party id reaches the caller. A
  result set serving fewer distinct EHRs than `cohort.small_cell_threshold`
  (default `5`) has its rows withheld and is marked suppressed rather than
  looking empty; a predicate matching more than `cohort.max_cohort_size`
  parties (default `100000`) is refused `422` rather than truncated. Every
  execution, suppressed and failed ones included, records one `linkage`-domain
  access event naming the cohort by a digest of its predicate list and never by
  its values. The surface is off until a predicate is bound, and the route
  answers `404` until then.

- **AQL EHR scoping binds one array instead of one parameter per id.** A query
  scoped to a set of EHRs now emits `ehr_id = ANY($n)` with a single uuid-array
  bind, lifting the PostgreSQL 65535-parameter wire ceiling that previously
  bounded how many EHRs one query could name. Behaviour is otherwise identical.
  The per-EHR served-row count behind the access records and the existence
  check of a scoped id list are now keyed rather than probed, so a population
  page of a hundred thousand rows from as many EHRs is served in linear time
  (the 100 000-EHR cohort measured at fifty seconds before, under a second in
  the database itself).

- **The viewer shows the connected CDR's deployment profile** (#3264). The
  header's status chip carries the declared profile (`· production` or
  `· sandbox`), and a sandbox deployment raises a persistent notice under the
  header on every authenticated screen stating that it has not made the
  production separations and must not hold real patient data, naming the open
  separations the server reported. Production renders quietly; a server that
  declares no profile changes nothing.

- **Deployer-facing privacy documentation** (#3161). Three pages under
  Security: a data protection impact assessment (processing description, data
  categories per schema, roles, retention including the Dutch five-year
  access-log floor, a risk register, and the shipped controls mapped to GDPR,
  EHDS, EDPB 01/2025 and NEN by closed issue), an Article 30 records-of-processing
  template pre-filled with the activities FerroEHR performs by design, and a
  go-live checklist linked from the production checklist.

- **The threat model, architecture and storage pages describe the
  pseudonymisation boundary as shipped** (#3166): what each database credential
  and each backup artefact can reach, the re-identification paths and the
  control against each, which service module runs on which pool and role, and
  all seven schemas including `linkage`.

- **The FHIR page records the priority-category mapping decision** (#3206):
  profile mappings are not authored here and belong to FerroBRIDGE once the
  EHDS Article 36 implementing acts fix the exchange format; the trigger that
  reopens the question is stated, and the two priority categories with no
  committed template are recorded as an adjudicated corpus boundary.

- **The access log records the origin of the data it served** (#3212, EHDS
  Annex II 3.2(e)). Every committed version now carries the distinct set of
  `FEEDER_AUDIT.originating_system_audit.system_id` values found in its body,
  or this server's own system id when it carries none (`vo_version.origins`,
  derived at commit; an unstamped row is assessed at read). A composition,
  status or folder read records the origins of the version it served, an AQL
  execution the union over the version rows its result page came from, each
  with the true distinct count beside a set capped at 32; the FHIR
  `AuditEvent` carries one named entity per origin and ITI-81 serves it.

- **The server mints the subject pseudonym** (#3232). Where FerroEHR itself
  makes a party the subject of an EHR, `service::linkage::link_as_subject`
  derives the pseudonym (a keyed, tenant-bound derivation over the party under
  the linkage key, an RFC 9562 custom UUID), writes it as the EHR's
  `EHR_STATUS.subject.external_ref` in the first declared namespace, and opens
  the mapping; no caller-supplied value enters that path. Minting refuses,
  typed, when no namespace is declared or no identifier-protection key is
  configured.

- **The access log has a retention floor per jurisdiction** (#3242). A
  non-zero `audit.store.retention_days` below the floor a jurisdiction the
  active identifier rules name publishes is a boot error naming both numbers.
  Registered: NL, five years (`1830` days) under the Besluit vaststelling
  bewaartermijn logging. `0` keeps forever and always passes; the effective
  retention is on the boot line and on `/management/info`.

- **A declared deployment profile** (#3226). The new top-level
  `deployment_profile` key (`sandbox`, the default, or `production`) says what
  a deployment may hold. `production` refuses to start while a separation it
  asserts is missing and not accepted by name in `deployment_accepts`:
  separated database credentials, separated clusters (read from
  `pg_control_system().system_identifier` on each pool, never from the DSN
  text), a declared subject pseudonym namespace, an audit trail with a durable
  sink, schema preparation on its own credential. `sandbox` names every missing
  separation on the banner, at `warn` with a structured `posture =
  "deployment"` field, and on `GET /rest/status` under `deployment`. Our own
  posture, not a legal requirement; the documentation says so and why.

- **The database guards the subject pseudonym** (#3241). Once
  `privacy.subject_namespaces` is declared, a trigger on `ehr` refuses a
  subject reference that is not a UUID whichever code path or session writes
  it; the server stamps the posture on every boot and reports how many stored
  EHRs a newly declared namespace finds already carrying a non-pseudonym
  subject.

- **A subject-scoped read of the access log** (#3240). `GET /fhir/r4/AuditEvent`
  was admin-only, so a portal serving a person's right to know who accessed
  their record (GDPR Art. 15, EHDS Art. 9) had to hold an admin credential
  over every patient's log. A new `authz.rbac.subject_audit_role` reads the
  log for one subject at a time, `patient` required and refused with a `403`
  without it; the admin keeps the unscoped retrieval, and reading a subject's
  log is itself recorded as an access naming that subject. Unset, nothing
  changes.

- **The access record carries the caller's roles** (#3239). NEN 7513 asks
  under which role or authority a person accessed a record; the RBAC layer
  held the caller's roles and the record dropped them. Every access event,
  the request-level ones and the domain-level linkage, subject, identifier
  and EHR-Extract records alike, now stores the roles as a JSON array
  (`audit.audit_event.roles`, `NULL` for an unauthenticated refusal), renders
  them as FHIR `agent.role` on the requestor and as a `RoleIDCode` per role on
  the DICOM source participant, and serves them through ITI-81.

- **Auditing off and `fail_mode = "open"` are said where an operator looks**
  (#3238). Auditing disabled used to be one info-level line; the fail-open
  default was silent. Both are now a `warn` at boot with a structured
  `posture = "audit"` field, the `audit_sender` readiness indicator is
  registered whether or not auditing is on (`DEGRADED` with the consequence
  stated when it is off, `UP` with `fail_mode`, the local store and retention
  otherwise), and `GET /management/info` carries the `audit` posture. The
  audit page states at its top what each posture means.

- **The licence in force is explicit, verified and reported.** Every build
  embeds the licensor's `non-commercial` grant (what BUSL-1.1 gives everyone);
  a `commercial` licence is installed with the new `[licence] file` key
  (`FERROEHR__LICENCE__FILE`), an OpenPGP-signed token verified at boot against
  the licensor's embedded public certificate. The server behaves identically
  under either grant: the outcome is logged once at boot and served on
  `GET /rest/status` as `licence: {state, use, licensee, not_after,
  configured_token}`. Every identifier the server mints (EHR ids, versioned
  object ids, contribution ids) carries sixteen keyed bits derived from the
  licence id in the trailing bytes of its UUIDv7, so a record says which grant
  it was written under; the bits replace random bits only, never the timestamp
  or counter, and carry no patient data. A configured token that does not
  verify leaves the embedded grant in force and is reported as `refused`.

- **The linkage resolution service: one path from a person to a record, over
  two credentials, under audit** (#3158). `ferroehr::service::linkage` opens,
  resolves, merges and splits the party-to-EHR mappings the previous increment
  created the schema for. `resolve_ehr_for_identity` is the only path that
  crosses from an external identity to an EHR, and it crosses in the
  APPLICATION: the sealed identifier map answers with a party on the
  demographic pool, the linkage map answers with an EHR on the linkage pool,
  and no statement on either could perform the join, because no credential
  holds both schemas. Merge and split are period-closing writes in one
  transaction — the mapping in force gets an end date and its successor opens
  at the same instant — so no mapping is ever deleted and which party was the
  subject of an EHR when a composition was written stays answerable. Every
  operation records a `linkage`-domain access event naming the actor, the
  purpose of use the caller declared and whether anything matched; the
  resolved EHR is deliberately counted and not named, because the audit trail
  lives outside this domain's role and a party/EHR pair there would be a
  second copy of the map. `GET /ehr?subject_id&subject_namespace` is
  unchanged on the wire and still resolves through the clinical columns — it
  matches an opaque pseudonym, not an identity — but now records a
  linkage-domain access event of its own, because who resolved a subject to a
  record is worth knowing wherever it happened.

- **`[db] linkage_url`, the third runtime credential** (#3158). Alongside
  `url` and `demographic_url` (and `migrate_url` for schema preparation), a
  deployment can now point the linkage pool at its own DSN, with the
  `linkage_url_file` mounted-secret sibling every other credential has;
  setting both is a boot error, and leaving it unset shares the clinical DSN
  exactly as before. The Helm chart carries it as
  `database.linkageExistingSecret` / `linkageExistingSecretKey` /
  `linkageUrl`, mounted as a file so only its path reaches the pod's
  environment (chart 8.2.0). With all three set, no credential the server uses
  can re-identify a record in SQL.

- **A third pseudonymisation domain: the `linkage` schema holds the
  party-to-EHR map** (#3158). A migration adds the `linkage` schema, the
  `linkage.party_ehr` table and the `ferroehr_linkage` role. The table records
  which demographic party is the subject of which EHR, temporally: a
  PostgreSQL 18 `WITHOUT OVERLAPS` primary key admits one open mapping per
  party per tenant at any instant, so a merge or a split closes a row and
  opens another instead of deleting history. It carries identifiers only — no
  name, no address, no plaintext identifier — because a row is already the
  additional information that re-joins a pseudonymised record to a person
  (GDPR Art. 4(5)). The grants run in both directions: the clinical and
  demographic roles are revoked from `linkage`, and `ferroehr_linkage` is
  revoked from `ehr`, `cold`, `demographic` and `cold_demographic`, so no one
  credential holds the map and either side of it. The boot gate
  (`verify_domain_isolation`) now covers all five runtime roles and refuses to
  serve when any of them can read across. Nothing reads the new schema yet;
  the resolution service is still to come. Upgrading installations get an
  empty schema and no data movement.

- **A third backup: the `linkage` domain is dumped on its own, and the
  deployment probes cover its boundary** (#3220). The Compose `backup` profile
  gains `ferroehr-backup-linkage` (`FERROEHR_BACKUP_LINKAGE_DIR`, defaulting to
  `./backups/linkage`) and the chart gains a third CronJob under
  `backup.linkage`, with its own schedule, claim and BYPASSRLS credential. It is
  deliberately a separate artefact rather than a place in the demographic dump:
  `linkage.party_ehr` is the map that re-identifies a pseudonymised record, so a
  file carrying it beside either side of that map rebuilds the join the split
  exists to withhold (GDPR Art. 4(5)). The chart's render guards are now
  pairwise — any two domains naming the same claim is refused, not just the
  original pair — and the operations page documents three dumps and a restore
  that supplies all three. The linkage dump carries `--extension=btree_gist`,
  because a `--schema` dump carries no extension and `linkage.party_ehr`'s
  temporal primary key is a GiST index over that extension's operator classes:
  without it the table restores with its rows and without the key, and
  `pg_restore` reports that as an ignored error. Enabling `backup` therefore
  now requires a claim and a Secret for the linkage domain too; a values file
  that sets only the first two is refused at render, naming what is missing —
  a values contract change, so the chart goes to 8.0.0.

- **The audit trail records the accessing organisation, and the declared
  purpose reaches the FHIR export** (#3204). An access record now carries an
  `organisation` column: the organisation the caller acted for, read from the
  access token's `[authz.abac] organization_claim` — the same claim the ABAC
  layer decides on, so no second setting can name a different one, and
  recorded whether or not that layer is switched on. It answers EHDS Annex II
  3.2(a) beside the natural person of 3.2(b), and where no claim is configured
  or the caller used Basic authentication it stays empty rather than being
  guessed. The FHIR `AuditEvent` export renders it as a second `agent`
  referencing an `Organization`, and the purpose of use as
  `agent.purposeOfUse` on the requesting person's agent, coded with the
  deployment's own vocabulary and no code system. The DICOM syslog form
  carries neither: PS3.15 §A.5 defines no organisation attribute and no
  purpose element. Existing records keep a NULL organisation — the column is
  added by a migration and nothing is backfilled.

- **Which EHDS priority categories the FHIR façade carries, measured rather
  than asserted** (#3171). The FHIR chapter now carries a generated matrix of
  the six Annex I priority categories: which has a committed CKM template,
  which FHIR resource a profile mapping would target, and whether the
  transform is proven end to end. Four of the six — patient summaries,
  electronic prescriptions, imaging reports and laboratory results — have a
  real committed template whose example composition flattens and reverse-maps
  into a FHIR resource carrying its own values, asserted per category with the
  mapped leaf derived from the flat map rather than written into the test.
  Electronic dispensations and discharge reports have no committed template,
  and the table says so. It also says the three things it does not establish:
  no profile mapping ships for any category, the test exercises the transform
  rather than the endpoint, and none of it is conformance to an exchange
  format that has not been adopted. Those gaps are #3206.

- **The EHDS logging elements are mapped onto the audit trail, gaps included**
  (#3170). Annex II 3.2 of Regulation (EU) 2025/327 lists five things the
  European logging software component must record on every access event. The
  audit page now maps each of them onto an access-event field and onto both
  renderings the trail leaves the server in, and says plainly where there is
  nothing: the accessing organisation (a) and the origin of the data (e) have
  no field, the category (c) is an openEHR resource class rather than an
  Annex I priority category, and the declared purpose of use is stored but
  reaches neither export. Every row is asserted by a test — **including the
  gaps**, which fail the day the field arrives, so the table and the code
  cannot drift apart in either direction. The retention default and the two
  export routes are documented beside it, with the reason `retention_days = 0`
  keeps records forever rather than choosing a horizon for an operator.
  Closing the gaps is #3204.

- **Two EHDS readiness pages, generated from one source** (#3169). Chapter III
  of Regulation (EU) 2025/327 requires an EHR system to include two harmonised
  software components, meet the essential requirements of Annex II, and carry
  technical documentation and an EU declaration of conformity. The
  documentation site now states, requirement by requirement, what FerroEHR
  provides today: an EHDS readiness page with a status and evidence for each
  of the 14 Annex II requirements, and a technical-documentation page
  mapping each Annex III element to material that exists. Both are rendered
  from a committed YAML by `scripts/render/ehds-pages.sh`, and a CI job
  re-renders and diffs, so a status typed into a page fails the build. Both
  pages say plainly that no conformity assessment has been carried out, no
  technical documentation has been drawn up and no declaration of conformity
  exists — a "shipped" row is a statement about the software, never a claim of
  conformity.

- **`openehr-its` compiles to `wasm32-unknown-unknown` with the flat and OPT
  code in it** (#3149). The crate's one `full` feature dragged axum, moka and
  jsonschema in with the parsers, so a browser consumer that wanted
  `flat::build_web_template` or the OPT 1.4 reader had to take an HTTP server
  and a cache too — and `default-features = false` left only the SMART scope
  grammar. The features are layered now: `json` → `xml` → `opt14` → `flat` is
  the parsing spine, and `cache` (moka), `schema-validation` (jsonschema) and
  `rest-server` (axum, http, async-trait) are the layers a browser declines.
  `full` is still the default and still means all of them, so no existing
  consumer sees a change. A CI lane builds the wasm selection, and a second one
  builds the dependency-free `rest::smart_scopes` island, so both are checked
  rather than claimed.

- **Backups are taken per pseudonymisation domain** (#3157). A single
  `pg_dump` of the whole database produces one file holding both the
  pseudonymised clinical record and the identities of its subjects, and
  whoever can read that file can re-identify every record in it. The Compose
  stack gained an opt-in `backup` profile with one job per domain, writing to
  two separate directories (`FERROEHR_BACKUP_CLINICAL_DIR`,
  `FERROEHR_BACKUP_DEMOGRAPHIC_DIR`), and the operations page carries the
  `pg_dump --schema` recipe, the restore procedure, and the two properties no
  configuration can enforce for you: different access control on the two
  targets, and a credential per job that reaches one domain. Point-in-time
  recovery stays instance-wide and the page says so. A backup credential needs
  `BYPASSRLS`: every tenant-scoped table carries `FORCE ROW LEVEL SECURITY`, so
  `pg_dump` refuses a table it would read through a policy and the application
  role cannot take a backup at all — and `--enable-row-security`, which makes
  the dump succeed, quietly writes a single tenant's rows. The chart therefore
  requires a backup Secret per domain and refuses to render without one.
  `ferroehr db verify` is the restore gate — it issues no DDL and refuses a database whose grants let
  a runtime role read across the boundary, which is the same check the server
  runs at boot.

- **The deployment artifacts can reach the demographic role split** (#3179).
  The chart gained `database.demographicExistingSecret` (and its
  `demographicExistingSecretKey` / inline `demographicUrl` siblings), mounted
  as a file exactly like the clinical DSN so neither credential enters the
  pod's environment. Set it and the schema separation becomes a credential
  separation: one leaked connection string then reaches one domain rather than
  both. Leave it unset and both pools share the clinical DSN, which is the
  schema-only posture the boot self-check still verifies. The compose stacks
  now provision all four `NOINHERIT` domain roles — previously the migrations
  skipped them with a `NOTICE`, because the compose migrator holds no
  `CREATEROLE`, so those stacks ran with no domain grants at all — while
  deliberately keeping one login credential, which the init script says in
  place. `scripts/deploy-probe.sh` gained a family that reads the posture out
  of the database catalogue rather than the compose file: the roles exist, each
  is `NOINHERIT`, no clinical role can read a demographic relation, and the
  stack is single-credential on purpose. What it cannot show — a two-DSN
  deployment, and role provisioning on a managed database — it declares as not
  exercised.

- **National identifiers in the demographic domain can be held sealed**
  (#3155). With the new `[demographic.identifier_protection]` section on, an
  identifier of a configured scheme never sits in the versioned body: the value
  moves to `demographic.national_identifier`, sealed with AES-256-GCM under a
  key derived per tenant from one configured root key, and the body keeps a
  reference in its place. Beside the ciphertext sits an HMAC-SHA-256 digest, so
  "which party holds this identifier" is answerable without decrypting
  anything — and, being keyed, it cannot be reversed by enumerating a
  nine-digit space the way an unkeyed hash could. Sealing runs before the body
  is decomposed and signed, so the stored, signed and served body stay one
  form. Only the demographic writer reaches the sealed value; the read-only
  twin is refused the ciphertext and the digest by column-level grant and the
  clinical roles are refused the table outright. Resolution goes through a
  `SECURITY DEFINER` function that takes the digest rather than the value, and
  every call is recorded as a `linkage`-domain access naming the scheme and
  whether it matched, never the value. Off by default, and enabling it without
  a key — or with a scheme list that would protect nothing — is a boot error.
  See [Privacy & data minimisation](https://ferroehr.eu/book/installation/config-privacy.html#protecting-national-identifiers-in-the-demographic-domain)
  and the key-rotation runbook in
  [Operations](https://ferroehr.eu/book/operations.html#rotating-the-national-identifier-key).

- **Reads are logged per record, not just per operation** (#3156). Every
  retrieval in the EHR, Query and Demographic APIs already produced an audit
  record; each one now also says which pseudonymisation domain it read (`ehr`,
  `demographic`, `linkage`, or `system` for an operation that touches no
  domain), the purpose of use the caller declared, the legal basis the
  deployment processes under, how many records were served, and the
  `x-request-id` of the request it belonged to. An AQL statement additionally
  produces **one access record per EHR it served**, each with that EHR's
  served-row count, because one execute record over a population query cannot
  answer whose record was read. The served set is read off the rows the caller
  actually received rather than the rows the statement matched: a legal record
  of access must not name an EHR whose content was never disclosed.
  `SELECT DISTINCT` and aggregate projections carry no per-EHR breakdown — the
  shape makes it underivable — and record the statement and its row count.
  Two new keys configure the declaration surface: `[audit] purpose_header`
  (default `x-purpose-of-use`) and `purpose_codes`, an allow-list that keeps
  unagreed free text out of the trail, plus `[audit] legal_basis`. A test walks
  the generated route tables so a newly generated `GET` that emits nothing
  fails the build, and the trail's append-only property — no rewrite, no
  delete outside the retention reaper, no truncation — is now asserted rather
  than assumed. See
  [Audit trail](https://ferroehr.eu/book/audit.html#reads-are-logged-too-per-record),
  which maps the NEN 7513 event content onto the recorded fields.

- **The clinical side refuses identifying data, and the subject reference is
  bound to a pseudonym.** A new `[privacy]` configuration section carries three
  write-path rules, all reported as `422` with one entry per finding naming the
  RM path and never the offending value. `privacy.subject_namespaces` names the
  pseudonymisation domains this deployment issues subject pseudonyms in; once
  one is declared, `EHR_STATUS.subject.external_ref` must name a listed
  namespace and carry a UUID. `privacy.allow_identified_parties_in_ehr`
  (`false` by default, announced at boot when on) decides whether a
  `PARTY_IDENTIFIED` or `PARTY_RELATED` in clinical content may carry `name` or
  `identifiers` — with it off they carry `external_ref` only, which still
  satisfies the RM's own validity rule. `[privacy.identifier_scan]` checks every
  string leaf of every clinical write against a jurisdiction-keyed ruleset,
  refusing in `strict` (the default) and recording in `warn`. The shipped rules
  are `fi-hetu`, `gb-nhs-number`, `nl-bsn`, `no-fodselsnummer` and
  `se-personnummer`, each transcribing the checksum its own issuing register
  publishes, and all are active by default; a deployment narrows the list to its
  own jurisdictions and adds `patterns` for local medical-record numbers and
  address forms. Every rule publishes how often it claims a value that is not
  one of its identifiers, and those figures are measured rather than estimated:
  one in 11 for `nl-bsn` and `gb-nhs-number`, one in 31 for `fi-hetu`, one in
  120 for `no-fodselsnummer`, one in 139 for `se-personnummer`, and zero
  findings across the 258 clinical documents in this repository's vendored
  corpora. See
  [Privacy & data minimisation](https://ferroehr.eu/book/installation/config-privacy.html).

- **Demographic parties live in their own schema, behind their own database
  role** (#3153). Parties (PERSON, ORGANISATION, GROUP, AGENT, ROLE,
  PARTY_RELATIONSHIP) and their change control leave the clinical `ehr` schema
  for a new `demographic` schema with its own `cold_demographic` archival tier,
  and the database refuses the mix in both directions: a version with no owning
  EHR cannot enter `ehr`, and one that has an owner cannot enter `demographic`.
  Four runtime roles replace the single `ferroehr_app` pair:
  `ferroehr_ehr`, `ferroehr_demographic`, and a read-only twin of each. Each
  holds explicit grants on its own domain and carries an explicit revoke on the
  other, all four are `NOINHERIT`, and none is a member of another. The schema
  separation is unconditional. The new `[db] demographic_url` (or
  `demographic_url_file`) turns it into a credential separation as well, by
  pointing the demographic pool at its own DSN. **The server now refuses to
  boot when the grants are wrong**: a self-check enumerates every table, view,
  sequence and function in each domain and fails, naming the role and the
  object it can reach. `ferroehr db verify` runs the same check, and it passes
  when the roles do not exist, which is the development, compose and
  test-harness case. The ITS-REST Demographic API, the RM change-control
  semantics and every version identifier are unchanged. GDPR Art. 4(5) and
  Art. 32(1)(a), EDPB Guidelines 01/2025; no openEHR spec governs storage
  layout or database roles. Existing installations migrate in place: the
  upgrade moves the parties by kind, and refuses before moving anything if it
  finds an EHR-less row of a kind it does not classify.

- **The documentation site carries a compliance control matrix, generated from
  the tracker** (#3164). A hand-kept compliance table is wrong the first time a
  control ships, so this one is not kept by hand: an issue declares the control
  it delivers as a `Control: <legal source> <article or clause>` line in its
  body, and `scripts/render/control-matrix.sh` renders
  `website/book/src/compliance/control-matrix.md` from the tracker. Each row
  shows the legal source as a link to its official publisher, the article or
  clause, the issue, whether the control is shipped, in progress or planned,
  and the pull request that closed it. Short names resolve through a registry
  inside the generator, so a citation on the page can never be free text: an
  undeclared source stops the render and names the issue. A CI job re-renders
  and diffs, so a control that shipped since the last render fails the build
  instead of sitting on the public site as "planned".

- **The documentation site has a compliance section** (#3163, #3165). Two
  pages join the generated control matrix. The compliance overview states the
  pseudonymisation boundary, then walks GDPR, the EDPB Guidelines 01/2025 on
  pseudonymisation, the EHDS, the Dutch UAVG and Wabvpz, and NEN 7510, 7512 and
  7513, saying per requirement what the product ships, what is planned and
  under which issue, and what the deploying organisation still has to do. The
  shared-responsibility page draws the same line obligation by obligation,
  because most duties in these texts rest on the controller and the processor
  and cannot be met by software. Both pages carry the wording policy of the
  compliance programme: FerroEHR aims to be the first openly developed,
  source-available openEHR CDR with a published, tracker-backed EU compliance
  posture and an EHDS conformity self-assessment on its roadmap, and it claims
  no certification, no conformity and no compliant deployment. The
  schema and role split is described as shipped; the resolve map, the
  identifier constraints and the per-domain logging are described as planned,
  with their issue numbers.
  Every legal source is linked to its official publisher, and every openEHR
  statement to the published specification section it comes from.
- **The storage-parity sweep has a repair to go with it** (#3143).
  `POST /admin/integrity/verify` reported that a version's decomposed rows and
  its stored document disagree, and then there was nothing to run: dump and
  load was the only re-decomposition path, and its dump side reads each version
  through the node rows themselves, so it reproduced the damage it was asked to
  fix. `POST /admin/integrity/rebuild-nodes` re-derives the rows of every
  damaged version from that version's stored document, one transaction per
  version. It runs the same sweep first and writes only what the sweep reports
  damaged, so a run over healthy data writes nothing. Repair goes in one
  direction only: the stored document is the canonical serialized form a point
  read serves and a digest was taken over, the rows are an index derived from
  it, so a version whose document is the damaged copy is refused rather than
  having the damage copied into the index. Each version is read under a row
  lock, decomposed, its rows replaced, and the version re-derived from the new
  rows and compared with the document before the commit; anything that fails
  rolls that version back untouched and the run continues. An archived object
  is thawed and re-archived in the same transaction, marker included. A
  logically deleted version rebuilds to no rows.
- **Contributor rules for personal data, and a guard that holds them** (#3167).
  `CONTRIBUTING.md` and the contributing page of the book gained a "Personal
  data" section: synthetic data only in tests and fixtures, no identifiers in
  logs, traces or `Debug` output, no database grant spanning the clinical and
  demographic domains, and a review step for any change at the privacy
  boundary. The pull request template carries the matching "Privacy boundary"
  checklist, and the new `privacy-boundary-guard` CI job fails a pull request
  that touches the boundary paths without a ticked checklist. The same job
  reads the added lines of every diff and refuses a nine-digit value passing
  the BSN eleven-test or a Dutch postcode followed by a house number, proving
  both detectors against passing and failing fixtures before it judges
  anything. `.claude/rules/personal-data.md` carries the rules for agents.

### Changed

- **The control matrix states the control, not the defect** (#3249). An issue
  titled as the defect it fixed may carry a `Control-text:` line beside its
  `Control:` declaration, and the generator renders that text in the Control
  column; seven shipped rows that read as defects beside "Shipped" now read
  as controls. The Helm chart's linkage backup comment and the deployment
  probe's coverage note describe the three server pools as shipped (#3268).

- **The library's default privacy policy is the shipped one** (#3243).
  `PrivacyPolicy::default()` used to permit identified parties and scan
  nothing, so a service built without an explicit policy ran a more permissive
  posture than the binary; it now compiles to the same refusing posture the
  default configuration does. Embedding hosts and tests that mean to accept
  more install the policy that says so.

- **The control matrix lists a control's closing pull request only once it
  has merged** (#3253). An open pull request that says `Closes #N` no longer
  moves a row, and the pull request that closes a control regenerates the page
  as it will read after the merge (`--closing <pr>`), so the published matrix
  never lags a shipped control and unrelated pull requests stop failing its
  check.

- **The two-DSN posture is exercised, and the deployment probe says exactly
  what still is not** (#3222). Pointing `[db] demographic_url` at a role of
  its own turns the schema separation between the clinical and demographic
  pseudonymisation domains into a credential separation, and nothing ran it:
  every test drove both domains through one login role. A new integration
  suite assembles the server the way the binary does — two DSNs, two pools —
  on one login role per domain against a real PostgreSQL, serves an EHR and a
  party through the wire, and asserts that each of the running server's own
  pools is refused the other domain's relations for want of privilege. The
  single-DSN fallback is covered beside it, so the split stays a deployment
  choice. The Compose deployment probe keeps its honest "not exercised" entry
  for credential separation, now narrowed to what genuinely remains: a real
  deployment booting on two mounted DSNs. No behaviour changes.

- **A default deployment accepts a named composer again** (#3190). With
  `[privacy] allow_identified_parties_in_ehr` off, the clinical side refused
  both `name` and `identifiers` on both party proxy classes, so a server
  running the shipped configuration rejected the very composition its own
  `/definition/template/adl1.4/{id}/example` endpoint hands out — anything
  built from `ctx/composer_name` included. The default now follows what the
  two Reference Model classes actually mean. A `PARTY_IDENTIFIED` name is
  accepted: that class is the proxy for a party "other than the subject of
  the record", "Typically for health care providers", so a composer or
  performer name is provider identity. Still refused: `identifiers` on either
  class, which is the national-identifier slot, and `name` on a
  `PARTY_RELATED`, which is defined by its relationship to the subject and
  may be the subject itself. The configuration key is unchanged and means
  what it always did when set to `true`, so a deployment that opted in keeps
  working, and nothing that committed before this change is refused now.

- **The published model crates no longer carry a random-number generator.**
  The workspace pinned `uuid` with `v4`/`v7`/`fast-rng` for everyone, but only
  the server and the test harness ever mint an identifier; `openehr-base`,
  `openehr-rm` and their siblings parse and serialize them. The unused
  generator is what made those crates refuse to compile for
  `wasm32-unknown-unknown`, which has no randomness source unless one is
  configured — so a browser consumer had to supply a backend for a capability
  none of the crates uses. The generator features now sit on the two crates
  that generate.

- **A key derived for one pseudonymisation domain opens nothing in another**
  (#3157). The per-tenant subkeys protecting national identifiers now take the
  domain as part of their derivation context, so the clinical domain's key
  material cannot decrypt a demographic record or reproduce its lookup digest
  even when both derive from the same configured root key. That is what makes
  a per-schema backup a separate artefact under a separate key rather than two
  files under one. `[demographic.identifier_protection]` has not shipped in a
  release yet, so no stored data is affected; a deployment that enabled it on
  an unreleased build re-seals its identifiers.

- **The Rust toolchain moves to 1.98.1 and the MSRV to 1.97.** 1.98.1 is a
  point release for a vtable-generation miscompilation, which is reason enough
  on its own; the toolchain step also brings the 1.98 lint set, and the
  declared minimum supported version moves with it, one minor behind as
  before. Container builds follow: the builder base is the digest-pinned
  `rust:1.98.1-slim-trixie`, resolved fresh rather than copied. Nothing here
  uses a 1.98 language feature, and the `msrv` job verifies the declared
  minimum independently.

- **Dependencies bumped, and one advisory left the tree with them.** argon2
  0.6, jsonschema 0.53, tower-http 0.7.1, rust_decimal 1.43, futures 0.3.34,
  toml 1.1.5, indexmap 2.14.2, leptos-use 0.19.2, wasm-bindgen 0.2.128.
  `rust_decimal` 1.43 dropped its optional `rkyv ^0.7` requirement, so
  RUSTSEC-2026-0235 — an out-of-bounds read no feature of ours ever compiled,
  but which lock-file scanners reported — is gone from `Cargo.lock` and its
  VEX statement is deleted rather than renewed. The three remaining advisory
  exceptions were re-checked against their upstreams on 2026-09-10 and all
  three still have no fixed release.

- **Database migrations are append-only, so an existing installation upgrades
  in place.** A released migration is never edited again. sqlx records a
  checksum of every applied migration and refuses a database whose recorded
  checksum no longer matches the file, so editing one would not revise history:
  it would stop every server that had already run it from starting, reporting a
  checksum rather than the change that caused it. A correction now arrives as a
  new migration that carries the schema forward, and a CI guard fails any pull
  request that modifies, renames or deletes a migration its base branch already
  carries. This replaces the earlier greenfield policy, under which baselines
  were edited in place and deployments were expected to recreate their volume.

- **Both storage-integrity routes can be pointed at a single version**
  (#3143). `vo_id` and `sys_version` join `ehr_id` and `committed_since` as
  scope parameters on `POST /admin/integrity/verify` and the new rebuild, so
  checking or repairing one record after a support incident no longer means
  reading the repository. The ordinal is per-object, so `sys_version` without
  `vo_id` is a `400` rather than a filter that would match one version of every
  object.

### Security

- **Three tenant-scoped tables enforce tenant isolation that they carried a
  column for but no policy** (#3219). `demographic.national_identifier` holds
  sealed national identifiers and the keyed digests that resolve them;
  `demographic.contribution` and `demographic.audit` hold the change-control
  trail over the demographic domain. All three carried a `tenant_id`, and the
  equality lookups on them were tenant-scoped, so no resolve could ever cross
  a tenant. What was exposed is the unscoped read: a plain `SELECT` by the
  demographic role returned every tenant's rows. Each now enables and forces
  row-level security with the same `tenant_isolation` policy its sibling
  relations have carried since the baseline. Deployments that run a single
  tenant are unaffected, because an unset session resolves to the reserved
  default tenant exactly as before.

  The cause is worth stating, because it will recur otherwise: a baseline
  migration applies row-level security by looping over a list of relation
  names, and a table added by a later migration joins nothing. The omission is
  invisible in the table's own definition. A test now asks the catalog instead
  of a reviewer — any relation carrying a `tenant_id` must enable and force
  row-level security — so the next table added after a baseline is caught by
  the build rather than by an audit. `audit.audit_event` is the one named
  exception: its drain batches many tenants' records in a single insert from a
  task outside any request's tenant session, so a policy there would reject
  audit rows rather than isolate them.

### Fixed

- **A party's contacts, addresses, identities and capabilities are decomposed
  into node rows** (#3273). `CONTACT`, `ADDRESS`, `PARTY_IDENTITY` and
  `CAPABILITY` were stored inline on the party root, so nothing under them
  carried a row and no row-level predicate could reach it — a cohort predicate
  could only be bound to an archetype under the party's own `details`, never to
  the address a party actually lives at. Each container now gets its own row,
  along with the `ITEM_STRUCTURE` and `ELEMENT`s beneath it, and an archetyped
  container scopes its own leaves. `openEHR-DEMOGRAPHIC-ADDRESS.address.v1` is
  bindable in `[cohort.predicates]`; the served body is unchanged.

  **Upgrade step.** After deploying this version, run the storage-parity sweep
  (`POST {base}/admin/integrity/verify`). Every party version stored before it
  reports the new `stale_decomposition` defect: the rows hold the right
  content, in the shape the previous release wrote. Run the node rebuild once,
  unscoped (`POST {base}/admin/integrity/rebuild-nodes`), and the next sweep is
  clean. Until then a predicate bound to a container archetype matches a party
  only from its next commit onward.

- **Every documented least-privilege posture names a credential that can do
  what the page asks** (#3228). `db.migrate = "verify"` reads all five
  `_sqlx_migrations` bookkeeping tables, which no least-privilege runtime role
  can do, `ferroehr_app` included; the Compose, Kubernetes, CLI, server
  configuration and installation pages now name `[db] migrate_url`, and the
  operations page says which postures a test exercises and which none does.

- **The split clinical role can write the audit trail** (#3267). The local
  Audit Record Repository was granted to `ferroehr_app` and `ferroehr_reader`
  only, so a deployment whose clinical credential is a member of
  `ferroehr_ehr` alone wrote no access record at all (dropped under
  `fail_mode = "open"`, every auditable operation refused under `"closed"`);
  `ferroehr_ehr` and `ferroehr_ehr_reader` now hold in `audit` what the
  single-domain pair holds.

- **A CONTRIBUTION into a deactivated EHR reports the state conflict first**
  (#3257). The privacy and content validation ran before the
  `EHR_STATUS.is_modifiable` gate, so a body defect on a record that cannot be
  written was reported as `422` where the `409` was the answer; the state is
  now checked before any member's content, and the locked in-transaction gate
  stays authoritative.

- **Documented: under `[smart] require_smart_scopes = true` a Basic caller
  cannot reach the template family** (#3255). The conformance stack runs that
  posture, which is why its Basic dev users answer `403` on template upload;
  the configuration reference and the templates page say so, and a test pins
  that fail-closed refuses a Basic principal while advisory defers.

- **The management surface is inside the audit trail** (#3244). `/management/*`
  was mounted outside the audit layer, so a runtime filter change through
  `/management/loggers` left no record. Every management request is now
  recorded as a system-domain access under its own operation id
  (`management_info`, `management_env`, `management_loggers_set`,
  `management_loggers_reset`, …).

- **Formal identifiers on a provider proxy are accepted** (#3254). The
  identified-party rule refused `identifiers` on every party proxy, which
  turned away a clinician's registration number on the composer and the
  simplified-format `ctx/participation_identifiers` forms, the Reference
  Model's own paradigm case for `PARTY_IDENTIFIED`. The slot is now refused
  only on a `PARTY_RELATED` whose relationship is `self`; a national-identifier
  value is still refused by the identifier scanner wherever it sits.

- **A named `PARTY_RELATED` is accepted unless it is the subject** (#3252).
  The identified-party rule refused every `PARTY_RELATED` carrying a `name` in
  clinical content, which turned away a composition whose consenting mother or
  guardian is named, content the Reference Model models on purpose and the
  openEHR REST API obliges a server to accept; the conformance suite went from
  1104/1104 to 917/1104 on it. The rule now refuses the name only where the
  relationship codes `self`, the party being the patient; `identifiers` stay
  refused on both classes, and `allow_identified_parties_in_ehr` still opens
  everything.

- **The identifier scan covers the verbatim-replay writes** (#3237). EHR-Extract
  import and the admin archive load stored clinical content exactly as
  received and skipped the data-minimisation pass every other clinical write
  takes, while the configuration reference said every clinical write was
  scanned. Both paths now refuse a record whose body carries a national
  identifier, a non-pseudonym subject reference or an identified party, with
  the same `422` and RM path the commit path reports; nothing is rewritten.
- **The control matrix is populated** (#3236). The generated compliance page
  rendered an empty table because no issue declared a `Control:` line, and the
  staleness check passed on the empty set. Every shipped and planned control
  in the pseudonymisation and EHDS programme now declares the article or
  clause it supports, the page lists all of them with their status, and the
  generator refuses an empty matrix while the compliance pages link to it.

- **`fail_mode = "closed"` now covers the domain-level access records too**
  (#3235). The linkage resolutions and merges, the subject-to-EHR lookup
  behind `GET /ehr?subject_id`, the national-identifier resolution and the
  EHR-Extract export and import each record their own access event, and each
  used to discard the sender's answer, so a record the audit trail could not
  take under `fail_mode = "closed"` left the operation completing unrecorded.
  A rejected record now withholds the result with the same `503` and
  `Retry-After` the request-level record already produces. Under
  `fail_mode = "open"` nothing changes.

- **The documented two-DSN posture boots: schema preparation has its own
  credential** (#3224). The server prepared the schema on the CLINICAL runtime
  pool, and preparation spans every schema — the DDL of all five migration
  sets under `db.migrate = "apply"`, all five `_sqlx_migrations` bookkeeping
  tables under `"verify"`. A role that is a member of `ferroehr_ehr` and
  nothing else — what the book recommends and the chart configures — holds
  neither, so it was refused on the first set and the deployment could not
  start; `ferroehr_app` could not run `verify` either. The new `[db]
  migrate_url` (or `migrate_url_file`, and `database.migrateExistingSecret`
  on the chart) names the credential that prepares the schema, used for that
  one boot step on a connection that is opened and closed again — no pool is
  held on it and no request is served through it. Unset, it falls back to
  `[db] url`, so a single-credential deployment is unchanged. The
  pseudonymisation boot gate deliberately stays on the RUNTIME pool: it
  measures what the serving credential can reach, and asking the migration
  credential — which holds every domain by design — would stop measuring
  anything. A credential that cannot read a set is now refused with the
  schema and the role it authenticates as, and the remedy, instead of a bare
  `42501` about a table no operator has heard of. `ferroehr db verify` splits
  the same way. Chart values contract grows three optional keys, so the chart
  goes to 8.1.0.

- **The template upload dialog opens empty** (#3200). It cleared the source
  it was holding only when the CDR accepted it, so after an upload that was
  refused — or one whose click never reached the handler — re-opening the
  dialog showed the previous attempt's source with the send button already
  live, offering to send something the reader had not just chosen. Opening
  the dialog is now a fresh attempt: the editor starts empty, and the button
  stays inert until a file has actually been read into it.

- **The migration hook's pod is no longer selected by the server's Service and
  PodDisruptionBudget** (#3197). A selector is a subset match, and the hook pod
  carried the server's selector pair plus a `component` label, so for as long
  as it existed it satisfied both — during exactly the install and upgrade when
  a rollout or a drain reads them. It exposed no port named `http`, so the
  Service had no endpoint to route to it, but the PodDisruptionBudget match was
  real and the port was a property of that pod rather than a promise about the
  next one. The hook now carries its own `app.kubernetes.io/name`, as the
  viewer and the backup jobs already do. The chart's selector gate was looking
  only at Deployments and could not have seen this; it now judges every
  workload that carries a pod template, and the migration Job renders in a
  committed value set so the gate has something to judge.

- **A physically deleted party takes its multimedia with it** (#3180).
  `physical_delete_party` removed the party's rows without collecting the
  externalized blob keys they referenced, so a blob held only by that party
  stayed in the object store forever — a storage leak, and on a party also a
  deletion that did not delete, since a blob is content and an operator who
  ran a physical delete has reason to believe it is gone. It now collects the
  keys inside the same transaction and hands them to the same garbage
  collection `delete_ehr` uses, which scans both pseudonymisation domains, so
  a blob a clinical record still references survives.

- **The ADL 2 corpus provenance claimed a licence the files do not state**
  (#3150). `corpus/archetypes/adl2/PROVENANCE.md` said the archetypes were
  "predominantly CC-BY-SA 3.0 where stated" and pointed at the root CC-BY-SA
  3.0 reference copy. Measured over the vendored tree: of 652 archetypes
  exactly one states a licence, and it states CC-BY 4.0. The other 651 carry
  an openEHR Foundation copyright and no grant, and upstream carries no
  `LICENSE` file at the pinned commit either. The record now says that, the
  subtree is declared `LicenseRef-openEHR-unstated` in `REUSE.toml` with its
  own text in `LICENSES/`, and the licensing chapter carries the same
  statement. The vendor script that generates the provenance file was
  repeating the claim and is corrected too, so re-running it cannot restore
  the old wording. The `openehr-adl` regression library from the same
  upstream measured differently and is declared separately: 111 of 302 state
  a licence, predominantly CC-BY-SA 3.0, with CC-BY 4.0 and CC-BY 3.0 present
  and CC-BY-SA 4.0 absent — which the previous glob had asserted over it.
  Whether material whose terms are unstated may stay committed is a decision,
  tracked separately.

- **The storage-integrity sweep and its repair cover both pseudonymisation
  domains** (#3178). Moving the parties into their own schema left the sweep
  reading only the clinical one, so damage to the copy no read-path check
  recomputes went undetected on the domain that holds the identifying data.
  A sweep with no scope now walks both in one pass, and every mismatch,
  every streamed line and every rebuild record names the `domain` it came
  from, so a report cannot describe half the store while looking like it
  described all of it. The repair reaches the damaged version through that
  same domain.

- **The compliance control matrix says which jurisdiction each legal source
  belongs to, and its links work** (#3177). The NEN 7510 entry pointed at a
  page that returns 404, and NEN 7513 pointed at the publisher's homepage
  rather than the standard. Both are corrected, NEN 7512 and the EDPB
  pseudonymisation guidelines are added, and every URL in the registry was
  checked by hand on the day it landed, which is the only way it can be
  checked: the site's link gate runs offline on purpose, so an external URL
  that rots is invisible to it. The table now carries an "Applies to" column.
  `EU` and `INT` apply to every deployment; a two-letter code is national law
  and applies to a deployment in that country. openEHR is not a Dutch
  standard, and a page listing Dutch law beside EU law without saying which
  is which reads as though every deployment answers to both.

- **A FLAT or STRUCTURED read no longer drops the content of a template-named
  event** (#3142). An operational template can fix the name of a node the
  simplified formats collapse away, such as the single event of a `HISTORY`.
  The constrained name then survives only inside the leaf's `aqlPath`, and a
  composition written through the FLAT surface came back short: the reporter's
  body weight and comment were present in the canonical JSON and absent from
  `application/openehr.wt.flat+json`, with no error to say so. Two halves are
  fixed. Building a composition named that collapsed node after its archetype
  node id rather than the name its own template path constrains, so the node
  did not satisfy the path it had just been created for, and every datum
  beneath it became unaddressable. Reading now tolerates it either way: a node
  whose runtime name differs from the template's is still reached by archetype
  node id, exactly as the archetype-conformance validator already did. That
  applies only where the name identifies nothing the node id does not. A
  template that distinguishes same-node-id siblings by name keeps matching
  strictly, so one sibling can never claim another's content. Compositions
  already stored with the wrong name read correctly without being rewritten.

## [4.1.1] - 2026-09-05

### Added

- **A whole large repository can now be checked for storage damage in one pass**
  (#3122). `POST /admin/integrity/verify` computed its whole report before
  answering, so it had to finish inside the server's 30-second request timeout
  and a large enough repository lost the report to a `408`. Sending
  `Accept: application/x-ndjson` streams the same sweep instead: one JSON object
  per line, a `mismatch` as each is found, a `progress` tick per page read, and
  a closing `summary` with the counts. Nothing bounds that response, and the
  stream carries every mismatch rather than the first thousand. The status code
  is committed before the work is, so a sweep that fails part-way ends with an
  `error` line instead of a `summary` — read to the end to tell the two apart.
  A request that does not name the media type, `*/*` included, keeps the
  aggregated document exactly as before.

### Changed

- **A refused operational template lists every violation, not the first one**
  (#3129). Repairing a template meant one upload per defect: a reporter with a
  1.7 MB template uploaded it to learn `use` was empty, fixed it, uploaded again
  to learn `misuse` was too, and a code list with seventeen duplicated codes
  named one of them at a time. `validationErrors` now carries an entry per
  violation, the way a refused composition already did. A violation that makes
  the tree below it meaningless, a constrained type the reference model does not
  have or an attribute its parent does not declare, says that its subtree went
  unchecked, so a short list is never mistaken for a clean one; past 200
  violations the response says it stopped counting.

## [4.1.0] - 2026-09-05

### Added

- **A terminology server that answers with the wrong FHIR resource is refused
  by name** (#3099). The R4B `Parameters` resource has no mandatory member, so
  any JSON object decoded as an empty view: a `$validate-code` answered with a
  `ValueSet` or an `OperationOutcome` read as "no result" instead of the wrong
  answer it was. The decoders now check the `resourceType` first and report
  `unexpected FHIR resource: expected Parameters, got OperationOutcome`, which
  `terminology.external.fail_on_error` then decides on like any other upstream
  fault.

- **A refused OPT or canonical-XML document says where the defect is** (#3067).
  A mandatory element or attribute that is absent used to be reported as
  `xml parse error: missing element id`, with no position and a token that
  matches hundreds of unrelated attributes. The refusal now names the element
  that should hold it with its `xsi:type`, its line and column, its path from
  the root with sibling ordinals, and the class attribute the child realises:
  `element <default_value xsi:type="DV_IDENTIFIER"> at line 4270, column 13
  (/template/definition[1]/…/children[1]/default_value[1]) is missing
  mandatory child <id> (DV_IDENTIFIER.id)`. A value a class constructor
  rejects, and an empty `1..*` container, carry the same location. The
  wording no longer calls a cardinality refusal a parse error.
- **`tenancy.header` is refused at boot when authentication is enabled**
  (#3093). The header override lets a request name its own tenant and wins
  over the JWT claim, so on a deployment with real users any authenticated
  caller could read and write any tenant. The server now refuses to start on
  that pair, naming the key and the remedy, unless the new
  `tenancy.insecure_header_override = true` accepts it explicitly for a
  development deployment. With authentication off, or tenancy off, nothing
  changes.

### Changed

- **The storage-parity sweep reads a page of versions per round trip instead of
  one, and can be scoped** (#3110). `POST /admin/integrity/verify` compared each
  stored version's node rows with its materialized body using two sequential
  statements per version, so a store of 60 000 versions cost about 120 000 round
  trips and the request died at the 30 s timeout with the report lost. Content
  is now read 32 versions at a time, two statements per chunk: the same store
  costs under 4 000 round trips. Two optional query parameters narrow what a
  sweep covers, `ehr_id` and `committed_since`, so verifying one record or
  everything committed since an incident no longer means reading the whole
  repository. The report, its defect vocabulary and its status codes are
  unchanged.
- **A tenant key that names no registered tenant is refused instead of running
  unscoped** (#3111). It used to fall through to the reserved default tenant,
  on the argument that a cross-tenant access should look like an empty result
  set rather than a `403` confirming another tenant exists. That argument holds
  only while the default tenant is empty, and the default tenant owns every row
  written while tenancy was off: on a deployment that enabled tenancy after
  going live, a misspelled claim read and wrote the entire pre-tenancy store.
  Such a request now gets a `403`. Set
  `FERROEHR__TENANCY__UNKNOWN_TENANT=default_tenant` to restore the old
  behaviour where the existence-hiding is worth more. A request carrying no
  tenant key at all is unchanged. With tenancy enabled, boot now counts the
  default tenant's stored versions and warns when it holds any, because both
  paths lead there.
- **A composition commit checks out one pooled connection instead of three**
  (#3097). The EHR-writability gate, the persistent-duplicate gate and the
  commit each took their own connection from the pool. With multi-tenancy on
  every checkout costs a `set_config` round trip that stamps the tenant GUC, so
  the acquire count was the cost: measured over 100 commits, the path went from
  9.3 to 6.3 connection checkouts each. Behaviour is unchanged — the gates run
  in the same order, answer the same statuses, and the commit is still one
  folded statement (or one transaction when attestations or the outbox are in
  play).

- **A large composition no longer holds a request worker through its whole
  validation** (#3097). The RM, terminology and archetype-conformance passes
  are CPU work that ran inline on the async worker, costing about 260 ns per
  JSON node: the largest form the CKM publishes takes 2.5 ms, and at the
  default 16 MiB body limit a single commit could hold one worker for tens of
  milliseconds while every other request on it waited. Above 400 nodes the
  passes now tell the runtime to relieve the worker first. Validation itself is
  unchanged: the same passes run in the same order and refuse the same
  content.

- **`openehr-query`, `openehr-adl` and `openehr-its` move to the Business Source
  License 1.1** (owner decision 2026-09-04). The three hand-written engines (the
  AQL parser, the ADL 2 engine, and the canonical codecs, REST contract and
  Simplified Formats) now carry the same licence as the application, each with
  its own `LICENSE` naming the crate as the Licensed Work; `openehr-its`
  declares `BUSL-1.1 AND Apache-2.0` and keeps the Apache-2.0 text for the
  openEHR-derived codecs, contract and ITS-JSON schema it embeds. The five
  generated model crates (`openehr-base`, `openehr-rm`, `openehr-am`,
  `openehr-lang`, `openehr-term`) stay Apache-2.0. Published versions keep the
  licence they were published with: the three engines are Apache-2.0 up to
  0.0.59 and BUSL-1.1 from 0.0.60.

- **`server.swagger_ui` is an access level, and the Swagger UI needs a credential
  by default** (#3094). The key used to be a boolean that mounted the UI and the
  OpenAPI documents outside the authentication layer, on by default, so an
  unauthenticated client received the complete operation surface, admin and
  message groups included. It now takes the management vocabulary `off`,
  `admin_only`, `private` (any authenticated principal; the new default) or
  `public`, and the non-public levels sit behind the same access guard the
  management endpoints use: unauthenticated is `401` with the `WWW-Authenticate`
  challenge, so a browser prompts for the Basic credential. An existing
  `swagger_ui = true` or `false` is a boot error naming the new spellings;
  the hosted sandbox keeps the UI `public` explicitly. The Helm chart default
  follows (`config.server.swagger_ui: private`).

### Fixed

- **Signing in again no longer lands back on the login card** (#3066). After the
  viewer signed a user out, an answer to a request issued under the old session
  could still arrive while they were signing back in. The transport reads every
  such answer, and the shell treated one as proof the session was over, so a
  freshly signed-in user was sent straight back to the login screen and only a
  full page reload got them past it. The shell now re-reads the session instead
  of trusting the detection, because a detection names no session, so a late
  answer to a dead one is ignored and a real expiry still signs out. The visible
  behaviour on a genuine expiry is unchanged.

- **A large operational template uploads in milliseconds again** (#3114). The
  XML reader resolved every closing element's line and column by scanning the
  document from byte zero, which made a parse quadratic in document size: the
  International Patient Summary template, 1.9 MB, took 10.08 s, and the largest
  form the CKM publishes was on track for about 59 s. A template upload holds
  that CPU on the request, and large uploads were answering
  `408 Request Timeout`.
  The position and the element path are now resolved when a refusal is raised
  rather than on every close, so a refused document names exactly what it named
  before. The same two templates parse in 10.2 ms and 25.1 ms.

- **An explicit `fetch` or AQL `LIMIT` can no longer ask for a page larger than
  `query.max_result_rows`** (#3092). The ceiling used to apply only to a query
  that carried neither bound, so `fetch=10000000` materialised the whole
  matched set into one `RESULT_SET`. A page above the ceiling is now refused
  with `400` naming the requested size and the ceiling; the effective page
  (the smaller of `LIMIT` and `fetch`) is what is checked, and a page at or
  below the ceiling is served as written. The page is refused rather than
  shortened because a client paging with its own `fetch` as the stride would
  silently skip rows.
- **An ADL 2 upload can no longer take the server down through unbounded
  recursion** (#3062). The ADL engine now carries one nesting bound of 512
  levels: the cADL, ODIN and rule-expression readers refuse an artefact nested
  past it with a `400` that names the bound (the `SUNK` syntax bucket), the
  flattener refuses a specialisation lineage longer than the bound or a flat
  form that composes deeper than it, and the operational-template transform
  refuses fillers that reference each other in a cycle (naming the cycle) or
  inline past the bound, as a `422`. Every remaining engine call, including
  compiling a stored template to its operational form and loading the stored
  repository, runs on the dedicated engine thread. Published archetypes and
  templates stay far below the bound.

## [4.0.18] - 2026-09-03

### Changed

- **The licence of the project's own code is the Business Source License 1.1**
  (`LICENSE`, SPDX `BUSL-1.1`; owner decision 2026-09-03). All non-production
  use is free, and production use is free for Non-Commercial Purposes only
  (personal use, academic or scientific research, teaching, and non-profit or
  public bodies outside the course of a business); any other production use,
  including the delivery of health care or another service for payment, and in
  every case offering FerroEHR or a derived work to third parties as a hosted,
  managed or embedded service or distributing it for a fee, needs a commercial
  licence from the Licensor. Each version becomes Apache License 2.0 four years after
  its publication. The copyright holder named everywhere is Ruben Talstra.
  Every SPDX header, manifest, badge, OCI image label, the Helm chart's
  Artifact Hub licence, the citation metadata and every page that named MIT
  follow, and a CI guard now fails on a stale MIT claim in a first-party file.
  Releases v3.0.0 through v4.0.17 stay under the MIT terms they were published
  with. The eight `openehr-*` spec crates on crates.io are NOT under the BUSL:
  they are `Apache-2.0` (`openehr-term` also `CC-BY-SA-3.0` for the
  terminology XML it carries) from 0.0.58, the licence of the openEHR inputs
  they are generated from, so any Rust project can consume them; 0.0.56 and
  earlier stay `MIT AND Apache-2.0` as published.
- Contributions now carry an inbound relicensing grant (`CONTRIBUTING.md`
  § Licensing of contributions): contributors keep their copyright and license
  their work under the project licence, and additionally grant the Licensor the
  right to sublicense and relicense it, so the work stays one work under one
  licensor.
- Every migration file carries a licence header, which changes its recorded
  checksum: a database created by 4.0.17 or earlier refuses to start this
  version with `migration 1 was previously applied but has been modified`.
  The schema is greenfield (see the operations chapter, "Upgrades"), so the
  remedy is to recreate the database, or drop the `ehr`, `ext`, `audit` and
  `cold` schemas, and reload; the hosted sandbox was reset the same way.

## [4.0.17] - 2026-09-02

### Added

- `server.base_path` (`FERROEHR__SERVER__BASE_PATH`) is a supported, validated
  knob: a deployment behind a path-prefixed reverse proxy can shorten the REST
  base path to as little as `/ferroehr/v1` instead of stacking the proxy prefix
  on top of the full default. The API nest, `Location` headers, the System
  Options manifest, the served OpenAPI documents, the Swagger UI, the status
  document and SMART discovery all follow the configured value; the health
  family stays at the process root. The default is unchanged.
- The FerroEHR Viewer gained the mirror key `cdr.base_path`
  (`FERROEHR_VIEWER__CDR__BASE_PATH`, default `/ferroehr/rest/openehr/v1`), so a
  viewer can drive a CDR that shortened its base path.

### Changed

- `server.base_path` is now checked at boot, and a value that breaks a rule
  stops the server with an error naming the key and every rule it broke. The
  first segment must be `ferroehr`, the last must be the openEHR API version
  segment `v1`, and the value must carry no trailing slash, no empty segment,
  and only unreserved URL characters. Values that previously booted and served a
  broken surface are now refused: a trailing slash, a missing `/ferroehr`
  segment, a path not ending in `v1`. The default value is unaffected.
- `ferroehr healthcheck` no longer hard-codes the default REST root: with no
  `--url`/`FERROEHR_HEALTHCHECK_URL` it loads the same configuration as the
  server and probes `http://127.0.0.1:<server.bind port><REST root>/status`, so
  the container health check keeps working when `server.base_path` is
  shortened. The URL it probes under the defaults is unchanged.
- The hosted sandbox (sandbox.ferroehr.eu) now holds a complete demo dataset
  instead of four templates in three EHRs. Each nightly reset loads 16 ADL 1.4
  operational templates from the openEHR CKM pack, 235 ADL 2 archetypes and 5
  ADL 2 templates (two of them source templates the server flattens against
  that archetype library), 8 EHRs carrying 182 compositions with mixed
  records, 6 compositions with more than one version and 2 EHRs with a second
  `EHR_STATUS` version (one not queryable, one not modifiable), 11 demographic
  parties, a FOLDER directory per EHR referencing real compositions, and 5
  stored AQL queries under `eu.ferroehr.sandbox` that all return rows. The
  seeder (`scripts/sandbox/reseed.sh`) still drives only the public REST API
  and is now a walker over `scripts/sandbox/seed/manifest.json`, so adding
  demo content is editing data. The ADL 2 corpus is uploaded parents before
  children, and the split it pins (233 stored, 89 refused by AOM2 validation)
  is the one the corpus gate in `openehr-adl` adjudicates file by file. A run takes 39–45 s against a local stack and
  leaves a 24 MB database; a repeat run detects the marker EHR it wrote and
  exits without changing anything.

### Fixed

- An ADL 2 upload can no longer take the server down. The AOM engine's
  validation now runs on a dedicated thread with a stack sized for its
  recursive walks over the stored repository, and every runtime thread starts
  with a larger stack; previously a deep enough artefact overflowed a worker's
  2 MiB stack and the process aborted with every in-flight request.
- ADL 2 validation no longer reports VCATU (duplicate sibling attribute) for a
  specialised archetype whose root-level differential paths end in the same
  attribute name (`/items` beside `/items[id9]/items`): the two address
  different nodes of the flat parent. Twenty-nine archetypes of the vendored
  2013 CKM export were refused on that false positive.
- An ADL 2 upload whose `specialise` clause names a parent that is not in the
  repository is refused with `422` carrying VASID and the missing parent's id,
  on both validation paths. Previously the parent-conformance checks were
  skipped and the archetype was stored as valid, so a child uploaded before its
  parent was never checked against it.
- An AQL `SELECT` that mixes an aggregate function with a non-aggregated
  column (`SELECT e/ehr_id/value, COUNT(c/uid/value) …`) is refused with a
  typed `400` naming the rule. AQL 1.1 defines the aggregate functions over
  the selected rows and no grouping construct, so the shape has no defined
  result; previously the ungrouped SQL reached the database and its error
  surfaced as a `500`.
- The FerroEHR Viewer no longer leaves an interactive shell over a session that
  has ended. When a session expires or is revoked, the viewer moves the whole UI
  to the signed-out state on its own — no manual refresh — and lands on the
  sign-in screen with "Your session ended. Sign in again to continue." plus the
  screen the user was on, so signing back in returns them to it. Two things
  drive it: any request the CDR or the viewer refuses with a `401`/no-session
  answer signs the UI out immediately (this covers a revocation the browser
  could not have predicted), and the browser also re-checks the session on the
  session's own deadline and on the connection indicator's poll, so an expiry
  nobody clicks through is caught as well. The indicator itself now reads
  "Session ended" rather than blaming the CDR for a refusal that is not its.
  The OIDC sign-in path carries the destination too: `/auth/oidc/login` accepts
  a `next` parameter and the callback lands on it, so an OIDC user is returned
  to the screen they were on instead of the dashboard. Only a same-origin
  relative path is honoured — an absolute URL or a protocol-relative value
  falls back to the dashboard, so the login route cannot be used as an open
  redirect.

- The hosted-sandbox CDR no longer crashloops when the box's `.env` carries a
  `FERROEHR_`-prefixed Compose image variable the server does not recognise.
  The CDR service in `deploy/hosted/docker-compose.yml` stops loading the whole
  `.env` into the container and now receives only `DATABASE_URL` (interpolated,
  and a boot error if unset) plus its config-file path. Compose image variables
  stay substitution-only and never reach the server's strict configuration
  check, so a renamed or stale image variable in the operator's `.env` cannot
  stop the server from starting.

## [4.0.16] - 2026-09-02

### Changed

- The canonical-XML reader and the TDD/OPT XML readers refuse a non-UTF-8
  document at the first read (quick-xml 0.42 validates encoding while
  constructing events) instead of tolerating invalid bytes into text values.
  Well-formed UTF-8 documents — the entire wire corpus — are unaffected; the
  full serialization fidelity battery passes unchanged.
- The generic noun "console" is retired from everything a human reads: the
  Helm chart's `viewer.*` value descriptions (which Artifact Hub publishes),
  the viewer's own on-screen copy, the book, and the doc comments all say
  "viewer" now. The hosted sandbox's Compose service is renamed `console` to
  `viewer` and its container `ferroehr-sandbox-console` to
  `ferroehr-sandbox-viewer`, which the box picks up at the next release
  deploy. The Helm chart is version 7.0.2. The name of the entry the viewer
  keeps inside its encrypted session cookie changes too, so open sessions
  need one fresh sign-in after the upgrade.
- The hosted sandbox (sandbox.ferroehr.eu) runs on a Hetzner CX33 (4 shared
  vCPU, 8 GB RAM, 80 GB NVMe), resized in place from the CPX22. The hosted
  compose memory limits that ship inside the server image move with it: the
  CDR container to 4096m and the FerroEHR Viewer container to 1536m, so the
  services actually use the larger box.
- The sandbox database moved from Neon to a second, dedicated CX33 running
  the published `ferroehr-postgres` image (PostgreSQL 18), reachable only
  over a Hetzner private network — no public database port exists. The
  committed posture gains `deploy/hosted/cloud-init-postgres.yaml`; the
  nightly reseed's schema wipe now runs on the app box through the deploy
  script's new `wipe` verb instead of a `psql` from the CI runner, so CI no
  longer holds any database credential (the `SANDBOX_DATABASE_URL` secret is
  retired).
- The admin console is now **FerroEHR Viewer**, and every name it carries
  changes with it. Deployments must switch their references: the OCI image is
  `ghcr.io/rubentalstra/ferroehr-viewer` (was `ferroehr-admin-ui`), the Compose
  service and profile are both `viewer` (`docker compose --profile viewer up`,
  was `--profile admin-ui`), the Compose image and port overrides are
  `FERROEHR_VIEWER_IMAGE` and `FERROEHR_VIEWER_PORT`, the Helm values key is
  `viewer` (was `adminUi`) and its OIDC client secret mounts at
  `/etc/ferroehr-viewer-secrets`, the config file the viewer looks for is
  `./ferroehr-viewer.toml` then `/etc/ferroehr/viewer.toml` (was
  `ferroehr-admin-ui.toml` / `/etc/ferroehr/admin-ui.toml`), the binary and
  crate are `ferroehr-viewer`, and the session cookie is
  `ferroehr_viewer_session`, so open sessions need one fresh sign-in.
  Documentation moved from `/docs/*/admin-ui/` to `/docs/*/viewer/`, with
  redirects from the old pages. This rename is the chart's 7.0.0 major bump.
- The viewer's own configuration environment grammar is renamed with it:
  every `FERROEHR_ADMIN__<SECTION>__<KEY>` variable becomes
  `FERROEHR_VIEWER__<SECTION>__<KEY>` (so `FERROEHR_ADMIN__CDR__BASE_URL` is
  now `FERROEHR_VIEWER__CDR__BASE_URL`), and the config-file pointer
  `FERROEHR_ADMIN_CONFIG` becomes `FERROEHR_VIEWER_CONFIG`. The chart's
  default `viewer.existingSecretKey` moves from
  `FERROEHR_ADMIN__AUTH__OIDC__CLIENT_SECRET` to
  `FERROEHR_VIEWER__AUTH__OIDC__CLIENT_SECRET`, so an existing Secret needs
  its key renamed or `viewer.existingSecretKey` set to the old spelling. A
  variable left under the old prefix is not read and not reported: the
  viewer's strict deserialization only sees what arrives under its own
  prefix.
- The Helm chart renames the viewer's Kubernetes objects from
  `<release>-admin-ui` to `<release>-viewer` and its
  `app.kubernetes.io/name` label from `<name>-admin-ui` to `<name>-viewer`.
  A Deployment's selector is immutable, so a release that already runs the
  viewer with `adminUi.enabled: true` must have that Deployment, Service,
  ServiceAccount and NetworkPolicy deleted before the upgrade, or be
  reinstalled. The CDR's own objects are untouched.

### Security

- The `ferroehr` and `ferroehr-viewer` images build from the rebuilt
  `gcr.io/distroless/cc-debian13:nonroot` base carrying openssl
  `3.5.7-1~deb13u2`, which fixes CVE-2026-14456 (QUIC denial of service) and
  nine sibling advisories in `libssl3t64`. The library was never linked by the
  shipped binaries (the TLS stack is rustls/aws-lc throughout); the bounded
  scanner exception and its VEX statement are removed now that the fixed base
  exists.

### Fixed

- Composition validation now enforces a template's decimal-precision
  constraint on `DV_QUANTITY` and `DV_PROPORTION` leaves: an instance whose
  optional `precision` attribute falls outside the constrained interval is
  refused (422) instead of silently accepted. Temporal-range constraints
  were already enforced; the module documentation claiming otherwise was
  stale and now states the real contract.
- The FerroEHR Viewer's session cookie sets `Secure` by default
  (`session.cookie_secure` now defaults to `true`, fail closed): a TLS-fronted
  deployment needs nothing, and plain-HTTP contexts (the compose quickstart on
  localhost, local development, the e2e harness) opt out explicitly. If you
  serve the viewer over plain HTTP without one of those postures, set
  `FERROEHR_VIEWER__SESSION__COOKIE_SECURE=false` or logins will not stick.
- TDD import no longer discards instance data a TDD spells out on wrappers
  the WebTemplate compacted: `HISTORY.origin`, an event's `time` and `name`,
  and the other LOCATABLE metadata (`uid`, `links`, `feeder_audit`) now land
  on the re-materialised nodes instead of RM-mandatory defaults, each value
  parsed as its model-declared type. Spelled-out wrapper data that cannot
  legally sit on the corresponding node is refused with a named error rather
  than silently dropped.
- The platform validity checker (`definitions_valid`) now checks archetype
  identifiers, not just template identifiers, per the SM clause it realizes:
  every `archetype_details` declaration in the checked content contributes its
  archetype id and template id, archetype ids resolve through a declared
  template's inlined nodes or against the stored ADL 1.4 and ADL 2
  repositories, and an unknown identifier at any depth now answers `false`.
- The `ETag` served with a query `RESULT_SET` now derives from SHA-256, a
  pinned published algorithm, instead of the standard library's default
  hasher, whose algorithm may change between Rust releases. Query ETags are
  now stable across server builds; every served query ETag changes value once
  at this upgrade.

## [4.0.15] - 2026-09-01

This release carries everything prepared for 4.0.14, which was tagged but
never published: its release pipeline failed before any artifact was built,
and the `v4.0.14` tag is immutable, so the content ships here under the next
patch number. No 4.0.14 artifacts (release, images, chart, crates) exist.

### Added

- The admin console's template Example tab lets you pick how the example is
  generated. Beside the format selector, both template families (ADL 1.4 and
  ADL 2) now offer a detail level (**Required**, **Medium**, **Complete**) and
  the form the example is shaped for (**Input** as submitted, **Output** as
  retrieved). The pane opens on Required/Input, as before, and changing either
  control asks the CDR for a fresh example.
### Security

- The `ferroehr-postgres` image builds from the respun upstream `postgres:18.6`
  base (the tag was re-pointed upstream with rebuilt base packages and bundled
  binaries); the digest pin follows it everywhere it appears. The rebuilt
  candidate scans clean at the published-image gate.

### Changed

- The hosted sandbox (sandbox.ferroehr.eu) moved off Vercel onto a dedicated
  Hetzner server (`deploy/hosted/` is the whole committed posture: cloud-init,
  compose, Caddy TLS, a key-restricted deploy script). No more scale-to-zero
  cold starts or free-compute slowdowns; the same URL, credentials, nightly
  reset and released images. The database is a standalone Neon project on its
  direct endpoint, no longer a platform integration.
- The hosted sandbox (sandbox.ferroehr.eu) now serves the Admin API: the
  nightly-reset demo is a development/testing deployment in the released
  spec's own terms, and the conformance statement's AdminApi claims become
  exercisable against it. The public demo credential reaches the whole admin
  group, bulk delete included — it wipes what the nightly reset wipes anyway.
- The repository's commit history on `main` now starts at a single labelled
  import commit holding the tree FerroEHR was forked from, followed only by
  this project's own commits. The 4864 inherited upstream commits and their
  authors are no longer part of `main`. Nothing published changed: all release
  tags still point at the commits they were cut from, release assets, Sigstore
  bundles, image attestations and archived deposits verify exactly as before,
  and the pre-rewrite lineage stays reachable in this repository through those
  tags. Existing clones must be re-cloned rather than pulled, and comparisons
  spanning the rewrite (for example `git describe` against `main`, or a tag
  range that crosses it) no longer share ancestry.
- The admin console's Template Manager uploads both template families the same
  way: one button in the page header (**Upload OPT** / **Upload ADL2**) opens a
  dialog offering a file picker and a paste area for either family, so ADL 1.4
  gains the paste path and ADL 2 no longer carries a permanent card above its
  listing. A refusal keeps the dialog open with the server's diagnostic beside
  the source, and a successful upload closes it and refreshes the list.

### Fixed

- The admin console's two template Example panes report a failed example read
  the same way: an inline error in the pane, in both template families. The
  ADL 1.4 pane used to render the whole-screen error with a back link even
  though the template itself had loaded.
- The hosted sandbox serves the console's content-hashed script and wasm
  bundle with immutable caching, so a repeat visit stops re-downloading it.
- A template example request with a present but empty `detail_level` or
  `type` query value is refused with `400`. The declared defaults apply only
  to an absent parameter; an empty or whitespace-padded value is outside the
  closed enums and was silently read as the default.
- The admin console's System screen reports SMART correctly. It probed the
  discovery document at the origin instead of under the platform base path
  the CDR serves it from, and rendered whatever came back as a raw status
  echo (`CDR answered 302: HTTP 302` on the hosted sandbox, where the origin
  path belongs to the console itself). It now probes
  `/ferroehr/rest/.well-known/smart-configuration`, states plainly that SMART
  is not enabled when the CDR serves no document, and gives an unexpected
  answer actionable copy that never claims SMART is disabled when the probe
  could not tell.

## [4.0.13] - 2026-08-30

### Fixed

- AQL `LIKE` honors the escaped single-character wildcard: `\?` in a pattern
  now matches a literal `?` (QUERY master03 §Operators/LIKE), symmetrically
  with the already-correct `\*`. The string reader was consuming `\?` as a
  string escape before the pattern layer could see it, so the escaped form
  still matched as the wildcard.
- Console sessions survive multi-instance deployments. The session moved
  from an in-process store to a sealed cookie (AES-256-GCM): any replica
  holding the configured `session.secret` (base64, at least 64 bytes; new,
  with `session.secret_file`) can serve any signed-in visitor — on
  serverless platforms the in-process store made every request that landed
  on another instance fail as 500 and the console report the CDR as down.
  With no secret configured the console behaves as before (one replica,
  ephemeral key) and says so at startup. The cookie is encrypted and
  authenticated, HttpOnly, SameSite=Lax, `Secure` per configuration; idle
  expiry is unchanged and rides inside the sealed payload.

## [4.0.12] - 2026-08-30

### Added

- The admin console's sign-in card takes a deployment-configured notice and
  links: `login.notice` (informational text, line breaks preserved — a demo
  states its public credentials and usage expectations there) and
  `login.links` (label + href pairs, e.g. an API reference). Both default
  empty and render nothing when unset.

### Changed

- The hosted sandbox (sandbox.ferroehr.eu) leads with the admin console: the
  deployment now runs both published container images, the console is the
  landing surface (a visitor gets its sign-in screen, demo credentials
  `ferroehr` / `ferroehr`), and the CDR keeps its path families —
  `/ferroehr/*` (including the Swagger UI, which stays the API reference),
  `/health*`, and `/management*`. The console drives the sandbox CDR over
  the public REST base as the same demo user; the CDR's admin and
  management surfaces stay off.

## [4.0.11] - 2026-08-29

### Added

- Validation enforces the one testable rule of `DV_TEXT.formatting`: a value
  formatted `"plain_no_newlines"` is refused when the text contains a
  newline (RM: "newlines are not allowed"). Every other formatting value
  stays unvalidated — markdown, `"plain"`, the deprecated CSS form, and
  unknown names all remain legal, with or without newlines.

### Fixed

- The `is_modifiable` content-write gate is evaluated inside the commit
  transaction, under a row lock, instead of a separate read before it — a
  deactivation committed concurrently with a content write is now always
  observed (one or the other strictly wins; previously the content commit
  could land on a just-deactivated EHR). The gate's semantics are unchanged:
  the atomic change set is judged as a whole — a deactivating set may carry
  its final content, a reactivating set enables its own content, and
  `EHR_STATUS` itself is always writable.
- AQL date/time comparisons work on reduced-precision values. openEHR admits
  partial date/times (`2019`, `1985-06`), but one stored anywhere in the
  scanned data made every temporal comparison on that path fail as a 400
  ("your request is malformed" — for a data property). Both sides of a
  temporal comparison now floor a partial value to the first instant it
  contains (first month/day, zero time), the same completion the promoted
  timestamp column uses, so stored partials compare, order, and match
  `NOW()` instead of erroring. A comparison value that is not an ISO 8601
  date/time at all is still the caller's 400, now named at plan time; non-ISO
  forms PostgreSQL happened to parse (e.g. `June 15, 2020`) are refused —
  AQL temporal literals are ISO 8601.
- AQL predicates and projections now see every element of a list-valued
  attribute (`links`, `context/participations`, composer `identifiers`,
  `mappings`, ...). Previously the extraction took only the first match, so
  `WHERE c/links/target/value = ...` missed a match on the second link and
  `SELECT` over such a path served one value where the data held several —
  silently. Comparisons, `LIKE`, `MATCHES` and `EXISTS` are any-match;
  projection returns every match as one JSON array cell (`null` when nothing
  matches).
- An AQL node predicate written on the ROOT of an identified path
  (`c[openEHR-EHR-COMPOSITION.report.v1]/name/value`) now constrains the
  query — it was silently discarded, so the path answered as if the
  predicate were not written. A predicate on a non-structure path step
  (`links[meaning/value = ...]`) lowers as a SQL/JSON path filter selecting
  the matching elements, and a root predicate on a whole-object projection
  serves the object where the source matches and a `null` cell where it
  does not.
- Reads serve the exact canonical bytes the commit accepted: `_type` first
  and every field at its spec-declared position — including the
  server-stamped `uid`, which now lands at its place in the document instead
  of the end. Previously the stored body passed through a `jsonb` column
  and PostgreSQL's own key ordering leaked onto the wire (reported by the
  community). Dump archives carry the same faithful bytes, so an
  export/load round-trip is byte-equal again.

## [4.0.10] - 2026-08-29

### Added

- The metrics surface gains `process_resident_memory_bytes` (the standard
  Prometheus name), sampled from procfs every five seconds beside the pool
  and runtime gauges — so "is the server's memory growing?" is a dashboard
  read instead of a shell into the host. Absent on platforms without
  procfs.

### Changed

- AQL `FOLDER f1 CONTAINS FOLDER f2` now also matches the folders of a
  `VERSIONED_FOLDER` that a folder's `items` reference — the same
  reference-resolution ground as `FOLDER CONTAINS COMPOSITION`, one
  reference hop (chain `CONTAINS` to follow further hops). Strict
  sub-folder matching is unchanged.
- The Helm chart's committed `appVersion` now always equals the product
  version, bumped in every release PR beside the compose image tags (chart
  6.0.27). Published charts are unchanged — the release pipeline keeps
  injecting the released version at package time — but a from-tree
  `helm install` now defaults to the current version instead of a release
  that could lag by one.
- The repository's default branch is now `main` (renamed from `develop`;
  full history preserved, old GitHub URLs redirect). The moving development
  image tag follows the branch: pull `ghcr.io/rubentalstra/ferroehr:main`
  (and the admin-ui/postgres siblings) instead of `:develop`, which is
  frozen tags are deleted. `:latest` keeps its meaning — the newest
  stable release — and release tags are unchanged.

### Fixed

- The v4.0.9 Helm chart publishes as **6.0.26** (appVersion 4.0.9). The
  release's own chart leg failed on a CI guard defect before publishing, so
  chart 6.0.25 never existed on the registry; 6.0.26 is the same chart with
  the committed appVersion default refreshed to the released 4.0.9.

## [4.0.9] - 2026-08-28

### Fixed

- `FOLDER f CONTAINS COMPOSITION c` now resolves the folder's `items`
  references (transitively over its sub-folders) instead of returning every
  composition in the EHR — the containment edge was silently dropped for any
  versioned object under a non-EHR parent, and the same defect made
  `NOT CONTAINS` vacuously false under a folder and let `ORDER BY` change the
  row count of `EHR e CONTAINS FOLDER f`. `FOLDER CONTAINS FOLDER` is now a
  strict sub-folder match (no self-pairs), and a `CONTAINS` pair the RM
  defines no containment relationship for (such as
  `COMPOSITION CONTAINS COMPOSITION`) is a typed refusal instead of a
  cartesian product (#2880).

- The quickstart no longer publishes PostgreSQL to a host port, so a natively
  installed PostgreSQL (the classic Windows collision: the installer's
  auto-started `postgresql-x64-*` service holding 5432) can no longer fail
  `docker compose up`. Host access for psql/GUI clients is the new
  `docker-compose.db-publish.yml` overlay, attached to releases beside the
  base file; in-stack access needs no port
  (`docker compose exec ferroehr-postgres psql`) (#2879).
- The admin console stops re-downloading its whole WebAssembly bundle on every
  page load. Its `/pkg/` filenames now carry a content hash and are served
  `public, max-age=31536000, immutable`, so a browser reuses them until the
  console is rebuilt — measured on the shipped image, a second page load
  transfers 0 bytes of `/pkg` instead of 7.6 MB. Documents keep `no-store`
  unchanged: they carry patient data and a per-request CSP nonce (#2875).
- A bare `docker compose up` in a repository checkout now runs the same
  published-images quickstart as the downloaded standalone file. The
  development overlay no longer carries Compose's auto-merged `override`
  name (it is `docker-compose.dev.yml`, applied only by an explicit `-f`),
  so a checkout can no longer fail on unpublished `:local` tags or missing
  build contexts (#2868).
- A port conflict never requires editing the compose file: every published
  port was already a variable (`FERROEHR_PORT`, `FERROEHR_DB_PORT`,
  `FERROEHR_ADMIN_UI_PORT`, `FERROEHR_S3_PORT`), and the quickstart and the
  book now say so at the point of failure (#2869).

### Changed

- Console builds no longer touch `Cargo.lock`. `cargo leptos` resolves the
  workspace through an unlocked `cargo metadata` call of its own, which had
  silently re-resolved a transitive dependency during a build; every
  invocation now runs through a wrapper that refuses a stale lockfile up front
  and passes `--locked` to both compile legs, matching every other build in
  the repository (#2877).
- The full browser e2e battery now runs against the PUBLISHED console image, in
  a weekly lane that drives the latest release's three images at that release's
  own tag. The battery's shipped-artifact mode had no caller at all; the
  release pipeline keeps gating the login journey through the image it pushes
  (#2876).
- The `openehr-*` spec crates step to `0.0.43`. The change is internal
  structure only: over-complex functions in the ADL engine, the Simplified
  Formats web-template builder and the ISO 8601 duration parser were split
  into named, documented helpers. The wire behaviour, the generated output and
  every gate are unchanged, and nothing an API consumer calls moved.

## [4.0.8] - 2026-08-27

### Changed

- The container images' distroless base (`gcr.io/distroless/cc-debian13:nonroot`)
  moves to the current upstream digest, picking up Debian package updates for
  the runtime layer of the server and admin-console images.

## [4.0.7] - 2026-08-27

### Added

- The release pipeline publishes the `openehr-*` spec crates itself, as an
  approval-gated leg: the run pauses at the `crates-io` environment for an
  explicit human approval, then publishes any unpublished crate versions in
  dependency order with registry read-back (a release with no crate changes
  passes the leg as a no-op). A cut can no longer forget the crates; the
  dispatch lane remains as the dry-run/recovery path.

### Fixed

- The chart packaging lane installs the pinned `helm-docs`, so the chart
  README drift check runs where the chart is actually packaged. The v4.0.6
  chart publish initially failed on exactly this and was recovered by
  dispatch; from this release the leg carries the tool.

## [4.0.6] - 2026-08-27

The first stable cut of the 4.0.6 line. The bulk of the line's changes landed
in the two release candidates below: the CI/release-pipeline redesign
(4.0.6-rc2) and the conformance-instrument split to the standalone Veredictum
project (4.0.6-rc3).

### Changed

- The `openehr-*` spec crates step to `0.0.42`. The change is internal
  structure only: long functions across the spec crates, the code generator and
  the test harnesses were split into named, documented helpers, with the wire
  behaviour, the generated output and every gate unchanged. Nothing an API
  consumer calls moved.

## [4.0.6-rc3] - 2026-08-26

### Changed

- The conformance instrument is no longer built from this repository. It is
  [Veredictum](https://github.com/rubentalstra/Veredictum), an independent
  Apache-2.0 project with its own release line, and the pipeline consumes it at
  a version pinned in one place (`scripts/lib/veredictum.sh`): the runner comes
  from crates.io (`cargo install veredictum --version <pin> --locked`) and the
  catalogue with its vendored specification oracle comes from a cached checkout
  of the matching tag. `scripts/conformance.sh` and the render scripts take the
  same arguments as before and produce the same artifacts, so the committed
  conformance record and every published chart, badge and document derived from
  it are unchanged. An instrument built separately from the system it judges can
  no longer be adjusted to suit that system.
- The party set — each SUT's `ixit.json` and its party statement, plus the
  static test issuer the SMART conformance posture trusts — moved to
  `docs/conformance/party/<sut>/`, beside the record it belongs to. A vendor
  running the pipeline against their own deployment points `CONF_IXIT` and
  `CONF_STATEMENT` at their own files exactly as before.
- The vendored clinical-model and fixture corpus this repository's own test
  suites read moved to `corpus/` at the repository root. It carries what FerroEHR's gates
  exercise — the CKM ADL 1.4 and AM XML archetype packs, the CKM operational
  template library, the fixture bodies and the version-signing test key. The
  catalogue-side records (the per-file corpus manifest, the generated-set recipe
  contracts and the synthesized-template generator) stayed with the instrument.

### Removed

- `docs/conformance/coverage-report.md` and the CI step that regenerated it.
  The report describes the catalogue's coverage of the openEHR wire surface, so
  it is a claim about the instrument and is now published by Veredictum, which
  is the only repository that can regenerate it.
- The CNF 2.0 design record left with the instrument; it is `ARCHITECTURE.md` at
  the root of the Veredictum repository.

## [4.0.6-rc2] - 2026-08-26

### Added

- A weekly report of container manifests in the registry that nothing can reach
  any more, the residue of a publish that pushed an image by digest and then
  failed before its tags were applied. It files one tracking issue listing the
  digests and the command that removes each. Deletion stays manual: a
  miscomputed reachability would destroy published signatures and attestations.

### Changed

- A published Helm chart now takes its `appVersion` and its
  `artifacthub.io/images` tags from the release being cut, injected when the
  chart is packaged, instead of from values edited into `Chart.yaml` beforehand.
  What the published chart says does not change; it can no longer say the
  previous release. The committed values are the default for a chart packaged
  between releases and for `helm template` against a checkout, and they may lag.
- The conformance instrument (`tools/cnf-runner` — the CNF 2.0 runner, its
  machine-readable catalogue, schemas, and party artifacts) is now licensed
  under Apache-2.0 instead of MIT: attribution travels with every copy and
  derivative (license + NOTICE retention), and the license carries an
  explicit patent grant. The rest of the project stays MIT; the vendored
  test corpora keep their upstream terms unchanged.

### Fixed

- The chart's generated `README.md`, the page Artifact Hub renders, was a
  release behind: it advertised chart 6.0.19 and image tag 4.0.4 while the chart
  was 6.0.20 with an appVersion of 4.0.5. It is regenerated, and a guard now
  refuses a README that disagrees with the chart it ships in.
- Two `gh attestation verify` examples in *Verifying releases* still pinned
  image tag 4.0.4 at the v4.0.5 cut, and that page was then frozen into the
  4.0.5 documentation. The docs guard now checks release-asset filenames and the
  substitute-this-tag note on that page, and it reads the workspace version
  rather than the chart's `appVersion`.
- Three checks in the documentation-claims guard could never fire: they tested
  membership of a newline-separated file list against a space-delimited pattern,
  so the quoted-wire-evidence check and the Rust-version check on the
  from-source page had never run once.
- The documentation-claims guard now recognises an `image.tag=X.Y.Z` pin on a
  chart page. That spelling is the defect the check was written for, and it was
  the one spelling the pattern did not match.

## [4.0.5] - 2026-08-26

### Changed

- Refusing a supplied `EHR_STATUS` without `archetype_details` (a 422 the
  Reference Model requires — `EHR_STATUS` is always an archetype root) now
  names the remedy in the error message, and the API book documents both
  create-EHR branches: the content of the server-minted default, and the
  RM-completeness a supplied status must have.

### Fixed

- AQL parsing no longer leaks memory. The parser's internal recursive
  combinators formed reference cycles that outlived every parse, so each
  parsed query grew the process permanently. Found by the fuzzing lane,
  which now runs the AQL target under LeakSanitizer without findings.
- Printing a parsed AQL query re-escapes its string literals (backslashes,
  quotes, and the decoded control escapes). The parser decodes escape
  sequences into the AST, and four printer sites emitted the decoded value
  verbatim, so a printed query re-decoded them on the next parse and
  drifted. A stored query now round-trips to the identical AST.
- Refusing an invalid AQL query with nested `[` predicate openers took
  exponential time (a 222-byte input parsed for seconds to minutes, doubling
  per nesting level). The node-predicate grammar now parses its shared path
  prefix once, so refusal time stays near-linear in input length.

### Security

- Container image tags (`:latest` and the version tags) now move only
  after the vulnerability scan passes. The images are pushed by digest,
  scanned, and tagged last, so a failing scan leaves every previously
  published tag untouched (the v4.0.4 lane moved `:latest` before its scan
  verdict).
- CVE-2026-14456 (libssl in the distroless base image) is adjudicated not
  affected: no FerroEHR binary links OpenSSL (rustls only). The finding is
  suppressed by an id-scoped, expiring ignore plus a published OpenVEX
  statement, pending a rebuilt upstream base.

## [4.0.4] - 2026-08-26

### Fixed

- An unauthenticated request to a guarded admin-console URL now answers its
  `302 → /login` with an empty body. Previously the redirect carried a fully
  rendered console document — chrome, screen markup, and the serialized
  failure diagnostics of every server function the screen runs — visible to
  any client that does not follow redirects, and each anonymous hit paid a
  full server render. A pre-render session guard now refuses the request
  before any rendering happens; the sign-in screen, OIDC handshake, assets,
  and server-function endpoints are unaffected, and the in-view guard stays
  as the client-side navigation gate.

### Changed

- Composition updates got faster: the whole write pre-check (ownership,
  `If-Match`, lifecycle, `is_modifiable`, template stability, the
  cross-version invariants) now rides the write transaction's own placement
  statement under the per-object lock, instead of a separate pre-read round
  trip. The refusal outcomes and their order are unchanged.
- `GET …/composition/{uid}` with a JSON `Accept` now serves the stored
  canonical body verbatim instead of parsing and re-serializing it. The JSON
  is semantically identical; insignificant whitespace now follows
  PostgreSQL's jsonb rendering (a space after `:` and `,`), so byte-exact
  comparisons against previous responses will differ while every JSON parser
  sees the same document. XML, FLAT and STRUCTURED representations, and
  `expand_multimedia=true` reads, are parsed exactly as before.
- The performance page documents the durability floor: why a single durable
  commit cannot beat the WAL flush, how group commit scales concurrent
  writers past it, and the `synchronous_commit` trade-off as an explicit
  operator choice (never a FerroEHR default).
- The Helm chart's default container resources now bound ephemeral storage
  (requests 128Mi, limits 1Gi) alongside CPU and memory: the root filesystem
  is read-only and the container writes only logs and tmp, so an unbounded
  local-write default could evict a node's neighbouring pods. Chart 6.0.18.
- The landing page's Content-Security-Policy drops `'unsafe-inline'` from
  `script-src`: the page's only script element is a JSON-LD data block,
  which browsers never execute, so the allowance permitted inline script
  injection for nothing in return.
- The hosted sandbox's delivery pipeline is redesigned end to end
  (`deploy/vercel/README.md`): one trigger owner (the Sandbox deploy
  workflow, firing after a release image publishes, on sandbox-posture
  changes, or manually), `Dockerfile.vercel` tracking the `:latest` release
  pointer instead of a hand-bumped version pin, an automatic wipe-and-reseed
  after every successful redeploy, and no ignore script — the ordering race
  it guarded cannot occur when the only trigger fires after the image
  exists. Sandbox failures now surface only in the sandbox-named workflows.
  The published `:latest` container tag no longer moves on prerelease tags.

## [4.0.3] - 2026-08-25

### Added

- The hosted sandbox resets every night around midnight UTC: a scheduled job
  wipes the database (fenced so it can only ever run against a Neon
  endpoint), the next boot re-runs the migrations, and
  `scripts/sandbox/reseed.sh` seeds demo EHRs with example compositions from
  published CKM templates through the public API. Visiting the sandbox root
  or its favicon now lands on the Swagger UI instead of a 404.

- The server understands two more deployment-platform conventions. `PORT`
  (Vercel, Cloud Run, Heroku inject it) binds `0.0.0.0:<PORT>`, below
  `FERROEHR__SERVER__BIND`. The libpq environment set (`PGHOST`, `PGUSER`,
  `PGPASSWORD`, `PGDATABASE`, `PGPORT`, `PGSSLMODE` — what managed-Postgres
  integrations such as Neon inject) assembles the database DSN when no URL
  form is set; `DATABASE_URL` beats the assembled form and
  `FERROEHR__DB__URL` beats both. Direct endpoints beat pooled ones inside
  that layer (`DATABASE_URL_UNPOOLED` over `DATABASE_URL`,
  `PGHOST_UNPOOLED` over `PGHOST`): a transaction-pooled endpoint hands
  statements to backend connections without the session `search_path`,
  which surfaced as intermittent "relation does not exist" errors on the
  hosted sandbox. A `Dockerfile.vercel` plus `vercel.json` ship for
  Vercel's Container preset (the hosted sandbox); the Dockerfile references
  the published release image, so Vercel's build step is a pull measured in
  seconds instead of a half-hour Rust compile, and the sandbox runs the
  exact bytes CI built, signed and tested. The release-cut guard now holds
  its `FROM` tag to the workspace version alongside the compose defaults.
  Vercel deploys only from the develop branch (no per-PR previews), and
  because the Dockerfile pins the release tag, a develop push is a
  seconds-long re-pull of the same release image: the sandbox always runs
  the latest release, and its content changes only at a release cut.

- A one-click tester sandbox: opening the repository in a GitHub Codespace
  boots the published quickstart stack (server, PostgreSQL 18, admin console)
  automatically and forwards the API and console ports. The committed
  `.devcontainer/` pins every compose command to the standalone
  `docker-compose.yml`, so the sandbox runs release images rather than
  building the checkout. Documented at *Installation → Try it in Codespaces*
  and linked from the README quick start.

- `POST {base}/admin/integrity/verify` sweeps the stored data for
  content-copy disagreement. Every version's content is stored twice — as
  the materialized document a point read serves and as the decomposed rows
  the AQL engine queries — and read-time signature verification only ever
  recomputed the first, so a row edited behind the server's back could reach
  a client through an AQL scalar result with no integrity signal. The sweep
  re-derives every stored version from its decomposed rows, compares, and
  reports every mismatch by identifier with a defect of `content_differs`,
  `nodes_missing`, `nodes_unreadable`, or `unexpected_nodes`. It reads the
  archived tier too, takes no lock, runs off the request path, and never logs
  or returns content. A finding is reported in the **200** body, not as a
  failed request. Behind the existing admin switch and admin role.

### Changed

- The documentation site no longer carries its own OpenAPI reference at
  `/api/`. Every "API reference" link now opens the live sandbox's Swagger UI
  at <https://sandbox.ferroehr.eu/ferroehr/rest/swagger-ui>, served by the
  running server from its own handlers with the public demo credentials
  `ferroehr` / `ferroehr`. The reader gets the current release rather than a
  copy assembled at site-build time, and "Try it out" issues real requests
  against real data. Every deployment still serves the same UI at
  `/ferroehr/rest/swagger-ui` when `swagger_ui` is on.

- A composition CREATE skips the explicit transaction when the commit is the
  one folded statement (no accompanying attestations, event outbox off — the
  common case): a single SQL statement is atomic on its own, so the
  `BEGIN`/`COMMIT` round trips are gone and a create is two database
  statements end to end (the merged pre-check read and the folded commit).

- A version UPDATE gets three round trips faster. Closing the superseded
  lineage tip now rides the one folded commit statement (its leading CTE, at
  the same bound instant, so the close boundary and the new version's open
  bound are one value by construction); the archived-object thaw rides the
  version-tree placement read instead of its own statement; and the
  first-version invariant read (archetype and category stability across
  versions) rides the update's merged pre-read instead of a transaction-time
  statement. Measured on a 14k-version store: composition update p50 drops
  from 10.6 ms to 5.6 ms, within ~1 ms of create.

- The first-version invariant read now consults both storage tiers, so
  updating an archived composition enforces the cross-version archetype and
  category invariants instead of silently skipping them (the read used to run
  before the thaw and saw only the primary tier).

- The write path sheds most of another btree: the node CONTAINS-anchor
  index narrows from `(rm_type, archetype)` to `(rm_type)` alone — 83% of
  node rows carry at-code archetype text whose key bytes dominated the
  index's maintenance, while every measured anchor plan either used the
  subsumption index (full-HRID anchors) or filtered archetype after the
  rm_type probe at identical latency. Measured: the one-statement commit
  drops a further ~0.3 ms; the read probes (EHR-scoped, archetype-anchored,
  rm_type-only, and the no-match worst case) are unchanged.

- A versioned-object commit is ONE SQL statement: the decomposed node rows
  ride the folded commit CTE (ordered after the version row, which also
  satisfies their foreign key in-statement), removing the separate node
  insert round trip from every local write — composition, EHR_STATUS,
  directory, demographic, and the CONTRIBUTION route alike. A logical
  delete folds through with empty arrays.

- Every versioned-object write sheds one btree: EHR-scoped AQL routes its
  scoping through the version spine (where the engine already mirrored the
  predicate), so the per-node-row `ehr_id` index — maintained for every row
  of every commit and, measured over the generated plans, serving no read
  the spine route does not — is gone from the baseline. Measured at a
  100k-composition seed: the node insert drops ~0.5 ms per commit (~13%);
  EHR-scoped reads are unchanged. Deployments recreate to pick the baseline
  up (greenfield policy).

- Commit latency's tail shrinks: the large stored JSON columns
  (`vo_version.body`, `vo_version.wrapped_original`; the node fragments and
  audit columns already had it) compress with lz4 instead of PostgreSQL's
  default pglz, and the bundled compose database enables
  `wal_compression=lz4` — measured together, per-commit WAL volume roughly
  halves (~52 KB → ~24 KB) and the post-checkpoint p99 drops from ~180 ms to
  ~19 ms on the reference workload, with the median commit ~0.5–1 ms faster.
  Greenfield deployments pick the column compression up by recreating the
  database; the WAL setting is the compose default and an operator knob
  (`BENCH_PG_WAL_COMPRESSION`) elsewhere.

## [4.0.2] - 2026-08-24

### Added

- A mixed CONTRIBUTION now honours its own EHR_STATUS member when the
  content-write gate runs: content members beside an `EHR_STATUS` member
  setting `is_modifiable = true` are accepted against a deactivated EHR
  (reactivate-and-write in one atomic commit), and a deactivating
  CONTRIBUTION may carry its final content updates. Content against a
  deactivated EHR without a reactivating member stays a 409. The rule is
  order-independent over the change set (community report; adjudication
  with citations on the tracker).

- The book gained a Version signing chapter: a digest-signing page and a
  PGP-signing page, each with a flow diagram (commit-time signing and
  read-time verification), covering what is signed, the client-supplied
  signature rules, key configuration and rotation, and the
  `IMPORTED_VERSION` wrapper signature.

- Admin console: the ADL 2 template detail shows the path catalog — the same
  expandable tree and node inspector the ADL 1.4 detail has — built by the
  console from the served AOM2 JSON. The CDR's REST surface is unchanged (the
  released API defines no Web Template representation on the ADL 2 resource).
- Helm chart 6.0.16: `metrics.grafanaDashboard.enabled` ships the default
  "FerroEHR — service overview" Grafana dashboard as a ConfigMap the Grafana
  dashboard sidecar (kube-prometheus-stack) auto-imports; the compose
  observability overlay provisions the same dashboard, now extended to AQL
  rate and phase latency, plan-cache hit ratio, database pool, Tokio runtime,
  and audit throughput — every panel query written against the served metric
  names.

- Docs: the compose installation page states the supported container
  engines — the quickstart, both profiles and both overlays are verified on
  Docker and Podman.

### Changed

- Current-version lookups gained dedicated partial indexes: EHR-scoped
  current-row reads (EHR_STATUS/FOLDER resolution, EHR summaries, the
  directory join) and the persistent-COMPOSITION duplicate probe now hit an
  index that holds exactly one entry per live object instead of walking an
  EHR's version history. Deployments recreate their database to pick the
  baseline change up (greenfield migration policy).
- The composition-create write transaction no longer spends a round trip
  reading the clock: the commit instant rides the EHR writability gate's
  own statement, and the folded commit statements now BIND that instant as
  the audit time and version validity open bound — the stored commit time
  and the signed one are a single value by construction.
- A signed commit stopped copying the composition body twice: the signature's
  canonical form is produced over a shallow reference view (the `data`
  subtree and the `signature` drop are joined/filtered by reference, never a
  deep copy of the version body).
- The admin archive load batches per relation — one `unnest` statement each
  for version rows, node rows, audits, contributions, attestations, item
  tags, folder ranks and archive markers — instead of a statement per row
  (a 1,000-version EHR record previously cost ~2,000+ round trips); the
  EHR-Extract export's demographics chapter batches its party reads and
  version counts the same way.
- ABAC-checked AQL queries run the scope collection concurrently with the
  main query under the one execution budget (wall-clock is the maximum of
  the two, not their sum).
- Template existence checks on the TDD prepare and template-example surfaces
  probe with `EXISTS` instead of moving the stored OPT XML; commit
  decomposition and content negotiation shed their per-node and per-request
  allocations.

- Version point reads (composition, EHR_STATUS, directory, party — every
  `.../{uid}` and `version_at_time` read) execute as ONE SQL statement, and
  the served canonical body is now MATERIALIZED at commit
  (`vo_version.body`, written from the same value the node rows are
  decomposed from) instead of being reassembled from the node subtree on
  every read. Measured: point-read latency drops under 1 ms (mean ~0.7 ms
  at c=1 on reference hardware) with the database-side read cost down ~8×;
  the node rows remain the AQL source of truth. Storage grows by roughly
  the size of each stored version's canonical JSON.

- The EHR_STATUS reads (current and `version_at_time`) and the current
  directory read resolve their container and read the version in ONE
  statement each (previously a container-resolution query followed by the
  version read); a revision history folds its attestations into the one
  metadata statement; the versioned-object container reads carry their
  ownership check inside the bounds statement; and a tag-collection replace
  returns the stored collection from its own insert instead of re-reading
  it after commit.
- Two per-item read loops became single statements: a resolved CONTRIBUTION
  read (`Prefer: resolve_refs`) loads all its member versions in one batched
  query instead of one read per member, and the persistent-COMPOSITION
  uniqueness pre-check is one `EXISTS` probe instead of reading every live
  composition body in the EHR.
- Response compression now runs at the fastest level for every negotiated
  encoding (it previously used the libraries' default levels — real clients
  negotiating brotli paid visible per-response compression CPU for a
  marginal ratio gain), and a bare object read no longer re-stamps a `uid`
  the stored body already carries.
- Every object-addressed read (full version reads and the metadata/existence
  lookups behind `ETag`s, revision histories, and 404 checks) queries the
  two-tier union view in ONE statement instead of retrying the cold archival
  tier in its own transaction on a primary miss — a miss (an unknown id, a
  not-yet-created resource, a probing client) no longer costs four extra
  database round trips, and archived objects stay retrievable exactly as
  before.
- AQL whole-object projections got cheaper end to end: a root-anchored
  projection (`SELECT c FROM … CONTAINS COMPOSITION c`) serves the
  materialized version body directly instead of re-aggregating and
  reassembling the node subtree, and the `RESULT_SET` assembly moves each
  document into its cell and into the response envelope instead of
  deep-copying the page three times on the way out.
- Commit and update paths do less per-request work: the version body is
  assembled once per commit (it previously reassembled twice — once for the
  signature, once for storage), the composition-update pre-read and the
  cross-version invariant checks read two text scalars off the materialized
  body instead of fetching and parsing JSON fragments, and the per-request
  EHR_ACCESS gate shares the cached settings instead of cloning the access
  list.

### Fixed

- A composition decomposing to more than 4,095 node rows now commits: the
  node insert previously bound 16 parameters per row against PostgreSQL's
  65,535-parameter statement cap, so a sufficiently large document failed
  the write outright. The insert is now one fixed-text `unnest` statement
  with one array bind per column at any row count (which also ends the
  per-commit statement-text churn in the prepared-statement cache).
- Multi-tenant deployments carry `db.statement_timeout_ms` again: the
  tenant-scoped pool's connection hook replaced the base hook and silently
  dropped the timeout, leaving those deployments without the database-side
  runaway-query guard.
- Compose: `docker compose --profile s3 up -d --wait` no longer fails after
  everything succeeded. Current Compose (v5.4, Docker and Podman alike)
  treats any exited container as a `--wait` failure, including the bucket
  initializer's successful exit; the server now declares a
  `service_completed_successfully` dependency on the initializer (inert
  without the `s3` profile), which both orders startup after bucket creation
  and marks the initializer as a one-shot. The documented invocation drops
  the initializer from the service list.
- Compose: the observability overlay boots under Podman. Every component of
  the LGTM container creates its state under `/data`, which the image cannot
  create under Podman (its root directory ships mode 555 and the overlay
  drops all capabilities); a `tmpfs` mount at `/data` fixes it on both
  engines.

- Helm chart 6.0.15: the chart-signing step retries transient Sigstore
  failures instead of stranding a published version unsigned. Chart 6.0.14
  was pushed but lost its signature and provenance attestation to a single
  Fulcio connection reset; it stays published as-is (an OCI push is
  immutable under this repository's lanes) and 6.0.15 is the signed,
  attested build of the same content. Kubernetes users should install
  6.0.15.

## [4.0.1] - 2026-08-24

### Added

- Every published artifact class now builds at SLSA Build Level 3: the
  container images and the Helm chart moved into reusable build workflows
  (`build-image.yml`, `build-chart.yml`) alongside the release binaries'
  existing lane, so the attestation-signing material is out of reach of any
  caller-defined step and a consumer can pin the exact signer workflow. The
  README carries the SLSA Build L3 badge, linked to the verification guide.
- Release binaries and the server container image are built with
  `cargo auditable`: the shipped binary carries its own compressed
  dependency list in a `.dep-v0` section (it survives stripping — verified
  first-hand), so binary-reading scanners such as syft, trivy, grype and
  osv-scanner recover the crate graph from the artifact itself.
- The verifying-releases guide now covers the SBOM-attestation verify
  command (with its required predicate type), digest-form image
  verification, the registry-only and fully-offline verification paths,
  and a plain statement of what crates.io Trusted Publishing does and does
  not provide.

### Fixed

- The container lane's change detection no longer degrades with a warning
  on tag pushes and manual dispatches: building everything in those cases
  is now declared instead of falling out of a missing event field.

- The Helm chart publishes again as 6.0.13, carrying the v4.0.0 server as
  its `appVersion`: the v4.0.0 cut changed packaged chart content
  (`appVersion`, the generated README, the image annotations) without
  bumping the chart's own version, so the tag-triggered publish was
  correctly refused by the immutability guard and no chart for v4.0.0
  existed until this one.

## [4.0.0] - 2026-08-24

### Added

- Admin console: a **Subscriptions** screen (shown only when the CDR's
  event-subscription admin API is enabled) — list, create, edit, and
  two-step delete of change-event subscriptions.

- Admin console: the EHR detail's Compositions tab gains **filters** —
  template, date range, and composer, all AQL-backed and kept in the URL so
  a filtered view can be shared — and its rows open the composition's
  rendered clinical view directly. The EHR header now shows the subject's
  identity and the queryable/modifiable badges.

- Admin console: a **Commit** tab on the EHR detail — stage several changes
  (create a composition, amend one, modify the EHR status), set the
  contribution's audit, and commit them as ONE contribution, all-or-nothing:
  the openEHR-native way to make correlated changes. A refused commit keeps
  the staging intact and shows the per-version diagnostics verbatim; staged
  changes live only in the open session.

- Admin console: a **FHIR** screen (shown only when the CDR's FHIR API is
  enabled) — the mapping-store editor (list, create, edit, and two-step
  delete of the connector's mapping definitions, edited as JSON documents
  with the CDR's validation diagnostics verbatim), a read-path viewer
  showing what a mapping produces on `GET /fhir/r4/{type}`, and a
  validate-only dry-run panel over `$validate` that commits nothing. The
  console deliberately has no path to the committing FHIR ingest door.

- The tenancy extension gains `GET /admin/tenant/current` — the tenant the
  calling credential resolves to (`{"default": bool, "tenant": record|null}`;
  the reserved default tenant when the request runs unscoped). Multi-tenancy
  stays credential-derived: the read reports the middleware's own
  resolution, and nothing selects a tenant.

- Admin console: a **Tenants** screen (shown only when the CDR serves the
  tenancy extension) — the tenant registry with create, edit, and two-step
  delete, plus a read-only card naming the tenant this session's credential
  resolves to. There is deliberately no tenant switcher.

- Admin console: the ADL2 template rows gain the same two-step delete the
  ADL 1.4 rows have, driving the CDR's existing ADL2 artefact delete (with
  its never-orphan refusal surfaced verbatim).

- Admin console: the **ADL2 template family** — the Templates screen gains a
  family switch (ADL 1.4 | ADL 2, kept in the URL): list and upload ADL2
  operational-template sources (validation diagnostics surface verbatim,
  like the OPT upload), and a per-template detail screen serving the stored
  source, the AOM2 canonical JSON, an example composition (JSON or XML), and
  version navigation across a template's stored versions. ADL2 templates
  carry no Web Template, so the detail states that instead of a path
  catalog.

- Admin console: a **Terminology** section — browse the terminologies the CDR
  serves, define a code, test subsumption, and expand or validate a value
  set — plus a terminology-aware code picker on the query builder's coded
  criterion: the terminology field offers the served ids, a code can be
  looked up (rendered `code — term`) or pulled from a value-set expansion,
  and free-text entry keeps working for terminologies the server does not
  host. The read-only terminology extension API is now enabled in the dev
  compose stack's configuration so these screens work out of the box.

- Admin console: the openEHR **item-tag** surfaces — a tag panel on the
  composition viewer (following the version the viewer shows) and on the EHR
  detail's Status tab (the versioned container, so tags survive new
  versions), plus a Tags tab listing every tag in an EHR grouped by the
  object it sits on, filterable by key, value and target path in the URL.
  One shared tag kit serves these and the demographic tag editor.

- Admin console: a full **Demographics** section — a per-kind party browser
  and editor for all five party kinds (people, organisations, groups, agents,
  roles) with create, view, verbatim-merge update and logical delete;
  versioned-party history with time-travel; a relationships index and detail
  (both linked ends, edit, history, delete) plus a relationships tab on every
  party; demographic tag editing per party and a repository-wide tag index;
  and a demographic contribution viewer. The openEHR demographic API is
  published in the development state within the implemented REST release, and
  the section's book page says so.

- FHIR mapping dry run: `POST /fhir/r4/{resource_type}/$validate` (the HL7
  FHIR R4 validation-operation convention) runs the whole ingest pipeline —
  mapping resolution, the FLAT build, the provenance stamp, and the same
  validation the real commit runs — and commits nothing. The
  `OperationOutcome` carries the verdict: the validator's rejections
  verbatim, or the valid verdict plus the EHR disposition (the target EHR is
  resolved and reported, never created). Same starter-set scope, config gate
  and access class as the ingest door; the mapping-definition contract and a
  worked blood-pressure example are now documented in the book's FHIR
  chapter.

### Changed

- Every JSON error body now has ONE uniform shape:
  `{"error", "message", "validationErrors"}`. Semantic-validation failures
  populate `validationErrors` with their `"<path>: <message>"` entries (as
  before); every other error now carries the member as an empty list, and
  validation refusals additionally gain the machine-readable `error` reason
  phrase. This makes the `400` bodies satisfy the released `Error.yaml`
  schema's required member list; clients matching on the previous
  two-shape split keep working (the change is additive on both shapes).

- The FHIR connector's release identity is recorded as **FHIR R4**: every
  resource and element it touches is unchanged in R4B (HL7's own R4B scope
  statement), so the wire's `/fhir/r4` is truthful and connector-side text
  and citations now say R4 consistently. The terminology integration keeps
  its deliberate R4B identity — the two vocabularies are intentionally
  different, and the documentation says which is which.

- An event-subscription update is an explicit full replace: `enabled` is now
  required on the body — omitting it previously defaulted to `true` and
  silently re-enabled a deliberately disabled subscription — and any unknown
  body key is refused instead of silently dropped (echoed read-only members
  from a prior read stay tolerated).

### Fixed

- Admin console: the directory history panel loads a bounded window of the
  newest versions (with a "Load older versions" affordance) instead of one
  request per version back to v1, and opening the history of a logically
  deleted directory shows the empty state instead of a bogus row; the
  template detail screen fetches and parses its OPT once instead of three
  times; the `/system` repository-usage card runs its per-template counts
  concurrently and says when it is showing a truncated list.

- Admin console: a CDR-refused session (`401`) and a wrong-role refusal
  (`403`) now read as two different situations with two different next
  actions — previously both collapsed into the same "forbidden" copy. The
  update flows send back the very `ETag` the CDR served instead of
  re-deriving the precondition from the body, and an already-zoned
  date-time entered in a time-travel picker (e.g. `…+02:00`) is no longer
  double-stamped with `Z`.

- A composition `DELETE` that volunteers an `If-Match` precondition now
  honours it: a stale condition is refused with `412 Precondition Failed`
  carrying the latest version's `ETag`, evaluated only when the delete would
  otherwise have proceeded (RFC 9110 precondition precedence). Previously
  the received header was silently ignored and the delete performed.

- Admin console, from the close-out wire audit: a refused commit's
  per-path validation details (`validationErrors`) now render line by line
  instead of collapsing to one generic message; opening a deleted
  composition version shows a first-class "deleted at this instant" state
  instead of a blank pane; the `/system` repository-usage card sends the
  template id as a query-parameter binding (an apostrophe-bearing template
  id previously broke the card's AQL — and could escape the string
  literal); every time-travel picker states that the entered instant is
  interpreted as UTC; the conformance-manifest reader tolerates a peer CDR
  omitting optional members instead of hiding the admin affordances; the
  composition delete no longer sends a self-satisfying `If-Match`; tag
  updates state `Prefer: return=minimal` explicitly; and a FHIR connector
  answer now counts any success status as completed, not only `200`.

- An AQL query parameter whose value cannot be coerced to the type its
  predicate compares against (an invalid date/time or text representation)
  is now the caller's `400`, naming the defect class — previously the
  database's coercion failure surfaced as an opaque internal `500`.

- Every member-scoped refusal on the contribution commit now names the
  offending member (`versions[i]`) — the data-parse, template-resolution,
  change-type and audit refusals previously carried no index, so a client
  staging several changes could not tell which one was rejected.

- Admin console: a FHIR-shaped refusal (an `OperationOutcome`) now renders
  its human diagnostics wherever an error surfaces — the shared diagnostic
  reader speaks both error vocabularies, so no screen can hand raw JSON to
  a toast.

- The FHIR read facade serves one Bundle entry per stored composition —
  with several enabled mappings over one template it previously served the
  same composition once per mapping, duplicating `fullUrl`s within a Bundle
  (HL7 FHIR R4 `bdl-7`) and over-counting `total`. The mapping that wins a
  composition follows the same precedence ingest uses.

- The published conformance statement's product version had sat at 3.6.0
  since that release while the record beside it moved on — it now matches
  the workspace version, and the release-cut guard fails whenever the two
  diverge again.

- The product is spelled **FerroEHR** everywhere a human reads it as a name:
  the admin console's sidebar wordmark and every browser-tab title (both
  previously `ferroehr-admin`), the three published container images'
  display titles, and the conformance report's opening line (which now
  leads with the product name and keeps the machine `sut` key beside it).
  Technical identifiers — package names, URLs, paths, environment
  variables, registry references — deliberately stay lowercase.

- Admin console: the sidebar is grouped — the domain sections (Dashboard
  through Terminology, plus FHIR and Tenants when served) sit above a
  divider, and the platform group (Operations, Audit log, System) sits
  below it, with System last.

- Admin console: the query-results chart draws with linear interpolation —
  the smoothing default emitted invalid SVG (a `NaN` control point) whenever
  two result rows shared a timestamp on a time axis, and it drew smoothed
  values between samples that were never measured.

- The unknown-configuration-key diagnostic now names the key's full section
  path (`auth.oidc.enabled`) and attributes a file line only when the key is
  really defined under that section — a key arriving from the environment is
  named by its `FERROEHR__…` variable instead of pointing at an unrelated,
  valid line of the configuration file.

- The template lists' `version` query filter now matches the template id's
  version axis, as the API documents ("taken from `template_id`") — an exact
  version or a prefix pattern like `1.2.*` previously matched nothing on
  either the ADL 1.4 or the ADL2 list, leaving `*` as the only working
  filter.

- Admin console: the EHR_STATUS editor and the demographics party editor are
  now inert (fields and save disabled) until the served document has seeded
  the form, so a keystroke can no longer race the seed, be silently
  overwritten, and then be committed as the overwritten text.

- Admin console: the composition links on the EHR detail's Compositions tab
  now percent-encode the EHR id and the versioned-object id, like every
  other link the console builds.

- The COMPOSITION and EHR_STATUS resource reads now refuse a `uid_based_id`
  of another RM kind with `404`, as the REST specification requires — an
  EHR_STATUS id addressed on the composition route (or vice versa) previously
  answered `200` with the other kind's body. Every kind-scoped read
  (latest, at-version, at-time, revision history, and the directory family)
  now discriminates the stored kind beside the owning EHR.
- Importing an EHR Extract that advances an already-held BRANCH lineage (a
  later branch version arriving while the stored branch head is still open)
  now lands as an append: the stored tip is superseded and the new version
  becomes the lineage's head, mirroring the trunk behaviour — RM common
  master06 §Copying / §Semantics in Distributed Systems. Previously the
  import was refused with a bare `409 Conflict`. A stale receipt (a branch
  version at or below the stored tip) is now refused with a message naming
  the stored tip, exactly like a stale trunk re-import.

### Removed

- The event subscription's `archetype` predicate: it was stored and served
  but consulted by nothing — the AMQP routing key carries no archetype
  segment, so the predicate never filtered anything in any combination.
  Filter by template instead (an operational template names its root
  archetype).

## [3.20.0] - 2026-08-21

### Fixed

- Committing a CONTRIBUTION whose EHR_STATUS member carries
  `lifecycle_state = incomplete` is now accepted, with the incomplete
  relaxation (existence and cardinality lower bounds lifted) applied to the
  status body exactly as for every other content type — RM common master06
  defines the incomplete state generically and no released text excludes
  EHR_STATUS. The direct EHR_STATUS update route likewise honours an
  incomplete lifecycle it previously ignored. Previously such commits were
  refused with a 422.
- The version lifecycle transition table now follows the formal state
  machine the RM designates (the `RM-version_lifecycle` diagram), in both
  directions: `complete → incomplete` (the drawn `update` edge) is
  accepted, while same-state re-commits of `inactive` or `abandoned`
  content — permissions the machine does not draw — are now refused with
  a 422 (resume editing via `reactivate`/`retrieve` first).

### Changed

- The from-source server image build (`docker compose --build`, the
  conformance stack, the repo-dev posture) now keeps the whole cargo target
  directory in a persistent BuildKit cache mount — the official Docker Rust
  pattern — instead of the cargo-chef dependency layer. Cargo's own
  freshness now decides what recompiles, so an app-only edit no longer
  recompiles the eight generated `openehr-*` spec crates (previously every
  edit paid the full 20+ minute workspace compile; cargo-chef documents that
  local workspace crates cannot ride its layer). The first build on a
  machine seeds the cache; every later build compiles only what changed.
- The entire outbound upstream-report corpus — all 215 reports of defects,
  contradictions, and silences in the released openEHR specifications — was
  re-verified first-hand against the vendored spec text in one adversarial
  pass. Three reports were refuted and withdrawn, one rewritten to its
  provable narrow claim, and every correction the pass surfaced travels on
  the closed reports for the future submission to openEHR. Verification is
  now terminal: a verified report closes as the standing record, so the open
  tracker stays near zero by design.
- The conformance catalogue follows the pass's refutations: committing a
  CONTRIBUTION whose EHR_STATUS member carries `lifecycle_state = incomplete`
  is now a gating acceptance case (RM common master06 defines the incomplete
  state generically; the reject lean existed only in the stalled CNF guide),
  and the empty-directory 404 cases gate unconditionally (the released OAS
  defines the branch — the former empty-vs-error option selection is gone).

## [3.19.0] - 2026-08-21

### Fixed

- **Error responses name the SM call status the service model actually
  declares.** Several refusals previously reported a neighbouring generic
  status: a duplicate EHR id now reports `ehr_create_fail_duplicate_id`, a
  subject another EHR already holds reports `ehr_for_subject_already_exists`
  (on create, status update, EHR-Extract import and archive load alike), an
  invalid uploaded template reports `invalid_template`, an invalid uploaded
  archetype reports `invalid_archetype`, and conflicts the service model
  names nothing more precise for (duplicate templates or stored queries,
  referenced-artefact deletes, directory and modifiability conflicts) report
  the honest generic `conflict` instead of `composition_already_exists`. No
  HTTP status moves — only the `error` token in the body becomes accurate.

## [3.18.0] - 2026-08-20

### Added

- **FHIR mapping definitions gained `where()`/`first()` paths and
  cross-terminology code translation.** The connector's FHIRPath subset now
  supports single-condition `where(path = literal)` filters and `first()`
  (picking, say, a component by its code instead of its position), and a
  `coded` entry can declare `translate` to convert codes between
  terminologies at ingest time via a configured FHIR terminology server's
  `ConceptMap/$translate`. Only strictly equivalent matches are taken; an
  untranslatable required entry refuses the ingest, an optional one writes
  nothing, and a translate mapping without a configured terminology server
  is a configuration error — the untranslated code is never passed through
  under the target terminology.

- **The whole-repository dump archives standalone demographic containers.**
  `POST {base}/admin/dump` now writes a demographic wave beside the EHR wave:
  parties and party relationships that live outside any EHR land in
  `demographic-commons.json` (their shared audits and contributions) plus
  `demographic-NNNN.json` segments, in every logical format and container
  (loose, zip, 7z; canonical-XML exports externalize their versions as
  `versions/*.xml` like the EHR wave). `POST {base}/admin/load` restores them
  verbatim, reports an already-present container under its own kind
  (`PERSON`, `ORGANISATION`, `GROUP`, `AGENT`, `ROLE`, `PARTY_RELATIONSHIP`)
  instead of failing, and still reads archives written before the wave
  existed.
- **Template and archetype uploads now validate the artefact's own meta-data
  against the RM resource-package rules.** An OPT 1.4 whose header violates
  the RM class invariants — a language outside the openEHR `languages` code
  set, `is_controlled` without a revision history, a description without an
  author or details, duplicate detail languages, an empty `lifecycle_state`
  or `purpose` — is refused with a `422` naming the violated invariant, and
  `validate` answers with the same catalogue as upload, so the two can no
  longer disagree. ADL 1.4 source uploads refuse the structural and
  language-code rows the same way; the empty-prose rows
  (`purpose`/`use`/`misuse` as `<"">`) are reported as named warnings there,
  because the empty string is how real-world 1.4 authoring spells absence
  (adjudicated against the full vendored CKM library, which uploads
  unchanged).

### Fixed

- **`DV_URI` accepts scheme-less and plain-text values.** The validator
  previously refused any `DV_URI.value` without an RFC 3986 scheme — a floor
  the class does not declare: its only invariant is `Value_valid: not
  value.is_empty`, and the RM's URI chapter explicitly allows "plain-text
  URIs" (encoding is the consumer's duty at the point of use). Relative
  references and plain text now commit; `DV_EHR_URI` is unchanged (its own
  `Scheme_valid` invariant still requires the `ehr` scheme). The conformance
  catalogue's URI cases carry the re-derived expectations.

- **`ITEM_TABLE.as_hierarchy` produces the specified row encoding.** The
  `openehr-rm` function previously emitted a column transpose (following a
  contradictory one-line summary in the class table); it now encodes one
  `CLUSTER` per row renamed to the stringified row number, as the RM's own
  ISO 13606 encoding rules, class description and instance figure define
  (the contradiction is reported upstream).

- **`validate` and `upload` can no longer disagree about a template.** The
  ADL 1.4 OPT validation endpoint now answers with the same artefact-validity
  catalogue the upload enforces, so a template that would be refused on
  upload is reported invalid by validation too.
- **A CONTRIBUTION delete member with a contradictory `lifecycle_state` is
  refused.** Declaring `deleted` as the change type while stating a non-deleted
  lifecycle previously committed with the instruction silently dropped; it is
  now a `400` naming the contradiction, matching the header-seam behaviour.
- **The `openehr-its` crate's default XML lineage is v2.** The convenience
  serializer previously emitted the v1 namespace, whose published schema
  bundle cannot describe every RM 1.2.0 class this model emits; the v1
  lineage stays available by explicit selection.

## [3.17.8] - 2026-08-19

### Fixed

- **The quickstart's S3 bucket initializer no longer races the gateway.**
  `seaweedfs-init` retried nothing: the gateway's healthcheck only proves
  the S3 port answers, and a single-shot CreateBucket against a still-warming
  filer could fail once and never run again, leaving every multimedia commit
  against a missing bucket. The initializer now retries until the bucket is
  verifiably listable (the same read the deployment probe makes) and exits
  loudly after 30 attempts — verified over repeated fresh-volume compose
  cycles.

### Added

- **Directory folder references that claim this system must now resolve.**
  A `FOLDER.items` `OBJECT_REF` whose `namespace` is `local` (or the
  server's configured system id) and whose target is not a versioned object
  of that EHR refuses the commit with `422`, naming each unresolvable
  reference at its tree path — on the `/directory` routes and on folder
  hierarchies committed through a CONTRIBUTION alike. References into
  foreign namespaces (and `unknown`) are stored verbatim, unchecked: openEHR
  object references are explicitly distributed, so only a reference that
  claims *this* system and cannot resolve here is a client defect. No
  released openEHR text assigns this behaviour in either direction (register
  AMB-211, reported upstream); reported by an external user as a missing
  safety net for mistyped composition references.

### Changed

- **PostgreSQL 18.6 everywhere** — the `ferroehr-postgres` image base, the CI
  service containers, and every documented pin move from 18.4 to 18.6, the
  latest PG 18 patch (2026-08-13; 18.5 was never released). 18.6 is itself a
  security release fixing 28 CVEs, several in surface this CDR uses
  (`pgcrypto`, `pg_trgm`, `to_char()`/regexp buffer overruns, plan-cache
  invalidation). Operators upgrading an existing cluster in place should note
  the upstream release notes advise checking (and possibly reindexing)
  `btree_gist` indexes — the temporal `vo_version` keys use `btree_gist`.
- **Helm chart `6.0.9`** — no template changes: `appVersion` moves to
  3.17.8 and the chart README states the PostgreSQL floor as 18.6 (6.0.8
  was cut mid-cycle and never published; the publish lane ships 6.0.9).

### Security

- **h2 0.4.16** — RUSTSEC-2026-0258: the HTTP/2 implementation under hyper
  accepted and queued empty DATA frames without limit (low severity,
  unbounded-memory class). In-range lock upgrade; no code change.

- **The published `ferroehr-postgres` image is rebuilt clean of its 15
  fixable HIGH findings.** Nine were Debian `util-linux` packages
  (CVE-2026-53615) whose fix sits in trixie-security while the pinned
  upstream base rebuilds on its own cadence — the image now applies Debian
  security updates at build time, so a published fix reaches the image at the
  next rebuild instead of waiting for upstream. The other six are new
  Go-standard-library advisories in `gosu`, the privilege-dropping helper the
  upstream `postgres` image bundles (CVE-2026-33818, CVE-2026-56853,
  CVE-2026-56858, CVE-2026-56859, CVE-2026-56860, CVE-2026-56862 —
  `encoding/asn1`, `net/http`, `html/template`, `encoding/xml`, `net/url`,
  `crypto/tls`): gosu sets uid/gid and execs, opening no socket and parsing
  no untrusted input, so none of that code is reachable; they join the
  per-CVE, path-scoped adjudication with published OpenVEX statements in
  `security/vex/postgres-gosu.openvex.json`, which upstream's next gosu
  rebuild deletes.

- **The vulnerable quick-xml 0.26 is eliminated from the build, not ignored.**
  pprof's `flamegraph` feature pinned `inferno ^0.11`, whose quick-xml 0.26
  carried two DoS advisories (RUSTSEC-2026-0194/0195, fixed upstream in
  0.41). The feature is now off and the flamegraph SVG (the
  `/management/flamegraph` endpoint and the bench profiler) renders through a
  direct `inferno` 0.12 dependency on quick-xml 0.41 — the two advisory
  ignores and their VEX statements are deleted, and the only quick-xml in the
  dependency graph is the fixed line. Every remaining advisory exception now
  names the upstream event that deletes it; none of the remaining four has a
  fixed release anywhere to upgrade to (rsa's Marvin advisory has no patched
  release, two are unmaintained-crate notices on compile-time proc-macros via
  latest-release carriers, and the rkyv pin exists only in the lock file with
  zero resolved nodes).

## [3.17.7] - 2026-08-15

### Added

- **Releases archive to Zenodo with a citable DOI.** Every release tag now
  deposits a snapshot under the concept DOI
  [10.5281/zenodo.21940279](https://doi.org/10.5281/zenodo.21940279) (the
  README badge; it always resolves to the latest archived version — v3.17.6
  is 10.5281/zenodo.21940280). The deposit metadata is generated from
  `CITATION.cff` in the flat legacy shape Zenodo's GitHub integration
  documents, after the first deposit demonstrably ignored the
  InvenioRDM-record-shape file and archived raw repository metadata instead;
  the concept DOI is recorded in `CITATION.cff` for citation managers.

### Changed

- **Helm chart `6.0.7`** — no template changes: the chart re-releases because
  `appVersion` moves to `3.17.7` and the published `6.0.6` is immutable by
  the publish lane's own refusal-to-overwrite.

### Security

- **The postgres image's scan gate is green again.** The new Go-stdlib
  advisory CVE-2026-39821 (`golang.org/x/net/idna` Punycode via net/http)
  matches `gosu`, the privilege-dropping helper the upstream
  `docker-library/postgres` base ships unchanged; it joins the fifteen
  already-adjudicated gosu entries as a per-CVE, path-scoped exception with
  an OpenVEX `not_affected` statement (gosu resolves no hostnames and issues
  no HTTP requests, so the IDNA path is never entered). The entries go when
  upstream rebuilds gosu on a fixed Go line.

## [3.17.6] - 2026-08-15

### Security

- **The rsa advisory's published VEX statement was false, and is re-grounded.**
  It claimed RSA is "reached only through openidconnect" with "no RSA
  private-key operation" — the OpenPGP signing path (`pgp → rsa`) was
  missing. The corrected statement rests on the true ground (the default
  `signing.mode = "digest"` performs no asymmetric operation; OIDC is
  public-key verification) and explicitly places an RSA-keyed `pgp`
  deployment outside its scope. The quick-xml statements now describe the
  current graph (the advisories fire on `inferno`'s 0.26, writer-only, not
  the upgraded `object_store` path), and a new CI gate fails any advisory
  exception whose advisory cargo-deny no longer raises — a resolved advisory
  cannot keep its exception silently.

### Added

- **The documented development→stable read-time refusal now exists.** Every
  accepted commit records whether the released generation's own reader can
  express the body (`vo_version.stable_compatible`; the extra in-memory
  parse runs only under the development profile), and the one seam every
  stored version body leaves storage through refuses a stamped-incompatible
  version under `spec_profile = "stable"` with a `409` naming the profile,
  the version and the remedy — never a silent down-conversion. Rows from
  before the column existed are assessed on the fly at read. AQL takes the
  same stamp wherever it serves a version body: a whole-object projection is
  gated at result assembly, so a page reaching a version the released
  generations cannot express refuses the whole query with that same `409` —
  never a row silently elided from a `RESULT_SET`, which has no per-row
  diagnostic channel. Leaf projections over the identical rows stay ungated
  (they serve data values over paths the planning gate already bounded), and
  under the default `development` profile the assembly gate costs nothing.
  Our own extension; no openEHR spec governs runtime generation selection.
- **The conformance pipeline exercises both generation sets in the one
  record.** The `ferroehr` lane composes a third server of the same image
  over the primary's database under `spec_profile = "stable"` (host port
  8082), declared as the ixit's `sut_stable` instance, and the catalogue
  drives the profile boundary end to end: the development server commits and
  serves the development-only body, the stable server refuses it at the
  version read and at whole-object AQL (`409`, register AMB-210), and a
  released-surface body serves identically under both. The generation set a
  deployment runs is a new ixit/case declaration (`spec_profile` /
  `requires.spec_profile`), selected on at ISO/IEC 9646 selection time like
  the signing and terminology postures.
- **Archive restore is on the wire.** `POST /admin/archive/ehrs/restore` and
  `POST /admin/archive/parties/restore` (admin-gated, our own extension —
  the SM declares no un-archive call) bring archived records back from the
  cold tier: all-or-nothing over the same existence checks as archiving,
  idempotent, `204` with no body. Archiving is no longer one-way for an
  operator.
- **The chart's NetworkPolicies say what they admit, and can refuse the
  accident** (chart `6.0.6`). An empty `ingressFrom` renders an ingress rule
  with no `from` — every source admitted — while reading as default-deny.
  The open posture is now an explicit value (`networkPolicy.ingressAllowAll`
  and `adminUi.networkPolicy.ingressAllowAll`, both shipped `true`): the
  rendered object's description, the install notes and the chart README all
  state it, and setting it `false` with no `ingressFrom` refuses at render —
  so "no open ingress" becomes a machine-checked fact where it matters.
- **A `fhir_outbound` readiness indicator.** When the FHIR outbound emitter is
  enabled, `/health/readiness` now carries its broker-delivery liveness —
  non-required, so a broker outage reports `DEGRADED` (the outbox retains
  messages) and never fails readiness. Previously a PHI-bearing outbound
  stream could be down with nothing on the health surface saying so.
- **A boot advisory for RSA OpenPGP signing keys.** With
  `signing.mode = "pgp"` and an RSA key, every signature is an RSA
  private-key operation — the operation the Marvin timing sidechannel
  (RUSTSEC-2023-0071) concerns, with no fixed `rsa` release. Boot now emits
  a prominent warning naming the advisory and the Ed25519/ECC rotation path
  (`signing.retired_key_paths` keeps history verifying); deliberately not a
  refusal, because signatures are immutable committed facts an operator must
  stay able to verify. The signing configuration page carries the guidance.

### Changed

- **The ad-hoc-query scope cases are split per carrier, and every zero-row
  scope assertion now has something to exclude.** The bundled bare-EHR case
  becomes three one-behaviour cases (grammar acceptance with the released
  `#0` column-naming rule; scope via the `ehr_id` query parameter; scope via
  the `openehr-ehr-id` request header — the only released carriers), and the
  scoped zero-row cases each first commit a composition into a SECOND EHR,
  so a green row evidences the scope rather than an empty store. The
  conformance records regenerate on the new case ids; the EHRbase record is
  honestly redder where that server ignores the scope.
- **The `openehr-*` crates move to `0.0.30`, and the published packages are
  self-consistent.** `openehr-term` ships openEHR's verbatim terminology XML
  (CC-BY-SA 3.0) but declared only `MIT AND Apache-2.0` — the expression now
  names all three licenses and the text travels in the package. The
  published `openehr-its` package carries attribution for the openEHR JSON
  Schema it ships (pinned commit, upstream path, license). Packaged test
  code no longer embeds fixture paths the package does not carry, so
  `cargo test` inside a published `.crate` compiles again. Two new guards
  keep all of it true: a pinned attribution that stops travelling fails, and
  a packaged source embedding an unpackaged path fails. All eight bump in
  lockstep.
- **The documentation site was read end-to-end against the tree and
  rewritten.** Every page's substantive claims were verified against the
  code, the chart, the workflows and the committed conformance artifacts;
  drifted claims were corrected rather than catalogued (among them: the
  binary is self-contained but not statically linked, the FHIR surfaces are
  R4B, the TLS floor defaults to 1.3-only, rate limiting ships enabled, the
  query execution budget ships at 30 s, and a removed `probes.exec` chart
  option is gone from the docs too). Long chapters split into sub-pages:
  cluster hardening (five), the configuration reference (five), release
  verification, and the admin/messaging API walkthroughs.

### Fixed

- **The FHIR read façade and outbound emitter respect the spec-profile read
  gate.** Both read stored bodies around the gated versioning seam, so a
  development-only body was mapped to FHIR under the `stable` profile with
  no refusal; both now go through the gated read, mutation-proven.
- **The boot banner follows the resolved log format.** With the default
  `log.format = "auto"` in a container (stdout not a terminal), logs render
  as JSON but the multi-line ASCII banner still printed first, handing every
  log collector unparseable lines on each boot. The banner now keys on the
  same TTY-aware resolution the log layers use: `json`, and `auto` off a
  terminal, are parseable JSON from the first byte.
- **Partial feature builds of the server compile again.** The FHIR outbound
  emitter's wiring was gated on the `events` cargo feature while its module
  exists only under `fhir`: a `--features events` build (no `fhir`) failed to
  compile, and a `--features fhir` build never started the emitter. Both
  gates now sit on `fhir`, which implies `events` at every level.
- **The Helm chart is `6.0.5`.** Reading the site against the tree corrected
  chart comments and operator-facing messages (the stale distroless base
  name, the secret-routing key list, the config-in-Secret cause in
  NOTES.txt, two book citations that moved in the hardening split, and the
  egress refusal's pointer); the packaged bytes changed, so the version
  moves.
- **Wire and OpenAPI text corrected while verifying the site.** The `406`
  answer for an unrecognized XML `version` parameter now names `version=2`
  as the default (it said `version=1`); the `expand_multimedia` parameter
  description no longer claims the served body is byte-identical to the
  committed one (expansion restores `data` while keeping the offload-added
  `uri`/integrity fields); and `DELETE /definition/artefact/adl2/{id}`
  documents its real `409` still-referenced refusal.
- **The Helm chart is `6.0.4`.** The v3.17.5 cut moved the chart's
  `appVersion` to 3.17.5 without moving the chart's own version, and 6.0.3
  was already published — a published chart version is immutable, so the
  publish lane refused rather than replacing it. Install with
  `--version 6.0.4` to get the 3.17.5 application image by default.

## [3.17.5] - 2026-08-13

### Security

- **The `ferroehr-postgres` image moves to a rebuilt PostgreSQL 18.4 base.** The
  upstream `postgres:18.4` image was rebuilt with current Debian packages, and
  this pulls that rebuild in. Nothing about FerroEHR changes: the schema, the
  extensions and the entrypoint are untouched.

## [3.17.4] - 2026-08-13

### Added

- **A published threat model.** A new
  [Threat model](https://ferroehr.eu/docs/latest/threat-model.html) chapter
  names the actors, the protected assets and eight trust boundaries, and for
  each boundary states the control that holds **and the residual risk that
  survives it** — plus an explicit list of what FerroEHR does not defend
  against, so silence is never read as coverage. It exists because the parts of
  this system that protect patient data are precisely the parts no openEHR
  specification governs: the five-stage authorization order, multi-tenancy
  isolation, the audit trail and the archive tier are all this project's own
  design, and until now every deployer had to reconstruct them from
  configuration prose.
- **Governance, maintainer and support documents.** `GOVERNANCE.md` (how
  decisions are made and how to become a maintainer), `MAINTAINERS.md` (the
  roster, every publishing identity, and what happens if the holder is
  unavailable), `SUPPORT.md` (question versus defect versus vulnerability), and
  a `CODEOWNERS` file routing review per area. All four state the current
  single-maintainer reality plainly rather than describing a process that does
  not exist.
- **A published support and end-of-life policy.** `SECURITY.md` now states
  which versions receive security fixes (only the newest), when a version stops
  receiving them (the moment a newer release exists), and how a fix reaches
  you — covering the server, the Helm chart and the published crates, which
  follow separate version lines. The same policy is reachable from the
  Operations chapter's upgrade guidance, where an operator planning change
  control actually looks.
- **Machine-readable licensing (REUSE 3.3).** A `LICENSES/` directory carries
  the full text of every licence any file in this tree is offered under, named
  by SPDX identifier, and a root `REUSE.toml` declares by glob which files are
  offered under which — so licensing now survives a file being copied out of
  the repository, which per-tree provenance files could not do. No vendored
  file was modified. Two positions are represented rather than flattened: the
  MPL 1.1 election under a tri-licensed corpus, and one upstream file whose own
  header contradicts its repository's licence. `reuse lint` and a
  declarations-versus-documentation check both gate the merge.
- **A published VEX document for the Rust dependency advisories.** Every
  advisory the `cargo deny` gate accepts now carries an
  [OpenVEX](https://openvex.dev) statement with a controlled-vocabulary
  justification and a checkable impact statement, at
  `security/vex/rust-advisories.openvex.json`, so a downstream scanner ingests
  the argument instead of reporting unexplained findings. It also covers the
  advisory that only a `Cargo.lock`-reading scanner reports for a crate this
  project's feature set never compiles. The document is generated from
  `deny.toml` joined with the published reasoning, and CI fails if the two
  disagree in either direction — an advisory cannot be accepted without its
  justification reaching you.
- **About 190 openEHR spec functions are now callable on the generated types.**
  The Reference Model, Archetype Model, BASE and LANG classes declare functions
  the specifications define — `ITEM_LIST.ith_item`, `DV_QUANTIFIED.magnitude`,
  `ARCHETYPE_HRID` decomposition, `Iso8601_timezone`'s eight accessors, the
  AOM2 primitive-constraint validity checks, and many more. They existed only
  as declarations; each one now has an implementation written from the
  governing spec section, with that citation in its documentation and a test
  asserting the published post-condition. Functions the released text leaves
  undefined, or that need state the class does not carry, are deliberately
  still absent and recorded as such rather than guessed at.

- **`Time_Definitions`'s eleven validity functions are now public** on
  `openehr-base` — `valid_year`, `valid_month`, `valid_day`, `valid_hour`,
  `valid_minute`, `valid_second`, `valid_fractional_second`, `days_in_month`,
  and the four `valid_iso8601_*` string predicates. They had no realization
  anywhere despite being declared by the spec.

### Changed

- **Every first-party Rust source file now states its licensing inside itself.**
  All 2469 tracked `.rs` files carry an `SPDX-FileCopyrightText` and an
  `SPDX-License-Identifier` header: the six published spec crates state
  `MIT AND Apache-2.0` — the position their manifests already declared — and
  everything else states `MIT`. Licensing was previously declared only by glob
  in `REUSE.toml`, which is complete but does not travel with a file that is
  copied out, and file-level redistribution is the expected case for a project
  whose premise is that people build on it and ship it. The 1471 generated
  files receive their header from the code generator, not from a hand edit, and
  a new CI check fails the build if any first-party Rust file loses its header
  or states a licence other than the one declared for its path. Vendored trees
  are untouched and stay glob-declared, as they must be. The eight published
  `openehr-*` crates are bumped to `0.0.26` in lockstep, since their packaged
  sources changed.

- **BREAKING — enumeration-constrained OPT and AOM2 fields are now typed
  enums, not strings.** `EXPR_OPERATOR.operator` becomes `OperatorKind`, and
  `C_DATE`/`C_DATE_TIME`/`C_TIME.timezone_validity` become
  `Option<ValidityKind>`. The XML schemas constrain these fields to a fixed set
  of values, and carrying them as free text meant an out-of-range value was
  indistinguishable from a valid one until something downstream misread it;
  they are now refused when read. The wire form is unchanged — the same
  characters go out, and every operational template in the corpus still
  round-trips byte-for-byte — so only code that reads these fields as strings
  is affected.

- **A Web Template's `tz_validity` no longer silently disappears when the
  template is malformed.** It was parsed with a fallback that turned any
  unreadable value into "no timezone constraint at all", which is a different
  and weaker statement than the template made.

- **An `ehr:` URI must name the `EHR` attribute it addresses.** The server
  accepted a short form that put the versioned-object id straight after the
  EHR id (`ehr:/<ehr_id>/<uid>::SYSTEM::1`); no released spec defines it. BASE
  `master11-paths` enumerates the locator's values — they "come from attribute
  names of the class `EHR` … namely `compositions`, `directory` etc." — and
  every example carries one. The short form is now refused; write
  `ehr:/<ehr_id>/compositions/<uid>::SYSTEM::1` instead. This affects any
  `DV_EHR_URI` value stored in a `LINK` or elsewhere that used the short form.

- **A refused log-filter swap now answers `400` or `503`, not always `400`.**
  `POST`/`DELETE /management/loggers` carried one flattened error string, so a
  reload layer that had gone away was reported as if the caller had written bad
  directives. The two faults are now distinct types: unparseable directives stay
  `400`, a subscriber whose reload handle is gone is `503`.

- **Errors carry their cause across the remaining request-path seams.** Six
  sites in the platform library — Web Template building, FHIR terminology
  decoding, the log-filter seam, and admin dump/load payload reading — flattened
  their cause into a message, so nothing downstream could match on it. Each now
  carries a typed source (RFC 0201). The sites that remain are structural and
  say so at the site: `prometheus::Error` has no source-bearing variant, and the
  console's error types must be serializable (server-fn boundary) or
  `Clone + Eq` (reactive-signal storage), which no underlying error is.

- **The Rust toolchain moves to 1.97.1.** It carries an LLVM miscompilation
  fix whose underlying bug has been present since at least 1.87 — the pinned
  1.96.1 sat inside that window. The MSRV stays 1.96: nothing here uses a 1.97
  feature, and the published crates are the only artifacts whose consumers
  would feel a bump.


- **One ISO-8601 grammar, not two.** `openehr-rm` validated dates, times,
  date-times and durations with its own hand-written reader while depending on
  the `openehr-base` one that owns them. The two drifted twice and both drifts
  shipped, each letting a value pass validation and then behave as invalid.
  The RM side now calls the spec functions; 341 lines of duplicate grammar are
  gone.

- **`ITEM_TABLE`'s twelve accessors** on the published spec crates — `row_count`,
  `column_count`, `row_names`, `column_names`, `ith_row`, `named_row`,
  `has_row_with_name`, `has_column_with_name`, `has_row_with_key`,
  `row_with_key`, `element_at_cell_ij` and `as_hierarchy`, which transposes the
  stored row-per-CLUSTER form into the column CLUSTERs the spec asks for.

- **Every `DV_*` arithmetic and accessor function the openEHR RM defines** is
  now implemented on the published spec crates — the whole quantity and
  date/time family, not a subset: `DV_QUANTITY`, `DV_COUNT`, `DV_PROPORTION`
  and `DV_DURATION` arithmetic; `DV_DATE`, `DV_TIME` and `DV_DATE_TIME`
  displacement and difference; the `DV_AMOUNT` and `DV_ABSOLUTE_QUANTITY`
  dispatchers over their subtypes; and `DV_PERIODIC_TIME_SPECIFICATION`'s
  `period`, `calendar_alignment`, `event_alignment` and
  `institution_specified`, parsed from the HL7 PIVL/EIVL syntax the spec
  publishes.
- **Accuracy now propagates through arithmetic as the RM specifies it.**
  `DV_AMOUNT` defines the rule — accuracies sum for both addition and
  subtraction, an unrecorded accuracy on either side makes the result's
  unknown, and a mixed percent/absolute pair is expressed in the form of the
  larger operand — and every descendant follows it. `DV_ABSOLUTE_QUANTITY`'s
  duration-valued variant is applied to the date/time types. A combination no
  valid value can carry (a percentage past 100) refuses the operation rather
  than returning a value that fails its own class invariant.

- **A decimal factor now means the decimal its author wrote.** `DV_COUNT` and
  `DV_PROPORTION` arithmetic reads a `Real` factor as the number it denotes
  rather than as the binary approximation carrying it, so `count * 0.1` is a
  whole count and `1/3` equals `0.1/0.3`. The previous reading refused both.
- **`DV_COUNT` arithmetic and `DV_MULTIMEDIA`/`COMPOSITION` predicates** on the
  published spec crates: `add`, `subtract`, `multiply`, `is_inline`,
  `is_external`, `is_compressed`, `has_integrity_check`, `is_persistent`.
  Overflow and non-whole products are refused rather than wrapped or rounded —
  openEHR defines neither, and a wrapped count is a silently wrong clinical
  value.

- **Releases are archived on Zenodo with a citable DOI.** `.zenodo.json` is
  generated from `CITATION.cff` so the two cannot disagree — necessary because
  Zenodo ignores `CITATION.cff` entirely whenever a `.zenodo.json` is present,
  and a deposit's metadata is immutable once its DOI exists. It adds what the
  citation file cannot express: which openEHR specifications a release is
  derived from, and where the published crates live.

- **A fuzz target for the identifier readers.** `OBJECT_VERSION_ID` and its
  siblings are parsed straight from the `{version_uid}` URL path parameter —
  before any body is read, any content type negotiated, or any authorization
  runs — and were previously unfuzzed. The harness also checks the composite's
  own contract: an identifier recomposed from the three parts its reader just
  produced must equal itself, which catches a mis-sliced separator that still
  returns `Ok`.

- **Every optional integration now documents how to enable it on Kubernetes.**
  Nine chapters — change events, FHIR connectors, terminology servers, Subject
  Proxy, S3 multimedia, the admin console, observability, version signing and
  multi-tenancy — described only the environment-variable and Compose paths, so
  an operator deploying with the chart had no documented way to switch any of
  them on, even though the chart's `config` passthrough has always reached every
  key. Each now carries a values snippet, the prerequisites stated **before** the
  reader hits a boot failure (a bucket that must exist, a claim the identity
  provider must issue, an egress rule an exporter needs), and how to turn it off
  again.
- **`authz.rbac.ehr_access_default` — object-level default-deny, as one setting.**
  An EHR that carries no `ACCESS_CONTROL_SETTINGS` was reachable by any caller
  the coarse layers admitted, and the only way to change that was to author a
  settings object *per EHR, through a CONTRIBUTION commit per record* — so the
  safer posture was reachable only by the most laborious path. Setting this to
  `restricted` makes a setting-less EHR reachable only by `authz.rbac.admin_role`.
  The default stays `open`, because changing it changes who can read existing
  records; explicit per-EHR settings win over it in both directions. The admin
  carve-out is deliberate — a plain deny would leave such a record unreachable by
  the operator who would author the settings that fix it.
- **Every pod the Helm chart deploys now runs in its own user namespace**
  (`hostUsers: false`, chart `6.0.0`). Container UIDs are mapped onto an
  unprivileged host range, so a container escape arrives as a host user that owns
  nothing — verified on a live v1.36.1 node, where the pod's `/proc/self/uid_map`
  reads `0 838860800 65536` while the process still sees uid 65532. This is the
  reason the chart's Kubernetes floor moves to 1.36 (see *Changed*). Set
  `hostUsers: true` to opt out on nodes whose runtime cannot map UIDs; there, the
  pod refuses to start rather than downgrading silently.
- **The chart spreads replicas across nodes by default.** A soft `maxSkew: 1`
  constraint over `kubernetes.io/hostname` means two replicas prefer two nodes,
  so one node failure is no longer a total outage. It is `ScheduleAnyway`, so a
  single-node cluster still schedules. Supplying `topologySpreadConstraints`
  replaces it wholesale.
- **`supplementalGroupsPolicy: Strict`** on every pod: the process gets only the
  groups the manifest names, so a group baked into an image cannot widen file
  access.
- **The migration Job no longer counts a drain as a migration failure.** A
  `podFailurePolicy` ignores pod failures caused by disruption, so ordinary
  cluster maintenance during a release cannot exhaust `backoffLimit` and fail the
  upgrade with no migration error in any log.
- **New chart values:** `service.trafficDistribution` (zone-local routing, off by
  default), `autoscaling.behavior` (scaling policies passed through),
  `adminUi.terminationGracePeriodSeconds` and `adminUi.preStopSleepSeconds`. The
  console also gains a startup probe, a `preStop` pause and the server's full
  security posture.
- **The Helm chart can deploy the admin console.** `adminUi.enabled` renders the
  console as its own Deployment/Service/ServiceAccount, with an optional Ingress
  and a NetworkPolicy that confines its egress to the CDR and DNS — the console
  is a REST client of the CDR by mandate, so that is now enforced rather than
  documented. Off by default, and off renders nothing.

- **The Helm chart validates your values file** (`values.schema.json`, chart
  `5.0.0`, unchanged in `5.0.1`). `helm install`, `upgrade`, `lint` and
  `template` now refuse a values
  file that misspells one of the chart's own keys, gets a type wrong, or names a
  value outside the permitted set, instead of rendering and ignoring it — a
  breaking chart change, which is why the chart's major version moves. The
  `config:` tree stays deliberately open: that vocabulary belongs to the server
  and is validated by the binary at boot, so copying it into the chart would fork
  it. Artifact Hub renders the schema on the chart's listing. (#2184)
- **The published Helm chart is signed** with a keyless
  [cosign](https://docs.sigstore.dev/cosign/) signature, from chart `5.0.1`
  onward, alongside the SLSA build provenance attestation it already carried —
  the attestation says what the chart was built from, the signature says who
  signed the artifact you pulled. No key material exists anywhere; verification
  requires an identity and an issuer, and both are in the installation guide.
  `helm install --verify` still does not apply: the chart ships no PGP `.prov`.
  Chart `5.0.0` is published but carries **neither** signature nor attestation —
  its publishing run pushed the chart and then failed at the signing step, and a
  published chart version is never replaced. It installs and runs normally; pin
  `5.0.1` or newer if you verify what you deploy. (#2183, #2184)

- **The PGP signing key can be rotated without losing history.** Signing now
  uses a signing-capable *subkey* selected by its OpenPGP key flags, which is
  the rotation mechanism the standard provides: issue a new subkey, the
  certificate retains the old one, and every previously-signed version keeps
  verifying with no configuration change. For a certificate that is genuinely
  replaced — a compromised key, an organisational change — `signing.retired_key_paths`
  holds the retired **public** keys, verify-only, so a retired key can never
  sign again. Verification also no longer ignores subkeys, which it did before.

- **`deploy/helm/ci/boot-check.sh`** — runs a real ferroehr image against every
  values overlay the chart ships, replaying the complete delivery surface: the
  rendered `ferroehr.toml`, every `config.files` entry, every file-borne secret
  at its real value, and the environment the Deployment declares (including
  `valueFrom.secretKeyRef`). Point `FERROEHR_IMAGE` at the tag you intend to
  deploy and pass your own values file to check it before installing. (#2159)
- **CI lane `chart-boot`** — boots every committed values overlay against an
  image packaged from the branch under test, on every pull request that touches
  the chart or the configuration vocabulary the overlays are written against.
  Previously the equivalent check ran only on a release tag, which is after the
  cut has already failed. (#2159)

- **The server can run with no DDL rights at all.** `db.migrate = "verify"`
  makes it issue no schema statements: at boot it checks that the database
  already carries exactly this build's migrations and refuses to start
  otherwise, naming the schema and what is wrong with it (never migrated,
  behind this build, ahead of it, a failed migration, or a migration applied
  from different source text). The runtime DSN can then authenticate as
  `ferroehr_app` alone, so an application-level SQL flaw reaches rows and never
  the schema. `apply` remains the default, so an empty configuration still
  boots against an empty database. (#2049)
- **`ferroehr db migrate` and `ferroehr db verify`** — the out-of-band schema
  step and its read-only check, for running migrations under a
  `ferroehr_migrator` DSN separately from the serving process. (#2049)
- **A migration Job in the Helm chart** (`migrations.job.enabled`): a
  `pre-install,pre-upgrade` hook Job that Helm waits on, so a failed migration
  fails the release instead of rolling pods against a schema that was never
  applied. It authenticates from its own Secret — deliberately a different
  credential from `database.existingSecret` — and rendering is refused if that
  Secret is not named. The install NOTES report the resulting posture,
  including the case where the Job is enabled but `config.db.migrate` is still
  `apply`. (#2049)
- **Tamper evidence on the audit trail.** Every record in the local Audit
  Record Repository is linked into a SHA-256 hash chain maintained inside
  PostgreSQL, committing to its predecessor and to its own content, so the
  protection covers every writer rather than only the server's own code. The
  table now refuses every rewrite path but the per-sink forwarding stamp — a
  content `UPDATE`, a `DELETE` and a `TRUNCATE` are all rejected — and
  retention pruning goes through the one sanctioned deletion path, which
  records which positions it removed so reaping is distinguishable from
  tampering. `SELECT * FROM audit.verify_audit_chain()` reports one row per
  damaged record naming what is wrong with it, and nothing at all when the
  trail is intact; it is also reachable in-process as
  `AuditStore::verify_chain`. Detection, not prevention: the chain is unkeyed,
  so the controls for a party with unrestricted write access remain the
  least-privilege role and the off-box syslog/ATX:FHIR sinks. (#2059)

- `openehr-query`: a spanned lexing entry point, `lexer::lex_spanned`, returning
  each token together with the byte range of the source it was lexed from
  (`lexer::SpannedTokens`, with a `byte_span` mapping from token indices to a
  source range), plus `parser::parse_spanned` over that stream. `lexer::lex` and
  `parser::parse` are unchanged (#2145).

- Six more OCI annotation keys on every image: `authors`, `vendor`,
  `documentation` (the docs site, which no image pointed at), `url`, and
  `base.name`/`base.digest` naming the exact pinned base image each one is built
  on — so "what is this built from" is answerable from the image rather than from
  the Dockerfile. Every image also reports its own title and description now,
  where all three previously inherited the repository's.

- **The Helm chart can deploy by digest.** `image.digest` renders the pod's
  image as `repository@sha256:…` and takes precedence over `image.tag`. The
  hardening chapter had been telling operators to deploy by digest "so what you
  verified is what runs" — and the chart had no such key and no template that
  read one, so the instruction silently did nothing. A digest is what a build
  provenance attestation is made over, so this is what makes verification bind
  to the image actually running. The value is accepted with or without the
  `sha256:` prefix.

- The [OpenSSF Best Practices badge](https://www.bestpractices.dev/projects/13982) at
  the passing level, alongside the Scorecard badge in the README. Every
  criterion is answered from something a reader can check — the vulnerability
  process in `SECURITY.md`, the release signing and provenance in the security
  chapter, the test and analysis gates in CI — rather than self-asserted.

- Fuzzing for every parser that reads attacker-controlled bytes off the wire:
  `cargo-fuzz` (libFuzzer) harnesses for the canonical-JSON reader, the
  canonical-XML reader, the AQL lexer/parser, the FLAT/STRUCTURED simplified
  formats, the ADL source parser and the OPT 1.4 template reader, each a pure
  parse of a byte slice, seeded from the openEHR corpora already committed in
  the repository. A bounded campaign per target runs on a nightly schedule with
  its corpus persisted between runs; long campaigns are a documented local
  command (`fuzz/README.md`). The harnesses live in their own workspace, so no
  ordinary build, clippy or test run is affected.

- **The cluster-hardening chapter now covers breach containment, logging, managed
  control planes and the supply chain.** What scaling to zero does and does not
  stop (it halts clinical access immediately — a clinical-safety decision to make
  before an incident, not during one — but does not undo an append-only commit,
  does not stop an attacker holding the DSN, and does not truncate the audit
  trail). A rotation procedure for every credential, with the restart requirement
  stated, because configuration is read at boot and Kubernetes propagating a Secret
  into a volume does not make a running process re-read it. A checkable
  supply-chain map: each control against the artifact that satisfies it and the
  command you can run to verify it yourself, including the two gaps that remain.
  And an honest account of what becomes your cloud provider's on a managed control
  plane, with the instruction to **test** NetworkPolicy enforcement rather than
  read the provider's documentation and conclude.

- **The application log and the ATNA audit trail are documented as two streams that
  must not be treated alike.** The application log is diagnostics on stdout and may
  be sampled or dropped; the audit trail is the accountability record — who
  accessed which patient's data — travelling by a different path, with its own
  store, its own ITI-81 retrieval endpoint and its own retention. A collector
  configured to drop a noisy stream under volume is a reasonable policy for the
  first and a compliance failure for the second, and the two are separated
  precisely so that one can be lossy.

- **Rotating a PGP version-signing key is documented as the operation it actually
  is.** In the default `digest` mode there is no key and nothing to rotate. In
  `pgp` mode, replacing the key makes **every previously-signed version fail
  verification**: the stored signature carries no key identifier, verification
  checks against the single currently-configured key, and with the default
  `verify_on_read: strict` that is served as a 5xx on reading historical data. The
  signatures cannot be re-issued, because a version's signature is an immutable
  committed fact. The three real options — treat the key as long-lived, move to
  `verify_on_read: warn` across a rotation as a recorded reduction in guarantee, or
  use `digest` mode — are documented with their trade-offs, along with the fact
  that no keyring or multi-key verification exists today.

- **A default-deny egress policy you can actually write correctly.** `networkPolicy.egress.enabled` refuses all outbound traffic except what you list; DNS is always included, and the database is now a first-class `networkPolicy.egress.database` key rather than something to remember. Enabling egress with **no** database destination is refused at render time, because that mistake presents as a database fault — readiness reports the DB down, the log shows a connect timeout, and nothing mentions the network policy. The cluster-hardening chapter carries the full destination table derived from the configuration tree: every outbound destination the server can make, which config key turns it on, and the port it needs, so a policy can be assembled from what you have enabled rather than guessed from what a default install happens to use.

  Two failure modes are documented beside it because neither announces itself. **A blocked OTLP collector drops spans without failing any request**, so an over-tight policy produces a server that is healthy by every check and has silently stopped being observable. And **tightening egress under a running pod appears to work when it has not**: a NetworkPolicy is enforced on new connections, so a pod whose connection pool is already established keeps serving after you remove its database rule and fails at the next restart — measured, along with the recovery, on a live cluster.
- **The published container images are re-scanned every week, not only when they are built.** CI scanned an image at the moment it was built, which catches what was known then and nothing after — so a CVE published the week after a release applied to the image you were running and nothing looked again. A scheduled lane now scans all three published images at the tag you pull, with the same severity floor and the same adjudicated exceptions, and with the project's OpenVEX documents applied so an accepted finding keeps its argument attached. A finding both opens a tracking issue and fails the run, because a red scheduled run nobody watches is not a control. At the time of writing all three published images carry **zero** fixable HIGH/CRITICAL findings.

- **A copyable admission policy for verifying image provenance**, for Kyverno and for sigstore-policy-controller, with the exact certificate identity and OIDC issuer our publishing lanes use — the deployment-time half of the signing work. It is documented rather than shipped in the chart, because an admission policy is cluster-scoped and governs workloads the chart knows nothing about, so `helm uninstall` must not be able to remove it. The chapter is explicit that the policies are written from the workflow definitions and **not yet verified against a live attestation**, and gives the command to reconcile them once the first attested release exists.

- **A cluster-hardening chapter, split by who can actually apply each control.** The [OWASP Kubernetes Security Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Kubernetes_Security_Cheat_Sheet.html) mixes controls a workload chart owns (pod security context, resource bounds, NetworkPolicy, ServiceAccount, secret consumption) with controls only a cluster operator can reach (node patching, etcd encryption, kubelet authentication, admission control, the API server's own access model). The new chapter carries an ownership map naming, per control, who applies it and what happens if nobody does — including the ones **no application-level hardening can compensate for**, since an anonymous-auth kubelet or a readable etcd bypasses every control this server ships. Claims about the running deployment come from `kubectl`, `crictl` and `/proc` on a live cluster, not from reading `values.yaml`.

  Two facts the chart had been conflating are now stated separately: `kubeVersion: ">=1.25.0-0"` is a **compatibility floor** (the newest API the chart uses is HPA `autoscaling/v2`, GA in 1.23, so it genuinely runs there), while the **supported** platform is upstream's three most recent minor releases — a window that moves three times a year, which is why it lives in the book rather than in a constraint that would refuse to install on a cluster the chart works on. The chapter also records a decision: this project does **not** watch Kubernetes platform advisories, because we run no cluster and could only ever close such an issue as the operator's — `kubernetes-announce` and the official CVE feed are named as the operator's duty instead, beside what we do watch.

- **The chart's rollout strategy is explicit** (`strategy` in values), rather than inherited from the API server's `25%/25%` defaults. `maxUnavailable: 0` guarantees served capacity never drops during an upgrade: a new pod must pass readiness before an old one is removed. At the default two replicas this is exactly what Kubernetes already computed, so nothing changes for a default install — it only bites above two replicas, in the safer direction. The trade-off is documented rather than hidden: with no spare scheduling capacity the rollout stalls visibly instead of proceeding at reduced capacity.

- **The Helm chart is published, to GHCR as an OCI artifact.** Installing no longer means cloning the repository:

  ```shell
  helm install ferroehr oci://ghcr.io/rubentalstra/charts/ferroehr --version 4.0.0 \
    --set database.existingSecret=ferroehr-db --set image.tag=3.17.3
  ```

  OCI only — there is no HTTP chart repository, no `index.yaml` and no chart branch, so a chart version exists in exactly one place and the chart inherits the authentication, retention and access control of the images it deploys. The cost is stated rather than hidden: **`helm repo add` does not apply to this chart**, so a client older than Helm 3.8 cannot install it. The chart version and the image tag are separate SemVer lines — `--version` pins the templates and values schema, `--set image.tag=` pins the server binary — and the documentation now explains pinning both, which is the thing that is easy to get wrong.

  The chart carries **signed keyless [Sigstore](https://docs.sigstore.dev/) provenance**, verified the same way as the images: `gh attestation verify oci://ghcr.io/rubentalstra/charts/ferroehr:4.0.0 -R rubentalstra/FerroEHR`. It deliberately ships no PGP `.prov`, so `helm install --verify` does not apply: a `.prov` needs a long-lived private key in CI, which is exactly what this project's publishing lanes are built to avoid.

  Two properties of the lane are worth knowing because a registry does not provide them. **A published chart version is never overwritten** — an OCI tag is mutable and `helm push` replaces one silently, so the lane refuses to push a version that already exists. And **a chart that cannot deploy its own default image is not published**: the lane renders the chart's default configuration and runs the `appVersion` image's own `config check` against it, because `helm install` with no `image.tag` pins `appVersion` and a configuration key that image does not know is a crash-loop, not a warning.

  The chart is listed on [Artifact Hub](https://artifacthub.io/packages/helm/ferroehr/ferroehr) with category, links, maintainers, screenshots, and all three published images with their platforms so the hub's security report covers what the project ships. Its per-release changelog is **derived from `CHANGELOG.md`** at package time rather than maintained a second time, since Artifact Hub's change kinds are Keep a Changelog's subsections.

- **Bounds on the resources a single request can consume.** `query.max_result_rows` caps an AQL query that neither the query text nor the request bounds (previously such a query returned every matching row, making one request an unbounded allocation); `query.timeout_ms` is now on by default and `db.statement_timeout_ms` backs it at the database, because the HTTP request timeout answers the client without cancelling the statement; and `[server.connection]` bounds the connection itself — an HTTP/1 header-read timeout for slow-header attacks, plus HTTP/2 stream-concurrency and keep-alive bounds, since every other limit engages only after a request head is parsed.
- `[server.tls].min_version` — the TLS protocol floor, defaulting to **1.3 only**, with `"1.2"` available as a named compatibility widening. Previously both were enabled unconditionally. TLS 1.1 and 1.0 are not selectable at all.

- Per-caller **request-rate limiting** (`[server.rate_limit]`, on by default): a caller past its rate is refused `429 Too Many Requests` with `Retry-After`. Two tiers — an **address** tier outside authentication, so a flood is refused before the server verifies a signature per request, and a **principal** tier inside it keyed on the authenticated subject, because a hospital behind one NAT is a single address and address-keying a clinical API would throttle a whole site for one busy client. The defaults sit above this implementation's own measured whole-server ceiling, so the limiter cannot refuse a caller the server could still have served — that boundary belongs to the existing `max_in_flight` shed, which answers `503`. Three statuses, three meanings: `503` the server is full, `429` you are asking too fast, `413` your payload is too big. **Turn it off before benchmarking**, or the benchmark measures the limiter.
- **Configurable request-body limits** (`[server.limits]`), replacing a fixed internal 16 MiB ceiling: a clinical tier plus a bulk tier for the routes that accept bulk by design (operational-template upload, `/message/import`, `/message/tdd`). Over-limit is `413`. The defaults are sized against measured payloads — the largest operational template in the vendored CKM corpus is 5.4 MB — and a deployment whose compositions embed large `DV_MULTIMEDIA` data raises `body_bytes` deliberately.
- **Response security headers** on every response, transport-layer ones included (`Cache-Control: no-store`, `X-Content-Type-Options`, `Referrer-Policy`, `Cross-Origin-Resource-Policy`, `X-Frame-Options`, and a minimal `Content-Security-Policy`). `Strict-Transport-Security` is deliberately not sent: RFC 6797 §7.2 requires a browser to ignore it over plain HTTP, which is how this server is commonly reached behind a terminating proxy, so the TLS edge owns it. The documentation site carries its policy as a `<meta>` element, since GitHub Pages sets no headers.
- **Signed build provenance and SBOM attestations** on every artifact published from this release onward. The images already carried BuildKit's SLSA provenance and an SPDX SBOM but unsigned — readable, not verifiable; they are now signed through Sigstore, and the release binaries gained both from nothing. Verify with `gh attestation verify oci://ghcr.io/rubentalstra/ferroehr:TAG -R rubentalstra/FerroEHR`. Signing landed in this cycle, so `3.17.3` and earlier tags carry no attestation and that command reports none for them — nothing to verify rather than a verification failure. Each release also attaches a CycloneDX SBOM of the Rust dependency graph with `pkg:cargo/…` purls, licences, checksums and the direct-versus-transitive dependency edges. The isolation-based SLSA build levels are deliberately not claimed, since the provenance is generated in the job that builds.
- Container scanning in CI: image vulnerability scanning, Dockerfile lint, and secret plus misconfiguration scanning over the tree. Adjudicated exceptions carry their reasoning — for an unreachable finding in an inherited upstream layer, as a published OpenVEX document under `security/vex/`.
- Boot warnings for two deliberate weakenings that are easy to leave switched on: `cors_permissive`, and authentication enabled on a **plaintext** listener bound to a routable address.


- **Every import from `openehr-its` now names the module that defines it.** The
  crate's nine convenience re-exports are gone, so consumers write
  `opt14::types::CObject` and `xml::runtime::ToXml` rather than borrowing those
  names from a parent module. Longer at the use site, and it says where a type
  comes from. Affects anyone depending on the published `openehr-its` crate; no
  behaviour, wire format or serialization changes with it.

- **The Keycloak quickstart now demonstrates the authorization it documents.**
  It shipped one user holding every role with RBAC switched off, so every
  authenticated caller could use every surface and no refusal was possible. The
  demo realm now carries four identities — an admin, a clinician, a read-only
  auditor and a user with no roles at all — and the overlay enables RBAC, so a
  reader can watch a valid credential be refused `403` instead of taking it on
  trust. The existing `ferroehr`/`ferroehr` login is unchanged.

- **`probes.exec` is removed** (breaking, for anyone who set it). It ran
  `ferroehr healthcheck` for liveness, readiness *and* startup, passed no
  `--url`, and that flag defaults to the openEHR status document rather than a
  health endpoint — so readiness never touched the database and a pod with a
  dead database reported Ready and took clinical traffic. The httpGet probes
  that were already the default are correct and are now the only path:
  `/health/liveness` for liveness and startup, `/health/readiness` for
  readiness.
- **The chart now requires Kubernetes 1.36 or newer** (`kubeVersion:
  ">=1.36.0-0"`, breaking; chart `6.0.0`). It is a compatibility floor: 1.36 is
  the release where the newest field the chart renders (`hostUsers`) became
  stable, which is what lets every field render unconditionally with no version
  gates left. `Chart.yaml` carries the field-to-KEP table that is its evidence.
- The PodDisruptionBudget sets `unhealthyPodEvictionPolicy: AlwaysAllow`, the
  documented recommendation. With the previous default a node drain waited for
  pods to become healthy that were unhealthy *because* of the drain, so it never
  completed. It and the `preStop` sleep action are now rendered unconditionally
  rather than version-gated, since both are stable below the new floor.
- **The chart's render gate covers the admin console**, as a fourth overlay with
  its own golden render — the per-container restricted-profile check exists
  precisely because a second workload is where a posture gets lost, so it has to
  see one. Two assertions ride along: pod isolation must be identical across
  every workload of a release, and a multi-replica Deployment must carry a spread
  or affinity rule.
- The chart's pinned Helm version moves to 4.2.3 (current release), with the
  golden renders regenerated on it.

- **Metric names change: `/management/prometheus` now derives its suffixes.**
  The server had two metrics systems and now has one — OpenTelemetry, feeding
  both the Prometheus pull surface and the OTLP push from a single meter
  provider, so a metric can no longer exist on one surface and not the other.
  **Counters are unaffected on the wire** (`compositions_committed_total` and
  friends render exactly as before). **Histograms and gauges lose their
  hand-written `_seconds`/`_bytes` suffix from the instrument name** and gain it
  from the declared unit instead — the rendered name is the same in almost every
  case, but check any dashboard that pinned a name literally. Bucket boundaries
  are unchanged, deliberately and test-pinned: a re-bucketed latency histogram
  would silently invalidate every alert built on it.
- `telemetry.metrics_push` now exports every family the Prometheus surface
  exposes. It previously carried four of ten, missing the build identity, the
  request-duration histogram and the ATNA audit counters, and the boot warning
  added earlier in this cycle to disclose that is removed as no longer true.

- `deploy/helm/validate.sh` now ends every run by stating the properties it does
  **not** check — that the server accepts the render, that the selected image
  understands its keys, that a pod starts and serves — each with the command
  that does check it. A green render was being read as a working deployment.
  (#2159)

- The Kubernetes, Compose, Operations, Audit, Security, From-source and
  configuration-reference pages now state both migration postures plainly —
  what the quickstart does and why, and what a least-privilege deployment does
  instead — and document the audit trail's tamper evidence and its verification
  query. The `[db]` reference table gained `migrate` and the previously
  undocumented `statement_timeout_ms`.

- Bearer-token refusals are now classified: the authentication layer records
  *why* a token was rejected — expired, not yet valid, bad signature, wrong
  issuer, wrong audience, missing claim, malformed, unusable key material and
  twelve more — instead of collapsing every one into a single opaque string.
  Each refusal reaches the log as a stable `reason` field alongside the
  authentication mechanism, so a burst of expired tokens (clock skew, or a
  token lifetime that is too short) is countable and no longer hides a burst of
  bad signatures (someone presenting tokens this server was never meant to
  accept). The underlying `jsonwebtoken` error is carried as the error's
  source, so the cause chain can be walked instead of grepped.

  No wire change: RFC 6750 §3.1 assigns one `invalid_token` code to a token
  that "is expired, revoked, malformed, or invalid for other reasons", so every
  refusal keeps answering `401` with the identical `WWW-Authenticate` challenge
  it did before — now asserted for every refusal kind.

- `openehr-query`: a syntax failure now locates itself in the source.
  `parser::SyntaxFault` carries `bytes: Option<Range<usize>>` — the byte range of
  the offending text — alongside the existing token indices, so a caller can
  underline the offending characters or derive a line and column instead of
  mapping a token index back through the stream itself. `parser::parse_str` fills
  it in on every failure; `parser::parse` over a bare token slice reports `None`,
  because a bare slice carries no source positions. The rendered `Display` — and
  therefore the body of an AQL `400` — is unchanged (#2145).

- **The AQL parser reports a typed error instead of a string.**
  `openehr_query::parser::parse_str` and `parse` now return `ParseError`, which
  separates a lexing refusal — carrying the lexer's own error as a real
  `source()`, so the cause chain is walkable — from a grammar refusal, which
  carries every position the parser reported along with the token found there.
  A caller can finally branch on *which pass* refused without matching a
  substring. The message text is unchanged, so the `400` body for an invalid
  query reads exactly as before and no wire behaviour moves. The published
  crate also no longer exposes `chumsky`'s error type in its public API.

- The Kubernetes hardening chapter's image-provenance admission policies now
  carry the certificate identity the publishing lanes actually issue — read off a
  published image rather than inferred from the workflow files — and both are
  corrected to the form that can read a GitHub Artifact Attestation at all.
  Kyverno needs `type: SigstoreBundle` with an `attestations:` block (the field
  defaults to `Cosign`, which looks for a detached signature these images do not
  carry and so refuses every one of them), and `sigstore-policy-controller` needs
  `signatureFormat: bundle` with an `attestations:` entry. Minimum versions are
  stated, `failureAction` moves off the deprecated spec-level field, and each
  policy names the ref set it admits: as published they accept released images
  only and deliberately refuse the `develop` tag, with the one-word change a
  staging cluster needs spelled out.
- Provenance verification commands across the security and Kubernetes chapters
  now point at artifacts that actually answer. `gh attestation verify` on the
  three `:develop` images succeeds today, and `--signer-workflow` is shown as the
  way to require the specific publishing lane rather than any workflow in the
  repository. The release-tag, release-binary and chart forms are marked as
  starting at the `3.17.4` cut instead of being demonstrated against a tag that
  returns `HTTP 404`.

- **Release binaries reach SLSA v1.0 Build Level 3, and carry provenance a scanner and an offline verifier can both read.** Build L3's distinguishing requirement is that signing material must be out of reach of the build steps — and every step of a GitHub Actions job shares one runner VM, so attesting inside the building job cannot satisfy it. The release build now happens inside a *reusable* workflow: it runs on its own VM, a caller passes declared inputs and cannot add steps, and the caller job has no steps at all. You can now **require** that signer rather than trusting any workflow in the repository: `gh attestation verify <tarball> -R rubentalstra/FerroEHR --signer-workflow rubentalstra/FerroEHR/.github/workflows/release-build.yml`. Each release also carries its provenance as `<tarball>.intoto.jsonl` beside the Sigstore bundles. The container images and the Helm chart remain Build L2 and now say so where they are built, rather than leaving the level to be assumed.

- **A direct push to `develop` is refused.** The branch ruleset already blocked deletion and force-pushes and required signed commits, but not a pull request — so the discipline every change has followed was convention rather than enforcement, and the `main` ruleset had the rule all along. Requiring zero approvals, so it changes nothing about how a maintainer merges their own work.

- **The conformance baseline moves to 1014 of 1014, and three new cases pin behaviour that had none.** The run that produced it went red first, with five cases failing because the two shared-definition deletes now require the admin role. Every red row was attributed before anything was touched, and the attribution landed on the catalogue: those five pin the *semantics* of a delete — none of them declares a `forbidden` outcome — so they were calling an operation their principal is not authorized for and never reaching it. They now drive the delete as admin, with their `204`/`404` expectations untouched. The refusal the authorization change introduced gets its own two cases, so the behaviour is tested rather than merely described, and a read-only case that had quietly stopped isolating anything — its subject route became admin-gated, and it was the only write on that surface — is joined by one on a route where the restriction is still observable. The comparison record for the upstream EHRbase server is regenerated from the same run of the same catalogue.

- Server-fault log records now carry the underlying failure that caused them.
  A `500`-class fault writes a `cause` field holding the full error chain — the
  PostgreSQL driver error, codec refusal or HTTP transport failure, and
  whatever caused *that* — instead of a single flattened sentence. Response
  bodies are unchanged: every status code and every message a client receives
  is byte-identical, and the cause never appears in one. ABAC attribute
  resolution and policy-engine failures additionally name which step failed
  (`resolve the EHR subject attribute`, `resolve the template attribute`,
  `reach an authorization decision`) rather than one generic authorization
  label.
- **The published conformance badges are derived by the conformance runner, from the same rule as the verdicts they sit beside.** They were re-derived downstream by a separate ~90-line implementation of the tier semantics, which is how a badge once read `FAIL 5/5 capabilities`: the count was tier-local while the verdict was cumulative, and both were individually right. A count now quantifies over the very capability set its verdict was judged on, so that contradiction is no longer expressible and the self-check that used to catch it is gone rather than ported. Every committed badge regenerates byte-identically, apart from `badge.json`, where a middot is now a literal UTF-8 character instead of a `·` escape — the same string, differently spelled.

- **Deleting a shared definition artefact now requires the admin role, whichever route you use.** `DELETE /definition/archetype/adl1.4/{archetype_id}` and `DELETE /definition/artefact/adl2/{artefact_id}` were clinical-class, so any authenticated principal could remove an archetype every EHR in the deployment validates against — while the neighbouring `DELETE /admin/template/{template_id}`, with exactly the same blast radius, required admin. The difference came from which path prefix a route happened to be given, not from a decision. **This tightens the wire**: a client deleting archetypes with a clinical role now gets `403`. Uploads are unchanged and stay clinical, as they always were. The specification decides that the three must match rather than which class they take — the SM puts removal of archetypes *and* templates in one clause and one pair of interfaces, and its Admin component names no definition artefact at all — so the choice of admin is ours, following the one privilege recommendation in the conformance text for irreversible deletion.

- **The database DSN, the two AMQP broker URLs and a Basic user's password hash are now delivered as mounted files instead of environment values.** An environment variable is readable through `/proc/<pid>/environ` and is inherited by every child process, which is why the [OWASP Kubernetes Security Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Kubernetes_Security_Cheat_Sheet.html) asks for a read-only volume instead — and the DSN is the credential that reaches patient data. All four now arrive at `/etc/ferroehr-secrets/…` (mode `0440`, owned `root:65532`, not a `subPath` mount so a rotated Secret still propagates) with only the *path* passed as environment. A DSN supplied through `database.existingSecret` is projected from your own Secret, so nothing about how you create it changes. `audit.fhir_feed.url` and `multimedia.access_key_id` are the only credential-bearing keys still passed as values, because they are the only ones without a `*_file` sibling.

  **What an upgrader must change:** a Basic user's hash moves from `config.auth.basic.users[].password_hash` to `secrets.basicUserPasswordHashes.<username>`, leaving only `username` and `roles` under `config:`. Setting the hash under `config:` is now refused, and the error names the key that carries it. Nothing else in your values file changes.

  A consequence worth knowing: configuring a Basic user no longer moves the whole rendered configuration into a Secret. That behaviour existed only because the hash had nowhere else to go; the ConfigMap is now used in every supported configuration, and the "no route" fallback has no reachable input — it is kept for the next secret key that arrives without a file sibling.

- **The chart requires a server build that understands the `*_file` keys above.** A chart from this cycle deployed against `3.17.3` or earlier refuses to boot with `unknown configuration key url_file`. `appVersion` moves with the release, so a published chart and its default image always agree; a chart taken from a development checkout and pinned to an older image does not.

- **The Helm chart is version `4.0.0`, and the major bump is a real break in its values contract.** The chart version is SemVer over the chart's own contract and is independent of the application version (`appVersion` still names the release). `.Values.config` is no longer rendered verbatim: a secret-bearing key under `config:` either fails the render naming the `secrets:` key that carries it, or moves the whole configuration into the Secret. **What an upgrader must change:** move `config.audit.fhir_feed.url` to `secrets.auditFhirFeedUrl`, move any `config.terminology.external.oauth2_clients.<name>.client_secret` to `secrets.terminologyOauth2ClientSecrets.<name>` (declaring the client's `token_url`/`client_id` under `config:` as before), and move a DSN, AMQP URL, OIDC HMAC secret, signing passphrase or S3 secret key set under `config:` to its existing `secrets:` key. `helm template` names every offending key at once, so a dry run is the migration checklist. Values files that set none of these render byte-identically apart from the chart version label. A `config.auth.basic.users` entry keeps working and needs no change, but the release now carries its configuration in a Secret rather than a ConfigMap — automation that reads `kubectl get configmap <release>` must read the Secret instead.

- **The chart's golden-render gate runs in CI.** `deploy/helm/validate.sh` — the only check in the repository that pins the pod security context, the resource limits, the probes and the NetworkPolicy — ran in no workflow, so the golden renders went stale on `develop` and nothing failed. A `helm-golden` job now runs it (plus `helm lint`) on every pull request touching `deploy/helm/**`, inside the aggregate gate rather than beside it. The helm version is pinned in `deploy/helm/.tool-versions` and asserted by the validator, because `helm template` output is not byte-stable across helm releases.

- **A missing mandatory key in a terminology-provider, terminology-`OAuth2`-client or subject-proxy-system table is now reported by the boot-validation pass instead of by the file parser.** Those sections read every default from one place, so a `[terminology.external.providers.<name>]` table with no `url`, an `[terminology.external.oauth2_clients.<name>]` with no `token_url`/`client_id`, or a `[subject_proxy.systems.<name>]` with no `base_url` still refuses to boot and still names the key — but it arrives with the rest of the configuration errors at once, with did-you-mean, rather than as a bare "missing field" that stops at the first one.

- **Chart secrets are mounted as read-only files instead of environment values wherever the configuration tree allows it.** An environment variable is readable through `/proc/<pid>/environ` and is inherited by every child process, which the [OWASP Kubernetes Security Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Kubernetes_Security_Cheat_Sheet.html) asks deployments to avoid. The OIDC HMAC secret, the signing passphrase and the S3 secret access key are now mounted at `/etc/ferroehr-secrets/…` (mode `0440`, owned `root:65532`, not a `subPath` mount, so a rotated Secret still propagates) and only the *path* is passed as environment through the existing `*_file` configuration keys. The database DSN, the two AMQP URLs and the S3 access key id continue to travel as environment values, because those keys have no `*_file` sibling to point at a file; the chart says which is which at each value.
- **The chart renders a Prometheus Operator `ServiceMonitor`** under `metrics.serviceMonitor.enabled`. The existing `prometheus.io/*` pod annotations are honoured only by a Prometheus that does its own Kubernetes service discovery; an operator-managed Prometheus ignores them entirely, so on a kube-prometheus-stack cluster the chart previously published no discoverable target at all.
- **The Compose deployment path is hardened to match the Helm chart.** The Kubernetes path already ran non-root with a read-only root filesystem, no privilege escalation, a `RuntimeDefault` seccomp profile and every capability dropped; the quickstart Compose file — the one an evaluator downloads and runs — set none of the equivalents. It now does: every service drops all capabilities and adds back only what its entrypoint provably needs (five for PostgreSQL, two for the S3 gateway, none at all for the two distroless images), refuses privilege escalation, bounds its file descriptors, restarts unless stopped, and PostgreSQL and the admin console run read-only roots.
- **Every published port binds `127.0.0.1` by default.** A published container port is DNAT'd ahead of the host firewall's own chains, so `ufw deny 5432` does not stop it — the quickstart previously exposed PostgreSQL, with the dev credentials that ship in the same file, on every interface. Set `FERROEHR_BIND_HOST=0.0.0.0` (or a specific address) when a deployment genuinely needs to be reachable from another machine.

- **The SMART App Launch discovery document is now boot-validated, not relayed.** `/.well-known/smart-configuration` is published to third-party applications, so an empty or plaintext value is a claim they act on. With `smart.enabled = true`: every advertised endpoint must be an absolute `https` URL (new `smart.endpoints.allow_insecure_endpoints` opts out for development, mirroring `auth.oidc.allow_insecure_issuer`); the advertised `issuer` must carry no query or fragment (RFC 8414 §2 — the same rule `auth.oidc.issuer` follows, since it is the same identity); `response_types_supported` must be non-empty (RFC 8414 §2 marks it REQUIRED); `token_endpoint_auth_methods_supported` must name at least one method; and `code_challenge_methods_supported` must include `S256`, because SMART App Launch requires PKCE (RFC 7636) and publishing a list without it tells every app the authorization server cannot do it. A deployment relying on any of those being empty now fails to boot with the key named.
- **`smart.endpoints.issuer` and `auth.oidc.issuer` must agree.** One tells applications where to obtain a token; the other is what this server accepts. Configured independently and never compared, a mismatch was silently broken in the most confusing way available — every app obtained a valid token and every request was refused as invalid. Both are known at boot, so a mismatch (and `smart.enabled` with no `[auth.oidc]` at all) is now a boot error naming both keys. A trailing slash is not a mismatch.

- **Authentication and authorization rewritten against the OAuth2/JOSE RFCs** (the wire-visible half; the boot-validation half is under Added/Fixed). What an integrator sees change:
  - A malformed `Authorization` header — unparsable, an unknown scheme, or a bearer credential outside the RFC 6750 §2.1 `b64token` grammar — now answers **400** instead of 401. The old answer said "your credential was rejected" about a request the server never read a credential from.
  - The `WWW-Authenticate` challenge now names the outcome: `error="invalid_token"` for a rejected credential, `error="insufficient_scope"` on a bearer 403, `error="invalid_request"` for a malformed header, and no error code at all when the request carried no credentials (RFC 6750 §3.1 asks for exactly that). The Basic challenge advertises `charset="UTF-8"`, and a bearer 403 now carries a challenge too.
  - An unreachable token issuer (the JWKS cannot be fetched) answers **503 with `Retry-After`** instead of 401. A client holding a valid token was previously told its credential was bad, which a well-behaved client responds to by not retrying.
  - A Basic credential must be **padded** base64 (RFC 7617 §2 → RFC 4648 §3.2). The unpadded form was accepted by a decoder setting no specification requires.
  - An external policy server answering **5xx** is now a fail-closed **500**, not a deny. A broken PDP could previously refuse clinical access indefinitely while looking like policy.
  - Bearer tokens are validated more strictly: `nbf` is honoured, `iss`/`aud`/`sub` are required alongside `exp`, an unexpected `typ` is refused, and a token claiming the RFC 9068 `at+jwt` profile must carry `iat`, `jti` and `client_id`. An audience-less configuration is no longer possible, so a token minted for another resource server — or an OIDC ID token — can no longer authenticate.
- **`scope` no longer grants roles — read this if you use OIDC.** Roles were mined from `realm_access.roles` AND the OAuth2 `scope` claim. A scope is a grant of delegated authority to a client (RFC 6749 §3.3), not an assertion about the subject, and treating it as one made the "at least one role" check pass for **every** OIDC token, because `openid` counted as a role. Roles now come from the RFC 9068 §2.2.3.1 carriers — `roles`, `groups`, `entitlements`, then `realm_access.roles` — configurable as before via `authz.rbac.role_claims`. A deployment whose callers relied on a scope naming a role must move that role into a role claim; scopes remain on the principal for SMART enforcement.
- New `auth.oidc.require_at_jwt` (default `false`): refuse a token that does not claim the RFC 9068 access-token profile. Off by default because §2.1 makes the type a SHOULD for the authorization server, so requiring it would reject conforming issuers.

- **Authentication and authorization configuration is now validated at startup, and six previously-accepted configurations refuse to boot.** Each one could only have produced a silently weaker deployment than it appeared to describe, so the server names the offending key and stops instead. What fails, and the fix:
  - `auth.enabled = true` with **no mechanism configured** (no `[[auth.basic.users]]`, no `[auth.oidc]`). Such a server can only refuse every request while advertising an authentication scheme it does not implement, which RFC 9110 §11.6.1 forbids. **Fix:** configure a mechanism, or set `auth.enabled = false` for a development server. (This was a boot warning during the current development cycle and never shipped as one.)
  - `[auth.oidc]` with **empty `audiences`**. This disabled audience checking altogether, so the server accepted access tokens minted for a *different* resource server, and could not tell an OpenID Connect ID token (whose `aud` is a client id) from an access token — RFC 7519 §4.1.3, RFC 9068 §4 step 4, RFC 8725 §3.9/§3.12. **Fix:** set `audiences` to the `aud` value your identity provider issues for this CDR. There is no opt-out.
  - `[auth.oidc] issuer` that is **blank, not a URL, not `https`, or carries a query or fragment**. RFC 8414 §2 defines the issuer identifier as an `https` URL with no query or fragment, and §6.2 requires TLS — over plain HTTP an attacker can serve their own signing keys. **Fix:** correct the issuer; for a development identity provider on plain HTTP, set `allow_insecure_issuer = true` (the no-query/no-fragment rules still apply).
  - `[auth.oidc] hmac_secret` **shorter than 32 bytes**. RFC 8725 §3.5: a human-memorizable password must not be used directly as the key to a keyed-MAC algorithm such as `HS256`. **Fix:** supply at least 32 bytes of high-entropy material, or move to a JWKS/discovery key source. Configuring any symmetric secret now also logs a boot warning that it is a development posture: the key is shared with the authorization server, so the CDR can mint the tokens it accepts.
  - `[auth.oidc] clock_skew_leeway_seconds` **above 300**.
  - any `[[auth.basic.users]]` **`password_hash` below the OWASP Argon2id floor** (`m>=19456`, `t>=2`, `p>=1`, algorithm `argon2id`), or one that is unparsable, or an entry with a blank `username`. The password verifier takes its cost parameters *from the stored hash*, so a deliberately cheap hash verified happily and silently weakened that account. **Fix:** re-hash the affected passwords at the floor or above. The bundled development and quickstart hashes were regenerated accordingly (same passwords).
  - With `abac.engine = remote`, a **missing policy entry** for any resource kind the enforcement point consults (`ehr`, `ehr_status`, `composition`, `contribution`, `query`, plus `directory` when `check_directory = true`). Previously an unconfigured kind was silently **permitted**, i.e. a whole class of requests bypassed the decision point. **Fix:** add the missing `[authz.abac.policy.<kind>]` entries. At runtime an unconfigured kind is now denied and logged, never permitted.
- The conformance comparison lane is named for the system it composes: `CONF_SUT=ehrbase` (was `ehrbase-java`), with its Compose file, party set and committed artifacts under the same name, and the two environment knobs `FERROEHR_EHRBASE_IMAGE` / `FERROEHR_EHRBASE_PORT` (was `FERROEHR_JAVA_*`). Committed measurement records keep the container names and topology they observed — a rename never restamps a measured run.
- The `openehr-its` Web Template builder for ADL2 sources is renamed to match the generation modules: `build_web_template_am24` becomes `build_web_template_v2_4` (the `am14`/`am24` module names became `v1_4`/`v2_4` in the multi-generation refactor, and the retired spelling survived in this API, two file names and their prose). A crate-API change with no behaviour change.
- Snapshot tests now resolve their workspace root from a pinned `INSTA_WORKSPACE_ROOT` rather than a runtime lookup, so cargo-lock contention can no longer make stored snapshots appear missing or write pending snapshots to a doubled path.
- Dependency updates: `base64` 0.23, `fancy-regex` 0.19, `jsonschema` 0.49 and `jsonwebtoken` 11. No behaviour change — each was checked against its upstream changelog and then against this workspace's own call sites, including that `jsonschema`'s new single-error `validate` signature does not reduce our error reporting (both call sites already collect every error).

- The documentation site's search metadata is produced by a small Rust tool (`tools/docs-meta`) rather than a Python script — the docs workflow is Rust-only by design, and this repository ships no Python.
- **The bearer-token command in the documentation uses `jq` instead of `python3`.** The quickstart told you to pipe a Keycloak token response through a Python one-liner, which is an interpreter you may not have on a machine running a pure-Rust server. The site-build and corpus-vendoring scripts lost their embedded Python for the same reason — a vendoring script must still run years from now, and a hidden `python3` requirement is exactly what stops it. Their output is unchanged: both regenerated corpus provenance files were checked byte-for-byte against the committed ones, including two archetype display names that genuinely end in a space.
- **Search-result presentation on the documentation site.** Every documentation page now carries a canonical URL (so the dev, latest and frozen-release copies of a page no longer compete in the index), its own meta description taken from the page's opening prose (they previously all shared one generic sentence, which is why search snippets repeated), Open Graph and Twitter card tags with the brand social card, `noindex` on non-canonical versions, and `BreadcrumbList` structured data mirroring the URL hierarchy. The landing page gains `WebSite`, `Organization` and `SoftwareSourceCode` structured data naming the site's top-level sections.
- **The documentation version picker works again.** It hardcoded the site base path `/ferroehr`, left over from before the move to the `ferroehr.eu` apex domain, so it requested `/ferroehr/versions.json`, got a 404, and could never populate the selector — the docs appeared to have no other versions at all. The base is now derived from the page's own location, so it is correct at a domain root and under a project path alike, and cannot go stale on the next move. The published version set is also pruned to the current release: the manifest had 28 entries, 22 of them unreachable because their paths carried the base in use when they were cut (and their frozen books still carry the pre-rename product name).
- The configuration guide now documents `spec_profile` in full — what it selects, why it is one coupled key rather than three, where to see the active profile, what changes on the wire in both directions, and the asymmetric change contract — and the "Migrating from 3.x environment variables" section is gone: it mapped spellings from a layout that was never released, so it documented a migration nobody can need. The variables that genuinely are not configuration keys (the config-file pointer, the healthcheck URL, build stamps, Compose parameterization) are documented as such instead.
- The eight published `openehr-*` crates' crates.io front pages (their `README.md` and one-line descriptions) now describe the crates as they actually are: the generation modules they expose and which is current, the `Generation` enum as the way to ask which openEHR version a generation implements, and — for the five generated crates — that no `SPEC_VERSION` constant exists (every one they advertised was removed by the multi-generation refactor). Also corrected: the retired `am14`/`am24` module names, `openehr-lang`'s conflation of specification units with generations, and three capability claims that were simply untrue (a feature that gates nothing, a parser library the crate does not depend on, and sibling behaviour files that do not exist).
- The project README now documents the two selectable openEHR specification generations (the `spec_profile` key, with the change contract in both directions) and the eight `openehr-*` specification crates published on crates.io — neither was mentioned before, and the architecture diagram, CI-gate list and roadmap link are corrected to current reality.
- The vendored openEHR specification tree no longer carries upstream's own dependency manifests (the CNF Robot harness's `requirements.txt`, the ITS-REST and TERM PHP tooling's `composer.json`). They are upstream build tooling, not specification text, and their presence made this repository's dependency graph claim Python and PHP ecosystems it does not use — raising advisories against pinned third-party versions nothing here installs. The vendoring script now excludes dependency manifests across the common ecosystems, so a future re-vendor cannot reintroduce them; the vendored spec text itself is unchanged.
- Hand-written spec-behaviour files that were byte-identical across the `openehr-rm`/`openehr-base` generation modules (89 twin families, ~29k lines of duplicated source) are now generation-twin TEMPLATES: one hand-written source under the code generator's `templates/` tree, with the per-generation copies stamped by `emit` under an `@generated-from-template` header — generation divergence becomes impossible instead of policed, and an emitter invariant refuses any new unconverted twin. The one genuinely generation-specific file (`ITEM_TAG`'s construction under RM 1.1.0's field order) is an explicit per-generation override carrying its adjudication. Crate behaviour is unchanged; the packaged sources now say which file is stamped from which template.


- **The Docker Compose quickstart is now a standalone, zero-configuration file.** `docker-compose.yml` pulls the published images (pinned to the release it shipped with) instead of building, and carries the server configuration inline, so `docker compose up` works in any empty directory — no repository checkout, no bind mounts, no environment variables. It needs Docker Compose 2.23.1 or newer.
- The admin console moved behind the `admin-ui` Compose profile: it is opt-in (`docker compose --profile admin-ui up`, then <http://localhost:3000>) rather than started by every `docker compose up`. As a side effect, the observability overlay's Grafana no longer collides with it on port 3000.
- The quickstart's authentication posture is now Basic auth with a single user (`ferroehr` / `ferroehr`, holding the `ADMIN` and `USER` roles) and role-based access control **disabled**, so every enabled surface works out of the box; the admin API, the management endpoints, and permissive CORS stay on. The previous three-user, RBAC-enabled development posture moved to the repository development override. Both are development defaults — see the security chapter for turning RBAC on.
- The conformance stack composes its own self-contained `docker/sut-ferroehr.yml` instead of overlaying the root Compose file, so the end-user quickstart and the conformance instrument evolve independently. Starting the development FHIR terminology server is now `docker compose -p ferroehr-cnf --project-directory . --profile terminology -f docker/sut-ferroehr.yml -f docker/sut-terminology.yml up -d --wait ferroehr` from a repository checkout.
- `openehr-lang` now models generations by COMPONENT VERSION like every other spec crate: one `openehr_lang::v1_1` generation holding the version's published specifications side by side — the STABLE, tool-implemented BMM v2.x model (`bmm`/`bmm_persistence`/`beom`, on the prelude) beside the PAUSED BMM3 model (`bmm3`, full-path only) — together with the hand-written ODIN/BEL/EL readers and the shared lexer for that version's notations (all previously crate-root modules). The released LANG 1.0.0 machine-readable BMM is recorded as unusable for code generation (no `includes`, unnamed `BMM_CLASS`/`BMM_PACKAGE`, an explicitly obsolete package; BMM is TRIAL in that release), so no 1.0.0 generation exists until upstream republishes usable artifacts.
- The published `openehr-*` spec crates now expose every BMM generation under a version-named top module — `openehr_base::v1_3`, `openehr_rm::v1_2`, `openehr_lang::v2`/`v3` (replacing `bmm`/`bmm_persistence`/`beom` and `bmm3` at the crate root), `openehr_am::v1_4`/`v2_4` (replacing `am14`/`am24`), `openehr_term::v3_1` — with a new per-crate `Generation` enum (derived `Default` marking the current generation, `spec_version()`/`as_str()`, `FromStr`/`Display`) and the crate prelude re-exporting the current generation only. Import paths into these crates change accordingly; the served wire formats are unchanged.

### Removed

- **`authz.rbac.management_access` and `management.access_default` are gone**
  (breaking, for a configuration that set either). Both were inert: the
  management surface is gated per endpoint, and neither key was ever consulted
  by the code that gates it. That was the dangerous kind of dead setting —
  someone setting `management_access = "admin_only"` to lock the surface down
  had not locked anything down, and the security chapter presented it as the
  gate. **`[management.endpoints]` is now the single authority**, one level per
  endpoint, with no global default beside it: an endpoint you do not name is
  `off` and is not mounted. A test pins that the levels are independent, so no
  endpoint's level can leak onto its neighbour.

  If you set either key, delete it — the server refuses unknown configuration
  keys at boot, so this fails loudly rather than silently. The behaviour you had
  is unchanged, because neither key did anything.

- `openehr-rm`'s `ehr-extract` cargo feature, which gated nothing (the EHR Extract modules were always compiled), and `openehr-its`'s empty `bmm` placeholder module — a published module that promised future surface nobody could use.


- The generated `openehr-*` spec crates no longer carry a crate-level `SPEC_VERSION` constant: a multi-generation crate has no single implemented spec version, and a fixed crate-root pin would contradict a configured non-current generation. The ONLY pin authority is the emitted `Generation` enum (per-variant `const fn spec_version()`; the derived `Default` variant is the current generation) — the generation modules carry no version constant either; the hand-written single-spec crates (`openehr-its`, `openehr-query`, `openehr-adl`) keep their literal constant.

### Fixed

- **A `TERMINOLOGY_ID` is no longer refused for its lexical form.** The CDR
  rejected composition values such as `snomed_ct(3.1)` and `SNOMED CT` with
  `Invariant Value_valid failed on type TERMINOLOGY_ID` — an invariant the
  released BASE model does not declare anywhere: the class table carries no
  Invariants row and the machine-readable model no `invariants` key. Released
  QUERY 1.1.0 settles it from the other side by publishing
  `terminology_id/value='snomed_ct(3.1)'` in its own normative example, which
  the enforced production forbade. Values that were being rejected at commit
  are now accepted.

- **Five CI gates ran without gating anything.** Branch protection routes
  through a single aggregate check, which is what lets jobs be added or renamed
  without editing repository settings — but a job missing from that check's
  dependency list still runs, still goes red, and still merges. The compose
  hardening guard, the error cause-chain ratchet, the no-Python rule, the
  spec-citation check and the chart boot check were all in that state. All are
  wired in, and a new guard fails the build when a job is missing from the list
  in either direction, so the next one cannot slip through silently. The compose
  guard is the one worth calling out: `docker-compose.yml` and the operations
  chapter both told readers it enforced on every compose artifact, and it had
  never run.

- **The forbidden-licence check now actually runs.** It was configured as a
  FOSSA `customLicenseSearch`, which could not upload on this organization's
  plan and did not honour the configured path exclusions — every one of its
  findings came from vendored third-party trees rather than from this project's
  own code. It is now a local check that shares the same exclusion list and
  fails the build.

- **An uploaded operational template no longer silently loses its slot
  constraints.** A slot assertion was rendered into ADL by writing the raw
  numeric operator code the OPT XML encodes — text the assertion parser cannot
  read — so any slot whose template did not also carry a pre-rendered string
  expression ended up with no constraint at all, and would admit archetypes it
  was written to exclude. The operator is now rendered as its ADL symbol, and a
  test reads the result back through the parser.

- **Temporal constraint conformance answers where it used to give up.** A
  child archetype narrowing a date, time or duration to a range under a parent
  that constrains it by pattern reported "cannot tell" instead of judging it,
  on the stated grounds that the generated date/time types offered no
  ordering. They have offered ordering for some time, so the check now reaches
  a verdict.

- **An operational template whose archetype id carries an ADL 2 version is
  accepted again.** Uploading an OPT whose root id is
  `openEHR-EHR-COMPOSITION.minimal.v1.0.0` was refused as a malformed archetype
  identifier (`VARID`), even though deployed OPT 1.4 exports carry exactly that
  form and the validator documented the tolerance. The check had been routed
  through a grammar whose version production is single-part by definition, so
  the tolerance it described could never fire.

- **AQL archetype subsumption works again for ADL 2 identifiers.** A query
  naming a parent archetype is supposed to match data created with its
  specialisations. For an AOM2-era identifier — `openEHR-EHR-OBSERVATION
  .lipid_panel.v1.0.0`, the form an ADL 2 archetype actually carries — it did
  not: the predicate read the identifier through a decomposition that accepts
  only the single-part `.v1` version, silently fell back to matching the whole
  identifier string, and left the index built for the subsumption query unused.
  The answer was narrower than the spec requires, with no error. Both eras'
  forms are now read through the same decomposition the lineage index is built
  from.

- **`TERMINOLOGY_ID` now enforces its grammar.** A value was accepted if it was
  merely non-empty and printable, so `"SNOMED CT"` and
  `"http://snomed.info/sct"` passed. The identifier is now the production BASE
  `base_types` master05 §Syntaxes states. The version part in parentheses
  admits a leading digit, because every example the same chapter gives
  (`ICD9(1999)`, `ICD10AM(3rd_ed)`) starts with one — reported upstream, since
  the grammar as written refuses openEHR's own identifiers.

- **A Web Template no longer reports a query string as a terminology.** A
  `C_CODE_REFERENCE` whose reference-set URI carries parameters —
  `terminology:NSI?subset=doctor_category&language=en-GB` — had the entire
  remainder copied into the input's `terminology` field, which is a terminology
  identifier. Only the identifying part is taken now, for both the bare form
  the conformance templates use and the addressed form AQL documents.

- **The release lane would have shipped a release with no binaries.** GitHub
  release immutability is now enabled, which freezes a release's assets the
  moment it is published — and the binaries, SBOMs and Sigstore bundles are
  attached by a job that runs *after* the release is created. The release is now
  created as a draft, its assets are attached, the expected set is verified, and
  only then is it published, so nothing can be locked out.

- **Three list-typed configuration settings could not be set from the
  environment at all.** `signing.retired_key_paths`,
  `smart.endpoints.capabilities` and every `authz.abac.policy.<kind>.parameters`
  are list-typed but were missing from the loader's list registry, so an env
  value was not merely mis-split — it was **refused at boot**: `invalid type:
  string "…", expected a sequence`. All are now registered, and tests assert
  that every env-settable list key parses from one value, splits on several, and
  that a list nested under a map key works too.

  `signing.retired_key_paths` is the key that carries PGP key rotation: it holds
  the retired PUBLIC keys that keep versions signed before a rotation
  verifiable. Until now it was reachable only from a TOML file, so any
  environment-driven deployment — Compose, plain Docker, a chart's `extraEnv` —
  could not configure key rotation at all, and found out only when the server
  refused to start.

- **The fuzzing lane had not run at all.** Every one of the six libFuzzer
  harnesses failed to *build* on every scheduled run: `cargo-fuzz` defaults the
  build target to the triple it was itself built for, CI installs its musl
  asset, and musl's static libc cannot carry a sanitizer — so the campaign died
  before executing a single input. The target triple is now named explicitly,
  and the harnesses are compiled on the pull-request path whenever they or a
  fuzzed crate change, so a scheduled-only lane can no longer rot unnoticed.

- **Multimedia committed inside an `EHR_STATUS` or a `FOLDER` can now be read
  back.** Externalization is applied by the versioning path every versioned
  object commits through, but re-inlining was wired to the COMPOSITION read
  alone — so a large `DV_MULTIMEDIA` committed in an EHR status left the
  database, sat in the object store, and **no API call returned it**;
  `?expand_multimedia=true` on that read was an undeclared parameter and was
  ignored. All nine reads that can serve externalized content now honour it (the
  bare COMPOSITION / EHR_STATUS / FOLDER reads and the VERSION envelopes that
  wrap them), and the OpenAPI document declares it on exactly those operations —
  pinned by a test, so the declarations and the handlers cannot drift apart
  again.
- **Turning multimedia externalization off no longer strands the blobs already
  offloaded.** `?expand_multimedia=true` against an already-externalized record
  was **silently ignored** once `multimedia.enabled` went back to `false`: the
  read answered `200` with the compact `s3://` reference and no indication that
  the expansion had not happened. `enabled` now governs **new offloads only** —
  the fetch-and-verify path stays available as long as an `endpoint` is
  configured, so content this server externalized stays readable. With no store
  reachable at all, an expansion request now **fails** instead of quietly
  answering with the reference.
- **`FERROEHR__MULTIMEDIA__*` set in your shell now reaches the compose stack.**
  Every other tunable the quickstart documents is a pass-through; the multimedia
  keys were not, so the documented S3 recipe brought the stack up healthy with
  multimedia silently off. The whole `FERROEHR__MULTIMEDIA__*` set is now
  declared on the `ferroehr` service with no value, which is the Compose form
  that forwards a variable when it is set and **removes it entirely when it is
  not** — so an unset key stays absent rather than arriving as an empty string,
  which for `endpoint` is the difference between "use the default" and a boot
  refusal.
- **The development Keycloak realm now mints tokens the server accepts.** The
  realm declared no audience mapper, so Keycloak emitted no `ferroehr` audience,
  while the same development configuration requires that audience — every bearer
  token it issued was refused `401 InvalidAudience`, and no user of that realm
  could ever produce an accepted one. The server was right in both halves (RFC
  7519 §4.1.3); the realm was wrong. An `oidc-audience-mapper` on the `ferroehr`
  client fixes it, verified live: the access token now carries `aud: ferroehr`.
  The realm also carried identifiers from the deployment it was exported
  from — a hardcoded tenant claim naming a foreign UUID, a foreign client scope,
  and a default role named after another product — all removed.

- **`migrations.runByMigratorRole` now does what its documentation says.** The
  key was described as "rendered into NOTES for the operator" and no template
  read it, so the chart, its generated README and the book all described a
  marker that did nothing. It now surfaces at install time when set to false,
  stating plainly that the chart cannot verify the claim.

- **A stock `helm install` no longer appears to succeed and then crash-loops.**
  `config.auth.enabled` defaults to true and the server requires a mechanism with
  it (RFC 9110 §11.6.1 — a 401 challenge must name a scheme the server
  implements), so the default values produced a pod that exited at boot with the
  reason visible only in its log. The chart now refuses to **render** that
  combination, naming both ways to configure authentication and the explicit
  development-only opt-out. The default was deliberately *not* changed to
  disable authentication: that would hand a fresh install an unauthenticated CDR
  serving patient data, which is a worse outcome than a loud failure.

- **The CDR Service no longer load-balances openEHR traffic into the admin
  console.** A Service selector is a subset match, and the console's pods carried
  the server's `app.kubernetes.io/name` plus a `component` label — so they matched
  the server's own Service, whose `targetPort: http` then resolved to the
  console's port 3000. With the console enabled, a share of API requests were
  answered by a web UI. The same overlap inflated the PodDisruptionBudget's
  `disruptionsAllowed` from 1 to 2, enough for a node drain to evict both server
  replicas at once. The console now carries its own `app.kubernetes.io/name`.
  Verified on a live cluster: the CDR Service's endpoints are exactly the two
  server pods and the PDB reports `expectedPods=2`.

- **An Ingress with a path but no `pathType` is no longer rejected by the API
  server.** `pathType` is required on `networking.k8s.io/v1` but optional in the
  chart's schema; it now defaults to `Prefix`.

- **A Service with custom `annotations` no longer loses its description.** The
  template emitted two `annotations` mappings under one `metadata`, so the later
  one won and `kubernetes.io/description` silently disappeared.

- **An autoscaler with every metric disabled is refused instead of installed
  inert.** Setting both HPA targets to 0 rendered an HPA with no metrics, which
  the API accepts and the controller can never act on — while the Deployment
  omits `replicas` under autoscaling, so a fresh install silently ran a single
  replica.

- **The install notes no longer advertise a Prometheus endpoint that is off.**
  The endpoint was reported as public whenever metrics were enabled, ignoring
  `config.management.endpoints.prometheus`, which defaults to `off`.

- **The chart no longer promises drain-safety it cannot deliver on older
  clusters.** `PodDisruptionBudget.unhealthyPodEvictionPolicy` was rendered
  unconditionally under a `kubeVersion` floor of `>=1.25.0-0`, but the field is
  alpha in 1.26, beta in 1.27 and stable only in 1.31 (KEP-3017). Below 1.27 the
  API server prunes it or leaves it behind a disabled gate, so the install
  succeeded while the policy silently did not apply. It is now version-gated and
  rendered only where it takes effect; clusters below 1.27 get the documented API
  default (`IfHealthyBudget`) visibly rather than an ignored setting.

- **The chart-publish gate now judges every values overlay, at real secret
  values.** The tag lane carried its own copy of the boot replay, which wrote a
  placeholder into every file-borne secret (so the 32-byte HMAC floor and the
  Argon2id password-hash parse were checked against the placeholder, not the
  configured secret), rendered only `default-values.yaml`, and skipped
  `valueFrom.secretKeyRef` environment entirely. It now calls
  `deploy/helm/ci/boot-check.sh`, which mounts each secret at its rendered value,
  resolves `secretKeyRef`, and boots every committed overlay — and the replay
  logic exists in exactly one place.

- **`boot-check.sh` no longer hands one workload's environment to another
  image.** With the admin console added, the script collected `env:` from every
  rendered Deployment, so the console's `FERROEHR_ADMIN__*` variables (a separate
  binary with its own config root) reached the CDR image, whose strict sweep
  correctly refused them and reported a crash-loop for a deployment that runs
  fine. Environment collection is scoped to the CDR Deployment.

- **The Helm chart's shipped values overlays now produce configurations the
  server accepts.** All three rendered, linted and matched their goldens while
  none of them would boot: `default-values.yaml` enabled authentication with no
  mechanism, `basic-auth-values.yaml` carried a placeholder that is not an
  Argon2id PHC string, and `all-features-values.yaml` carried a 15-byte HMAC
  secret under the RFC 8725 §3.5 32-byte floor and enabled SMART without its
  required public base URL and endpoint metadata. The tag lane boots the
  `appVersion` image against the rendered default, so the next release cut would
  have failed. (#2159)

- **The `audit` schema was unreachable under the least-privilege role model.**
  It carried no grants at all, so only the database owner could write audit
  records — a deployment connecting as `ferroehr_app` could not record one. The
  runtime role now holds exactly `SELECT`, `INSERT`, a column-scoped `UPDATE`
  on the two delivery stamps, and `EXECUTE` on the reaping and verification
  functions. The compose PostgreSQL image also pre-creates the `audit` schema
  alongside `ehr` and `ext`. (#2049, #2059)

- Publishing a chart-only fix between releases works again. The chart lane's
  pre-flight check — that a plain `helm install` of the chart will not
  crash-loop — now compares the chart against the image it is actually judged
  with: the released image on a `vX.Y.Z` tag, where `appVersion` is that
  release, and an image built from the current tree on a manual chart-only
  run, where `appVersion` still names the *previous* release and therefore
  rejects every configuration key added since it. The check itself is
  unchanged in strength; only the image it compares against is now the right
  one, and the run log and job summary name that image and why it was chosen.
- A `vX.Y.Z` tag starts the image build and the chart publish at the same
  moment, and the chart lane's pre-flight pulls the image the other lane is
  still pushing. The chart lane now waits for that specific build rather than
  retrying a registry pull in the dark: if the image build fails, the chart
  lane fails immediately with a link to it instead of presenting the race as a
  broken chart.

- **The published images carry OCI annotations, so registries can read their
  description.** GHCR showed "No description provided" for all three packages
  even though each carried a description *label*: a label lives in the image
  config blob, while a registry reads the description from the image *index*
  annotation, and no annotation was being written at all. All three lanes now
  emit annotations at both index and manifest level.
- **The container images no longer claim the wrong licence.** All three
  Dockerfiles declared `org.opencontainers.image.licenses="Apache-2.0"` for an
  MIT-licensed project. Images published by CI were unaffected — the workflow's
  label overrode the Dockerfile's — so this was only ever visible to someone
  building the Dockerfiles directly, which is the documented compose path. The
  app and console images now declare `MIT`, and the postgres image the SPDX
  expression `MIT AND PostgreSQL`, which is what it actually contains.

- Service-layer refusals now report the precise openEHR Service Model call
  status they were raised with, instead of a generic stand-in. An unusable
  archetype-id regex pattern answers `invalid_id_pattern` (not
  `precondition_violation`) and an invalid stored query `invalid_query` (not
  `content_invalid`), and every conflict, bad request and semantic refusal
  keeps its own status through the service boundary. The HTTP status and
  response body of every affected request are unchanged — only the status a
  Service Model caller reads back becomes accurate.
- An unusable id pattern, an unparseable stored query and an unparseable
  ad-hoc AQL query now carry the underlying parser failure as an error cause,
  so an operator can read the full diagnosis from the server log.

- `auth.oidc.hmac_secret_file` and `auth.oidc.jwks_json_file` now appear as
  their own reference lines in the shipped `ferroehr.toml` template, so
  `ferroehr config default` teaches them. Both were real configuration keys
  mentioned only inside a trailing comment on another key's line, in a file
  whose header calls itself the complete server configuration.

- Documented `gh attestation verify` invocations no longer promise a result on
  the Helm chart or on `3.17.3`: no chart version has been published, and signing
  landed after that tag was built. Both chapters now say so, and the stale
  "no published artifact carries an attestation" warning on the admission
  policies is replaced by a narrower note about the one thing still unproven —
  that no admission controller has yet run these policies against a live image.

- **The AQL printer emitted queries that meant something different when read back.** Found by the new fuzzer, and it is a correctness defect rather than a formatting one: `to_aql` dropped parentheses that the grammar needs, so re-parsing the printed text produced a *different* query. Two causes. The `AND`/`OR` operators are stated in the grammar as binary alternatives of one recursive rule, which resolves left-associatively — so a same-precedence operand survives a re-parse on the left and silently **re-associates** on the right, and the printer treated both sides alike. And a `CONTAINS` chain used as a boolean operand absorbs whatever operator follows it, moving that operator *inside* the CONTAINS scope. The printer's own documented invariant — parse(print(q)) == q — now holds across every boolean shape, asserted rather than assumed.

- **The container images declare their user numerically, so Kubernetes can actually verify it.** Both images stated `USER nonroot:nonroot`, which reads well and defeats the check it was meant to support: the kubelet cannot resolve a username against an image it does not read, so `runAsNonRoot: true` without an explicit `runAsUser` refuses the pod with *"cannot verify user is non-root"*. The distroless base already declared the numeric `65532`, and that line was overriding it with a name. Both now declare `USER 65532:65532` — the same identity, stated the way every consumer can check. The Helm chart was unaffected because it pins `runAsUser: 65532` itself; a plain `kubectl run` of these images was not.

- **Release artifacts carry their signature with them, so verification no longer needs GitHub.** Each release now attaches the Sigstore bundles as assets beside the binary and its SBOM, which means a consumer can verify an artifact on an air-gapped host with nothing but the download in hand: `gh attestation verify <tarball> --bundle <tarball>.sigstore.json --repo rubentalstra/FerroEHR`. The attestations were already being produced and stored in GitHub's attestations API; what was missing was the form in which a signature travels with the file.

- **The Helm golden-render gate was inert in CI, and the chart publish lane could not run at all.** Both jobs passed `version-file:` to `azure/setup-helm`, which has no such input — its inputs are `version`, `token` and `downloadBaseURL`. An unrecognised input is only a **warning**, so the action quietly installed the newest Helm instead of the pinned one; the golden comparison then skipped itself ("running helm 4.2.3, goldens are pinned to 4.1.3") and the job went red without ever comparing a render. The workflows now read the version out of `deploy/helm/.tool-versions` and pass it as the input that exists, so the pin means what the file says it means.

- **The conformance lane could not boot its own server.** Two configuration rules added by the SMART hardening this cycle — a plaintext `smart.public_base_url` is refused without the named development flag (RFC 6749 §3.1.2.1, RFC 8414 §6.2), and an empty `token_endpoint_auth_methods_supported` is refused because a document advertising a token endpoint and no client-authentication method describes an authorization server that authenticates no client at all — are both correct, and the lane's compose overlay satisfied neither. The system under test exited during compose, so not one conformance case ran. Fixed in the overlay, with the reason for each key stated where it is set.

- **The chart's publish lane checked only half of what it hands the server.** It validated the rendered `ferroehr.toml` against the `appVersion` image but not the `FERROEHR__*` environment the Deployment declares — and the strict boot-time sweep refuses an unknown variable just as firmly as an unknown TOML key. The lane now replays the declared environment, with a real file behind every `*_FILE` path, so a chart whose environment grammar outruns its image is refused before publication rather than discovered as a crash-loop.

- **Two scripts on the chart's build and publish paths required Python.** The Artifact Hub changelog derivation and the chart validator's manifest check are now `awk`, matching the repository-wide rule that there is no Python here. The rewritten changelog derivation was verified byte-identical to the previous implementation across both a released section and the unreleased one (21 and 88 entries). The validator's structural check kept the coverage it uniquely provided — every rendered document must carry a top-level `apiVersion` and `kind` — after confirming that YAML well-formedness is already enforced twice, by `helm template` and `helm lint`.
- **The Swagger UI showed a blank page at the URL the documentation gives you.** `/ferroehr/rest/swagger-ui` answered `200` with the right HTML, and nothing on it loaded: the bundled `index.html` references its stylesheet and scripts relatively (`./swagger-ui.css`), so without a trailing slash the browser resolved every one of them against the parent directory and got a `404`. You had to know to type `/ferroehr/rest/swagger-ui/index.html`. The mount path now redirects there, so the documented URL works. The target is `index.html` rather than the trailing-slash form deliberately — the path-normalizing middleware strips a trailing slash, which is what made the previous attempt at this an infinite redirect loop.

- **The four credential keys that had no file route now have one, the database DSN among them.** The configuration rule is that every secret gets a `*_file` sibling so it can arrive as a mounted read-only file instead of an environment value — an environment value is readable through `/proc/<pid>/environ` and is inherited by every child process. Four keys were missing theirs, and they included the most valuable secret in the deployment: `db.url` (the DSN), `events.url` and `fhir.outbound.url` (AMQP URLs carrying credentials), and `auth.basic.users[].password_hash`, whose only route was inline in a configuration file. So the three secrets that already had a file route arrived as mounted files while the DSN travelled as environment. Now `db.url_file`, `events.url_file`, `fhir.outbound.url_file` and `password_hash_file` all work, through the config file or the environment grammar (`FERROEHR__DB__URL_FILE`, …), with redaction and boot validation identical on both paths — a file-delivered DSN redacts its credentials exactly like an inline one, and a file-delivered password hash faces the same Argon2id parameter floor. The inline and environment forms are unchanged: this adds a route rather than removing one, so a Compose deployment needs no changes. Setting a key both ways is a boot error, and the built-in development default does not count as "set", so a `*_file` alone works without blanking anything first.

- **The image scanners were ignoring a setting their own configuration claimed to carry.** `trivy.yaml` documented `ignore-unfixed` as one of its two deliberate choices, but the key lived only as an input in the container workflow — so any other lane reading that config got a different gate. Measured on the published PostgreSQL image, the same scan reports **55** HIGH/CRITICAL findings without the setting and **0** with it, all 55 being CVEs with no fix available. The setting now lives in `trivy.yaml` under `vulnerability:`, where every lane inherits it. (The nesting matters: a top-level `ignore-unfixed` key is silently ignored by Trivy.)

- **A partial database wipe bricked the deployment permanently, and now it explains itself instead.** Almost every object the server creates lives in the `ehr` schema, but the cold archival tier lives in its own `cold` schema — so a `DROP SCHEMA ehr CASCADE` (a restore gone wrong, a recreated volume, a wiped test database) took the primary tier and the migration bookkeeping and left the archived clinical rows standing. The next boot then failed with a bare `relation "vo_version" already exists` and **every restart retried the same failure**: permanent `CrashLoopBackOff`, with no message naming the cause and no path out short of hand-written SQL. Observed on a live cluster, not reasoned about.

  The server now detects the state before migrating and refuses with the cause and both remedies in the message — restore the whole database from backup, or, if the wipe was intended, `DROP SCHEMA cold CASCADE`. Making the migration silently re-runnable was considered and **rejected**: those mirror tables were created from the primary tables as they stood at the time, so adopting a survivor could leave the archive tier a different shape from the tier it mirrors, and its rows are clinical content belonging to a repository that no longer exists. A crash with a remedy beats a database that quietly re-adopts another repository's data. The operations page documents both directions of a partial wipe, including the reverse one the boot check cannot see, and says what to do instead: drop the **database**, which has no partial-state failure mode.

- **A Helm-configured Basic user's Argon2id hash was written into a ConfigMap.** The chart rendered `.Values.config` verbatim, so any secret in the configuration tree landed in an object nothing treats as sensitive: readable with namespace read, pasted wholesale into support tickets, collected by backup tooling that skips Secrets, and not covered by Secret encryption at rest. An Argon2id hash is not a plaintext password, but it is an offline cracking target — which is the whole reason this server enforces the OWASP parameter floor at boot. Two more live instances of the same defect came out with it: `audit.fhir_feed.url` and a terminology `OAuth2` client secret, both of which the chart had no secure route for at all.

  The chart now classifies every key it renders, by **name shape** rather than against a list of today's keys, so a secret key added to the configuration tree in a future release cannot leak by default. A secret with a `secrets:` route (`authOidcHmacSecret`, `signingKeyPassphrase`, `multimediaSecretAccessKey`, the new `auditFhirFeedUrl` and `terminologyOauth2ClientSecrets`, the DSN and the two AMQP URLs) is refused under `config:` with the key that carries it named in the error. A secret with **no** route — today only `auth.basic.users[].password_hash`, whose configuration key has no `password_hash_file` sibling to point at a mounted file — moves the whole rendered `ferroehr.toml` into the chart's Secret, and no ConfigMap is created at all; the install notes say which object your release used. Verified on a live cluster: the Deployment serves authenticated openEHR requests with its configuration mounted from the Secret, and no ConfigMap in the namespace contains the hash.

- **A rotated secret or an edited ABAC policy did not roll the pods.** The `checksum/config` annotation hashed the ConfigMap template only, so a change under `secrets:` or `config.files` reached the mounted volume while every running pod kept serving with the value it read at boot. The annotation now covers the rendered configuration, the mounted files and the secret values together.

- **The Helm chart could not start a pod at all, and only a real cluster could show it.** Everything about the chart had been verified by rendering its YAML; deployed for the first time, every pod crash-looped. Kubernetes injects a set of [Service link environment variables](https://kubernetes.io/docs/concepts/services-networking/service/#environment-variables) for each Service in the namespace, and for a Service named `ferroehr*` those (`FERROEHR_SERVICE_HOST`, `FERROEHR_PORT_8080_TCP_ADDR`, …) land inside the server's reserved `FERROEHR_` configuration namespace, where the strict boot-time sweep rejected them and refused to start — the install command from the documentation could not work. The chart now pins `enableServiceLinks: false`, which this workload never needed, and the chart's own validation gate asserts the line so it cannot be dropped again.
- **The chart's default-deny ingress NetworkPolicy was documented as narrower than it is.** With `networkPolicy.ingressFrom` empty the rendered rule carries no `from` selector, and a rule without `from` admits every source — other namespaces included, not "any pod in this namespace" as the values file claimed. Only the port list is narrowed in that state. The values file, the template and the documentation now say so, and point at setting `ingressFrom` for a PHI workload.
- The XML reader had no nesting bound, so a deeply nested document recursed the generated readers off the stack — and a Rust stack overflow **aborts** the process rather than unwinding, which means the catch-panic layer that renders this server's clean `500` could not intercept it: one request would have taken the process down for every caller. Nesting is now bounded (256 levels, released as elements close, so wide documents are unaffected) and refused with a typed error. A `DOCTYPE` declaration is also refused outright now: entity attacks were already impossible because the XML library parses no DTDs, but that is a property of a dependency's current behaviour, and canonical openEHR XML has no use for a `DOCTYPE`.
- The API's `Content-Security-Policy` was applied as an overriding outermost layer over a tree that includes Swagger UI, so `default-src 'none'` would have left the documentation page blank in a CSP-enforcing browser — in the default configuration, since Swagger UI is on by default. Swagger UI now carries its own policy, which needs no inline allowance at all (measured, not assumed, and pinned by a test).

- An over-limit **chunked** request body (no `Content-Length`) was answered `400` as a malformed body instead of `413`: the body-collection step turned the limit error into an empty body. Chunked and declared-length bodies now both refuse `413`.
- The unauthenticated `/health` family could disclose **database connection detail**: two indicators interpolated the raw driver error into the component `detail`, whose text can name the DSN host, the database and the connecting role, and a third served a failed check's panic payload. All three now serve the fact and log the cause.
- Request paths reached the log with their **query string** at debug level, and `subject_id` — an external patient identifier — is a query parameter by the specification's own design. The request span now records the path only.
- New top-level configuration key `spec_profile` (`development` | `stable`, default `development`; env `FERROEHR__SPEC_PROFILE`): selects the openEHR specification generation set the server runs as ONE coupled choice — `development` = RM 1.2.0 + BASE 1.3.0 + LANG 1.1.0 (today's behaviour), `stable` = the latest RELEASED generations, RM 1.1.0 + BASE 1.2.0 + LANG 1.0.0. The active profile and its generation versions appear on the boot banner and `GET /management/info`. Under `stable`, AQL that addresses specification surface the released generation does not define is refused with an error naming the active profile. Profile-change contract: `stable` → `development` is always safe (openEHR minor releases are additive); `development` → `stable` is supported only for data that never used development-only constructs — such stored objects are refused loudly at read, never silently down-converted.
- `docker-compose.keycloak.yml` — a standalone OIDC quickstart overlay. Download it beside the quickstart file and run `docker compose -f docker-compose.yml -f docker-compose.keycloak.yml up` to get a Keycloak identity provider on port 8081 with a ready-made demo realm (a confidential `ferroehr` client with the password grant enabled, and the user `ferroehr` / `ferroehr` carrying the `ADMIN` and `USER` realm roles) and the server's bearer-token validation pointed at it. Basic auth keeps working, so the server advertises `Basic, Bearer`. The realm travels inside the file — no repository checkout needed.
- `docker-compose.override.yml` — the from-source development stack for anyone working on FerroEHR itself. In a checkout it is merged automatically onto a bare `docker compose up`, so `docker compose up --build` builds the server, database, and admin-console images from the current sources and runs them with the fuller development configuration (`docker/ferroehr.dev.toml`: the three users `ferroehr` / `ferroehr-admin` / `ferroehr-readonly`, role-based access control enabled, and a `keycloak` profile with the full development realm).
- The quickstart Compose files are attached to every GitHub release, so a fresh evaluation is downloading `docker-compose.yml` from the release page and running `docker compose up`.
- Two new `[auth.oidc]` keys. `clock_skew_leeway_seconds` (default `60`, env `FERROEHR__AUTH__OIDC__CLOCK_SKEW_LEEWAY_SECONDS`) sets the leeway applied to the time-based token claims; values above `300` are refused at boot, because expiry leeway may be "no more than a few minutes" (RFC 9068 §4 step 6) and a large one silently extends every token's life. `allow_insecure_issuer` (default `false`) accepts a non-`https` issuer for development and test deployments only — it exposes token verification material to anyone on the network (RFC 8414 §6.2).
- A new `[authz.abac]` key `check_directory` (default `false`): submits DIRECTORY (`FOLDER`) operations to the policy decision point. It replaces the old rule that inferred the opt-in from a `directory` entry in the remote-PDP policy map, so the check now works under the embedded Cedar engine too — previously a Cedar deployment could only enable it by inventing a policy name Cedar never reads. If you enabled DIRECTORY checks that way, set `check_directory = true`.
- A CI guard that keeps the Compose files' default image tags pinned to the workspace version, so a downloaded quickstart file can never reference images from a different release.
- The `openehr-lang` crate's `v1_0` generation is now the TRUE LANG Release-1.0.0 surface, not a copy of the development line: it is generated faithfully from the release's own machine-readable BMM, its vendored grammar set is the release's actual syntax-appendix files (`vendor/grammar/v1_0/`, incl. that era's `base_lexer.g4`), its lexer carries the ODIN reading alone (1.0.0 publishes no EL grammar and BEL first appears in 1.1.0), and its ODIN reader enforces the release's own syntax: lowercase-only attribute keys, ANY primitive comparable value as a container key (real/boolean/character/term-code/duration keys, signed numerics), whole-document plug-in fragments, comma-only fractional seconds on times and dot-only on durations. Under the `stable` spec profile these are the rules ODIN text is read by. The ADL/cADL grammar set is likewise version-scoped by AM generation (`crates/openehr-adl/vendor/grammar/{v1_4,v2_4}/`, vendored by the new `scripts/vendor/adl-grammars.sh`).
- The `openehr-rm` and `openehr-base` crates now ALSO emit the latest RELEASED specification generations beside the development pins — `openehr_rm::v1_1` (RM 1.1.0, resolving against BASE 1.2.0 per its own BMM `includes`) and `openehr_base::v1_2` (BASE 1.2.0) — each a complete peer: full type model, canonical-JSON codecs, RM attribute model, invariant cores, and validation behaviour. The served wire is unchanged (the current generations stay `v1_2`/`v1_3`); runtime selection between generations arrives with the `spec_profile` configuration key.

- The `s3` profile's SeaweedFS healthcheck could never pass: it probed `http://localhost:8333/`, and `localhost` resolves to `::1` first while the S3 gateway listens on IPv4 only, so `docker compose --profile s3 up --wait` waited forever. It now probes `127.0.0.1`.

- An unknown Basic username now performs the same key-derivation work as a known one, against a fixed decoy hash, so response time no longer reveals whether an account exists.
- The embedded Cedar policy engine now receives the caller: the principal was a constant with an empty role and scope set, so no role-aware or scope-aware policy could ever match, and every decision log named the same anonymous entity. The authenticated subject, its roles, its scopes and the operation id are now all available to a policy.
- A Cedar policy that errors during evaluation no longer changes the outcome silently. Cedar skips an erroring policy and reports it in its diagnostics, which were discarded — so an erroring `forbid` stopped forbidding. A policy set that cannot be evaluated is now a fail-closed 500.
- An external policy server with no configured rule for a resource kind now denies (and logs) instead of permitting, and the missing rule is refused at boot.
- Three authorization gates reached for the authenticated caller and, finding none, invented one named `UNKNOWN` — writing a false subject into the audit trail. They now refuse, as the fourth gate already did.
- A CONTRIBUTION's template ids are read from the typed COMPOSITION value rather than a JSON pointer that a shape change could silently stop matching.
- `auth.oidc.algorithms` is now bound to the configured key source: `HS*` with a JWKS (or `RS*` with only a symmetric secret) is a boot error, closing the algorithm-confusion setup where a public key is accepted as an HMAC secret (RFC 8725 §3.1). `none` is refused outright.
- JWK selection honours the `use`, `key_ops` and `alg` facets and refuses an ambiguous key set when the token carries no `kid`, instead of taking the first key in the document.

- The upstream-EHRbase comparison lane could not boot. A product-wide rename had swept the upstream image's OWN contract along with our own names: the database container's `EHRBASE_USER*`/`EHRBASE_PASSWORD*` init variables, the database name baked into that image's init script, and EHRbase's `/ehrbase/rest/...` base path (in both the party ixit and the readiness probe). The lane composes, becomes ready and drives cases again; only the strings upstream owns were restored, and the credentials we choose are unchanged.

- Under `spec_profile = stable`, a demographic party body carrying the RM 1.1.0 `PARTY.reverse_relationships` attribute is now accepted: the payload is validated by the released generation's own strict reader and the attribute — derived data the development line removed as redundant (upstream SPECRM-124) — is dropped on ingress rather than stored. Previously the stable server refused a valid instance of its own advertised generation with `400`. The `development` profile still refuses the attribute as undeclared, and the generation delta ledger now pins the REMOVED direction too, so a future spec re-vendor changing either direction fails the build until adjudicated.
- An ODIN integer container key whose lexeme cannot be evaluated (an out-of-`i64`-range magnitude, an inexact negative exponent) is now a parse refusal; previously it silently became the key `0`, so two such keys collided as duplicates and stored data could be mis-keyed. Signed integer keys (`[-1]`, `[+2]`) and exponent forms (`[29e2]`) now parse per the grammar's `integer_value : ('+'|'-')? INTEGER`. ODIN parse errors also now report the position of the real defect instead of the first backtracked branch failure.
- A bearer token whose `sub` claim is missing or blank is now refused with `401`. Previously a validated token without a subject authenticated as a principal literally named `unknown`, and that fabricated identity was stamped into the audit trail; an unattributable caller is now rejected instead of silently mis-attributed.
- Configuring both a symmetric secret (`auth.oidc.hmac_secret`/`_file`) and a static JWKS (`auth.oidc.jwks_json`/`_file`) is now a boot-time configuration error naming both keys. Previously the server silently used the symmetric secret and ignored the JWKS.
- An unreachable or unresponsive OAuth2/OIDC issuer no longer stalls bearer-token requests or hammers the issuer with outbound connections. The client that fetches the issuer's discovery document and JWKS now carries explicit timeouts, and a failed fetch is remembered briefly, so bearer requests during an issuer outage are refused fast instead of each one opening a fresh connection and waiting for the operating system's TCP timeout. Three new `[auth.oidc]` keys tune this (they apply only when signing keys come from discovery — that is, when neither `hmac_secret` nor `jwks_json` is set): `connect_timeout_ms` (default `3000`), `request_timeout_ms` (default `5000`), and `negative_cache_ttl_seconds` (default `10`, `0` disables). Successfully fetched keys keep their existing five-minute lifetime, and recovery is automatic once the negative entry expires.

### Security

- **Release tags are now protected by a ruleset.** Three lanes publish off a
  raw tag push — the release, the Helm chart, and the documentation version
  cut — and until now the tags driving them could be created, moved or deleted
  freely. A `release-tags` ruleset on `refs/tags/v*` blocks tag deletion and
  non-fast-forward tag updates and requires signatures, codifying the signed-tag
  practice the project already followed. Release immutability protects the
  window *after* a release is published; this protects the window in which a tag
  drives a build, an image push and a chart publish. `SECURITY.md` now records
  the full expected repository security posture, with the two read-back commands
  that detect a settings reset.
- **Every release tarball now ships a `.sha256sum`.** It is the only
  verification available to an operator with neither `gh` nor `cosign`
  installed, which is what a locked-down clinical environment tends to be. The
  documentation is explicit that a checksum detects a corrupt download and not a
  substituted release — only the Sigstore bundle answers "who built this".
- **A release now carries an SPDX SBOM of the repository**, generated from the
  release commit, alongside the per-binary CycloneDX SBOM and the per-image
  SPDX SBOM. The three answer different questions — what is inside the binary I
  am about to run, what am I redistributing and under what terms, and what is in
  the image's OS layer — and the security chapter now says which is which, so a
  reader picks the right one instead of assuming they are redundant. The
  CycloneDX specification version (1.5, the highest the generator emits) is
  recorded with its reason.
- **The release asset-completeness guard now matches asset names exactly.** It
  compared by substring, so the check for the `.tar.gz` was satisfied by
  `.tar.gz.sigstore.json` alone and a release missing its actual tarball would
  have been published — unfixably, since publishing freezes a release. It now
  requires a whole-line match, and additionally checks the checksum, the
  repository SBOM and the quickstart compose files.
- **Dependabot now covers the fuzz workspace and the container base images.**
  The fuzz harnesses are a separate Cargo workspace with their own lock file
  that a `cargo` entry at the repository root could never reach, so they were
  receiving no security bumps. The three base images are digest-pinned, which
  is correct for reproducibility and means they never receive a patch unless
  something proposes one — so the weekly image scan kept finding CVEs in the
  clinical images while nothing offered the fix.
- The admin console's `Content-Security-Policy` no longer allows inline scripts.
  `script-src` is now `'self' 'wasm-unsafe-eval' 'nonce-…'`, where the nonce is
  freshly generated for every response and stamped on the only inline script the
  console emits — Leptos's hydration bootstrap and its resource-serialization
  chunks. An injected inline script no longer runs. `style-src` keeps
  `'unsafe-inline'`, because the console's component library creates its
  stylesheets in the browser through the DOM without a nonce attribute; adding a
  nonce there would suppress the inline allowance under CSP Level 3 and leave the
  console unstyled, so the allowance stays with its reason recorded rather than
  being traded for a policy that looks stricter and works worse.

- The error-body hygiene check now also covers Service Model faults
  (`SmError::exception` and `exception`-coded call statuses), which are a
  second route to a `500` response body it previously did not inspect. Three
  boot-time terminology and subject-proxy failures were rewritten to keep the
  underlying diagnostic out of the message and carry it as an error cause
  instead, so it reaches the server log and never a client.

- Every one of the 20 GitHub Actions referenced by the build, test, release and publish pipelines is now pinned to a full commit SHA instead of a mutable tag, so a retagged or compromised upstream release can no longer change what runs against the tokens that publish this project's releases, container images and crates. Each pin carries its human-readable version in a trailing comment and was verified to belong to the named repository.
- 38 of the 40 repository checkouts in CI no longer leave the job's API token in `.git/config` for the rest of the job (`persist-credentials: false`), so a later step — a third-party action, a build script, an uploaded artifact — can no longer pick it up and push with it. The two exceptions are the documentation jobs that genuinely use git against the remote, and both are now annotated with the reason.
- All nine workflows now start from `permissions: {}` and grant each job only what that job actually uses, so write access to releases, packages, issues or Pages exists in the four jobs that publish and nowhere else. Previously a single workflow-level grant applied to every job in the file — `contents: write` to all of `release.yml`, `packages: write` to all of `containers.yml` — and `codeql.yml` declared nothing at all and inherited the repository default.
- The release tarballs and the crates.io uploads are now built with no compile cache at all, so a published artifact can only contain bytes produced from the tag being released. Both lanes were silently restoring one: the toolchain action they use enables caching by default, which the release job's own comment already assumed it did not.
- The per-architecture release tarballs are now built from the exact commit the release notes were read from, resolved once and passed on as a SHA, instead of each job resolving the release tag separately. A tag that moved between the two jobs could previously publish assets that did not match the release they were attached to.
- The build pipeline is now itself statically analysed on every pull request: a new `zizmor` gate audits the workflow definitions, and CodeQL analyses them as source alongside the Rust code. The properties above therefore cannot silently regress — an unpinned action, a checkout keeping its credential without a recorded reason, or a context value spliced into a shell command each fail the build.

- **The PGP signing private key is no longer world-readable inside the pod.**
  The `secrets` volume was projected at `0440`, but the sibling `config` volume
  carried no mode and so defaulted to `0644` — and that volume holds every
  `config.files` entry, whose documented use includes the PGP signing private
  key (`signing.key_path`), mutual-TLS PEMs and a JWKS blob, plus `ferroehr.toml`
  itself when the configuration holds a secret the chart cannot route out. Now
  `0440`, matching the secrets volume. Verified on a live cluster: the applied
  `defaultMode` is 288 (0440) and both server pods read their configuration and
  became Ready.

## [3.17.3] - 2026-08-05

### Added

- Archiving an EHR or a party (`POST /admin/archive/ehrs`,
  `POST /admin/archive/parties`) now **physically moves** the archived
  objects' rows out of the primary storage tables into a cold storage tier
  held in the same database (a new `cold` schema, added by migration
  `0007_cold_archive_tier`), instead of only flagging them. The primary
  tables — and their indexes — shrink by exactly what was archived, while
  the wire is unchanged: an archived EHR, composition, folder or party is
  still retrievable, still carries its full revision history, and is served
  from the cold tier; unarchived reads are untouched and never consult it.
  Writing to an archived object brings it back automatically, a physical
  delete clears both tiers, and an admin export still dumps archived
  content. Multi-tenant isolation is enforced on the new tier by the same
  row-level-security policy as the primary tables.
- **`ferroehr-ext` — the optional-integration crate.** The FHIR conversion
  core (mapping model + FLAT builder, outbound reverse-map, feeder-audit
  probe), the events transport (the `EventPublisher` seam, the AMQP
  publisher, the routing-key grammar), and the multimedia engine
  (content-addressed S3-compatible blob store + offload/expand transforms)
  now live in their own crate behind one additive cargo feature each
  (`fhir`, `events`, `multimedia`). Default builds are unchanged (all
  features on, wire-identical); a `--no-default-features` build produces a
  slim CDR that compiles the integrations out entirely and refuses an
  enabled-but-unbuilt integration loudly at boot. Configuration sections
  are unchanged and stay in the one config tree.
- **The typed FHIR R4B surface.** The ATNA `AuditEvent` the audit trail
  stores and forwards, and the `Parameters`/`ValueSet` responses an external
  terminology server answers with, are now built and read through a typed
  FHIR R4B resource model (`fhir-model`, contained entirely in the
  optional-integration crate behind its `fhir` feature) instead of
  hand-written partial structs. The audit wire bytes are unchanged. The
  terminology client is correspondingly stricter: a server response that is
  not a valid R4B resource — for example a `$expand` result missing the
  required `ValueSet.status` or `expansion.timestamp` — is now reported as an
  upstream fault instead of being partially read, and its response cache
  holds decoded results rather than raw JSON. A binary built with
  `--no-default-features` refuses at startup when `audit.store`,
  `audit.fhir_feed`, or an external terminology provider is configured; the
  DICOM/syslog audit feed and the in-process terminology bundle stay
  available.
- The OPT 1.4 constraints that target computed RM functions (`EVENT.offset`,
  `DV_PROPORTION.is_integral`, the US-spelled `null_flavor`) are now
  visible as a typed per-template report of unenforceable constraints
  instead of a silent skip; nothing new is rejected.
- The BMM v3 model gains MODEL-level navigation (`type_conforms_to`,
  ancestor walks, flattened property lookup) and generic-substituted
  property synthesis per the LANG generic-inheritance semantics; a new
  P_BMM schema-validity pass reports duplicate package listings,
  case-folded class-name collisions, and non-conformant property
  redefinitions with spec citations.
- The Expression Language has a native parser (hand-written over the
  vendored normative EL grammars): BMM_ASSERTION class invariants and
  routine pre/post-conditions in the BMM v3 model now parse into the
  published EL expression classes, with unparseable published-schema
  invariant strings collected as typed findings (319 of 400 pinned-schema
  strings parse; the remainder are Eiffel-flavoured forms the normative
  grammar does not admit, reported upstream).
- ADL rules and slot assertions are now modeled as full expression trees
  (the BEL expression object model), with the string form derived from the
  tree. Printed ADL 2 output changes minimally where the old form was
  wrong: each assertion in a multi-assertion block carries its own string
  form (previously the whole block repeated), `include`/`exclude` emit
  their keyword once per list per the grammar, a symbolic `∈` prints as
  `matches` (one operator in the model), and an archetype-id constraint's
  `; "assumed"` value is no longer dropped.

### Changed

- **The `openehr-adl` serializer seam is fallible.**
  `openehr_adl::print::print` and `openehr_adl::print::assertion_text` now
  return `Result<String, openehr_adl::print::PrintError>`. An in-memory
  archetype whose `rules` assign an `EXTERNAL_QUERY` is refused instead of
  serialized with the assignment's right-hand side silently empty: no
  released grammar spells `EXTERNAL_QUERY`, so no rendering of it could be
  valid ADL. The same refusal now covers a function-call expression node
  carrying no string name and a value-reference node carrying no string
  path — the two remaining shapes the printer used to render as empty
  text. Printed text for every other artefact is byte-identical.
- Served OpenAPI descriptions no longer reference the internal conformance
  register ("register-documented …"); each affected description states its
  adjudicated handling with the released citation it already carried. Wire
  behaviour is unchanged.
- The documentation site's comparison chapter (and every page that echoed
  it) no longer frames EHRbase as "upstream": FerroEHR and EHRbase are
  presented as two independent open-source openEHR CDRs measured by the same
  neutral instrument. The rendered comparison charts and generated tables
  carry the new labeling; provenance and licensing statements are unchanged.

### Removed

- **BREAKING:** the deprecated `auth.admin_scope` configuration key is
  retired. The management surface's `AdminOnly` access level now gates on
  the RBAC admin role (`authz.rbac.admin_role`, default `ADMIN`) — the same
  gate every Admin-class API operation already uses. Deployments that
  disabled RBAC keep the previous behaviour (any authenticated caller
  passes `AdminOnly`); deployments that set `admin_scope` should grant the
  admin role instead (a JWT `scope` entry naming the role continues to
  surface as that role via scope→role extraction).

### Fixed

- A CONTRIBUTION commit whose version `data` cannot be converted to its RM
  resource — an empty mandatory `1..*` container, a missing mandatory
  attribute — is now refused as `400 Bad Request`, exactly as the same bytes
  are refused on every direct commit route, instead of `422`. The commit
  seam runs the same strict canonical-JSON door the direct routes run
  (the released `responses/422.yaml` scopes 422 to content that "could be
  converted to a resource"); demographic party and party-relationship
  bodies take the same correction. Incomplete (`553`) commits keep their
  master06 relaxation.
- `EHR_ACCESS.settings` now constructs typed: the spec leaves
  `ACCESS_CONTROL_SETTINGS` open for scheme-defined subtypes, and the
  generated model carries such an instance verbatim through a validated
  open-subtype carrier instead of refusing every legal scheme instance.
  Canonical JSON round-trips a scheme instance byte-identically; canonical
  XML — which defines no mapping for scheme members — refuses one honestly
  instead of dropping content.
- **Commits into an OPT 1.4 archetype slot are now checked against the slot's
  allowed archetypes.** The slot's `include`/`exclude` archetype-id patterns
  were read only from each assertion's optional `string_expression` string,
  which most operational templates do not emit — so for those templates the
  slot admitted any archetype of the right RM type, and a composition
  carrying a filler the template never allowed was accepted. The patterns are
  now read from the assertion's expression tree (the constraint itself), with
  the string form used only as a fallback, so slot fillers are validated
  against what the template actually constrains. Templates whose assertions
  did carry the string form behave exactly as before, and a slot whose
  assertion constrains something other than the archetype id stays open as
  before rather than being narrowed by a pattern that does not apply to it.
- **Uploaded OPT 1.4 templates no longer lose their archetype-slot
  constraints.** The XML codec discarded the content of every element the
  schemas declare as `xs:anyType`, which on an operational template is the
  `EXPR_LEAF.item` carrying each slot assertion's archetype-id regex and its
  left-hand attribute path — so every OPT 1.4 slot constraint read back
  empty. Such an element is now kept verbatim (attributes, text and child
  elements) and re-serialized unchanged, so slot patterns survive a
  template round-trip and the OPT-to-ADL2 conversion and template-validation
  paths that read them now see the real payload. Canonical RM XML is
  unaffected.
- **A served Web Template (`application/openehr.wt+json`) no longer reports a
  field as mandatory when the template leaves it optional.** A node's `min`
  was taken from the constraint's `occurrences` alone, ignoring the owning
  single-valued attribute's `existence`; the two are orthogonal, and for a
  single-valued attribute existence is what governs presence. So an optional
  attribute carrying a `1..1`-occurrences constraint — for example
  `ISM_TRANSITION/careflow_step`, which openEHR declares optional — was
  published as `min: 1`. `min` is now the lower of the two. Container
  attributes, node `max`, and commit-time validation are unchanged.
- **ADL2 slot-narrowing validation (VDSSM) no longer stops at the first
  include it cannot read.** A specialised `ARCHETYPE_SLOT` whose `include`
  list mixes archetype-id regexes with constraint-based assertions was
  skipped entirely, so a genuinely widening literal after such an assertion
  went unreported. Each `include` is now judged on its own — an unreadable
  one is skipped, the rest are still checked. Symmetrically, no widening is
  claimed when the PARENT slot's admitted set is itself unreadable, and a
  restatement is now judged over all assertions rather than the regex ones
  alone, so neither case invents a prohibition.
- **Flattening a specialised archetype no longer drops an inherited tuple
  constraint.** A child node's `[a, b]` attribute-tuple wholly replaced the
  flat parent's tuple set, so a parent tuple over a disjoint attribute group
  silently vanished from the flat form (and from every operational template
  and Web Template built from it). Tuple overlay now merges by
  member-attribute group: a child tuple redefines the parent tuple over the
  same group, tuples over other groups are inherited, and a group the parent
  does not carry is added.
- **A ROLE carrying an empty `capabilities` list (and a party carrying an
  empty `relationships` list) is now refused at parse (`400`)** — the RM
  invariants `Capabilities_valid`/`Relationships_validity` forbid
  present-but-empty; the generated model now emits these optional lists as
  `Option<NonEmptyVec<…>>` (the emitter's invariant matcher learned the
  BMM's `.empty` spelling and conjunction form), so a lenient acceptance is
  unrepresentable.
- **Template-mediated Simplified-Format commit failures now answer `422`
  instead of `400`**: a missing mandatory ctx field (`language`/`territory`),
  a `|code` outside a closed value set with no `|value`, a datatype mismatch,
  or any other post-conversion validation failure of a body that was readable
  as FLAT/STRUCTURED (register entries AMB-207/AMB-208). Refusal messages now
  name the actual defect. Template-independent FLAT syntax violations
  (`|other` conflicts, malformed keys) keep answering `400`.
- The six missing-mandatory conformance expectations, the `create_ehr` and
  `update_directory` binding outcomes, the `invalid-other-details` fixture,
  and the shared EHR-Extract import identity were corrected on the catalogue
  side after spec-adjudicated triage; the CNF runner's ETag matcher gained
  released-grammar structural tokens for the object-id and template-id
  segments plus a validate gate that refuses un-resolvable matcher
  placeholders.

## [3.17.2] - 2026-08-04

### Added

- **The eight `openehr-*` spec crates are published on crates.io** —
  `openehr-base`, `openehr-rm`, `openehr-am`, `openehr-adl`, `openehr-term`,
  `openehr-lang`, `openehr-query`, `openehr-its` — each with its own README,
  packaged license texts (MIT, plus Apache-2.0 where openEHR-derived material
  is embedded), and docs.rs documentation. Packages version on their own
  independent SemVer line (starting at `0.0.x`, permanently decoupled from the
  openEHR spec versions); the implemented spec version is carried by each
  crate's `SPEC_VERSION` constant. Releases after the first go through an OIDC
  trusted-publishing workflow (`publish-crates.yml`) — no long-lived registry
  token (#1886).

- RM validation now realizes every remaining register-visible class invariant:
  RESOURCE_DESCRIPTION(_ITEM), AUTHORED_RESOURCE, EXTRACT and
  EXTRACT_UPDATE_SPEC gained generated invariant cores wired into the typed
  dispatch — the machine-classified Unrealized register is at ZERO rows
  (#1623); EXTRACT_SPEC criteria and OPT-carried REVISION_HISTORY refusals are
  pinned by twins (#1648, #1737).
- EHR-Extract export now evaluates `EXTRACT_SPEC.criteria`: AQL criteria
  queries select each entity's primary set `$ehr`-bound (the entity's EHR
  scopes the query and a literal `$ehr` parameter binds to its id); a non-AQL
  formalism or an unparseable criterion is refused with `400`. The former
  blanket criteria refusal is gone (#1736).
- **On-demand CPU flamegraph of the running server:** `GET
  /management/flamegraph` — a new opt-in management endpoint (default off, like
  the whole surface) that samples the process with an in-process `pprof`
  profiler for a bounded window and answers with the rendered flamegraph SVG.
  `seconds`/`frequency` query parameters are capped by the new
  `[management.profiling]` config keys (`max_seconds`, `max_frequency`); a
  request beyond a cap is refused with `400`, a concurrent sample window with
  `409`. No openEHR spec governs the management surface — our own operational
  extension (#1861).
- **Span-timing flamegraph capture:** the new `telemetry.flame_file` config key
  installs a `tracing-flame` layer that writes folded stack samples of every
  span to the given file for offline rendering with inferno
  (`inferno-flamegraph < file > flame.svg`) — the async-attribution complement
  to the sampled-stack endpoint. Unset (the default) the layer is not installed
  at all. Our own telemetry extension (#1862).

- EHR Extract import now enforces the copy closure (RM common master06
  §Copying): a received branch version is refused with `400` unless its
  fork-point trunk version and same-branch predecessor travel in the same
  extract or are already stored (#1770).
- **FOLDER (DIRECTORY) resources can now carry ITEM_TAGs.** ITS-REST overview
  `Requests_and_responses.md` §openehr-item-tag and openehr-version-item-tag
  names FOLDER among the change-controlled resources the wrapper headers
  associate tags with, and the DIRECTORY write routes have always carried those
  headers — but the tag store rejected the FOLDER target type, so a tagged
  directory commit answered `409` with a raw PostgreSQL constraint string AFTER
  the directory version had already been created. FOLDER tags now store, echo
  and appear in the EHR-wide tag listing (`GET /ehr/{ehr_id}/tags`). No
  dedicated `/directory/…/tags` routes appear: the release defines none, and a
  FOLDER tag is reached through the wrapper headers and that listing.
- **Tag mutations emit IHE ATNA audit records.** Creating, replacing or
  deleting ITEM_TAGs now leaves an audit trail under the tagged resource's own
  DICOM class. openEHR is silent here — tags are outside change control, so
  they correctly produce no CONTRIBUTION and no AUDIT_DETAILS, and no released
  text substitutes anything — so this is our own design for a clinical
  repository.

- **A `553|incomplete|` commit may now omit mandatory data for every
  committable kind.** RM common `master06` §Incomplete Content states that in
  the `incomplete` state "mandatory attributes may be absent … single-valued
  attributes may have null values and container attributes may be empty, even
  though they may have minimum existence and cardinality respectively of one",
  and that such data "respects the same template and archetype(s), but with all
  existence and cardinality lower limits set to zero". Until now only the
  template/archetype layer relaxed, and only for COMPOSITION: a `553` commit
  whose content left a mandatory attribute absent or a `1..*` container empty
  was still refused `422` by the reference-model layer. It is now accepted —
  for COMPOSITION, FOLDER, `EHR_ACCESS` and the demographic party /
  relationship kinds, on the CONTRIBUTION route and on the direct
  COMPOSITION/DIRECTORY routes (via `openehr-version:
  lifecycle_state.code_string="553"`). `EHR_STATUS` is unchanged: the CNF
  schedule holds that the incomplete state does not apply to it, pending
  SPECPR-368. Only the presence and lower-bound checks relax — types, `_type`
  slots, terminology bindings, patterns, coded values and every other class
  invariant are enforced exactly as before, so an `incomplete` commit carrying
  data that is WRONG rather than merely missing is still refused ("data may be
  missing, but it may not be wrong").
- **The conformance catalogue can now state a CONTRIBUTION's commit audit.** A
  case's `audit:` block states only its delta against the derived envelope
  audit, and the reserved `absent` sentinel omits a member outright — the seam
  the mandatory-member refusals need (RM common
  `UML/classes/org.openehr.rm.common.audit_details.adoc` §Attributes makes
  `change_type` and `committer` 1..1; the released OAS
  `specifications/schemas/common/UpdateAudit.yaml` §required lists both on the
  commit DTO). Three cases land on it: an omitted `change_type`, an
  out-of-group `change_type` code, and the conformant twin that states both
  members and proves the client-supplied `time_committed` is not the recorded
  one. Cases that already authored an `audit:` block now actually put it on the
  wire.
- **A case pins which lineage head a default EHR-Extract exports.** With a
  trunk head and a branch head both open in one container, RM ehr_extract
  `master04-common_package.adoc` §Version Specification never says which is the
  "latest available version". The choice — the trunk head — is now adjudicated
  in the ambiguity register (AMB-206) and pinned by
  `I_EHR_EXTRACT_SERVICE.export_ehr_extracts-latest_across_lineages`.


- **Canonical-XML responses are now checked against the published openEHR
  XSDs.** A new gate serializes documents through the shipped codec and
  validates them with an XSD processor against both vendored ITS-XML bundles,
  and it records exactly where a served document and the schema its namespace
  declares disagree. The finding it makes visible: the `v1` bundle openEHR
  still publishes as the STABLE one is frozen at an older Reference Model
  generation, so a document that is a correct RM 1.2.0 instance can carry
  attributes that bundle never declared — `FOLDER.details` on a directory is
  the case you are most likely to meet, and 50 RM classes (EHR, EHR_STATUS,
  CONTRIBUTION, the demographic party types, …) have no `v1` schema at all.
  **Nothing served changes**: the default namespace is still `v1`, `Accept:
  application/xml; version=2` still selects `v2`, and the codec still writes
  the complete Reference Model rather than dropping clinical content to fit an
  older schema. Deployments that validate responses against the published XSDs
  should be aware that the `v2` lineage is the one that models RM 1.2.0 — and
  that it currently cannot be compiled by a standards-conformant XSD processor
  because of an invalid pattern in the upstream schemas. The full per-attribute
  breakdown ships in the conformance ambiguity register as `AMB-185`.

- **A conformance case for a party's inline, by-value relationships.** A
  `PARTY_RELATIONSHIP` is modelled twice and openEHR reconciles the two
  nowhere: the Reference Model stores it *inside* its source party ("the
  relationships attribute is by value"), versioned with that party and with no
  version container of its own, while the Service Model gives every
  relationship its own independently-addressed container. This server serves
  both, and they are **disjoint** — committing a party that carries an inline
  `relationships` list does not create a relationship container, and creating a
  relationship container does not append to any party's list. The catalogue now
  pins the inline half: a `PERSON` committed with a by-value
  `PARTY_RELATIONSHIP` is accepted and reads back with that relationship
  unchanged, unexpanded and un-repointed. Behaviour is unchanged; the
  adjudication ships in the conformance ambiguity register as `AMB-187`.

- **New conformance cases for the LOCATABLE root rules and the feeder-system
  audit.** The catalogue now pins the two refusals above from the wire side (a
  COMPOSITION root whose `archetype_node_id` contradicts its ARCHETYPED block;
  a COMPOSITION carrying an empty `links` list; a COMPOSITION whose inner
  `ITEM_TREE` carries an empty `archetype_node_id`), and four cases cover
  `FEEDER_AUDIT` end to end: a commit carrying audits at the COMPOSITION root
  *and* on an interior data node round-trips every modelled attribute
  (identifiers, inline `original_content`, both system audits, and the
  originating audit's `other_details`), an update that retains the feeder audit
  keeps it on the new version, an update that drops it is accepted and does not
  carry it forward, and an update whose content is identical to the preceding
  version still creates version 2.

- **Conformance cases for populated LINKs on a COMPOSITION.** A commit carrying
  a complete `LINK` at the COMPOSITION root *and* on an interior ENTRY now
  round-trips with both links intact (the accepting twin of the empty-`links`
  refusal), and a `LINK` whose `target` is not an `ehr://` URI is refused (422)
  — placed on the interior ENTRY so the case also proves the rule is applied
  below the resource root.

- **Conformance cases for `FOLDER.items` — the directory's reference slot.**
  The catalogue exercised directory folders, names, links and `details`, but
  never the attribute the whole abstraction rests on: a folder's `items` list
  holds *references* to other objects, never the objects themselves, and the
  same object may be referenced from more than one folder (that is what lets
  one directory classify a composition as both an episode and a problem). A
  directory whose two sibling folders both reference the same target — beside a
  second, distinct target — is now committed and read back with **both**
  references intact, so a server that collapsed the duplicate would fail; and a
  folder that carries a composition *by value* in `items` instead of a
  reference to it is refused with `422`, leaving the EHR without a directory.
  Two further cases pin how *wide* an identifier that reference slot accepts. A
  folder reference may be **version-pinned** — its id a three-part
  `OBJECT_VERSION_ID` naming one particular version of a composition — and it
  now round-trips with all three parts intact, so a server that truncated it to
  the leading UUID would fail. And a reference identified in a **foreign
  scheme** (a `GENERIC_ID`) is accepted and served back unchanged: the
  Reference Model types the slot at `OBJECT_ID`, whose family has six concrete
  members, while the published OpenAPI schema for the same slot enumerates only
  two — the adjudication ships in the conformance ambiguity register as
  `AMB-186`.

- **Conformance cases for the ATTESTATION wire family.** The catalogue now
  drives the `666|attestation|` CONTRIBUTION member end to end: attesting an
  existing COMPOSITION version is accepted, adds **no** new version, reports
  the attestation-only aggregate change type, and surfaces the completed
  `ATTESTATION` on both the version envelope and the revision history; the
  pending-then-signed pattern leaves **both** attestation objects on the one
  version; and three refusal twins pin the `ATTESTATION` invariants on the wire
  (a missing `reason`, a coded `reason` whose code sits outside the openEHR
  *attestation reason* group, and a present-but-empty `items` list). An
  `ORIGINAL_VERSION` carrying attestations is also pinned as a
  canonical-JSON/XML serialization vector. An attestation carrying an inline
  `DV_MULTIMEDIA` `attested_view` — the screen image of what was signed — now
  round-trips with its media type, size, inline data and alternate text intact
  on both the version envelope and the revision history.

- **Conformance cases for the generic-package party and participation rules.**
  A COMPOSITION whose context participation carries a bounded
  `DV_INTERVAL<DV_DATE_TIME>` `time` and whose ENTRY-level other participation
  carries an open-ended one now round-trips with every interval boundary
  intact, and four refusals pin the terminology and identity rules those
  classes carry: a coded `PARTICIPATION.function` outside the openEHR
  *participation function* group, a `PARTY_RELATED` participation performer
  whose relationship code is outside the *subject relationship* group, and — on
  the commit audit's `committer`, which sits beside the committed content and
  is therefore missed by a content-only validation walk — the same out-of-group
  relationship, a `PARTY_IDENTIFIED` carrying none of name, identifiers or
  external reference, and one whose name is the empty string. A `PARTY_RELATED`
  committer with an in-group relationship is pinned as the accepting twin, so
  refusing that party type wholesale no longer passes.

- **A conformance case for the third `PARTY_SELF` referral scheme.** The RM
  names three ways to refer to the record subject from inside an EHR, and the
  catalogue only exercised two of them. A COMPOSITION whose interior ENTRY
  carries a `PARTY_SELF` subject with a complete `external_ref` `PARTY_REF`
  (id, namespace and type) now round-trips with that reference intact, so a
  server that dropped or refused a per-instance subject reference — a
  spec-supported deployment style — no longer passes the catalogue.

- **A canonical-JSON output mode for the corpus fixture generator.** The
  `openehr-its` `canonical_convert` example now emits canonical JSON when the
  output path ends in `.json` (and handles `ORIGINAL_VERSION` documents under
  the published `<version>` root), so a committed JSON fixture can be the
  codec's own output rather than a hand-typed approximation of it.

- **Every figure the vendored specs reference is now vendored too.**
  `scripts/vendor/spec-docs.sh` additionally fetches, from the same pinned
  commits, exactly the figures the vendored chapters reference: the 129 UML
  class-diagram SVGs (`{uml_diagrams_uri}`, under
  `docs/specs/openehr/<COMPONENT>/docs/UML/diagrams/`) plus the 200
  per-document diagrams and images (`{diagrams_uri}` / `{images_uri}`, under
  `<COMPONENT>/docs/<doc_name>/diagrams/` and `.../images/`). Spec chapters are
  now readable offline with their figures intact instead of carrying dangling
  links. Only referenced files are taken, byte-for-byte; a referenced figure
  missing at the pin fails the vendoring run.

### Changed

- **The declared FHIR integration surface is now stated as R4B (4.3.0)
  everywhere** — the served OpenAPI descriptions, configuration docs, and
  website pages previously said "R4". Wire behaviour is unchanged: the
  resources FerroEHR touches (AuditEvent/BALP, terminology
  `Parameters`/`ValueSet`, the inbound connector starter set) are
  byte-identical between R4 (4.0.1) and R4B (4.3.0); the `/fhir/r4/…` connector
  paths are unchanged (#1885).
- The `openehr-*` spec crates are republished as `0.0.3` (lockstep): the
  generated sources are re-emitted in rustfmt-normalized form after the emitter
  changes that shipped with the typed-DTO campaign — no semantic change to any
  generated type or impl.


- The tenant admin API (`/admin/tenant`) and the event-subscription admin API
  decode their request bodies into typed definitions: a field of the wrong JSON
  type (`"enabled"` as a string, a non-string predicate or name) is now refused
  with `400` naming the offending member, where it was previously coerced to a
  default or treated as absent. Well-typed requests are unaffected; the
  response record shapes are byte-identical (#1694).
- **BREAKING (error class):** the spec model is now complete-by-construction
  across every crate: the cross-schema re-emission closure applies uniformly
  (openehr-rm re-emits BASE's `Interval`/`Iso8601` family, am14 re-emits
  `AUTHORED_RESOURCE`/`RESOURCE_DESCRIPTION`), and every `0..1` list carrying a
  present-implies-non-empty invariant is emitted `Option<NonEmptyVec<T>>` — a
  present-but-empty list (`"links": []`, `"contacts": []`, `"mappings": []`, …)
  now refuses with `400` at parse instead of `422`, on every strict write
  surface; the `553|incomplete|` relaxation is unchanged (#1699, #1730).
- **BREAKING (v1-pinned XML consumers):** the default canonical-XML lineage
  served for `application/xml` is now the ITS-XML **v2** namespace
  (`http://schemas.openehr.org/v2`) — the only published schema bundle that
  models the RM 1.2.0 this server emits. The v1 lineage stays selectable per
  request with `Accept: application/xml; version=1` (a non-default v1 response
  is labelled `Content-Type: application/xml; version=1`). Request payloads are
  unaffected — both namespaces are read regardless (#1666).
- **An `ITEM_TAG` whose key or value violates its own RM invariants is now
  refused when the payload is read, not after it is built.** RM
  `UML/classes/org.openehr.rm.common.item_tag.adoc` §Invariants states
  `Inv_key_valid` (`not key.is_empty and key.is_justified` — no leading or
  trailing whitespace) and `Inv_value_valid` (a present value must be
  non-empty) over `ITEM_TAG`'s own fields. Both now run at the type's
  construction door, which the canonical-JSON and canonical-XML readers build
  through, so an empty or whitespace-padded tag key is rejected at parse — in
  any document position, named by its path — instead of producing a value that
  existed in violation of its own class definition until something validated
  it. Tag requests that already conformed are unaffected.
- **The canonical-JSON reader honours the default values the vendored
  meta-model states.** A `Point_interval` payload that omits
  `lower_included`/`upper_included`/`lower_unbounded`/`upper_unbounded` now
  reads back with the schema's own declared defaults (included, bounded),
  matching what `Proper_interval` and `Multiplicity_interval` already did — the
  meta-model states those four values on `Point_interval` and only there, and
  the reader had been ignoring them.
- **BREAKING: a structurally invalid RM request body now answers `400`, not
  `422`, and the commit audit's coded members carry their released wire
  shape.** The commit routes (COMPOSITION, `EHR_STATUS`, DIRECTORY, EHR
  creation, the demographic party and relationship routes, and the EHR-Extract
  import) decode the request body into the concrete openEHR type before
  anything else runs, so the strict canonical reader is what judges its shape.
  A body that is not an instance of the addressed class at all — a wrong or
  missing `_type`, an undeclared member, a repeated member, an absent mandatory
  attribute, an empty `1..*` container, a malformed identifier, a PERSON posted
  to `/demographic/agent` — is refused there, and the ITS-REST overview
  (`Requests_and_responses.md` §HTTP status codes) assigns that class `400`
  ("could not be parsed or is invalid"), reserving `422` for a body that is
  "well-formed but was unable to be followed due to semantic errors". Bodies
  that ARE valid instances but break a semantic rule — a terminology binding, a
  template mismatch, a body `uid` naming a different versioned object — keep
  their `422` exactly as before. In the same change the commit envelope adopts
  the released `UpdateVersion.yaml` / `UpdateAudit.yaml` shapes end to end:
  `lifecycle_state` and `commit_audit.change_type` are `DV_CODED_TEXT`
  (`{"value": …, "defining_code": {"terminology_id": {"value": "openehr"},
  "code_string": "532"}}`) rather than the flat `TERMINOLOGY_CODE` spelling,
  `commit_audit.description` is a `DV_TEXT` object rather than a bare string
  (so a description may now be CODED when it travels in the body — the
  `openehr-audit-details` header still carries only the plain
  `description.value`), and `commit_audit` accepts its `UPDATE_ATTESTATION`
  subtype, which the released schema's discriminator has always allowed and
  which RM common `master06` §Committal and Audits names ("`AUDIT_DETAILS` … or
  its subtype `ATTESTATION`"). The committal request headers, the CONTRIBUTION
  route and every response body are unchanged. Simplified-Format
  (FLAT/STRUCTURED) input failures split along the same line: a
  TEMPLATE-INDEPENDENT format violation — a key breaking the FLAT
  field-identifier grammar, a `ctx/` key outside the master06 vocabulary, the
  forbidden `|other` + `|code`/`|value`/`|terminology` combination — is `400`
  (the document is not readable as FLAT), while template- or RM-mediated
  conversion failures (an unknown path, an undefined suffix, a missing
  mandatory `ctx` field, a closed value set) keep the composition endpoints'
  own `422`.
- **BREAKING (canonical XML): a version read now serves the `<version>`
  document element the published XSDs declare, instead of `<original_version>`
  / `<imported_version>`.** ITS-REST overview `Resources.md` §"XML Format"
  requires that "both request payloads and responses MUST conform to the
  [published XSDs]", and the ITS-XML schemas declare exactly one document
  element for a VERSION — `<xs:element name="version" type="VERSION"/>` — over
  an ABSTRACT `VERSION` type; neither published lineage declares an element
  named after a concrete subtype. The concrete class therefore rides on the
  root's `xsi:type` (`ORIGINAL_VERSION` or `IMPORTED_VERSION`), as XML Schema
  requires of an instance of an abstract type. Every `GET
  .../versioned_composition/{uid}/ version[/{version_uid}]` and
  `.../versioned_ehr_status/...` read requested with `Accept: application/xml`
  is affected, in both the v1 and v2 lineages. A schema-validating client
  rejected the old roots; a client that pattern-matched on them must now read
  `<version>` plus `xsi:type`. Canonical JSON is unchanged (the envelope keeps
  its `_type` self-tag), and no other resource's root changes.
- **`lifecycle_state` is now required on every CONTRIBUTION version** (`400`).
  SM `master03` §Version Update Semantics says "The `lifecycle_state` must be
  supplied in all cases", and the released `UpdateVersion` schema lists it
  under `required`; a member that omitted it was previously accepted and
  silently defaulted to `532|complete|`. A `666|attestation|` member is exempt
  — it commits no new version, so it has no version lifecycle state to supply.
- **A `DELETE` on a change-controlled resource now refuses a committal header
  that names a lifecycle other than `523|deleted|`** (`400`). The header was
  previously parsed and discarded, leaving a client believing an instruction
  had been merged that was not; a `DELETE` commits the logical-deletion
  procedure, which fixes the state. A `DELETE` with no lifecycle attribute is
  unaffected.
- **`other_input_version_uids` is refused on the CONTRIBUTION write wire**
  (`400`). The released `UPDATE_VERSION` schema declares no such property (and
  `NewContribution.versions` items are `UpdateVersion`), so the merge commit
  has no released shape — the same absence the import commit has. Merge
  provenance stays produce-only: it is still served on `ORIGINAL_VERSION`
  reads, and it is still preserved verbatim by the EHR-Extract import and the
  archive load, which reproduce a foreign version unchanged.
- **`422 Unprocessable Content` messages now follow one uniform shape** — `<RM
  attribute path> <what is wrong> (<invariant name>)`, for example
  `ATTESTATION.items must be a non-empty list when present
  (ATTESTATION.Items_valid)`. Internally the service layer carries every such
  refusal as structured data (the attribute path, the named openEHR invariant,
  and the nested class-invariant violations a validation pass produced) instead
  of a pre-formatted sentence, and renders it into the response body exactly
  once, at the REST edge. Most messages are byte-identical to before; the
  remainder were reworded into the uniform shape (chiefly the CONTRIBUTION
  version rules, item-tag rules, operational-template rules, `PARTY_REF`
  refusals, and the `VERSIONED_COMPOSITION` cross-version invariants, where an
  attribute is now written `COMPOSITION.category` rather than `COMPOSITION
  category`). A `PARTY_RELATIONSHIP` with an absent `source`/`target`, and a
  party with an empty `identities` list, are now reported by the RM decoder
  that already refuses them rather than by a second hand-written check — same
  `422`, different wording. Response body SHAPE, status codes and all
  `validationErrors[]` contents are unchanged, and the `422` body is
  spec-silent (`responses/422_COMPOSITION.yaml` declares no schema), so no
  declared contract changes.
- **A canonical-JSON object that repeats a member name is now REFUSED with
  `400`, naming the repeated member.** The reader previously let the last
  occurrence win. RFC 8259 §4 says object member names "SHOULD be unique" and
  that "when the names within an object are not unique, the behavior of
  software that receives such an object is unpredictable", and no conformant
  openEHR writer emits a repeated member — every attribute is written once, in
  model declaration order — so a repeated member is refused rather than
  silently resolved to one of its values. This applies to the `_type`
  discriminator as well as to modelled attributes.
- **Canonical-JSON refusal messages now carry the full JSON path to the
  offending node** (for example `(at $.content[0].data.items[2].value)`), so a
  client can locate the defect in a large document without bisecting it. The
  refusal wording itself is unchanged in kind: the offending member, the class
  that does not declare it, and the members that class does declare.
- **The canonical-JSON lexeme for a REAL in exponent form now writes a signed
  exponent** (`1e+21` rather than `1e21`). No openEHR specification governs the
  rendering of a REAL — both forms denote the same value and RFC 8259 §6 admits
  both — and the new form is what the reference JSON encoder produces. Only
  magnitudes outside the plain-decimal window (roughly `1e-5` to `1e16`) are
  affected; no clinical quantity in the conformance corpus changes by a single
  byte.
- **A canonical-JSON payload carrying an attribute the openEHR RM does not
  declare is now REFUSED with `400`, naming the path and the offending
  member.** The reader previously ignored an undeclared key. It is refused
  because that is the only reading under which the JSON and XML encodings of a
  resource share one data model: `ITS-REST/specifications/docs/overview/
  Resources.md` requires an XML payload to validate against the ITS-XML
  schemas, which declare no wildcards — an undeclared element cannot validate —
  and states the same for the JSON encoding at `SHOULD` strength, while
  openEHR's own published ITS-JSON schemas close 128 of their 134 object
  definitions with `additionalProperties: false`. The status is `400` rather
  than `422` because a document the reader cannot read never converts (the
  released status table: `400` is content that "could not be parsed or is
  invalid"; `422` is content that is "well-formed but was unable to be followed
  due to semantic errors"). The set of accepted attributes is the RM at this
  server's pinned version, so a payload that is valid openEHR is unaffected; a
  client sending a private extension member must move it into a modelled slot
  (for example `ITEM_TREE` `other_details`, or a `FEEDER_AUDIT`).
- **A malformed identifier is now refused wherever it appears in a document,
  not only in a request path.** `HIER_OBJECT_ID`, `OBJECT_VERSION_ID`,
  `VERSION_TREE_ID` and the `UID` family are built through a constructor that
  runs the released identifier grammar
  (`BASE/docs/base_types/master05-identification_package.adoc` §Syntaxes), and
  the canonical JSON and XML readers construct through it — so an identifier
  such as `PractitionerRole/12345-mock` in a `PARTY_REF`, or a one-part value
  tagged `OBJECT_VERSION_ID`, is rejected at parse with `400` instead of being
  stored and served back.
- **`UpdateItemTag` request bodies reject undeclared members.** The released
  OAS declares the schema `additionalProperties: false`, so an unexpected
  member is now a `400` rather than being silently dropped.
- **A committer `external_ref.id` supplied through the `openehr-audit-details`
  header must be a well-formed `HIER_OBJECT_ID`.** A malformed value is refused
  with `400` instead of being written into the commit audit.
- **`OBJECT_VERSION_ID` values on the wire are now checked against the full
  openEHR identifier grammar.** A version identifier in a request path, an
  `If-Match` header or a `VERSION.uid` previously only had to have three
  `::`-delimited parts with a well-formed version-tree id; its `object_id` and
  `creating_system_id` parts are now also required to be legal `uid` values —
  an ISO OID, a UUID, or an internet id
  (`BASE/docs/base_types/master05-identification_package.adoc` §Syntaxes).
  Values such as `bad id::sys::1` or `1234-5678::sys::1`, which no conformant
  client sends, are refused with `400` instead of being accepted; every
  identifier the spec admits is unaffected. The same grammar now backs the
  validating constructors of `HIER_OBJECT_ID`, `OBJECT_VERSION_ID` and the
  `UID` family, so a malformed identifier cannot be built through them.

- **AQL now rejects an `[archetype_node_id='…']` predicate whose value is
  neither an archetype identifier nor a node code.** `CONTAINS COMPOSITION
  c[archetype_node_id='openEHR-garbage']` used to be planned as an archetype
  constraint that could never match (a `200` with an empty result set); such a
  value is now a `400` naming it. The QUERY spec defines that bracket predicate
  as equivalent to the archetype (`[openEHR-EHR-…]`) and node (`[at0002]`)
  shortcut predicates (`QUERY/docs/AQL/master03-syntax.adoc` §"Archetype
  predicate" / §"Node predicate"), so those two forms are its whole admissible
  operand set — matching what the RM lets `LOCATABLE.archetype_node_id` hold.
  Well-formed archetype ids and at/id codes are unaffected.

- **Operational-template upload now enforces AOM2 VCACA's numeric arm.** A
  template that states a container cardinality wider than the reference model's
  — for example `CLUSTER.items cardinality {0..3}` against the RM's `List<ITEM>
  [1..*]` — is refused with `422` naming the rule
  (`AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc` §VCACA;
  `master08-validation.adoc` §Validate Definition). The fully-open `{0..*}`
  that published templates commonly carry is **not** affected: ADL 1.4 makes
  `C_MULTIPLE_ATTRIBUTE.cardinality` mandatory where AOM2 makes it optional and
  set "only if it overrides the underlying reference model", and cADL's open
  constraint means "any value permitted by the underlying information model"
  (`AM/docs/ADL1.4/master05-cadl.adoc` §"'Any' Constraints"), so an open
  interval states no override and defers to the RM. Every template that
  uploaded before still uploads.

- **Commit-audit and EHR-resource validation failures now report through the
  structured error body.** The `AUDIT_DETAILS.committer` and the `EHR_STATUS` /
  `EHR_ACCESS` / FOLDER / demographic-party commit checks are now produced by
  the shared Reference Model validator instead of duplicated by hand, so the
  same defects are refused with the same `422` status but render as the openEHR
  `Error` object (`{ message, validationErrors[] }`) rather than a flat message
  — the shape every other validation failure already used. The one
  message-detail change: a `PARTY_RELATED` committer whose `relationship` code
  is outside the openEHR `subject_relationship` group is still refused naming
  `Relationship_valid`, but no longer echoes the rejected code.

- **System-generated commits are now attributed to the product's own
  identity.** A write with no authenticated principal (auth disabled, or an
  internal write such as an import or a synthesized composition) records an
  `AUDIT_DETAILS.committer` of `PARTY_IDENTIFIED` name **`FerroEHR`**. It
  previously read `EHRbase` on the service paths and `ferroehr.local` on the
  REST adapter path; all layers now emit the one value. The deployment's
  configured `system_id` (`[server] system_id`, the value stamped into
  `AUDIT_DETAILS.system_id`, `EHR.system_id` and every
  `OBJECT_VERSION_ID.creating_system_id`) is unchanged and stays separately
  configurable. Audits already committed keep the committer they were written
  with.

- **Audit, attestation and revision-history wire bodies now follow canonical
  BMM field order.** `AUDIT_DETAILS`, `ATTESTATION`, `REVISION_HISTORY` and
  `REVISION_HISTORY_ITEM` are built as their RM types and serialized through
  the canonical codec instead of being assembled by hand, so every such body —
  the version `commit_audit`, the CONTRIBUTION audit, and both the EHR and
  demographic revision histories — now carries `_type` first followed by the
  Reference Model's own attribute order. Only key order changes; the same
  attributes with the same values are emitted, and JSON key order is not
  semantic, so parsing clients are unaffected.

- **Malformed attestation fields are now refused instead of stored.**
  `ATTESTATION.attested_view`, `proof` and `items` used to be copied into the
  stored attestation without being read; a submission whose `attested_view` is
  not a `DV_MULTIMEDIA`, whose `proof` is not a string, or whose `items` carry
  a member that is not a `DV_EHR_URI` is now rejected with `422` naming the
  offending attribute (and, for `items`, its index).

- **Outbound openEHR spec-defect reports moved to the issue tracker.** The
  `docs/conformance/upstream-reports.md` ledger is deleted; every report is now
  a GitHub issue labeled `upstream-report` (what the released spec says, what
  this implementation does, the resolution sought). The ambiguity register's
  `upstream_ref` field is renamed to `upstream_issue` and carries the GitHub
  issue number — the published `ambiguity-register.schema.json` changed
  accordingly.

### Removed

- **The AQL `RESULT_SET` no longer carries a top-level `id`.** Responses from
  `POST/GET /query/aql` and the stored-query execute routes previously added an
  `id` field holding a freshly minted UUID. The released ITS-REST `ResultSet`
  schema declares exactly `meta`, `name`, `q`, `columns` and `rows` with no
  `additionalProperties`, so the wire has no slot for it and the field was an
  undeclared property on a closed object schema. Clients that read `id` should
  use the response's `ETag` header instead — the released ITS-REST text names
  it "a unique identifier of the resultSet" (`query/Request.md` §"Common
  Headers and Query Parameters"), and it is unchanged by this removal.

### Fixed

- Licensing coverage corrected (#1883): the vendored openEHR **specification
  text** and the CKM-derived **clinical models** are CC-BY-SA 3.0 (per-file
  `licence` metadata for clinical models), not Apache-2.0 as the README
  previously implied. The repo now ships the CC-BY-SA 3.0 text
  (`LICENSE-CC-BY-SA-3.0`), vendors each upstream `LICENSE` alongside its tree,
  records the license in every vendored tree's `PROVENANCE.md`, and the
  documentation site gained a **Licensing & legal** page with the full
  reckoning. FerroEHR's own code stays MIT; the openEHR machine-readable
  artifacts and test corpora stay Apache-2.0.
- EHR-Extract import: the copy-closure check now matches the fork-point trunk
  version with ANY creating system (RM common master06 §Distributed Versioning
  — a branch legitimately forks off a foreign trunk); `/management/flamegraph`
  answers a well-formed SVG instead of a zero-byte body when the sample window
  catches an idle process.
- CNF runner: an unresolvable `<name>` placeholder in an outcome header matcher
  is now a loud case failure instead of silently wildcarding to `.*`; the
  structural tokens `<n>`/`<system_id>` resolve to their real grammars, and
  outcome matchers see the same merged variable scope as request building
  (#1852).
- AQL `LIKE` and `matches` predicates on multi-valued paths now use the
  existential (any-match) lowering the comparison operators already use — a row
  is matched when ANY node on the path satisfies the predicate, instead of an
  order-undefined single-node pick (#1448).
- The admin EHR dump/load archive now carries every `vo_attestation` row (with
  its `at_committal` flag), and load re-persists them verbatim — a restored
  version keeps its attestations and its stored signature verifies under
  `verify_on_read = strict` (#1685).
- **A node id carrying the at/id code leader but failing the code grammar is
  refused (`at0abc`).** AOM2's own code predicate is leader-based, so such a
  string claims code-hood and must satisfy the code syntax — previously it fell
  between the code family (whose grammar it fails) and the free-text family
  (whose leader-freedom it lacks) and no rule caught it.

- **The `OPTIONS /` conformance manifest serves the generated contract DTO, and
  the System API group joins the authorization and audit classifiers.** The
  manifest body is now the emitted `Options` DTO (byte-identical wire; a
  lockstep test pins the served OpenAPI's documentation shape to it), the
  System route table joins the RBAC route map explicitly, and the operation
  carries explicit authorization (any authenticated principal) and audit
  (application-activity) classifications instead of the fail-closed defaults.

- **A malformed ITEM_TAG refuses at construction, not after the fact.** The
  generated `ItemTag` type gains a validated constructor running its RM
  invariants (`Inv_key_valid`/`Inv_value_valid`), so a violating tag cannot
  exist as a typed value anywhere in the application — the JSON and XML readers
  refuse it at parse, path-named. Wire statuses are unchanged (the tag routes'
  422 mapping stays); a stored tag row that no longer constructs is reported as
  the server fault it is instead of being served.

- **An undeclared key on a CONTRIBUTION version member is refused (`400`, named
  at its member path) instead of silently ignored.** The released commit wire
  declares exactly six member properties (`UpdateVersion.yaml`) plus the
  adjudicated `_type` self-tag; the member seam was the last non-strict reader,
  accepting arbitrary extra keys without a diagnostic while every other read
  surface refuses them.

- **A CONTRIBUTION whose audit omits `committer` is now refused (`422`) instead
  of being attributed to the server's default identity.** The same released
  commit schema that requires `change_type` requires `committer`
  (`NewContribution.yaml` over `UpdateAudit.yaml`), and a server-invented
  committer would put an identity the client never named into the audit trail.
  The direct COMPOSITION/DIRECTORY routes are unchanged — there the committal
  headers stay optional and the authenticated default applies, exactly as the
  ITS-REST overview requires.

- **An emptied ITEM_TAG collection is echoed as no header, never an empty
  one.** The EHR-side write routes echoed an EMPTY `openehr-item-tag` header
  when the stored collection was empty — but the empty header value is the
  release's "remove all ITEM_TAGs" *request* instruction, so a mirroring client
  would read the confirmation of its own wipe as an instruction to wipe again
  (harmless) or, worse, treat state responses as carrying the destructive form.
  Both echo paths now share one rule: an empty collection emits no wrapper
  header.

- **Terminology failure bodies no longer disclose deployment configuration on
  the remaining two surfaces.** A terminology 404 named WHICH configured
  provider answered, and a commit whose archetype constraint binding had no
  configured terminology route answered a 500 revealing that routing gap; both
  bodies now carry only what the client can act on, with the operator detail on
  the trace record — completing the operator-detail adjudication for the
  terminology surface.

- **The generated ITS-REST contract is typed end to end.** The `emit-rest`
  generator resolved `$ref`s before emitting, so every request/response body
  and parameter lost its schema name and the trait/DTO surface degraded to
  untyped JSON values; RM/BASE payload references likewise degraded on a
  rationale that expired with the foundation rewrite (the spec types carry
  emitted strict serde impls). Body and parameter references now keep their
  names, RM/BASE references resolve to the typed spec structs — making every
  DTO field strict by construction — and a `discriminator.mapping` schema
  (`Versionable`) emits a real `_type`-dispatched enum instead of an untyped
  alias. Remaining untyped spots are honest: anonymous `oneOf` responses, query
  result rows, and schema-less OPT objects.

- **A node claiming a `_type` foreign to its slot is now refused everywhere,
  from the RM model.** The whole-instance validation pass dispatched each node
  on its own wire `_type`, so a tagged object sitting in a slot declared as
  something else was validated as the type it *claimed* to be — a `DV_TEXT`
  inside `COMPOSITION.content` (declared `List<CONTENT_ITEM>`) validated
  cleanly as a DV_TEXT. One model-driven rule now asserts every tagged node
  (root, single slots, and list members alike) conforms to its slot's declared
  RM type, read from the generated BMM attribute model; a scalar member of a
  class-typed list slot is refused the same way. The rule never relaxes on a
  `553|incomplete|` commit ("data may be missing, but it may not be wrong").
  The hand-written FOLDER member checks this replaces are removed; their
  refusals now come from the general rule.


- **The conformance suite's spec-citation gate now resolves the cited document
  and section.** It previously took only the second whitespace token of a
  citation and asked whether ANY path under the component directory contained
  it as a substring, so a citation naming a real component plus any common word
  passed even when the document and §section did not exist. The gate now
  resolves the whole path hint to a real vendored document (or chapter
  directory) and verifies every `§section` names a real section of it —
  following the `include::` directives that pull the UML class and interface
  tables into a chapter, and reading the class tables' own labels, the markdown
  chapters' titles, the AM validity-rule anchors and the OAS files' keys. It
  also covers the citations of fixture-set rows and of the corpus manifest, not
  just case cores. The 104 citations the strengthened gate found unresolvable —
  phantom chapters, sections that never existed, class tables cited under the
  wrong directory, and one citation of an internal proposal instead of a
  released spec — were re-derived first-hand and corrected.
- **A vendored corpus that had no vendor script has one.** The real-world
  canonical-JSON corpus under `crates/openehr-its/tests/vendor/` was
  hand-downloaded; it is now reproduced byte-identically from its pinned
  upstream commit by `scripts/vendor/openehr-sdk-json.sh` (with `--check` to
  report drift and write nothing). Its provenance record also named the wrong
  upstream repository — a product-rename sweep had rewritten `ehrbase/` to
  `ferroehr/` in the pin — which is corrected.
- **`authz.abac.enabled = true` now actually enables ABAC.** The server binary
  built an RBAC-only authorization handle unconditionally — the ABAC policy
  engine and its attribute resolvers were never constructed on the shipped run
  path, so a deployment that configured attribute-based rules ran without them,
  silently. The binary now boots the configured engine (Cedar or remote PDP)
  with database-backed attribute resolvers, logs the active authorization
  layers at startup, and **refuses to start** when an enabled ABAC block cannot
  be built (missing/invalid policy directory, unbuildable PDP client) —
  configuration that promises fine-grained authorization never degrades to
  authorization-off.

- **A one-group ISO OID now classifies as `ISO_OID`, not `INTERNET_ID`.** BASE
  `base_types` `master05-identification_package.adoc` §Syntaxes gives `iso_oid
  = number, { '.', number }` — one or more groups — while the UID subtype
  dispatch required two, so a bare numeric root such as `12345` was tagged
  `INTERNET_ID`, whose own production it violates (a multi-character
  `internet_id` label must begin with a letter). This picks the `_type` on the
  wire, so the value now round-trips under the subtype the grammar assigns.
- **Non-finite `Real` values now serialize to canonical XML in the `xs:double`
  lexical form.** The vendored XSDs type every `Real` element `xs:double`,
  whose lexical space spells the special values `INF`, `-INF` and `NaN` (XML
  Schema Part 2 §3.2.5); the serializer emitted Rust's `inf`/`-inf` spellings,
  producing a schema-invalid document. Finite values are unchanged (a whole
  `Real` still writes `120.0`).
- **The generated ITS-REST contract no longer drops single-`$ref` `allOf`
  composition.** Seven DTOs the released OAS defines as a named alias of
  another schema — `ItemTagOfComposition`, `ItemTagOfEhrStatus`, the five
  demographic `ItemTagOf*` — degraded to an untyped string map instead of
  resolving to their referent.
- **The generated ITS-REST contract now includes the SYSTEM API group.** The
  STABLE System API declares one operation, `OPTIONS /` (Options and
  Conformance), which the contract generator skipped: its group list omitted
  `system` and its HTTP-method table omitted `OPTIONS` entirely.

- **A composition committed against an ADL2-registered template is no longer
  refused with a `409`, and an in-use ADL2 template now refuses physical
  deletion.** The stored template identity on a version row was
  foreign-key-checked against the OPT 1.4 store only, so a commit whose
  template was provisioned through the ADL2 DEFINITION surface failed the
  constraint; the key now targets a registry spanning both template dialects.
  With that, the ADL2 template delete gains the same never-orphan guard the OPT
  1.4 delete always had — deleting a template still referenced by committed
  versions answers `409` with the reference count (previously the row was
  deleted silently) — and it now also evicts the template's compiled
  WebTemplate, so a re-uploaded template is never served from the deleted
  artefact's cached form.

- **Template-scoped authorization rules now bind to compositions committed
  through the direct routes.** A COMPOSITION committed with `POST`/`PUT
  /ehr/{ehr_id}/composition` stored no template identity alongside its version,
  while the same composition committed inside a CONTRIBUTION did — so an ABAC
  policy scoped to a template silently failed to match direct-route
  compositions (the attribute resolved to "no template" rather than to the
  template the composition declares). Both direct routes now record the
  template the version was committed against, exactly as the CONTRIBUTION route
  always has. Compositions committed before this release carry no template
  identity on their existing version rows; re-committing a new version records
  it.

- **A CONTRIBUTION whose audit omits `change_type` is now refused (`422`)
  instead of being given a server-invented one.** The released commit schema
  makes the change set's audit mandatory and its `change_type` a required
  member (`NewContribution.yaml` over `UpdateAudit.yaml`, for both the EHR and
  the demographic contribution routes), and RM common `master06` §Contributions
  calls the contribution-level value approximate and "not expected to be used
  as a computable value" — it is the client's account of its own change set.
  The server previously derived an aggregate from the member versions when the
  attribute was absent, putting an approximation into the audit trail under the
  client's name; it now answers `422` naming `CONTRIBUTION.audit.change_type`.
  A conformant client is unaffected. (The direct COMPOSITION/DIRECTORY routes
  are unchanged: there the committal headers stay optional and the server
  default still applies, exactly as the ITS-REST overview requires.)

- **An undecodable request-header value is refused instead of silently
  ignored.** A header whose bytes are not decodable as text — including the
  committal (`openehr-version`, `openehr-audit-details`) and item-tag wrapper
  headers — was dropped from the request as if it had never been sent, so a
  commit could carry different audit attributes or tags than the client
  supplied, with nothing on the wire saying so. Such a request now answers
  `400` naming the header.

- **An AQL query that cannot get a database connection now sheds with `503` +
  `Retry-After` instead of reporting `500`.** The query path's database leg is
  classified like every other one, so a pool-acquire timeout is reported as a
  temporary overload (retryable) rather than as a server fault. A corrupt
  stored FHIR mapping definition is likewise reported as a server fault (`500`)
  rather than as a client error (`422`) — it is not something the caller
  supplied.

- **A round-tripped demographic party carrying inline relationships is no
  longer refused.** `PARTY.Relationships_validity` requires every inline
  `relationships[i].source` to reference the party itself, and RM demographic
  `master02` §Party Relationships requires that reference to be a
  `HIER_OBJECT_ID` denoting the party's VERSION CONTAINER "rather than
  `OBJECT_VERSION_ID`s, which would denote particular versions" — but the check
  compared it against the body's `uid`, which on a served party is the
  three-part `OBJECT_VERSION_ID`. The two could never be equal, so a client
  that read a party, added a relationship and wrote it back got `422`. The
  comparison now uses the container id (the `object_id` of the version uid); a
  relationship sourced at another party is still refused.
- **The demographic create/update routes now honour the `openehr-version`
  lifecycle state.** ITS-REST overview `Requests_and_responses.md`
  §"openehr-version and openehr-audit-details" requires that "whatever is
  provided it MUST be merged with the default VERSION and
  `VERSION.audit_details` attributes on commit runtime"; the direct party and
  party-relationship routes threaded only the audit half, so a
  `553|incomplete|` demographic commit was reachable through a CONTRIBUTION
  alone. Both halves now merge on those routes (and on the SM-envelope entry
  points, which carry the same two attributes).
- **A read-only principal can export EXTRACTs again.** `POST /message/export`
  realizes SM `I_EHR_EXTRACT_SERVICE.export_ehr_extracts` — a query over held
  versions whose selector is a whole `EXTRACT_SPEC` — but the read-only gate
  classified the extension route by its HTTP verb and refused it `403`. It is
  now classified as the read it is (as the released ad-hoc AQL `POST` already
  was); the import routes stay writes.
- **A query result-set `ETag` is now stable across executions.** The tag is
  documented as identifying the `RESULT_SET` ("it changes as soon as the
  resource changes", overview `Requests_and_responses.md` §`ETag` and
  Last-Modified), but the digest covered `meta._created`, which is stamped per
  response — so every execution minted a fresh tag and conditional-request
  caching never hit. The digest now covers the result-determining content (`q`,
  the executed AQL, `columns`, `rows`) only.
- **A malformed `server.system_id` is now refused at boot.** The value occupies
  the `creating_system_id` position of every `OBJECT_VERSION_ID` this CDR mints
  (BASE `master05-identification_package.adoc` §Syntaxes: `creating_system_id =
  uid`), but the boot check only rejected an empty value and one containing
  `::` — so a configuration legal at startup could mint version identifiers
  this server's own reader refuses. The configured value is now validated
  against the openEHR `uid` grammar itself (`iso_oid | uuid | internet_id`).
- **`500`-class responses no longer echo internal diagnostics.** A server-side
  fault previously rendered whatever produced it straight into the response
  body: serde's parser message (naming Rust fields and byte offsets), the AQL
  executor's PostgreSQL driver string (naming generated SQL and schema
  objects), the node codec's RM attribute names and internal row shape, the
  authorization engine's failure reason, and the XML/Simplified-Format
  serializers' diagnostics. Every `500`-class body now carries a curated,
  opaque message and the full detail goes to the server's own log instead.
  `4xx` refusals are unchanged and still name the client-caused defect — a
  malformed request payload is still refused `400` with the parse error that
  explains it, which is the only thing a caller can act on.

- **A tag PUT body is now validated against the released write schema.**
  `schemas/common/UpdateItemTag.yaml` declares exactly `key` (required),
  `value` and `target_path`, with `additionalProperties: false`. Previously the
  body was read untyped: an undeclared member (`target`, `owner_id`, `_type`,
  anything) was silently dropped, and — worse — a `value` or `target_path` of
  the wrong JSON type was silently stored as ABSENT, losing a clinical
  annotation outright or changing the tag's identity so a later delete
  addressed nothing. All three are now refused `400`, naming the offending
  member by its JSON path, on the COMPOSITION, EHR_STATUS and all five
  demographic tag PUTs alike. An empty `target_path` still normalizes to
  absent, identically on both families.
- **A defective tag on a write no longer leaves the content committed.** The
  `openehr-item-tag` / `openehr-version-item-tag` headers are now parsed and
  invariant-checked BEFORE the content commit, so a request carrying an invalid
  tag is refused with nothing created. Previously the refusal arrived after the
  COMPOSITION / EHR_STATUS / DIRECTORY / party version was already durable, on
  a response with no `ETag` and no `Location` — leaving the client no way to
  learn what it had just created, and no recovery but a re-POST that duplicated
  the content. The tag write itself still happens after the commit, so tagging
  continues to cause no re-versioning of content.
- **The tag response header can no longer instruct a client to wipe its tags.**
  A valueless tag echoed as `value=""` (a shape the reference model forbids),
  and a tag list that could not be rendered as an HTTP header value — a control
  character in a tag key, which nothing in the reference model bars — fell back
  to an EMPTY header, which is exactly the byte sequence the spec defines as
  "remove all ITEM_TAGs". A client mirroring that echo back on its next write
  would have cleared the collection. A valueless tag now echoes without a
  `value`, and an unrenderable list omits the header entirely.
- **The tag wrapper-header parser is quote-aware and no longer silently drops
  entries.** A `target_path` containing a `;` inside quotes (an AQL predicate,
  say) shattered into fragments that then parsed as garbage; quoted runs are
  now opaque at the entry separator. An entry carrying no `key` was skipped
  past, silently discarding a tag the client believed it had set; it is now
  refused `400`.
- **Database integrity errors no longer leak schema names into client
  responses, and are no longer all reported as conflicts.** Every SQLSTATE
  class-23 violation mapped to `409 Conflict` carrying the raw PostgreSQL error
  text, so constraint, table and column names reached client bodies — and a
  CHECK or NOT NULL violation, which is a server-side invariant failure rather
  than anything a client can resolve, was presented as an optimistic-lock
  conflict to retry. Unique, foreign-key, restrict and exclusion violations
  keep their `409`; CHECK and NOT NULL now answer `500`. No branch returns a
  driver string: every client message is a fixed, actionable sentence, with the
  SQLSTATE, constraint and table recorded on the server's own trace record.
- **A tag that survives a whole-list replace keeps its creation instant.** The
  `PUT` is a full-collection replace, but re-asserting an existing tag identity
  is not the same as creating a new tag; previously every surviving tag's
  stored creation time was reset on any edit to a sibling, which the admin
  export then reported. Visible through `POST /rest/admin/…` EHR export.

- **A CONTRIBUTION version that declares a foreign version identity is now
  refused** (`400`, naming the offending key). The released commit wire
  declares six member properties (`preceding_version_uid`, `signature`,
  `lifecycle_state`, `attestations`, `data`, `commit_audit`) and no import
  shape at all — `master06` §Copying puts the import behind
  `commit_imported_version`, whose "details of version id etc come from the
  `ORIGINAL_VERSION`". Previously a member shaped like an `IMPORTED_VERSION`
  (`_type: IMPORTED_VERSION`, an `item` wrapping a foreign `ORIGINAL_VERSION`,
  or its own `uid`) was accepted and committed as a locally created
  `ORIGINAL_VERSION` under a freshly minted local identifier, silently
  discarding the identity and provenance the client had declared. All three
  keys are now refused. A member self-tagged `_type: ORIGINAL_VERSION` or
  `_type: UPDATE_VERSION` is unaffected — those name the class this wire
  commits — and importing versions that keep their foreign identity remains
  available through the EHR-Extract import route.
- **A version-container trunk position is now unique across creating systems.**
  `master06` §Copying has a second system BRANCH rather than extend the trunk
  of a copied container, and §Moving Version Containers continues the trunk
  increment under the new system's id, so a trunk line is one global sequence
  however many systems contributed to it. The schema previously admitted two
  versions of one container both claiming trunk position 2, one per creating
  system; the archive-load path could write such a pair. It is now refused with
  a message naming the container, the position and the system already holding
  it. Branch identifiers still legitimately repeat across systems, which is
  what the three-part version identifier disambiguates.

- **A version that carries data can no longer claim the `523|deleted|`
  lifecycle state** (`422`). `master06` §Logical Deletion states deletion as
  one procedure — create a new version, delete its data, set the state to
  `deleted`, commit — so a data-carrying deleted version is not producible by
  the spec's own steps. Previously such a commit was accepted through a
  CONTRIBUTION member pairing a content change type with the deleted state, or
  through `openehr-version: lifecycle_state.code_string="523"` on a `PUT`; the
  resource then read back as deleted (`204`) while its content stayed stored
  and AQL-queryable. Both routes now refuse it. Deleting through `DELETE`, or
  through a data-less `523` CONTRIBUTION member, is unaffected.


- **The demographic endpoints accept `PARTY_REF.type` `ANY`.** A `PARTY_REF`
  inside a party or `PARTY_RELATIONSHIP` body — `ACTOR.roles`,
  `ROLE.performer`, `PARTY_RELATIONSHIP.source`/`target` — was refused with
  `422` when its `type` was `ANY`, even though the composition endpoints
  accepted the same value. The demographic write boundary kept a second copy of
  the legal `PARTY_REF.type` set that had drifted from the single spec-cited
  one; it now judges every reference through that one definition, so the two
  surfaces give the same answer. Unknown type strings are still refused.

- **A `PARTY_REF` missing a mandatory attribute is refused.** A reference in a
  demographic body without an `id`, `namespace` or `type` (all `1..1` on
  `OBJECT_REF`) passed the write boundary and was only caught, if at all,
  further in. It is now a `422` naming the missing attribute.

- **The authorization gate refuses an unaddressable resource id instead of
  guessing.** A malformed `{uid_based_id}` — not a UUID, and not a well-formed
  three-part `OBJECT_VERSION_ID` — was previously read as if the whole string
  were the versioned-object id, so the template attribute the ABAC/SMART policy
  binds on silently came back empty and the request could pass a
  template-scoped rule. Such a request is now denied with `403`, in line with
  the gate's existing fail-closed handling of every other attribute-resolution
  failure. Well-formed ids (bare `HIER_OBJECT_ID` and full `OBJECT_VERSION_ID`,
  trunk or branch) are unaffected.

- **A version's digital signature now covers the attestations it was committed
  with.** openEHR signs "the entire Version object", excluding only the
  `signature` attribute itself, so an attestation supplied on the commit
  (`attestations` on the committed VERSION) belongs inside the signed form. It
  previously did not: such an attestation could be altered in the database
  without any signature check noticing. It is now part of the signed canonical
  form at commit and at read, in both the local-commit and the
  EHR-Extract-import paths, so tampering with it is caught by strict read-time
  verification and the served version verifies for an external reader that
  recomputes the digest itself. Attestations added *after* committal (a
  `666|attestation|` contribution) keep their existing behaviour: they
  post-date the signature by definition, and are served outside it. Versions
  committed before this change are unaffected — nothing is re-signed — unless
  they carried commit-time attestations, in which case their stored signature
  no longer matches and a `strict` `verify_on_read` deployment will report
  them; re-commit or set `verify_on_read = warn` while auditing.

- **An attestation whose optional `items` is sent as JSON `null` is no longer
  rejected.** A null optional means absent, and the sibling optional attributes
  (`proof`, `attested_view`, `description`) were already read that way; `items`
  alone treated it as a malformed list and returned `422`. That made
  commit-time attestations unusable through the typed API, which emits `null`
  for an omitted list. A present-but-*empty* list (`[]`) is still refused,
  which is what the RM invariant actually forbids.

- **A contribution mixing amendments and deletions now reports the amendment
  aggregate.** When a client sends no contribution-level change type, the
  server derives one; openEHR names `250|amendment|` for "a mixture of
  amendments and deletions that logically constitute a correction", which
  previously fell through to the general `251|modification|`. Uniform change
  sets and every other mixture are unchanged.

- **Versions received from another system are now served as
  `IMPORTED_VERSION`s, and imported records report their local chronology.**
  openEHR wraps a copied version: an EHR Extract import commits the received
  `ORIGINAL_VERSION` inside an `IMPORTED_VERSION` whose own contribution and
  commit audit record *this* server's act of importing, while the wrapped
  original keeps the source system's contribution reference, commit audit and
  signature. This server had never materialised the wrapper — it wrote the
  foreign commit audit as the version's own and discarded the received
  contribution reference entirely. Four visible changes:
- A `VERSION` read of an imported version
    (`…/versioned_composition/{uid}/version/{version_uid}`,
    `…/versioned_ehr_status/version[/{version_uid}]`, and a `resolve_refs`
    contribution read) now returns `"_type": "IMPORTED_VERSION"` with the
    received original under `item`. An `IMPORTED_VERSION` carries no `uid` of
    its own — it shares the wrapped version's identity — so read the version id
    from `item.uid.value`; the `ETag` is unchanged, so `If-Match` round trips
    are unaffected. Locally created versions are still `ORIGINAL_VERSION`s.
- An imported version container's `VERSIONED_OBJECT.time_created`, its
    `Last-Modified` header, its revision history and every as-of-instant read
    now report the **local import** instant instead of the source system's
    earlier clock, so a query for the record's past state returns what this
    repository actually held at that time.
- Re-exporting imported content now reproduces the received `ORIGINAL_VERSION`
    verbatim, including its source contribution reference; it previously
    carried this server's local contribution id under the source version's
    identity.
- With version signing enabled, the import act signs the `IMPORTED_VERSION`
    wrapper it creates; the wrapped original's own signature is stored and
    served untouched, and is never re-verified.

  An `ORIGINAL_VERSION` arriving in an extract without the mandatory
  `contribution` (or with a commit audit that is not a canonical
  `AUDIT_DETAILS`) is now refused with `400`, instead of being imported with
  the provenance silently dropped. Content imported before this release keeps
  the provenance it was stored with.

- **A demographic version container's `owner_id` now names the serving system,
  consistently, everywhere it is emitted.** `VERSIONED_OBJECT.owner_id` is a
  mandatory reference to "the containing EHR or other relevant owning entity",
  but a demographic party has no containing EHR — and this server had drifted
  into three different answers for it. `GET /demographic/versioned_party/{uid}`
  (and the party-relationship container read) served an `OBJECT_REF` in
  namespace `demographic` whose id was the container's *own* uid, a
  self-reference that merely duplicated the sibling `uid` field; the
  `X_VERSIONED_PARTY` wrapper of an EHR-Extract export served namespace
  `demographic` over the system identifier; and the demographic `ITEM_TAG`
  surface already served the shape the published openEHR `VersionedParty`
  example shows. All three now emit that one shape — an `OBJECT_REF` with
  `namespace: local`, `type: SYSTEM`, and a `HIER_OBJECT_ID` carrying the
  deployment's configured `system_id`. Clients that read `owner_id` off a
  demographic container will see the namespace, type and id all change; nothing
  else about those responses moves. The served OpenAPI example for the
  container read is corrected to match (it advertised a `PARTY_REF`, a type the
  published schema does not name there), the conformance catalogue now asserts
  the two tokens on the container read, and the adjudication is restated in the
  ambiguity register as `AMB-69` — whose previous text incorrectly claimed this
  server already emitted the published shape.

- **A path predicate carrying a parenthesised uniqueness modifier is now
  refused instead of silently matching nothing.** The Reference Model's
  directory chapter shows folder paths written with a name and a bracketed
  uniqueness modifier (`/folders[hospital episodes(car accident Aug 1998)]`), a
  form the formal openEHR path grammar this server implements does not define.
  Such a predicate used to be accepted and bound whole as an archetype node id,
  so the path quietly resolved to nothing; it is now a loud
  unsupported-predicate error. Plain node-id and archetype-id predicates
  (`[at0003]`, `[openEHR-EHR-COMPOSITION.x.v1]`) and bare name tokens are
  unaffected.

- **An OAuth2/OIDC committer's identifier now names the token issuer.** Every
  authenticated write stamps the committing principal into
  `AUDIT_DETAILS.committer` as a `PARTY_IDENTIFIED` carrying a `DV_IDENTIFIER`,
  whose `issuer` used to read `ferroehr` for every mechanism — including
  federated principals, whose subject the identity provider minted rather than
  this server. A Bearer principal's identifier now carries the validated token
  issuer (`iss`) as its `issuer`; a Basic principal, whose credential this
  deployment holds, keeps `ferroehr`. Audits already committed keep the issuer
  they were written with.

- **`ATTESTATION` commit audits and rich audit descriptions now round-trip
  instead of being silently flattened.** A CONTRIBUTION version whose
  `commit_audit` is an `ATTESTATION` — the openEHR way of committing content
  that is already signed, or that is marked as awaiting signature (`is_pending:
  true`) — used to be stored as a plain `AUDIT_DETAILS`: the concrete type and
  every attestation attribute (`reason`, `is_pending`, `proof`, `items`,
  `attested_view`) were dropped without an error, on the REST commit and on
  EHR-Extract import alike. They are now decoded, validated against the RM
  invariants, stored, and served back as an `ATTESTATION` on the version
  envelope, in the revision history, in the CONTRIBUTION rendering, and in
  exports/archives. A `commit_audit` whose `_type` names neither
  `AUDIT_DETAILS` (`UPDATE_AUDIT`) nor `ATTESTATION` (`UPDATE_ATTESTATION`) is
  now refused with **422** instead of being read as a plain audit.
  `AUDIT_DETAILS.description` is likewise kept whole: a `DV_CODED_TEXT`
  description keeps its `defining_code` instead of being reduced to its display
  string, and AQL can now address `commit_audit/description/value`,
  `commit_audit/description/defining_code/code_string` and
  `.../defining_code/terminology_id/value` as distinct fields (a coded
  description could previously never match). The `audit` table's baseline
  schema changes with this: `description` becomes `jsonb` (the whole `DV_TEXT`)
  and a nullable `attestation` column is added.

- **A coded description on an attestation keeps its code.** The
  `666|attestation|` commit path completed a submitted `UPDATE_ATTESTATION` by
  reducing its `description` to the plain text of a `DV_TEXT`, so a
  `DV_CODED_TEXT` description lost its `defining_code` permanently at committal
  — while the same attribute on a version's own `commit_audit` was already kept
  whole. An attestation's description is now stored and served back exactly as
  submitted (`_type`, display `value` and `defining_code`), and a `description`
  that is neither a string nor a valid `DV_TEXT` is refused with **422**
  instead of being dropped.

- **A simplified-format `ctx` participation without a function is now
  refused.** `ctx/participation_*` keys build `EVENT_CONTEXT.participations`,
  whose `function` the Reference Model requires; a FLAT/STRUCTURED commit that
  began a participation at some index (a name, an id, a mode or identifiers)
  but supplied no `ctx/participation_function:<i>` used to be completed with an
  empty function, committing a participation whose mandatory attribute carried
  no information. It is now rejected with an error naming the exact missing key
  (e.g. `ctx/participation_function:0 is required`).

- **The FLAT `_link:i` builder now reports a missing mandatory suffix instead
  of inventing an empty value.** `|meaning`, `|type` and `|target` are all
  required on a simplified-format `_link:i` datum; a submission omitting one
  used to be silently completed with an empty `DV_TEXT`/`DV_EHR_URI`, storing
  data the client never sent. The conversion is now refused with an error
  naming the exact key (e.g. `.../_link:0|meaning is required`).

- **Compositions and directories that contradict the RM's archetype-root rule
  are now refused (422) instead of stored.** At an archetype root the
  `archetype_node_id` is the archetype identifier in string form, so a node
  carrying `archetype_details` whose `archetype_id` names a *different*
  archetype declares two conflicting identities and can no longer be committed.
  Payloads that were accepted before and are affected by this must correct the
  mismatched root before they will commit.

- **An empty `archetype_node_id` is now refused (422) on every node type.** The
  RM requires a non-empty `archetype_node_id` on every archetypable node, but
  the check only ran on the node types with a hand-written invariant (ENTRY
  subtypes, CLUSTER, ELEMENT, SECTION, FOLDER, HISTORY and events). It is now
  applied to every RM type that inherits it — notably `ITEM_TREE`, `ITEM_LIST`,
  `ITEM_SINGLE`, `EHR_STATUS`, and the demographic and EHR-extract locatables —
  so a payload carrying `"archetype_node_id": ""` anywhere is rejected rather
  than stored. An *absent* `archetype_node_id` is unchanged: it is still
  reported as a missing mandatory attribute.

- **A present-but-empty `links` list is now refused (422).** `links` is
  optional, but the RM forbids it from being present and empty, so `"links":
  []` on any node of a committed COMPOSITION — or on any FOLDER of a committed
  directory — is now rejected rather than stored. Omit the attribute instead of
  sending an empty array.

- **RM class invariants are now enforced on every commit kind, not only on
  COMPOSITIONs.** The whole-instance RM + terminology pass previously ran only
  for compositions, so anything *below* the root of an `EHR_STATUS`,
  `EHR_ACCESS`, directory FOLDER, party or party-relationship body went
  unchecked. Payloads that were accepted before and are now refused with a
  `422` include: an empty `archetype_details.rm_version`; a `links` member that
  is missing `meaning`, `type` or `target`, or whose `target` is not an
  `ehr://` URI; a present-but-empty `links` list on a node nested inside
  `EHR_STATUS.other_details` or a party body; and an empty `feeder_audit`
  `system_id`. The same defects were already `422` inside a COMPOSITION.
  `EHR_ACCESS.settings` is deliberately unaffected — the RM leaves that slot's
  type to the implementation, so it carries no RM rules to enforce.

### Security

- **Admin dump/load failures no longer expose server filesystem paths.** The
  `file_not_writable` and container-fault bodies of the EHR export/import
  operations carried the configured archive path (server deployment layout) and
  the raw archive-parse diagnostics. The bodies now carry a curated message — a
  defect in the CALLER's archive still names the offending archive ENTRY, which
  is the actionable fact — and the path plus the underlying diagnostic go to
  the server's trace record only.

- **Terminology-server failures no longer expose the deployment's terminology
  configuration.** A `500` raised by an upstream FHIR terminology server (or by
  its OAuth2 client-credentials grant) named the configured provider, its
  operation and the upstream error in the response body. The body is now the
  curated internal-error message; the operator detail (provider name,
  operation, upstream diagnostic) is emitted on the trace record. Boot-time
  configuration errors are unchanged — they never reach a response body.

## [3.17.1] - 2026-08-01

### Added

- **Deprecated ADL 1.4 spellings are warned at ingest.** The paren-less
  domain-block spelling (`C_DV_QUANTITY <…>`) is marked deprecated by the
  ADL 1.4 text while its normative grammar defines only that form; it stays
  accepted (refusing it would reject the whole published CKM library), and
  every occurrence now raises the advisory `W14DEP` validation warning naming
  the preferred parenthesised spelling — the deprecation is enforced at
  exactly the strength the spec gives it, never silently absorbed.

- **The largest OPT the official CKM publishes is now a conformance scale
  probe.** The Congenital syphilis case investigation form (~5.2 MB of
  OPT 1.4 XML, the biggest template openEHR publishes) joins the curated CKM
  journey pack, and two new asserted CNF cases pin size-independence of the
  released contracts — the template upload and a COMPOSITION commit of its
  ~526 KB example must succeed and read back content-equal, because the
  released ITS-REST text defines no payload-size condition, so an
  implementation cap that refuses, truncates, or corrupts a large valid
  payload is a conformance defect. The template also rides the measured
  hospital simulation's case-investigation journey, so every future
  performance record exercises the large-payload path.

- **ADL 2 (AOM2) archetypes can arrive as XML, in both serializations openEHR
  publishes.** Only the AM 1.4 archetype XML was readable before; the generated
  canonical-XML codec now also covers the AOM2 **persistent** form
  (`P_Archetype.xsd`, root `<archetype>` = `P_AUTHORED_ARCHETYPE` — the shape the
  8 example documents in the openEHR ITS-XML bundle carry) and the AOM2 **model**
  form (`Archetype.xsd`, root `<archetype>` = `AUTHORED_ARCHETYPE`). The two are
  separate codecs because the two schemas declare the same top-level element with
  different root types.

### Fixed

- **`openehr-its` builds again for minimal consumers.** The `opt14` module
  (OPT 1.4 model + XML codec) was declared without the `full` feature gate
  its codec dependency carries, so any `default-features = false` consumer
  of the crate — whose documented minimal surface is only the dependency-free
  SMART scope grammar — failed to compile with 1,191 errors (first visible on
  the admin console's WebAssembly lane). The module is now gated like its
  siblings; default builds are unchanged.

- **Empty inline dADL domain blocks parse (`C_DV_QUANTITY <>`).** The ADL 1.4
  reader refused an empty domain block outright; the dADL chapter's own
  grammar admits the empty block and §Empty Sections allows it anywhere, so
  it now lowers to the open constraint (`DV_QUANTITY matches {*}`) in both
  the deprecated paren-less and the parenthesised spelling. 9 live CKM
  archetypes use the form. (The upstream regression fixture claiming the
  opposite is reported as a spec contradiction.)
- **The ADL 1.4 → 2 converter no longer emits unparseable tuples for
  heterogeneous `C_DV_QUANTITY` list rows.** Rows constraining different
  member sets (a `units`+`magnitude` row beside `units`-only rows, as in the
  published `range_of_motion` archetype) used to convert into one tuple with
  EMPTY members (`[{"mm"}, {}]`) that no conformant ADL 2 reader accepts —
  28 of the 1,142 published real-world archetypes were affected. Such rows
  now partition into one `DV_QUANTITY` alternative per member set, each tuple
  row constraining every member; co-constrained pairings and assumed values
  are preserved, and the whole real-world corpus now converts and re-parses.
- **ADL 1.4 archetypes that openEHR CKM actually publishes are no longer
  rejected over two spec-silent shapes.** Uploading a real-world ADL 1.4
  archetype failed to parse when it used either (a) a qualified term
  constraint with an empty code list — `defining_code matches {[local::]}`,
  `media_type matches {[openEHR::]}`, which names the terminology and
  constrains the code no further — or (b) the openEHR-profiled ordinal
  shorthand alongside another alternative in the same block, in either order
  (`DV_TEXT matches {*}` beside `0|[local::at0008], 1|[…]`). Both forms are
  read by the reference grammar and are used throughout CKM's own library, so
  the reader now accepts them; every other refusal is unchanged.
- **Mandatory `1..*` container attributes are enforced.** The canonical-JSON
  reader deliberately treats an absent list and an empty list as the same
  value (wire tolerance), so a committed `CLUSTER` with no `items`, a
  demographic `PERSON` with no `identities`, or a `REVISION_HISTORY` with no
  `items` was accepted although the RM declares those containers `1..*`. The
  validation walk now checks every model-declared mandatory container —
  absent, or empty where the lower bound is 1 — uniformly from the generated
  RM model, and such commits are refused 422.
- **Structurally defective RM nodes are now refused for EVERY openEHR class,
  not only the ones carrying a class invariant.** The per-node validation step
  deserialized a wire node into its concrete RM type — the check that surfaces
  a missing mandatory attribute or a wrong nested type — only for the classes
  with an RM invariant to run, so a defective node of any other type passed
  unnoticed: a `PARTICIPATION` with no `performer`, an `ISM_TRANSITION` with no
  `current_state`, a `LINK` with no `target`, a `DV_BOOLEAN` with no `value`,
  the demographic `ADDRESS`/`CONTACT`/`PARTY_IDENTITY` shapes, and more. The
  class-to-type dispatch is now generated from the openEHR meta-model and
  covers every emitted class, so such a commit is rejected with `422` naming
  the offending path and field. Wires that were already valid are unaffected.
- **A generated example COMPOSITION for a template that constrains
  `other_participations` is committable again.** The example carried a
  `PARTICIPATION` without its RM-mandatory `performer` (plus a stray `name`,
  which `PARTICIPATION` does not have), so posting the generated example back
  was rejected. The participation is now completed with an identity-free
  `PARTY_SELF` performer — no fabricated person — and carries no `name`.

## [3.17.0] - 2026-08-01

### Changed

- **The documentation website moved to its own domain,
  <https://ferroehr.eu/>.** It was published at
  `rubentalstra.github.io/ferroehr/`, a GitHub Pages *project* sub-path;
  it is now served at the root of the `ferroehr.eu` apex over HTTPS, with
  `www.ferroehr.eu` redirecting to it. Every published URL loses the
  `/ferroehr` prefix — the user guide is at `/docs/latest/`, the OpenAPI
  endpoint reference at `/api/`. The old GitHub Pages URLs keep working
  through GitHub's own redirect, so existing links and bookmarks do not
  break. This affects the website only; the CDR's REST base path
  (`/ferroehr/rest/openehr/v1`) is unchanged.

### Fixed

- **RM validation reaches legally untagged canonical-JSON nodes.** Canonical
  JSON requires `_type` only on polymorphic slots, so a node under a
  concretely-declared attribute (`COMPOSITION.context`,
  `EVENT_CONTEXT.participations`, …) may omit its tag — and the validation
  walk, dispatching on the wire tag alone, silently skipped every RM class
  invariant and terminology binding on such nodes (the same content was
  refused over canonical XML but committed over JSON). The walk now resolves
  an untagged node's effective RM type from the parent's declared attribute
  type (the BMM-generated static RM model), so commits like an out-of-group
  `EVENT_CONTEXT.setting` are refused 422 regardless of tag presence or
  format.

- **The documentation version switcher pointed at dead URLs.** Entries in
  the published version manifest kept whichever site base path was current
  when they were archived (`/ehrbase-rs/…` before the rename, `/ferroehr/…`
  before the domain move), so selecting an older release led to a 404. Each
  entry is now re-anchored to the live base at build time, which repairs
  every archived version without rebuilding the frozen documentation trees.

- **`EXTRACT_SPEC.extract_type` accepts the TERM `extract_content_type`
  vocabulary.** The EHR-Extract export refused the codes TERM 3.1.0 binds to
  the attribute (803 "openEHR EHR" … 808 "other", openEHR terminology —
  `SupportTerminology` §Vocabularies) and accepted only the RM-named string
  tokens (`openehr-ehr`, …). Both value spaces are now accepted; anything
  outside their union is still refused with a 400-family precondition error.

## [3.16.0] - 2026-08-01

### Added

- **ODIN plug-in syntax blocks parse.** `attr = (syntax) <# … #>`
  (`master09-plug_in_syntaxes`) is accepted, with the tag and the verbatim
  foreign-text body exposed as `OdinValue::PlugIn`; the body is handed to
  consumers uninterpreted, and a plug-in block is refused as an archetype
  `default_value` (it denotes no RM instance).

- **ODIN document artefacts parse in all three `master04` forms.** The ODIN
  reader now accepts Identified Object Documents (top-level
  `["id"] = <…>` keyed objects) and the optional leading
  `@schema = <uri>` schema identifier, exposed via the new
  `odin::parse_document`; the anonymous and implicit forms were already
  supported.

- **ODIN cross-object references parse.** Reference paths rooted at an
  object identifier (`<["tourism_db_13"]/hotels["sofitel"]>`,
  `master06-references` §Across Objects) are accepted alongside the
  existing within-object reference forms.
- **ODIN path extraction.** `OdinValue::paths()` extracts the
  `master05-content` tree-path set (attribute paths, `attr[key]` container
  paths, nested bare-key segments) from any parsed ODIN structure.
- **The BMM v3 behavioural surface: type lattice, class/feature functions and a
  v3 materialisation.** `openehr-lang` now answers the `master06-core-types`
  meta-type lattice (`is_abstract`, `is_primitive`, `type_base_name`,
  `unitary_type`, `effective_type`, `effective_base_class`,
  `is_open`/`is_closed`/`is_partially_closed`), the `master07`/`master08` class
  and feature functions (`BMM_SIMPLE_CLASS.type`, `BMM_GENERIC_CLASS.type` +
  `generic_parameter_conformance_type`, `has_ancestor_class`, `all_ancestors`,
  `flat_features`, `BMM_ENUMERATION.name_map`, `signature`, `arity`,
  `is_boolean`), and materialises a v3 `BMM_MODEL` from a P_BMM schema
  (`create_bmm3_model`) — which lands three things the v2.x transform cannot: a
  generic ancestor's parameter substitution, a class's routines and constants, and
  a `value_constraint` on the type it constrains.

### Fixed

- **A malformed enumeration is refused instead of loaded silently.** A P_BMM
  enumeration with more than one ancestor, or with `item_values` that are not 1:1
  with `item_names`, now fails model materialisation with a typed error
  (`EnumerationAncestorCount`, `EnumerationItemListsNotOneToOne`) — the two rules
  `master07-core-classes` §Range-Constrained Classes and the `BMM_ENUMERATION`
  class definition state. Stating names without values stays valid (the assumed
  values 0, 1, 2, … apply).
- **Container literal values are typed on the wire.** `BMM_CONTAINER_VALUE` and
  `BMM_INDEXED_CONTAINER_VALUE` bound their inherited `type` slot to free-form
  JSON; they now carry `BMM_CONTAINER_TYPE` / `BMM_INDEXED_CONTAINER_TYPE`, so a
  container literal's `type` member is a `_type`-tagged container type in
  canonical JSON instead of an arbitrary value. The two slots openEHR genuinely
  leaves untyped (`BMM_INTERVAL_VALUE.type`, `EL_CASE.value_constraint`) now carry
  their adjudication as a generated `NOTE` at the field.

- **The generated LANG model carries both extant BMM generations in full.**
  `openehr-lang` was generated from the two vendored LANG schemas name-merged
  into one class map, so 18 class names both declare emitted the stable v2.x
  shape at the v3 (`bmm3`) module paths and the v3 attribute sets were
  discarded — a `BMM_CONTAINER_TYPE` had no `is_ordered`/`is_unique`, a
  `BMM_CLASS` none of its feature/invariant maps, and `BMM_MODEL_TYPE` +
  `BMM_MODULE` were never emitted at all (185 of 187 declared classes). Both
  generations are now emitted completely, each at its own source-package path
  (v2.x under `bmm/`, `bmm_persistence/`, `beom/`; v3 under `bmm3/`), and the
  canonical-JSON codec covers both.
- **ODIN leaf lists are type-homogeneous and admit interval lists.** Mixed-kind
  lists (`<1, "x">`) are refused per the per-type list productions of the
  ODIN syntax specification, and interval lists (`<|0..5|, |8..9|>`,
  including the open `, ...` form) now parse.
- **Duplicate ODIN container keys are refused (rule VDOBU).** Sibling
  keyed objects sharing a key (`[1] = <…> [1] = <…>`) were silently
  accepted; they now fail with a typed error per
  `LANG/docs/odin/master05-content` §Container Objects — archetype uploads
  whose terminology sections duplicate a term code are refused at the ODIN
  layer.
- **ODIN sections accept `true`/`false`/`infinity` as attribute names and
  semicolons between keyed objects.** ODIN reserves no keywords
  (`LANG/docs/odin/master03-basics` §Keywords), so archetype/template
  uploads whose ODIN sections use those three words as attribute names —
  previously refused because they lex as ODIN value words — now parse; and
  the §Semi-colons separator is accepted between keyed-object entries, not
  only attribute pairs.

## [3.15.3] - 2026-07-31

### Fixed

- **The quickstart dev credentials authenticate again.** The v3.15.2 rename
  updated the dev usernames and documented passwords to `ferroehr` but left
  the committed Argon2id hashes (and the dev Keycloak realm's stored
  password hashes) verifying the old secret, so every Basic-auth request
  against the compose stack returned 401. The dev user store now carries
  hashes of the documented password, and the Keycloak realm import sets the
  dev passwords at import time. Development stacks only; no production
  surface stores these credentials.

## [3.15.2] - 2026-07-31

### Changed

- **The project's own code is now MIT-licensed** (owner decision
  2026-07-31). Vendored third-party material keeps its upstream terms: the
  openEHR machine-readable specification artifacts and CKM-derived clinical
  models remain under Apache-2.0 (`LICENSE-APACHE-2.0`). The upstream
  `NOTICE` file was removed together with the relicense — no upstream code
  is present in this tree.
- **The product is named FerroEHR** (owner decision 2026-07-31, tracked on
  #1353 — from *ferrum*, iron, the element Rust is named for). Every
  product-branded surface changes with it; deployments upgrading across this
  release must update:
  - **Configuration:** the environment prefix `EHRBASE_*` → `FERROEHR_*`; the
    config file search path `./ehrbase.toml` / `/etc/ehrbase/ehrbase.toml` →
    `./ferroehr.toml` / `/etc/ferroehr/ferroehr.toml`.
  - **REST base path:** `/ehrbase/rest/openehr/v1` → `/ferroehr/rest/openehr/v1`
    (likewise the admin/management extension routes under `/ehrbase/…`).
  - **Binary and crates:** the server binary `ehrbase` → `ferroehr`; the
    application crates `ehrbase`/`ehrbase-rest`/`ehrbase-server`/
    `ehrbase-admin-ui` → `ferroehr`/`ferroehr-rest`/`ferroehr-server`/
    `ferroehr-admin-ui`. The generated `openehr-*` specification crates are
    unaffected (they are versioned by the openEHR spec they implement).
  - **Containers and Helm:** the OCI images and the Helm chart are published
    under `ferroehr`/`ferroehr-admin-ui`; compose service names and the
    dev-stack database/realm names follow.
  - **Repository:** the GitHub repository is now
    `github.com/rubentalstra/FerroEHR` (old URLs redirect).
  - **Conformance artifacts:** the SUT identifier `ehrbase-rs` → `ferroehr`
    (`docs/conformance/ferroehr/`); measured numbers are unchanged.
  - The startup banner, served OpenAPI title, telemetry `service.name`, and
    the documentation website carry the new name and logo. The openEHR wire
    behaviour itself (canonical JSON/XML, AQL, status codes/headers) is
    unchanged.

## [3.15.1] - 2026-07-31

### Added

- **An archetype or template may now give an interval as a node's default
  value.** ADL 2 lets a `_default` block hold any ODIN value, and ODIN counts
  intervals of the ordered types (`|0..5|`, `|>=1939-02-01|`, `|<10.5|`,
  `|5.0 +/-0.5|`, the single-value `|5|`) among those values. Such a default
  used to be rejected outright; it is now read, stored as a proper interval
  with its own bounds and open/closed flags, and written back out in the same
  interval syntax. A `centre +/- delta` interval over dates, times or
  durations is still refused — reducing it to bounds would need calendar
  arithmetic the source does not state — with a message that says so.

### Fixed

- **Querying a parent archetype now also finds data recorded under its ADL 2
  specialisations.** An AQL archetype predicate naming a parent used to
  recognise a specialisation child only when the child's concept extended the
  parent's with a hyphen — the ADL 1.4 naming convention, which ADL 2 dropped.
  The server now reads the specialisation lineage from the ADL 2 archetypes and
  templates you have uploaded, so a query for the parent returns data committed
  under any of its stored specialisation children (and their children in turn),
  whatever their concept names are. Hyphenated ADL 1.4 identifiers keep their
  existing behaviour; a hyphenated ADL 2 identifier is no longer treated as a
  specialisation, because in ADL 2 the hyphen carries no such meaning. A
  parent with no uploaded family still matches only itself.
- **A CONTRIBUTION commit response now tells the client when the change set
  was committed.** `POST /ehr/{ehr_id}/contribution` (and its demographic
  sibling) returned only the `ETag`/`Location` identity; it now also sends
  `Last-Modified`, carrying the commit audit's recorded time. The header is
  present under both `Prefer: return=representation` and
  `Prefer: return=minimal` — on the minimal branch, where the response has no
  body, it is the only place the commit instant appears at all.
- **A backslash sequence the ADL/ODIN escape rules do not define is now
  refused with a clear message instead of read as literal text.** The escape
  set is closed — `\r`, `\n`, `\t`, `\\`, `\"`, `\'` and the two `\u` unicode
  forms — and anything else in a quoted string (a stray `\q`, a regex class
  such as `\d` written outside a regex, a string ending in a lone backslash,
  a `\u` with the wrong number of hex digits) is an authoring defect. Such a
  string used to be carried through as-is, so the defect reached the stored
  archetype as text; it is now reported at the offending literal. Regular
  expressions are unaffected: the backslash patterns inside a `matches {/…/}`
  constraint are still passed to the regex engine untouched.
- **A type cast written with its package path now parses.** ODIN and ADL 1.4
  dADL allow a type identifier to be qualified with dot-separated package
  names — `(org.openehr.rm.ehr.content.ENTRY)`,
  `(Core.Abstractions.Relationships.Relationship)`, and the same inside a
  generic such as `(List<org.openehr.rm.ehr.content.ENTRY>)` — which is how
  authors disambiguate same-named types from different models. Such a cast
  used to fail the parse and reject the whole archetype. The qualified name
  is kept exactly as authored; where the cast becomes a JSON `_type` tag, the
  class name is used.
- **Archetype text that carries a non-BMP character now reads correctly.**
  ADL and ODIN allow a unicode character above the base multilingual plane to
  be written as an eight-hex-digit `\uHHHHHHHH` escape (emoji, historic
  scripts, rare CJK). Those escapes were accepted but decoded wrong — the
  cADL reader kept the escape as literal text and the ODIN reader produced
  nothing usable — so the character was silently lost. Both forms now decode,
  in archetype definitions, ODIN sections and rule expressions alike, and an
  escape that names no character (a value outside the range the eight-digit
  form covers, a broken surrogate pair) is reported as a syntax error at the
  offending literal instead of being silently substituted.
- **Uploading an ADL 2 archetype now runs the complete validation schedule.**
  The parse-and-validate path skipped the final, flat-form phase, so two
  defects could pass: an internal `use_node` reference whose target path does
  not exist, and a container whose cardinality cannot hold the child nodes it
  declares as mandatory. Both are now reported. Archetypes that were valid
  remain valid — including specialised archetypes that redefine one parent
  node into several children, whose node identifiers are no longer
  mis-reported as duplicates.
- **A fixed numeric value written as a closed range now behaves like the
  point value it is.** `{5..5}` and `{5}` are the same constraint, and an
  ADL 1.4 assumed value spelled either way is read identically; conversely, a
  "point" whose bound is open or unbounded is no longer treated as a fixed
  value.

## [3.15.0] - 2026-07-30

### Changed

- **Release binaries are hardened for diagnosability**: production panics now
  carry file:line backtraces (`debug = "line-tables-only"`), and the unwind
  panic strategy the clean-500 error contract depends on is pinned explicitly
  in the release profile so it can never silently regress to `abort`.
  Runtime behaviour is otherwise unchanged.
- **The workspace now enforces the official Rust best-practice baseline as
  compile-time policy** (Rust API Guidelines, Clippy/rustdoc/Cargo books):
  every public item is documented (including the generated openEHR spec
  crates, whose docs now come from the BMM meta-model), panicking indexing
  and string slicing are compile errors outside tests, wall-clock/env/UUIDv4
  API bans are machine-enforced, lint suppressions must carry a machine-read
  `reason`, and doc links are verified by a new rustdoc CI gate. No wire or
  storage behaviour changes.

- **An ADL 1.4 upload is now validated as ADL 1.4, not as a permissive
  superset of ADL 2.** ADL 1.4's cADL keyword set is closed (master05
  §Keywords), so constructs that only ADL 2 defines are refused in a 1.4 text
  with a syntax error naming the construct: `use_archetype`, the archetype-slot
  `closed` marker, the `_default` pseudo-attribute, second-order attribute
  tuples, term-constraint strengths (`required`/`extensible`/`preferred`/
  `example`), and `@terminology` operational bindings. `before`/`after` sibling
  order stays accepted — master05 lists both as ADL 1.4 cADL keywords. ADL 2
  uploads are unaffected.

### Fixed

- **A malformed deprecated `concept` section in an ADL 2 archetype is now
  rejected with the `SACO` syntax code** ("must consist of the 'concept'
  keyword and a single local term") instead of being ignored silently;
  well-formed deprecated concept sections stay accepted and ignored.

- **A `use_node` internal reference targeting its own ancestor, or another
  internal reference, is now rejected at validation** (`VUNP` for ADL 2,
  `VDFPT` for ADL 1.4): an ancestor target defines an infinitely recursive
  expansion and previously validated clean; sibling and cross-branch targets
  are unaffected.

- **Illegal backslash escapes in ADL character values and rules-section
  strings are now rejected at parse** (only `\r`, `\n`, `\t`, `\\`, `\"`,
  `\'` are legal quoted forms, plus `\uHHHH` unicode escapes in strings):
  previously a form like `'\q'` was accepted silently across the ODIN,
  assertion and ADL lexers. Unicode content and the legal forms parse as
  before.

- **Converting an ADL 1.4 archetype to ADL 2 now carries its extended
  meta-data across.** The standardised `description/other_details` items of
  ADL 1.4 App.B (Extended Meta-data Guide) — items "intended to be
  implemented by any ADL 1.4 => ADL 2 conversion tool" — previously survived
  conversion only as opaque `other_details` strings (only `revision` was
  converted). They now land in their ADL 2 homes: `build_uid` becomes the
  archetype's build identifier; `original_namespace`, `original_publisher`,
  `custodian_namespace`, `custodian_organisation` and `licence` become the
  matching description attributes; and `references` / `ip_acknowledgements`
  become keyed lists, one entry per line with surrounding whitespace
  stripped. Each converted item is removed from `other_details`; a value that
  does not match its documented syntax (e.g. a `build_uid` that is not a
  GUID) is left in `other_details` untouched rather than guessed at, and the
  guide's "other items" (`MD5-CAM-1.0.1`, `current_contact`, `review_date`,
  `responsible_organisation`) pass through unchanged as before.
- **ADL 1.4 archetypes using the chapter's custom constraint forms now
  upload.** Two constructs of ADL 1.4 §Customising ADL (master09) were
  rejected outright:
  - the inline dADL `C_CODE_PHRASE <…>` section — the chapter's own worked
    example — is now read and lowered to exactly the constraint its compact
    `[local:: at0039, at0040]` twin produces (the chapter presents them as two
    spellings of the same constraint), including an `assumed_value`
    `CODE_PHRASE`; a block that is not a `C_CODE_PHRASE` instance (no
    terminology, no or empty code list, an attribute the type does not define)
    is refused with a syntax error naming the defect instead of being guessed
    at;
  - the openEHR-profiled ordinal shorthand `0|[local::at0005],
    1|[local::at0006]` — ubiquitous in real 1.4 scores and scales, and
    optionally carrying a `; assumed` value — now parses in ADL 1.4 and is
    lowered to the generic `DV_ORDINAL` `[value, symbol]` tuple that ADL 2
    names as its replacement. ADL 2, which removed the form, still refuses it.

  Both forms' codes take part in validation exactly as the equivalent standard
  spellings do (an undefined at-code still raises `VATDF`). `C_DV_STATE`, the
  remaining custom constrainer, has no shape in any openEHR specification and
  stays a loud refusal naming the type.
- **ADL 1.4 archetypes whose section keywords are not all-lowercase now
  upload.** The ADL 1.4 lexical specification (master08 §Symbols) spells
  every section keyword case-insensitively, so `ARCHETYPE (adl_version=1.4)`,
  `Specialise`, `CONCEPT`, `DEFINITION` and `ONTOLOGY` are valid headers; they
  were previously rejected with "expected an artefact keyword" / "expected a
  section header". ADL 2 keeps the exact lowercase spelling its own grammar
  defines.
- **Old-form ADL 1.4 archetypes with no `language` section now upload.** Where
  the archetype carries `primary_language`/`languages_available` in its
  `ontology` section instead — the form master08 §Language Section and
  §Ontology Header Statements tell tools to accept and upgrade — the language
  is now lifted into `original_language` plus one translation entry per other
  available language, instead of the upload failing with "no language section
  found". An archetype with neither a `language` section nor a
  `primary_language` is still rejected.
- **A missing or undefined ADL 1.4 `concept` section is now reported instead
  of passing silently.** The 1.4 grammar makes the `concept` section mandatory
  and master08 §Validity Rules VARCN requires its term to exist in the
  archetype ontology: an archetype with no concept section (or a concept
  clause that is not a term-code reference) is refused with a `SACO` syntax
  error, and a concept term missing from `term_definitions` now raises `VARCN`
  on the 1.4 validation path.
- **ADL 1.4 assertion expressions now accept the chapter's full operator
  set**: the ADL 1.4 inequality spelling `<>` and the symbolic existential
  quantifier `∃` applied to a path (equivalent to the `exists` keyword per
  the assertion-language symbol table) both parse in `invariant`/`rules`
  sections instead of being rejected.
- **ADL 1.4 assertions now accept every path form the ADL paths grammar
  defines**: movable path patterns (leading `//`) and single-segment
  relative paths with a node predicate (`items[at0001]`) parse instead of
  being rejected; absolute/relative multi-segment paths and the
  position/meaning/at-code predicate forms were already accepted.
- **ADL 1.4 `use_node` internal references with a target path that does not
  resolve within the definition are now reported as a `VDFPT` validation
  error** (path validity in definition) instead of passing silently; a
  resolving target stays clean.

- **A server-side failure of the password-verification task no longer
  masquerades as `401 Invalid credentials`**: if the blocking Argon2
  verification task itself fails (panic/cancellation), the API now returns
  `500 Internal Server Error`. A wrong password still returns 401.
- **Basic-authentication credentials are now decoded by the pinned RFC 4648
  `base64` crate** instead of a hand-rolled decoder. Canonical (padded) and
  unpadded credentials are accepted as before; malformed base64 with excess
  or interior padding is now rejected outright (it previously decoded to
  garbage and failed the user lookup — the response stays 401 either way).
- **ADL 1.4 archetypes carrying a `revision_history` section now parse and
  upload.** The section is defined by the ADL 1.4 specification (master08
  §Revision History Section) but was not recognised, so an entire spec-valid
  archetype was rejected with a "expected a section header" syntax error. It
  is now read and preserved on the 1.4 source model. (ADL 2 removed the
  section, so it has no ADL 2 counterpart; a 1.4 upload stores the source
  verbatim, so nothing is lost.)
- **The dADL/ODIN reader accepts the leaf and structure forms the ADL 1.4
  specification defines** (master04 §dADL), which were previously rejected:
  the `<...>` empty-section marker at any level; the whole partial date/time
  family (`yyyy-MM-ddT??:??:??`, `yyyy-MM-??T??:??:??`,
  `yyyy-??-??T??:??:??`); integers written with an exponent (`29e6`);
  booleans in any case (`TRUE`, `fAlSe`); `infinity` / `-infinity` / `*` as
  unbounded interval endpoints; and local term codes (`[at0200]`) as leaf
  values.
- **Values that were silently mis-read are now read correctly or rejected
  loudly.** A `(TYPE)`-cast section value is read through instead of being
  dropped; an `|N +/- M|` domain constraint becomes the interval
  `[N-M, N+M]` instead of collapsing to the centre; a duplicate sibling
  attribute is a typed error naming the attribute instead of the last one
  silently winning (rule VDATU); duplicate keys in the `language` section are
  now reported (VOKU); and an interval used as a `_default` value is rejected
  instead of silently becoming a null default.
- **Multi-line string values drop their continuation-line indentation**, as
  the specification requires (master04 §String Data), instead of carrying the
  source file's leading whitespace into the value.
- **An inline ADL 1.4 domain constraint is no longer lowered to the wrong data
  type.** `(TYPE) <…>` domain blocks (ADL 1.4 master09 §Customising ADL) other
  than `C_DV_ORDINAL` were all turned into a `DV_QUANTITY`, so e.g. a
  `C_CODE_PHRASE` coded-term constraint silently became a quantity constraint.
  `C_DV_QUANTITY` and `C_DV_ORDINAL` are lowered as before; any other domain
  type is now a syntax error naming the type instead of a wrong answer.
- **An inline ADL 1.4 domain constraint's `assumed_value` is no longer
  dropped.** It is now carried onto the constrained leaves (and an assumed
  value that satisfies none of the block's own `list` rows is rejected instead
  of silently ignored).
- **ADL 1.4 coded-term constraints are checked for definedness.** The
  dominant ADL 1.4 spelling of a value set (`[local:: at0004, at0005; at0004]`,
  master09 §Custom Syntax) bypassed the term-definedness rules entirely, so an
  archetype referring to codes its own ontology never defines uploaded
  cleanly. Every listed code — and the assumed-value code — is now checked
  (VATDF/VACDF, ADL 1.4 master08 §Validity Rules); codes of an external
  terminology are correctly exempt.
- **The ADL 1.4 cardinality/occurrences rule VCOC is enforced** (master05
  §Occurrences): a container whose children's occurrences cannot fit inside
  its cardinality is now rejected. The ADL 1.4 default `occurrences {1..1}`
  and `existence {1..1}` (master05 §Occurrences / §Existence) are applied when
  evaluating it, and the default occurrences is now written out explicitly by
  the ADL 1.4 → 2 conversion, where an unstated occurrences means something
  different.
- **`use_node` type conformance is enforced (VUNT).** An internal reference
  whose declared type is neither the referenced node's type nor an ancestor of
  it is now a validation error instead of passing silently. It is also reached
  by an ADL 1.4 upload: VUNT is a rule of the ADL 1.4 specification itself
  (master05 §Internal References), so the 1.4 validity check now runs the
  reference-model pass for that rule instead of stopping before it.
- **cADL keywords are recognised in any case.** `MATCHES`, `Occurrences`,
  `CARDINALITY`, `Is_In`, `TRUE` and every other keyword are now lexed as
  keywords, as both the ADL 1.4 specification's own lexical rules (master05
  §Symbols) and the normative ANTLR grammars require; previously only the
  lower-case spelling worked, so an upper-case archetype failed to parse.
- **`infinity` is accepted as an interval bound in cADL constraints.**
  `rate matches {|0..infinity|}` — the ADL 1.4 specification's own worked
  example (master05 §Interval of Integer) — parsed in a dADL section but was
  rejected in a cADL constraint. `-infinity` and `*` are accepted likewise,
  and each yields a genuinely unbounded endpoint.
- **The `^…^` regular-expression delimiter survives a re-print.** A constraint
  written `{^km/h|mi/h^}` (master05 §Regular Expression) was normalised to
  `{/km/h|mi/h/}`, which no longer re-parsed; the inner delimiters are now
  escaped, so parse → print → parse is lossless.
- **More date/time constraint-pattern forms are accepted:** patterns with
  literal date/time numbers substituted for the placeholder fields
  (`1995-??-XX`, master05 §Patterns), the ASCII timezone modifiers `+hh:mm` /
  `+hhmm` / `-hh` (previously only the literal `±` character worked), and the
  space-separated date/time pattern (`yyyy-mm-dd hh:mm:XX`), which the
  specification's own assumed-value example uses.
- **Character constraints are accepted** (`color_name matches {'r','g','b'}`,
  master05 §Constraints on Character), as are the `, ...` list-continuation
  marker on every primitive list and the exclusive-lower interval spelling the
  ADL 1.4 chapters write (`|0>..<1000|`).
- **An inline ADL 1.4 domain block containing a one-sided interval now
  parses.** A `C_DV_QUANTITY <… magnitude = <|>0.0|> …>` block was cut short at
  the interval's own `>`, so the archetype was rejected as invalid dADL.
- **Defects inside an ADL 1.4 listed term constraint are now reported**: a
  repeated code (STCDC) and an assumed-value code that is not a member of the
  list (STCAC).
- **A date/time interval must use a timezone on both endpoints or on neither**
  (master05 §Intervals), so two endpoints that cannot be compared are rejected
  instead of silently accepted.
- **Operators the ADL 1.4 chapter names but no grammar defines are refused
  with a message that says so** — the negated `~matches` / `~is_in` / `∉`
  family and the `=~` / `!~` regex-match operators — instead of being read as
  their affirmative counterparts, which would invert the constraint.

## [3.14.0] - 2026-07-30

### Fixed

- **AQL VERSION coded-field predicates respect their sub-paths.** Predicates
  on `commit_audit/change_type` and `lifecycle_state` compared every
  sub-path against the stored numeric code, so the rubric form
  (`…/value='creation'`) silently never matched. The three defined
  sub-paths now compare correctly (`defining_code/code_string` against the
  code, `value` against the terminology rubric, `terminology_id/value`
  against `openehr`); other suffixes are clean invalid-query rejections.
- **AQL `SELECT DISTINCT` with `ORDER BY` executes correctly.** Sorting a
  DISTINCT projection by one of its selected columns previously failed with
  a database error surfaced as HTTP 500; it now orders by the output column.
  Sorting a DISTINCT projection by an expression that is not selected is a
  clean invalid-query rejection (the AQL specification defines no semantics
  for it) instead of a 500.
- **AQL date/time functions work in temporal comparisons.** Comparing a
  temporal path against `NOW()`/`CURRENT_DATE_TIME` etc. previously failed
  with a database type error surfaced as HTTP 500; function operands now
  join the comparison in the same coercion space as literals.
- **Comma-fraction ISO 8601 timestamps compare correctly in AQL.** Canonical
  `DV_DATE_TIME` values using the ISO-permitted comma decimal sign
  (`21:22:19,501+00:00`) were silently excluded from temporal comparisons
  (and their promoted index column stored NULL). Both the write-time
  promotion and the query-time casts now normalize the comma form.
- **AQL coded-name node predicates match correctly.** The name term-code
  shortcut (`[at0002, snomed_ct(3.1)::313267000]`, and the
  `terminology::code|informational text|` form) was compared as one raw
  token against `code_string` and could never match. It now decomposes per
  the AQL specification's canonical expansion: `code_string` and
  `terminology_id/value` are compared separately, the informational `|…|`
  tail is ignored, and a bare at-code name operand asserts the archetype's
  `local` terminology.

### Changed

- **`TOP n BACKWARD` is now rejected with rewrite guidance.** The deprecated
  direction variant previously returned the *first* n rows silently. The
  server now refuses it as an invalid query whose message shows the
  recommended rewrite (`ORDER BY <path> DESC LIMIT n`). Plain `TOP n` and
  `TOP n FORWARD` are unchanged.

## [3.13.0] - 2026-07-30

### Added

- **The ISO 8601 date/time/duration types implement their computational
  functions** (BASE `foundation_types/master06-time_types.adoc`
  §Computational Functions + the four `Iso8601_*` class definitions): the
  DEFINITE `add`/`subtract`/`diff` on dates, times and date/times — a
  duration reduced to exact seconds with the `Time_definitions`
  `Average_days_in_year`/`Average_days_in_month` lengths — and the NOMINAL
  `add_nominal`/`subtract_nominal`, which advance the calendar to the same
  day-of-month and clamp it down where the target month is shorter (29 Feb
  `++ P1Y` → 28 Feb, 31 Jan `++ P1M` → 28/29 Feb). Durations gain
  `add`/`subtract`/`multiply`/`divide`/`negative`. Also added across the four
  types: `as_string` (the value in extended format), `is_extended`,
  `is_decimal_sign_comma` and `has_fractional_second`. Arithmetic on a
  partial value, or a result outside the representable 0000–9999 year range,
  is reported as no result rather than an invented one.

- **openEHR path expressions support general comparison predicates** (BASE
  architecture overview §Paths and Locators, "Other Predicates"): path
  predicates of the form `[at0007 and time >= '2005-06-24T09:30:00']` or
  `[value/defining_code/code_string = 'A04']` — a relative attribute path,
  an operator (`=`, `!=`, `<`, `<=`, `>`, `>=`), and a quoted-string or
  numeric literal — now parse and evaluate everywhere RM paths are resolved,
  including `ehr:` URI resolution. Strings compare lexically (ISO 8601
  date/times order temporally), numbers numerically, with XPath existential
  node-set semantics; predicate text outside the grammar is still rejected
  loudly. Previously these spec-defined forms were refused as unsupported.

## [3.12.0] - 2026-07-29

### Changed

- **The conformance verdict model no longer has excused capability states.**
  The published reports and certificates previously carried two
  non-verdict evidence tokens — `unrealized` (every case excused by a
  register citation) and `no_cases` (a claim the catalogue named no case
  for). Both are deleted: the catalogue gates now refuse those shapes before
  any server is assessed, and every capability a party claims is reported as
  exactly one of passed / failed / inconclusive / not-evidenced. A required
  capability without passing evidence now fails its tier with no excuse arm,
  for every assessed party alike; both committed records and the published
  comparison were re-derived under the stricter model (no tier verdict
  changed for either party).
- **An empty TDD batch answers `200` with `[]` instead of `201`.**
  `POST /message/tdd/{ehr_id}/batch` with an empty array creates nothing, and
  `201 Created` reported a creation that did not happen. Batches with members
  are unaffected.
- **`EXTRACT_SPEC.extract_type` now accepts every code the openEHR Reference
  Model names.** `POST /message/export` previously refused
  `openehr-synchronisation` and `openehr-generic` — two of the five extract
  types the RM's EHR Extract chapter lists by example — as out of group.
  Both are accepted now, alongside `openehr-ehr`, `openehr-demographic`,
  `generic-emr` and the catch-all `other`.

- **Conformance: a product is no longer excused from 186 test cases for
  declaring an older REST release** (#635). Every conformance case may declare
  the openEHR release its behaviour needs, and systems declaring an earlier
  release are skipped for it. That declaration had been copied onto 343 cases
  as authoring boilerplate, which quietly wrote off most of the EHR,
  COMPOSITION, DIRECTORY, CONTRIBUTION, QUERY and template surface for any
  product declaring ITS-REST 1.0.3 — behaviour those products do implement and
  should be judged on. Each case was re-derived against the released
  amendment record, and the requirement is kept only where the released text
  actually dates it: ITEM_TAGs, the Demographic API, admin EHR deletion, the
  Simplified Formats media types, `Prefer: return=identifier`, the
  audit-details `system_id`, the reserved `aql` query name, the template
  `/example` sub-resource, and SMART on openEHR. Everything else is now judged
  for every product, with the two genuinely release-dated header rules (the
  weak `W/` ETag form and the read/delete `Location` restriction) still
  applied only to the release that introduced them. No test was removed or
  weakened; the comparison against other openEHR products now covers what they
  really implement.

- **The conformance statement no longer claims nine capabilities it cannot
  demonstrate** (#623). ADL 1.4 and ADL 2 archetype provisioning, the admin
  Activity Report, EHR dump/load, EHR and demographic archiving, EHR Extract,
  TDS, and the MESSAGE API were all being claimed while every one of their
  test cases was excused: openEHR's released REST API publishes no endpoints
  for them, and EHRbase-rs exposes none of its own either — the underlying
  service methods exist, but nothing reaches them over HTTP, so a conformance
  runner has nothing to drive. Claiming a capability is the obligation to
  prove it, so those claims are withdrawn until the routes exist. Nothing was
  removed from the product; what changed is that the published statement now
  only claims what can be demonstrated.

- **Helm chart and operator-facing comments cite durable references** (#322).
  The chart's `values.yaml`, `Chart.yaml` and post-install NOTES pointed at
  internal design documents that no longer exist (the deleted design and
  enterprise doc trees) and at retired decision-record numbers. Each is now
  either the official upstream documentation it was standing in for (the
  PostgreSQL docs for the unprivileged app role and the `lock_timeout`
  migration wrapper, the Kubernetes Pod Security Standards for the container
  security posture), an explicit "our own extension, no openEHR spec governs it"
  flag on the optional integrations, or the rationale written out inline —
  so an operator reading the chart is never sent to a dead path. No default
  value, template, or rendered manifest changed.

- **The published per-chapter outcome bars are now a two-level chart with no
  `Other` bucket** (#613). The single bar per schedule chapter hid the EHR
  chapter's hundreds of cases behind one rectangle and swept the System API
  and anything unrecognised into an `Other` row. The chart now renders a
  chapter header carrying the chapter's total above one scaled bar per
  **band** — the surface a case actually exercises (EHR resource /
  EHR_STATUS / COMPOSITION / DIRECTORY / CONTRIBUTION / item tags / revision
  history, ADL 1.4 vs ADL 2 vs stored queries, ad-hoc vs stored query
  execution, parties vs relationships vs versioned party, and so on) — with
  the exact passed / FAILED / errored / cited-N-A counts printed beside every
  row, so a small band never loses its numbers to a short bar. Cited-N/A
  segments carry a hatch texture so "not executed, with a citation" can read
  as neither a pass nor a failure. The taxonomy is **total**: every case id
  maps to a named band and an unmapped id fails the render naming the id,
  rather than landing in a silent bucket. Both published SUTs render the same
  bands — a band with no case shows as an explicit `no cases` row — so the
  comparison page reads band-for-band.

- **The conformance pipeline now exercises BOTH claimed version-signing modes
  in every run, in the one committed record** (#609). openEHR defines a
  version signature at two depths of one mechanism — a plain digest (an
  integrity check) and an openPGP RFC 4880 signature (which additionally
  authenticates the author) — and a running server does one or the other. The
  product claims both, so `scripts/conformance.sh` now brings up a **second
  deployment of the same built image** in the openPGP posture alongside the
  standard stack (its own compose project, host port 8081), and the party's
  `ixit.json` declares it as an extra instance carrying its own signing block.
  The openPGP signature cases address that instance; one run, one
  `results.json`, and the Signing capability's evidence covers both modes.
  Consequently the `CONF_SIGNING_MODE=pgp` environment switch and the separate
  `ixit.pgp.json` party file are **removed** — there is nothing left to select,
  both modes always run. A conformance target that declares no such instance
  (upstream EHRbase) has the openPGP cases recorded not-applicable with that
  citation instead of failed, which is also now true for any case addressing an
  instance a party does not declare: it is excused at selection time rather
  than surfacing as an inconclusive row.

- **The conformance suite proves EHR-scoped querying against two EHRs, not
  one** (#604). The four cases that check a query is confined to the EHR named
  in the `openehr-ehr-id` request header used to run against a server holding
  a single EHR, so a server that ignored the header returned the same rows and
  passed. Each now creates a second EHR with its own content first: an
  unscoped answer carries the extra row and fails the case. The behaviour
  being checked is unchanged; the check can no longer be satisfied by
  accident.

- **The conformance suite now names a malformed request and invalid content
  differently everywhere** (#605). Fifteen more conformance cases used to
  report a rejected request as a content-validation failure when what the
  request actually broke was its own syntax — an unparseable template upload,
  a path segment that is not an identifier, a `version_at_time` outside the
  ISO 8601 form the specification mandates, or a tag list sent as something
  other than a list. Those now report as malformed requests. Nothing changes
  on the wire (all fifteen answered `400 Bad Request` before and after) and
  no server passes or fails differently; the published conformance report and
  case records simply name one rejection law one way, so a reader can tell the
  two families apart.

- **Two behaviours the conformance suite used to treat as optional are now
  required of every server** (#556). openEHR publishes its REST
  specification as normative prose *and* as OpenAPI files, and the prose is
  silent on more than it looks. Where the prose says nothing, those OpenAPI
  files are now read as part of the specification rather than set aside — so
  behaviours previously recorded as "the specification does not say" turn out
  to be specified after all, and the suite stops excusing them. Two change
  how a server is judged. Uploading an operational template under a
  `template_id` that already exists must answer `409 Conflict`; it was
  previously a declared choice between refusing and silently replacing the
  stored template, and a server could opt out of the refusal. Updating a
  COMPOSITION whose request body carries a `uid` naming a different version
  container than the URL must be rejected; the mismatch was previously
  reported without affecting the verdict. Both are now gating conformance
  cases. The published conformance artifacts and the ambiguity register
  record the specification sentence behind each.

- **The published Conformance Statement now declares the non-openEHR surface
  this server serves** (#527). A new "Additional non-openEHR surface" section
  lists every extension route family — health, status, the OpenAPI/Swagger
  meta routes, management, terminology, event subscriptions, multi-tenancy,
  the FHIR R4 connector and its mapping store, the ITI-81 audit read,
  `PARTY_RELATIONSHIP`, the bare stored-query list, the admin
  template/query/config routes and SMART discovery — with the routes it
  serves and the configuration that enables it. The section states plainly
  that none of it is part of any conformance claim: no openEHR specification
  governs these routes, no conformance case exercises them, and no verdict
  depends on them. A reader of the statement no longer has to discover the
  extension surface on the wire.

- **Canonical-XML support is now declared per resource family in the
  Conformance Statement, instead of being assumed for every resource**
  (#572). The openEHR release publishes an XML document element for only
  eight names — `composition`, `version`, `items`, `template`, `extract`,
  `extract_request`, `versioned_object`, `archetype` — while its REST API
  addresses `application/xml` to the whole resource surface. For a resource
  with no published document (EHR, EHR_STATUS, the directory FOLDER, the
  demographic party types, CONTRIBUTION) the specification therefore neither
  requires a server to serve XML nor forbids it, so the suite no longer
  asserts either answer: the statement declares, per family, whether this
  server offers canonical XML there, and the conformance run judges the
  matching branch — the XML read, or the `406 Not Acceptable` refusal the
  specification designates for an `Accept` a service cannot fulfil. This
  server declares XML support for EHR, EHR_STATUS, directory and the party
  families and declares it unsupported for CONTRIBUTION reads, exactly as it
  behaves. The full per-resource classification, its citations and the
  upstream report asking openEHR to reconcile the two inventories are
  recorded in the conformance ambiguity register as `AMB-167` / `UPR-127`.


- **Stored top-level objects now carry their copied `uid` at commit time**
  (#439). The full three-part `OBJECT_VERSION_ID` is stamped into the
  canonical body before it is decomposed, signed, and stored, so the
  contained object served inside an ORIGINAL_VERSION envelope, the bare
  resource reads, AQL projections, and EHR Extract exports all carry the
  identical uid value (ITS-REST overview *Resources* §Identifier types).
  Previously the uid was injected only on some read paths; clients now see
  one consistent shape everywhere. Imported (EHR Extract) content is
  exempt — its bodies are preserved verbatim.
- **`EHR.ehr_status` references the version container by its
  `HIER_OBJECT_ID`** (#426). The served EHR body's `ehr_status` OBJECT_REF
  (typed `VERSIONED_EHR_STATUS` per the RM invariant) previously carried an
  `OBJECT_VERSION_ID` naming one version — inconsistent with the sibling
  `ehr_access` ref and with the RM's container semantics (`OBJECT_REF.id` is
  the id of the referenced object; the referenced object is the
  VERSIONED_EHR_STATUS, whose uid is a `HIER_OBJECT_ID`). Both refs now
  carry the container id. Clients that read the current EHR_STATUS version
  uid from the EHR body must fetch `GET /ehr/{ehr_id}/ehr_status` and use
  its own `uid` instead.

### Fixed

- **ADL 1.4 archetypes with anonymous archetype slots are accepted.** The
  ADL 1.4 specification writes archetype slots without a node id in its own
  examples (`allow_archetype OBSERVATION occurrences matches {0..1} …`), and
  published CKM archetypes use that form — but the parser demanded
  `[atNNNN]` and refused such sources with a syntax error, so
  `POST /definition/archetype/adl1.4` answered `422` for spec-valid
  archetypes. Both the anonymous and the identified slot forms now parse;
  ADL 2 sources still require the node id, as ADL 2 defines.
- **An empty TDD batch aimed at a non-existent EHR is now refused.**
  `POST /message/tdd/{ehr_id}/batch` verified the target EHR once per
  document, so a batch with no documents answered success without checking
  the EHR at all. The target is now verified for every batch, empty ones
  included, and an unknown EHR answers `404`.

- **An activity-report request whose time interval runs backwards is now
  refused instead of answered.** `GET /admin/report/*` accepts an optional
  `time_interval=<lower>/<upper>`; a pair bounded on both sides with the lower
  bound *after* the upper one is not an interval at all (the openEHR BASE
  `Interval` invariant requires `lower <= upper`), and the server used to run
  it anyway and hand back the empty count such a range selects — a
  truthful-looking answer for a window nobody asked for. It is now `400`.
  Equal bounds remain a legitimate single-instant interval.

- **A corrupt dump archive no longer reports as an internal server fault.**
  `POST /admin/load` against a location whose manifest or segment is mangled
  or truncated used to surface as an unexpected-exception `500`, while a
  location holding no archive at all reported the openEHR service model's
  `file_not_writable`. Both are the same fact — the location does not hold a
  readable archive — and both now report `file_not_writable`, with nothing
  loaded either way.

- **An export requesting a format this server does not implement now answers
  `501 Not Implemented` instead of `400`.** `openehr_canonical_xml` and the
  `7z` compression format are valid values in the openEHR service model that
  this server does not build; reporting them as malformed requests was wrong,
  and the response now says the functionality is unsupported.

- **The template list collapses to the latest version of each template when
  the `version` parameter is absent** (#614). `GET
  /definition/template/adl1.4` (and the ADL2 twin) used to return every
  stored version regardless; the released openEHR REST API says an absent
  `version` returns "only the latest version". Pass `version=*` to list every
  stored version — the admin console's template inventory does exactly that,
  so its view is unchanged.
- **Conformance runner: a requirement the openEHR specification dates to a
  release is now judged only against the servers that claim that release**
  (#627, #628). Two rules the ITS-REST overview introduces at Release 1.1.0 —
  that an `ETag` carrying a resource identifier must be weak (`W/"…"`), and
  that `Location` is no longer returned on reads and deletes — used to be
  enforced or waived by accident, depending on whether a case happened to
  carry a version floor for some unrelated reason. Each rule now carries the
  floor itself, so a target declaring an earlier ITS-REST release is still
  driven for the operation and still judged on everything else, while a target
  declaring 1.1.0 or later faces the rule everywhere it applies; a test
  derives the affected set from the committed catalogue so no future binding
  can escape it. Separately, the query resultSet's `ETag` is no longer
  required to be PRESENT: the specification names the header without any
  requirement keyword and its only strength anywhere is a SHOULD, so a server
  that omits it is not failed — while a server that emits it must still emit
  it in the weak form. Conformance verdicts for this product are unchanged (it
  declares ITS-REST 1.1.0 and returns the header).

- **Conformance runner: one operation is now sent one way** (#629). The runner
  built request headers in two places — once for a case's own steps and once
  for the preconditions a case needs — and the two disagreed, so the same
  operation went on the wire differently depending on which path reached it
  (a template upload was refused as a case and accepted as a precondition).
  There is now a single header-construction path; a binding declares the
  `Accept` it intends; and a refusal of that `Accept` is recorded as a named
  outcome instead of vanishing into an unmapped status. The published
  conformance report also gains a per-capability table showing how many cases
  passed, failed, and came back inconclusive, so a divergence can no longer
  hide behind an inconclusive exchange. A binding may now also declare the
  release its wire first appeared in, and cases driving it are recorded
  not-applicable — with that citation — for targets that declare an earlier
  one.

- **Conformance runner: a create asked for a minimal response may answer
  either `201 Created` with an empty body or `204 No Content`** (#630). The
  specification says an empty-bodied response SHOULD use `204`, and its
  machine-readable artifacts declare `201` for creates; both are therefore
  conformant, and the suite now judges both instead of leaving one of them an
  inconclusive result. Updates are unchanged (`204` only, where both sources
  agree).

- **Web Template: a template that narrows a party to `PARTY_RELATED` now
  describes its party fields** (#600). When an operational template pins a
  party slot — a subject, a composer, a participation performer — to
  `PARTY_RELATED`, the generated Web Template used to describe that node as an
  empty container: none of the `|name`, `|id`, `|id_scheme` and
  `|id_namespace` fields the Simplified Formats specification gives every
  party appeared, so a form builder reading the Web Template could not offer
  them even though the server has always accepted and returned them. The four
  fields are now described, alongside the `relationship` sub-path the narrowing
  adds. The same held wherever a party node also constrained an attribute; the
  fields survive that too. Nothing changes for stored data or for the FLAT and
  STRUCTURED wire.

- **FLAT/STRUCTURED: the specification's other spelling of a related party's
  relationship is accepted on input** (#589). The Simplified Formats mapping
  table for a `PARTY_RELATED` writes the relationship sub-path
  `…/_relationship`, while every example block in the same section — and the
  participation-performer table — writes `…/relationship|code`. Only the
  example form was accepted, so a producer that followed the table row had
  its composition rejected with an unknown-path error. Both spellings are now
  read, and either one makes the party a `PARTY_RELATED`. What the server
  *emits* is unchanged: always the example spelling, so stored data and
  round-trips look exactly as before.

- **Three FLAT/STRUCTURED mapping gaps: an entry's subject, null-flavoured
  elements, and the event-context paths** (#532, #533, #534). An entry's
  `subject` now travels over the wire in both directions: a composition whose
  OBSERVATION, EVALUATION, INSTRUCTION, ACTION or ADMIN_ENTRY names someone
  other than the record subject emits `…/subject|name`, `…|id`,
  `…|id_scheme`, `…|id_namespace` plus the `/_identifier:i` and
  `/relationship` sub-paths, and the same keys are accepted on input —
  previously the subject was dropped on the way out and rejected as an
  unknown suffix on the way in, so the information was lost in both
  directions. A "self" subject carrying an external reference is marked
  `|_type: PARTY_SELF` so it comes back as itself rather than as an
  identified party. Second, an element that records *why* a value is missing
  (a null flavour, which the reference model makes mutually exclusive with
  the value) now keeps `/_null_flavour` and `/_null_reason` through a full
  round trip; the flattener reached elements only through their value, so a
  null-flavoured element vanished entirely. Third, the event-context fields
  the specification also spells as paths — `…/context/start_time` and
  `…/context/setting` — are honoured on input instead of being silently
  discarded in favour of the `ctx/` defaults, as are an entry's
  `…/language` and `…/encoding`; a bare `…/context/setting|code` resolves
  against the openEHR *setting* value set exactly as `ctx/setting` does.
  Paths the specification does not define are still rejected with a clear
  error rather than ignored.

- **The SMART discovery endpoint is fully described in the published API
  reference** (#535): the `application/json` requirement, the required
  `org.openehr.rest` service with its absolute `baseUrl`, the
  capability-honesty rules, the public pre-auth posture, and a worked
  document example — previously a one-line declaration.

- **The FLAT/STRUCTURED mapping of `INSTRUCTION_DETAILS` and
  `INTERVAL_EVENT.sample_count`** (#521). An ACTION's instruction details now
  travel over the wire exactly as the Simplified Formats specification maps
  them — three suffixes on the `_instruction_details` field itself:
  `|path`, `|composition_uid` and `|activity_id`. Previously the server
  emitted a nested `_instruction_details/instruction_id` field with generic
  object-reference suffixes, so `|composition_uid` was never produced, the
  instruction path sat one level too deep, and two suffixes the
  specification does not define were emitted; clients that sent the
  specified form had the details silently dropped. Both directions are now
  symmetric, so a composition round-trips through FLAT and STRUCTURED
  without losing the reference. Separately, an interval event's
  `|sample_count` (the count of samples the interval summarises) is now
  both emitted and accepted; it was previously ignored in both directions.

- **The template documentation no longer advertises a build flag that does
  not exist** (#521). The templates-and-validation page described an
  `ehrbase-quirks` build that renumbers duplicate node names and accepts two
  vendor-only `DV_QUANTITY` suffixes; that feature was removed long ago.
  There is one behaviour — the one the specification prescribes.

- **The published API reference is now honest about the non-openEHR surface,
  and follows a non-default base path** (#526). Every operation this server
  serves outside the standardised openEHR ITS-REST resource set — the
  management, terminology, event-subscription, multi-tenancy and FHIR R4
  groups, plus the IHE ITI-81 audit retrieval — now states in its own
  published description that no openEHR specification governs it (the flag
  previously lived only in source-module comments the document never
  carried), and each disabled-group `404` now says that an unauthenticated
  caller is answered `401` first, which is what the server actually does. The
  `/status`, `openapi.json`, Swagger-UI and System-`OPTIONS` declarations now
  follow a configured `server.base_path` instead of always printing the
  default deployment's paths. The twelve per-family
  `ehrbase-{family}.openapi.json` documents are documented, the ITI-81 and
  admin-extension operations now appear in a family document (previously in
  none), and the document itself declares a `servers` block, a link to the
  implemented ITS-REST release, descriptions for every tag it uses, and the
  implemented ITS-REST contract version as `x-openehr-its-rest` (distinct
  from `info.version`, which is the product version).

- **SMART App Launch conformance** (#519). The discovery document's
  `services.*.baseUrl` values are now absolute URLs built from the new
  required `smart.public_base_url` origin (the specification requires
  absolute URLs); the `openehr-permission-v1` capability is advertised only
  in fail-closed mode (`require_smart_scopes = true`) so advisory
  deployments no longer over-claim fine-grained enforcement; operators can
  advertise the HL7 base capabilities via `smart.endpoints.capabilities`;
  enabling SMART now boot-validates the origin plus the
  authorization/token endpoints; the published OpenAPI's discovery path
  follows a configured `platform_base_url`; and — the substantive gap —
  the **template and AQL scope families now enforce**: in fail-closed mode
  a token without a matching `template-…`/`aql-…` scope is denied `403` on
  the template and query routes (previously only the composition family
  was gated).

- **The published API reference now describes the admin endpoints in full,
  and the disabled-admin answer is documented correctly** (#513). The five
  admin operations — the released `DELETE /admin/ehr/{ehr_id}` and
  `DELETE /admin/ehr/all`, plus the template-delete, stored-query-version-
  delete and effective-config extensions — gained the branches they actually
  answer (`400` for a malformed EHR id, `401`/`403` from the admin role gate,
  `404`, the template `409`), the mandatory empty `Allow` header on every
  disabled-group `405`, and worked request/response examples. They now carry
  the released operation text verbatim, including the permanent-physical-
  delete cascade and its data-protection (GDPR) sentence, the
  development/testing note on the bulk route, and the fact that this server
  deletes synchronously (`204` only — the specification's optional
  asynchronous `202` is never returned). The bulk route documents both
  accepted query forms (`?ehr_id=a&ehr_id=b` and `?ehr_id=a,b`) and that an
  absent or empty list deletes every EHR; the three extension routes are
  flagged plainly as our own, governed by no openEHR operation. Reference
  documentation and configuration docs that claimed a disabled admin API
  answers `404` were corrected: it answers `405 Method Not Allowed` with an
  empty `Allow` header.

- **Demographic ITEM_TAG collections honour the released dual-form
  addressing** (#509). A version-addressed `uid_based_id` on the demographic
  tag routes now reads, replaces, and deletes that VERSION's own distinct
  tag collection (previously every form reached the container's set); the
  tags GET and DELETE now answer `404` for a nonexistent, wrong-kind, or
  cross-space target (previously an empty `200` list); both
  `openehr-item-tag` and `openehr-version-item-tag` request headers are
  accepted on party create AND update, each landing on its own target's
  collection with its own response-header echo; a tag's `target` is now the
  bare RM `UID_BASED_ID` (an `OBJECT_VERSION_ID` for version targets) and
  its `owner_id` follows the released examples' `local`/`SYSTEM` shape; and
  the PARTY_RELATIONSHIP extension's stale-delete `409` now echoes the
  latest `version_uid` in `ETag` like the party delete it mirrors.

- **The published API reference now describes the demographic item-tag
  endpoints in full** (#510). The sixteen `ITEM_TAG` operations — the
  person / agent / group / organisation / role `tags` read, replace and
  delete-by-key, plus the space-wide `GET /demographic/tags` — gained the
  status branches they actually answer (`400`, `404`, `406`, `415`, `422`),
  the `Prefer` / `Content-Type` / `Accept` request headers, the
  `Preference-Applied` echo on both replace branches, and worked ITEM_TAG
  examples. They now state plainly that a version-addressed `uid_based_id`
  and a container-addressed one name two DISTINCT tag collections, that an
  empty list on the replace clears every tag, that deleting by key alone
  removes every tag under that key whatever its `target_path`, and that a
  tag collection is never change-controlled — so no `ETag`, `Last-Modified`
  or `Location` is offered anywhere on the family. The space-wide list is
  documented for what it is: the one tag route with no scoping parameter at
  all, no paging, and `200` (an empty array when nothing matches) or `400`
  as its only outcomes.

- **Demographic header echoes** (#388). The stale-version party DELETE's
  `409 Conflict` now returns the latest `version_uid` in `ETag` (the released
  response requires it); and the demographic CONTRIBUTION read now carries
  the weak `ETag` (the contribution uid) and `Last-Modified` from the
  committal instant, matching its EHR sibling.

- **The published API reference now describes the demographic endpoints in
  full** (#505). The 26 person / agent / group / organisation / role,
  versioned-party and demographic-contribution operations in the served
  OpenAPI document gained the response headers they actually send (`ETag`,
  `Location`, `Last-Modified`, `Preference-Applied` and the two
  `openehr-*-item-tag` headers), the status branches they actually answer
  (`204`, `400`, `406`, `409`, `412`, `415`, `422`), the committal
  (`openehr-version` / `openehr-audit-details`), `Prefer`, `If-Match`,
  `Accept` and `Content-Type` request headers, worked PERSON /
  VERSIONED_PARTY / CONTRIBUTION examples, and a spec citation on every
  branch. Reads and deletes no longer suggest a `Location` header they never
  send, and the party routes now state plainly that Simplified (FLAT /
  STRUCTURED) media types are refused because a demographic party is not
  templated. The eight `PARTY_RELATIONSHIP` operations are labelled for what
  they are — an extension of this server, with no openEHR REST operation
  behind them — and the group carries a note that the openEHR Demographic
  API is itself a `DEVELOPMENT`-state specification.

- **Stored-query stores answer honest `Location`s and validate the version
  segment** (#498). The version-less `PUT /definition/query/{name}` now
  always names the version it actually wrote (`…/1.0.0`) in `Location` —
  previously, when a higher version already existed, the header pointed at
  that untouched neighbour. The versioned
  `PUT /definition/query/{name}/{version}` now requires an exact numeric
  `major.minor.patch` and rejects prefix, pre-release, or malformed version
  segments with `400 Bad Request` — previously any string was stored
  verbatim, and a single non-numeric version (e.g. `1.0.0-rc.1`) broke every
  later stored-query list and retrieval on the server. Both store forms also
  now refuse a payload declaring a media type other than their single
  `text/plain` body type with `415 Unsupported Media Type` (an absent
  `Content-Type` remains accepted).

- **The published API reference now describes the stored-query endpoints in
  full** (#499). The four stored-query operations in the served OpenAPI
  document gained the `Location` response header both stores actually send,
  the bodyless shape of their `200`, the status branches they actually
  answer (`400` everywhere, `406` on the reads, `409` on the versioned
  store, `404` on the version read), the qualified-name and `version`
  grammars (including the reserved `aql` name and the read-side
  prefix-resolution rule), request and response examples, and a spec
  citation on every branch. The bare "list every stored query" route is now
  labelled for what it is — an extension of this server, not a released
  openEHR operation.

- **Template rejection statuses are coherent across both upload routes**
  (#493). An ADL2 source with grammar-level syntax errors now answers
  `400 Bad Request` (the released "syntactically invalid … content" branch)
  instead of `422`; AOM2 validation-phase failures on a parsed source keep
  answering `422` with the rule codes in `validationErrors`. On the ADL 1.4
  side, an AOM2 artefact-validity violation on a successfully parsed OPT now
  answers `422` with the rule code in `validationErrors` (previously `400`) —
  syntax gates `400`, semantics gate `422`, on both routes.

- **Template-upload rejection statuses follow the released split** (#489).
  An ADL 1.4 OPT upload whose body is not well-formed XML now answers
  `400 Bad Request` (the released "syntactically invalid … content" branch)
  instead of `422`; well-formed XML that is not a valid OPT stays `422`.
  The ADL2 template upload now refuses a payload declaring a media type
  other than its single `text/plain` body type with `415 Unsupported Media
  Type` (an absent `Content-Type` remains accepted), mirroring the ADL 1.4
  guard.

- **The published API reference now describes the template endpoints in
  full** (#490). The nine ADL 1.4 / ADL 2 template operations in the served
  OpenAPI document gained the response headers they actually send (`ETag`,
  `Location`, `Preference-Applied`), the status branches they actually
  answer (`400`, `406`, `415`, `422`), request/response examples, and a
  spec citation on every branch; "Get template at version" is now marked
  deprecated, as the openEHR REST specification marks it.

- **Stored-query POST bodies accept `{}`; the query POSTs accept the URL
  parameter forms** (#481). The three body members of the stored-execute
  body are optional (the docs text gives `offset` a default and makes
  `fetch` implementation-default — the stalled required-list loses), so a
  parameterless stored query executes with an empty body; and all three
  POSTs now accept `offset`/`fetch`/named `$parameters` from the URL (the
  docs-text SHOULD-list draws no GET/POST distinction), with a body-vs-URL
  disagreement rejected 400.

- **Tag GET/DELETE verify the addressed target; empty `target_path`
  normalizes to absent** (#474). `GET`/`DELETE` on the per-target tag
  routes now answer 404 for a nonexistent, foreign-EHR, or wrong-kind
  `uid_based_id` (the released trigger: "when the `uid_based_id` does not
  exist"; previously the GET answered `200 []` and the DELETE was not
  kind-checked), and a `target_path: ""` on the tag PUT is normalized to
  the absent path so `""` and absent are one `(key, target_path)`
  identity. The EHR-wide tag listing likewise answers 404 for an unknown
  `ehr_id` (previously `200 []`).

- **Contribution change-type mismatch statuses follow the released
  assignment** (#467). A non-creation `change_type` committed as the FIRST
  version of a versioned object (the released `400_CONTRIBUTION` trigger:
  "the modification type does not match the operation - i.e. first version
  of a MODIFICATION") now answers 400; a `249|creation|` member carrying a
  `preceding_version_uid` — the unassigned mirror case — moves to 422.
  Previously the two were inverted.

- **The CONTRIBUTION GET serves `ETag` + `Last-Modified`** (#463).
  `GET …/contribution/{contribution_uid}` now carries the contribution-uid
  weak `ETag` (the same identity the 201 already carries) and a
  `Last-Modified` derived from the contribution audit's commit instant
  (ITS-REST overview *Requests and responses* §"ETag and Last-Modified").

- **Directory by-version reads verify the full addressed identity; the
  directory DELETE 204 carries the deleted version's identity** (#456).
  `GET …/directory/{version_uid}` now answers 404 when the addressed
  `creating_system_id` does not match the stored identity (ITS-REST
  overview *Resources* §Identifier types), and
  `DELETE …/directory` answers 204 with the NEW `523|deleted|` version's
  weak `ETag` + `Last-Modified` (previously header-less), matching the
  composition DELETE.

- **COMPOSITION update body-uid mismatch is 422, not 400** (#451). A PUT
  whose body `COMPOSITION.uid` names a different versioned object than the
  request path is now rejected 422 Unprocessable Entity — the body is
  well-formed and the contradiction is semantic (ITS-REST *Requests and
  responses* §HTTP status codes, the 422 row; no released sentence assigns
  the rejection — register-documented).

- **Versioned-composition version-by-id reads are container-scoped** (#449).
  `GET …/versioned_composition/{versioned_object_uid}/version/{version_uid}`
  previously ignored the container segment and served any version the
  `version_uid` named; a `version_uid` whose `object_id` does not match the
  path's container now answers 404 (ITS-REST overview *Resources*
  §Identifier types; RM `Owner_id_valid`).


- **EHR creation mints RM-valid EHR_STATUS and EHR_ACCESS objects; the RM
  archetype-root invariants are enforced on client bodies** (#423). The
  bootstrap defaults carried an archetype-HRID `archetype_node_id` with no
  `archetype_details` — violating RM `Is_archetype_root` (unconditional on
  both classes) with `Archetyped_valid` ("is_archetype_root xor
  archetype_details = Void"). Both defaults now carry the `ARCHETYPED` block
  (archetype_id = the node id, rm_version 1.2.0), and a client-supplied
  EHR_STATUS/EHR_ACCESS violating `Archetyped_valid` (a root without
  `archetype_details`, or a mismatching `archetype_id`) or `Links_valid`
  (an explicit empty `links` list) is rejected with `422`. Clients that
  previously committed root objects without `archetype_details` must now
  supply it.

- **Imported and archive-loaded EHRs are now complete, first-class EHRs**
  (#425). An EHR-Extract clone (`import_ehr`) created no EHR_ACCESS, so a
  source extract that carried none produced an EHR permanently violating the
  RM invariant `Ehr_access_valid` (`EHR.ehr_access` is 1..1) whose served
  `GET /ehr/{ehr_id}` body simply omitted the mandatory reference; the clone
  now commits the same default EHR_ACCESS the create path uses (RM ehr
  master04 §EHR Creation — a root EHR object, an EHR Status object and an EHR
  Access object), in the import's own transaction. Neither the import nor the
  admin archive load promoted the EHR's subject, so imported/loaded EHRs were
  invisible to `GET /ehr?subject_id=…&subject_namespace=…` and exempt from the
  one-EHR-per-subject `409`; both paths now derive the subject from the landed
  EHR_STATUS. Consequences: importing or loading an EHR whose subject this
  repository already holds is now refused — `409` for an import (naming the
  subject and the EHR that holds it), and, for the archive load, a per-record
  `DUMP_LOAD_FAIL_REPORT` entry that skips just that EHR exactly like a
  duplicate EHR id, leaving the rest of the archive to load.


- **Both EHR creates now accept and merge the committal request headers**
  (#422). ITS-REST `docs/overview/Requests_and_responses.md` §"openehr-version
  and openehr-audit-details" makes it a MUST that a service accept
  `openehr-version` / `openehr-audit-details` on the direct `PUT`/`POST`/
  `DELETE` commits of change-controlled resources and merge "whatever is
  provided … with the default VERSION and VERSION.audit_details attributes on
  commit runtime". Creating an EHR commits its EHR_STATUS and EHR_ACCESS in a
  contribution (RM ehr master04 §EHR Creation), but `POST /ehr` and
  `PUT /ehr/{ehr_id}` ignored both headers — while the served OpenAPI already
  claimed they were merged. They now are: the supplied `description`,
  `committer`, and `system_id` land on the creating contribution and on both
  committed versions' `commit_audit`, and `openehr-version:
  lifecycle_state.code_string` sets the new EHR_STATUS version's lifecycle
  state. `change_type` is constrained to `249|creation|` (a create commits a
  first version): restating `249` is accepted, another group code is `400`,
  and a token outside the `audit_change_type` group is `422`. The OpenAPI
  document now also lists both headers as documented parameters on the two
  create operations.

- **Served OpenAPI: the four EHR-resource operations now document the whole
  wire** (#427). `GET /ehr`, `POST /ehr`, `GET /ehr/{ehr_id}` and
  `PUT /ehr/{ehr_id}` under-described what the server actually does. The two
  creates now declare their `415` (an unprocessable request `Content-Type`,
  including a Simplified Format, which is defined only for templated
  COMPOSITION content) and `406` (an `Accept` that canonical JSON/XML cannot
  satisfy) branches, both a MUST in the REST spec's format sections; the two
  reads declare `406` as well, and `GET /ehr/{ehr_id}` declares the `400` it
  returns for a malformed (non-UUID) `ehr_id`. Every success response now
  carries a header block: `ETag`/`Location`/`Last-Modified`/
  `Preference-Applied` on the `201`s, `ETag` on the reads — where the absence
  of `Location` and `Last-Modified` on a read is now stated explicitly rather
  than left unsaid. The `Prefer`-conditional `201` body is documented as a
  named example pair (`representation` — the full RM `EHR`; `identifier` —
  the single-`uid` object), the `Prefer` header enumerates its three tokens
  and its default, and the request body, the read bodies, and the subject
  query parameters carry real served-shape examples. Corrected false claim:
  `PUT /ehr/{ehr_id}` described `ehr_id` as any `HIER_OBJECT_ID` with "a UUID
  strongly recommended", while the server accepts UUIDs only — which is what
  the abstract service model types the argument as, and every UUID is a valid
  `HIER_OBJECT_ID` root.

- **Served OpenAPI: the System API's `OPTIONS` operation is now documented**
  (#418). The Options-and-Conformance endpoint (`OPTIONS` on the API base
  path) was served but absent from the generated OpenAPI document and
  Swagger UI, because its route mounts outside the documenting router (above
  the CORS layer, deliberately). A documented twin now carries the full
  operation description — the `Options` manifest schema with field
  documentation and example, the `Allow`/`Content-Type` response headers,
  and the `406` negotiation branch.

- **`version_at_time` now accepts a datetime without a timezone, interpreting
  it in the server's local timezone** (#401). ITS-REST
  `docs/overview/Resources.md` §"Datetime format" requires the extended
  ISO 8601 form for datetime query parameters and states
  that "Timezone SHOULD be only supplied when needed, otherwise the local
  timezone is assumed" — so `?version_at_time=2016-06-23T13:42:16` is a valid
  request. It was answered `400 Bad Request`, because both at-time parsers
  (the EHR group's and the DEMOGRAPHIC group's duplicate) required an offset.
  A single shared decoder now backs every at-time read — EHR_STATUS,
  COMPOSITION, DIRECTORY, the `versioned_*` version reads, the demographic
  party/relationship reads, and the contribution `time_range` — and resolves
  an offset-less value against the server's system timezone (`TZ`, else the
  platform's local-time configuration); a value falling inside a
  daylight-saving fold or gap resolves to the earlier and the later instant
  respectively. Genuinely malformed input is unchanged: the basic ISO 8601
  format (`20160623T134216Z`), a date without a time (`2016-06-23` — the
  parameter is specified as "a given time", and reading it as midnight would
  silently serve a version the caller never asked for), a timezone-less value
  carrying an `[Area/Location]` annotation, and anything unparseable are all
  still `400`.
- **Every `405 Method Not Allowed` now carries an `Allow` header, and the
  `408`/`413` transport refusals use the openEHR error body** (#400). ITS-REST
  `docs/overview/Requests_and_responses.md` §"HTTP Methods" answers a method a
  resource does not serve with `405`, over RFC 9110 — the authority that
  section names — whose §15.5.6 requires that "the origin server MUST generate
  an Allow header field in a 405 response containing a list of the target
  resource's currently supported methods". The router's `405` already carried
  it; the `405` returned when the **admin API is disabled** did not, because it
  comes from a matched handler and so never reached the router's allow-header
  machinery. It now sends the empty field value RFC 9110 §10.2.1 defines for
  exactly this case ("An empty Allow field value indicates that the resource
  allows no methods, which might occur in a 405 response if the resource has
  been temporarily disabled by configuration"). Separately, a request that
  times out (`408`) or declares a body over the 16 MiB limit (`413`) was
  answered by the middleware with an empty or `text/plain` body; both now
  render the same `{ "error", "message" }` JSON every other error path emits.
  Finally, the deviation behind all of this is now recorded rather than
  silent: the same spec section *also* SHOULDs `501 Not Implemented` for an
  unrecognized method, and we answer `405` there too — the two SHOULDs overlap
  for any method outside the tabulated subset, `405` is a predefined
  non-conflicting code in the spec's own status table, and a blanket `501`
  fallback would misreport unknown **paths** that are owed `404`. `501` is
  still returned for a recognized but unimplemented operation. Adjudicated in
  the conformance ambiguity register as `AMB-60`, with the wire-surface
  boundary registered alongside it.
- **The `openehr-ehr-id` request header now scopes `GET` query execution too,
  and a scope named twice must agree** (#399). ITS-REST
  `docs/query/Request.md` §"About the `ehr_id` parameter" lets clients supply
  the single-EHR scope "as a query parameter `ehr_id` or alternatively as a
  request header named `openehr-ehr-id`", and §"Common Headers and Query
  Parameters" applies that to "all query execution requests". Only the `POST`
  forms honoured the header: `GET /query/aql`, `GET /query/{name}` and
  `GET /query/{name}/{version}` read the scope from the query string alone, so
  a header-scoped `GET` silently ran as a **population query** across every
  EHR. All six execution operations now resolve the scope through one seam.
  The released text never says what a request carrying *both* forms means, so
  the handling is adjudicated and registered (ambiguity register `AMB-59`):
  both forms naming the **same** EHR execute normally, and both forms naming
  **different** EHRs are rejected `400 Bad Request` rather than silently
  picking one — a request that names two EHRs cannot be answered correctly
  (`docs/overview/Requests_and_responses.md` §"HTTP status codes", row `400`).
  An empty header value carries no identifier and neither scopes nor
  conflicts. The deprecated `openEHR-EHR-id` spelling keeps working (HTTP
  field names are case-insensitive). The header is now also declared on every
  query operation in the served OpenAPI.
- **`Prefer` / representation polish: `return=identifier` is structurally
  never `204`, item-tag echoes are per-target, and `Preference-Applied` is
  emitted from one seam** (#398). Three divergences from ITS-REST overview
  `Requests_and_responses.md`:
  - `Prefer: return=identifier` could fall through to the empty (possibly
    `204 No Content`) minimal response while still claiming
    `Preference-Applied: return=identifier`. The identifier branch now
    carries the identifier it renders, so it is unreachable without one —
    §"Prefer only identifier": "the status will be `201 Created` or `200 OK`,
    never `204 No Content`". A write that genuinely produces no identifier
    applies, and declares, the default `return=minimal` instead of claiming
    an unapplied preference.
  - The `openehr-item-tag` and `openehr-version-item-tag` **response** echoes
    on a change-controlled write merged both targets' tags into one list and
    repeated it under both header names. Each header now confirms only its
    own target's stored list — §"openehr-item-tag and
    openehr-version-item-tag": "`openehr-item-tag` applies to
    *VERSIONED_OBJECT* targets" while "`openehr-version-item-tag` applies to a
    specific target *VERSION*", each confirming "the actual list of
    `ITEM_TAGs` stored". A header the request did not send is not echoed.
    (The demographic surface still emits both headers from one set, because
    its tags are stored against the `VERSIONED_OBJECT` only, so the two
    targets coincide there.)
  - `Preference-Applied` was emitted only by the canonical RM / JSON write
    helpers. It is now declared by every write path through the same seam —
    the demographic party, relationship and CONTRIBUTION writes, both ADL 1.4
    and ADL 2 template uploads, the `ITEM_TAG` collection writes, and the
    Simplified-Formats (FLAT/STRUCTURED) COMPOSITION commit — always naming
    the preference the response actually applied, including the applied
    default `return=minimal` when no `Prefer` was sent. Demographic party
    writes additionally honour `Prefer: return=identifier` (`{uid}` body),
    which they previously ignored.
- **ADL 1.4 template negotiation: the response type mirrors `Accept`, and a
  non-XML OPT upload is `415`** (#397). Two divergences from ITS-REST overview
  `Resources.md`:
  - `GET /definition/template/adl1.4/{template_id}` with
    `Accept: application/json` was answered `Content-Type:
    application/openehr.wt+json` — a type the client never accepted. It now
    returns the same Web Template document under `Content-Type:
    application/json` (§JSON Format: "Proper header `Content-Type:
    application/json` MUST be present in the response of the service unless
    the response has no content body"). `Accept:
    application/openehr.wt+json` keeps the Web Template media type and
    `Accept: application/xml` the canonical OPT, both unchanged. (The
    released source is internally inconsistent here — the operation
    description names only XML + `wt+json` while its `Accept`/`Content-Type`
    enumerations include `application/json` with no schema — so serving the
    Web Template body is the recorded fixed handling, not a `406`.)
  - `POST /definition/template/adl1.4` accepted any `Content-Type` and failed
    a JSON payload with `400` from the OPT parser. A request declaring a
    non-XML payload type is now refused `415 Unsupported Media Type` before
    parsing (§XML Format: "If the service cannot process the request payload
    as XML format, it MUST respond with HTTP status code `415 Unsupported
    Media Type`"). `application/xml` and `text/xml` upload as before, and an
    absent `Content-Type` still reads as the operation's single body type
    (the header is a client MAY).
- **`Last-Modified` and `ETag` completion on the EHR and DEFINITION surfaces**
  (#396). ITS-REST overview `Requests_and_responses.md` §"`ETag` and
  Last-Modified" requires both headers on "VERSION, VERSIONED_OBJECT, or other
  resources that have versioning or unique state identifiers", with
  `Last-Modified` "derived from `VERSION.commit_audit.time_committed.value`".
  Only the `ETag` half shipped previously:
  - `Last-Modified` (IMF-fixdate) is now emitted on every VERSION read
    (`…/versioned_composition/{uid}/version[/{version_uid}]`,
    `…/versioned_ehr_status/version[/{version_uid}]`), on all COMPOSITION and
    `EHR_STATUS` reads and writes (including the delete `204` and the
    FLAT/STRUCTURED representations, whose version identity is
    serialization-independent), and on the EHR create `201`. The value is the
    served version's commit instant — read off the VERSION envelope where the
    body carries one, and off the version row / commit result for the bare
    COMPOSITION and `EHR_STATUS` representations, which have no
    `commit_audit` of their own.
  - `GET /ehr/{ehr_id}` and `GET /ehr?subject_id=…` now carry the weak
    `ETag` built from `EHR.ehr_id.value` — the source the spec section itself
    names. (No `Last-Modified`: the RM `EHR` root is not a VERSION, and
    `time_created` is not a last-modification instant.)
  - The ADL2 template responses (`POST /definition/template/adl2`,
    `GET …/adl2/{template_id}`, `GET …/adl2/{template_id}/{version}`) now
    carry the weak `ETag` their ADL 1.4 siblings already emitted. The value is
    the **resolved** `ARCHETYPE_HRID`, so addressing a template by a partial
    id or major-version prefix still yields an `ETag` that changes when the
    served artefact does.
  `CONTRIBUTION` creation still omits `Last-Modified` (the commit instant is
  not carried out of the version-set commit yet) and is marked `TODO` in the
  service layer.
- **Committal request headers: client `change_type` honoured, DELETE accepts
  the headers, deprecated `openEHR-AUDIT_DETAILS` spelling restored** (#395).
  Three divergences from ITS-REST overview `Requests_and_responses.md`
  §"openehr-version and openehr-audit-details" + §"Deprecated headers":
  - A client-supplied `AUDIT_DETAILS.change_type` (e.g.
    `change_type.code_string="250"` for an amendment) is now merged into the
    commit instead of being silently replaced by the operation default — the
    spec lists `change_type` first among the client-suppliable attributes and
    requires "whatever is provided it MUST be merged". The value is validated
    against the openEHR `audit_change_type` group (out-of-group → `422`,
    `AUDIT_DETAILS.Change_type_valid`) and against the operation (a
    contradicting code such as `249|creation|` on an update is rejected; the
    exact status is spec-unassigned — see ambiguity AMB-54 — and returns
    `400`). Applies to the direct COMPOSITION/EHR_STATUS/DIRECTORY commits
    and the demographic party/relationship commits alike.
  - `DELETE /composition/{id}` and `DELETE /directory` now accept
    `openehr-version`/`openehr-audit-details` and merge the supplied
    description/committer/system_id into the `523|deleted|` commit audit —
    the spec requires the headers accepted on PUT, POST **and** DELETE.
  - The bare deprecated header name `openEHR-AUDIT_DETAILS` (the exact
    spelling in the spec's deprecation table, which is a different HTTP
    header name than `openehr-audit-details`) is accepted again alongside
    the 1.0.3 dotted forms and the current name; the current name still wins
    on conflict.
  The `audit_change_type` constant set now mirrors the complete TERM group
  (all nine codes), locked by a test against the terminology bundle. Five new
  CNF catalogue cases pin the merge family end-to-end.
- **Demographic API: response-header discipline and `If-Match` handling** (#394).
  Three MUST/SHOULD-level divergences from the ITS-REST overview
  (`Requests_and_responses.md`) are corrected on the `/demographic` surface:
  - `Location` is no longer emitted on reads, deletes, or `409`/`412` error
    responses. The header now rides create/update writes only, per §Location
    ("MUST NOT be used to indicate an alternate representation of an existing
    resource"; "MUST ONLY be used for resource creation … or redirect
    responses") and §"Deprecated headers", which deprecates it on `GET` and
    `DELETE`. Those responses keep the weak `ETag` (and `Last-Modified` where
    known), so a client reading the version identity is unaffected; a client
    that was following the `Location` of a `GET`/`DELETE` must use the request
    URL it already has.
  - `If-Match` now accepts the **weak** `W/"…"` form the server itself emits as
    the `ETag`, alongside the bare-quoted and unquoted forms — previously
    echoing the server's own `ETag` back was rejected as a malformed
    precondition (`400`). The full `OBJECT_VERSION_ID` is compared
    case-insensitively (BASE composite-identifier semantics), so a case-variant
    `creating_system_id` no longer raises a spurious `412`. A syntactically
    invalid `If-Match` remains a `400` and is never silently ignored.
  - The `versioned_party` / `versioned_party_relationship` reads (the container,
    its revision history, and the version reads) now carry the weak `ETag` and,
    where the served body exposes the commit instant, `Last-Modified` — both
    SHOULD-present on `VERSION`/`VERSIONED_OBJECT` responses.

### Added

- **Several terminology servers can now serve one instance at the same time.**
  Every entry under `[terminology.external.providers]` is started up, not just
  `default`, and a new `[terminology.external.routes]` map sends each
  terminology to the server that serves it — the key is a terminology id
  (`SNOMED-CT`) or a system URI (`http://snomed.info/sct`), matched
  case-insensitively, and the value names a provider. A terminology with no
  route goes to the provider named `default` (or to the sole configured one).
  Routing applies to the whole terminology surface: the `/terminology/*`
  extension API, AQL `TERMINOLOGY(…)`, and composition validation. So SNOMED CT
  can live on one server while LOINC or ICD live on others — the deployment
  reality openEHR's terminology chapter describes. Configuring a route to a
  provider that does not exist is now a startup error instead of a silent
  fallback.
- **Terminology servers can require OAuth2.** A provider's `oauth2_client` key
  now does something: it names an entry under
  `[terminology.external.oauth2_clients]` (token endpoint, client id, client
  secret or `client_secret_file`, optional scopes, `refresh_leeway_secs`, and
  `client_secret_basic` / `client_secret_post`), and the CDR obtains a
  client-credentials access token and sends it as a bearer credential on every
  request to that server. The token is cached and renewed shortly before it
  expires, so a validation burst costs one token request per token lifetime. A
  refused grant fails the call with a clear error — a request is never sent
  unauthenticated as a fallback.
- **Terminology servers can require a client certificate (mutual TLS).** A
  provider takes three new keys — `client_cert_path`, `client_key_path` and
  `ca_bundle_path` — so the CDR presents a client certificate to that
  terminology server and verifies the server against that server's own trust
  anchors. The identity is per provider because a client certificate is issued
  by the peer's PKI: a deployment enrolled with a national SNOMED CT service, a
  commercial value-set server and an in-house server holds three different
  certificates, and repeating the same paths covers the case where one identity
  really does serve them all. `ca_bundle_path` *replaces* the default trust
  anchors for that provider, so a privately-issued terminology server is pinned
  to its own CA instead of also accepting the whole public web PKI. There is no
  option to skip verification — server-certificate and hostname verification
  stay on for every provider; the bundle changes which anchors are trusted,
  never whether the server is checked. Anything broken (one half of an
  identity, an unreadable PEM, a key file holding no key, a CA bundle holding
  no certificate) fails at startup, never at the first validated code.
- **Composition commits can now check archetype value-set bindings against a
  live terminology server.** When a template binds an `ac` code to an external
  terminology query, and `[terminology.external]` is enabled, committing a
  COMPOSITION resolves that query and requires the coded value to be a member
  of the value set it returns. A non-member is a `422` naming the path, the
  code and the bound query. If the value set cannot be resolved at all (server
  down, error response, no server configured for that terminology), the
  existing `fail_on_error` switch decides: `false` (the default) accepts the
  commit and logs a warning, `true` rejects it. With `[terminology.external]`
  disabled — the shipped default — nothing is resolved, no request is made, and
  commit behaviour is exactly as before.
- **An opt-in `terminology` Compose profile runs a real FHIR R4 terminology
  server beside the CDR.** `docker compose --profile terminology -f
  docker-compose.yml -f docker/sut-terminology.yml up` starts a digest-pinned
  HAPI FHIR JPA server (host port 8090, `EHRBASE_TERMINOLOGY_PORT`) plus a
  one-shot container that seeds it — over the server's own FHIR API — with two
  synthetic test code systems and their value sets, one SNOMED-CT-shaped and
  one LOINC-shaped, and verifies `$validate-code` and `$expand` before exiting.
  The overlay switches on the `[terminology.external]` providers now shipped
  (disabled) in `docker/ehrbase.dev.toml`, so the plain quickstart is
  unchanged. No licensed terminology content is distributed: the fixtures live
  under the reserved `example.test` domain, and the SNOMED-CT-shaped and
  LOINC-shaped codes are invented for the test corpus.
- **The conformance record now covers the terminology-routed surface.**
  `scripts/conformance.sh` composes the terminology profile for every
  `ehrbase-rs` run, and the catalogue gained eight cases: AQL `TERMINOLOGY()`
  resolved through the routed server (the Boolean `validate` form answering
  true and false, and the `expand` filter over committed data), the
  two-simultaneous-servers routing proof, and commit-time archetype
  constraint-binding validation (a member code accepted, a non-member refused,
  and the unresolvable value set under each declared posture). A party's
  `ixit.json` gains a `terminology` block declaring its terminology servers,
  the namespaces each answers for, and its fail-open/fail-closed posture; a
  party that declares none has those cases recorded not-applicable with that
  citation instead of failed.
- **`POST /admin/dump` now serves the `openehr_canonical_xml` logical format,
  which used to answer `501`.** Both openEHR export formats are available:
  the default `openehr_canonical_json` keeps each version's content inline in
  the archive's segment files, while `openehr_canonical_xml` writes each
  version to its own `versions/<version_uid>.xml` entry — a complete
  `ORIGINAL_VERSION` document under the openEHR-published `<version>` root,
  readable by any tool that speaks canonical openEHR XML. The archive's own
  bookkeeping (`manifest.json`, the segment files) stays JSON in both formats,
  because openEHR publishes no XML document form for it. `POST /admin/load`
  is unchanged for callers: it still takes only a location and now reads the
  logical format out of the archive's manifest, exactly as it already detected
  the container. Both formats round-trip in all three containers (loose,
  `archive.zip`, `archive.7z`) and reproduce every record byte-for-byte. A
  single unreadable `versions/*.xml` entry is reported against the one EHR it
  belongs to and skipped, while the rest of the archive loads.

- **The definition and messaging extension routes now document their refusal
  branches.** Every ADL 1.4 / ADL 2 archetype route, every `/message` route
  and every `PARTY_RELATIONSHIP` route declares `401` (no valid principal)
  and — on the writes — `403` (a principal holding the configured read-only
  role) in the served OpenAPI, so a client can see the whole answer set of an
  endpoint before it calls it. The TDD batch additionally documents its `413`
  boundary: the batch has no cardinality limit of its own, only the
  server-wide request-body limit.

- **Conformance: the admin extension batteries now test what must be
  REFUSED, not only what must work.** A server that accepts what the contract
  forbids is as non-conformant as one that refuses what it must accept, and
  the activity-report, EHR/demographic archive and dump/load batteries proved
  only their happy paths. They now also drive the unauthenticated (`401`) and
  non-administrative (`403`) probes on every route of each family, every
  argument-type refusal (a service outside the enumeration, the three ways a
  time interval can be malformed, a malformed id in an archive list, an
  unknown export format, a non-positive segment size), the empty-selection and
  repeat-archive boundaries, and the zip / 7z / loose container-detection round
  trips on load — with the duplicate-report body now asserted rather than
  assumed. The branches that cannot be driven from a client (the
  admin-disabled `405`, which needs a differently configured deployment, and
  the corrupt-archive `5xx`, which needs bytes placed on the server's own file
  system) are recorded as explicit boundaries and covered by in-process tests
  instead of being silently absent.
- **The documentation site renders mathematics.** Formulas on the
  performance pages (the open-loop arrival schedule, the population-anchored
  write-rate derivation) are now typeset with KaTeX, pre-rendered to static
  HTML at build time — pages stay self-contained with no client-side script
  and no CDN request; the KaTeX stylesheet and fonts are served by the site
  itself.
- **Conformance: ADL 1.4 archetype provisioning is now tested rather than
  excused.** openEHR's released REST API defines no ADL 1.4 archetype
  resource, so the capability used to be reported as "excused — unrealized on
  this technology profile" even though this server serves archetype routes of
  its own design. Six conformance cases now execute against
  `/definition/archetype/adl1.4` — upload with source read-back, an
  unparseable-source refusal, listing, and the get/delete branches including
  their not-found halves. Because openEHR gives the capability no wire, the
  published certificate marks the row `extension` and it no longer gates the
  CORE profile — a conscious, register-recorded departure from the
  conformance profiles book, which requires a capability the release gives no
  wire for.
- **The admin dump/load archive now supports 7z compression.** `POST
  /admin/dump` accepts `compression_format: "7z"` alongside `zip` and the
  uncompressed form, packing the same archive entries into one `archive.7z`;
  `POST /admin/load` detects and reads all three container forms without
  being told which one it was given. (The `openehr_canonical_xml` logical
  format remains a declared `501` boundary — the archive's XML form is a
  design of its own, tracked separately.)
- **Repository dump/load and the whole messaging surface are now HTTP
  routes** — the last service capabilities that had no wire. Under the
  existing `EHRBASE__ADMIN__ENABLED` gate and `ADMIN` role,
  `POST /admin/dump` writes an archive of every EHR to a location on the
  server's file system and `POST /admin/load` populates the repository from
  one; both answer `200` with the per-entity report the openEHR service model
  defines (empty when everything succeeded), and a load into a non-empty
  repository reports each already-present EHR rather than failing. Under the
  ordinary clinical authentication — these are not admin routes — the new
  `/message` group serves EHR Extract export (`GET /message/export/{ehr_id}`,
  `POST /message/export` by specification) and import
  (`POST /message/import` for a whole-EHR clone,
  `POST /message/import/{ehr_id}` to add content to an existing one), plus
  Template Data Document import (`POST /message/tdd/{ehr_id}` and its
  all-or-nothing `/batch` sibling). Like the admin extensions, all of these
  are ehrbase-rs extensions: the openEHR service model defines the operations,
  the released REST API surfaces no endpoint for them, and no openEHR
  conformance claim rests on the URLs — see the book's Operations page and the
  served OpenAPI document, which flags every one of them.

- **The dump archive can be written as a ZIP.** `POST /admin/dump` accepts the
  service model's `zip` compression format, packing the manifest, segments and
  multimedia blobs into a single `archive.zip` instead of loose files. Load
  takes no format argument and detects the container, so an archive always
  reads back the way it was written.

- **The admin API gained an activity report and archiving, and the definition
  API gained archetype provisioning** — service capabilities that had no HTTP
  route until now. Under the existing `EHRBASE__ADMIN__ENABLED` gate and
  `ADMIN` role: `GET /admin/report/contribution[/count]`,
  `GET /admin/report/versioned_composition/count` and
  `GET /admin/report/composition_version/count` report CONTRIBUTION and
  COMPOSITION-version activity per service over an optional ISO 8601 time
  interval, and `POST /admin/archive/ehrs` / `POST /admin/archive/parties`
  mark a named set of EHRs or demographic parties archived (a read-neutral,
  idempotent, all-or-nothing marker — never a delete). Alongside the released
  template routes, the definition API now serves the ADL 1.4 archetype store
  (`POST`/`GET /definition/archetype/adl1.4`,
  `GET`/`DELETE /definition/archetype/adl1.4/{archetype_id}`) and the ADL 2
  archetype/artefact views (`GET /definition/archetype/adl2[/count]`,
  `GET /definition/artefact/adl2[/count]`,
  `DELETE /definition/artefact/adl2/{artefact_id}`). All of these are
  ehrbase-rs extensions: the openEHR service model defines the operations, the
  released REST API surfaces no endpoint for them, and no openEHR conformance
  claim rests on the URLs — see the book's Operations page and the served
  OpenAPI document, which flags every one of them.

- **The measured hospital-simulation workload now exercises every claimed
  capability** (#625). The performance run used to touch about a third of the
  capabilities the conformance statement claims, and the rest were listed as
  "not yet exercised" catalogue gaps. Sixteen new operations joined the
  measured workload — demographic registration (person create/read/amend plus
  relationship churn), template example and ADL 2 definition polls, advanced
  and terminology-backed AQL reads, Simplified-FLAT commit and read-back,
  version-provenance (signature) reads, the System API options probe, the
  SMART service-discovery fetch, and the two access-control refusals — so the
  published Workload Coverage table now answers "yes" for every claimed
  capability except eleven that carry a per-capability, register-linked
  reason: either the operation would destroy the measured population
  mid-run (physical deletion and the released admin delete API), or openEHR
  defines no wire and this product serves no route for it, leaving the load
  instrument nothing to send. No row is left undecided, and a future journey
  that lands one of those capabilities is forced to delete its exclusion.
- Measured runs can now drive a SMART-secured deployment: the load client
  mints the scope-limited access token its ixit principal declares (once per
  token lifetime, never per request), so a deployment running the SMART
  resource-server posture — the standard EHRbase-rs conformance posture — is
  measurable at all. Boundary probes address the read-only and
  unauthenticated principals a deployment declares; a deployment that
  declares none simply runs the workload without those journeys.
- **Conformance: the last two untested capabilities now carry real executed
  batteries** (#624). "Demographic archetype validation" and "Bulk EHR load"
  were the two capabilities the conformance report listed with *no cases* —
  named in openEHR's conformance profiles book, but never actually exercised.
  Both are now tested against the released REST wire. Demographic archetype
  validation gets eight isolated cases over the party-commit endpoints: a
  committed PERSON/ROLE is refused when it is not archetype-rooted, when its
  root archetype identifier contradicts its own archetype details, when an
  optional list (contacts, roles, capabilities) is present but empty, and when
  an identity's value is missing or carries the wrong openEHR type — plus an
  accept case proving a fully archetyped party, contacts, addresses and
  languages included, is stored and read back intact. Bulk EHR load is
  verified as what it actually is on released wire — a population loaded
  through the ordinary EHR and composition endpoints — with one case covering
  breadth (eight EHRs, one composition each, all identities distinct and every
  document read back unchanged) and one covering depth (four commits into a
  single EHR, each independently addressable, with an AQL query over that EHR
  returning exactly the loaded set). Both capabilities are now claimed in the
  published conformance statement, and their case-count floors are recorded so
  the coverage can only grow.

- **Conformance: the PARTY_RELATIONSHIP capability is now tested rather than
  excused** (#623). openEHR's released REST API defines no PARTY_RELATIONSHIP
  resource, so the six relationship operations used to be reported as
  "excused — unrealized on this technology profile" even though EHRbase-rs
  serves them. They are now driven for real: fifteen conformance cases execute
  against the `/demographic/party_relationship` routes this product serves of
  its own design, covering create, read, read-at-time, read-at-version, update
  and delete plus their refusal branches. The certificate marks the row
  `extension`, which is a promise as much as a label — no openEHR profile
  result may rest on a route openEHR does not specify, and the runner now
  fails validation if that line is ever crossed (a new `realization-scope`
  gate, with the binding's route required to appear in the published
  extension-surface declaration). Such cases are also skipped, with a cited
  reason, for any system under test whose conformance statement does not claim
  the capability — a route openEHR does not specify is an offer only the party
  making it answers for, so the published comparison against other products
  never charges them for routes they never offered.

- **Conformance runner: a certification claim can no longer be hollow** (#622).
  `cnf-runner validate` now reads the committed party statements beside the
  artifact root and relates every claim to the catalogue, so three new gates
  fail before any system under test is even composed. `claim-completeness`
  rejects a claimed capability with no verdict-bearing case at all, and
  requires a capability whose every case is excused (because the openEHR
  release publishes no wire for it) to name the register entry that
  adjudicated that — an excuse that outlives the missing wire is a finding
  too. `capability-depth` gives every capability a `min_cases` floor so one
  token case can never certify it; floors only ever ratchet up.
  `workload-coverage` requires every claimed capability the measured
  hospital-simulation workload does not exercise to carry an adjudicated
  exclusion, which the conformance certificate now prints with its reason in
  place of the previous bare "NO — catalogue gap" cell. The certificate's
  Profile Report also gains a **Realization** column saying whether a
  capability was verified over released ITS-REST wire or over routes this
  product serves of its own design (the latter can never gate an openEHR
  profile tier).

- **Conformance runner: the SMART on openEHR boundary is now executed, not
  declared** (#538). Three behaviours that were previously carried as
  statement-level claims are real conformance cases: the
  `/.well-known/smart-configuration` discovery document (served from the
  Platform base URL as `application/json`, advertising the required
  `org.openehr.rest` service at an absolute base URL), the resource-scope
  grammar that lets a granted scope reach exactly the operation it names, and
  the 403 refusal of a request the granted scopes do not permit. Because SMART
  is off by default, they run in their own **lane**:
  `CONF_SMART_MODE=1 bash scripts/conformance.sh` boots the server with the
  SMART resource-server posture enabled (`docker/sut-smart.yml`), drives the
  SMART group, and writes to `docs/conformance/<sut>-smart/`; the default lane
  is untouched and remains the published baseline. To exercise scopes at all
  the runner now mints its own short-lived access tokens against a **committed
  test issuer** (`tools/cnf-runner/party/smart/` — public test key material for
  the harness, never usable for anything else), because a CDR validates tokens
  and never issues them and the conformance stack runs no Authorization
  Server. A conformance target that does not run the SMART role simply does not
  declare the lane in its `ixit.json`, and these cases are recorded
  not-applicable with that citation rather than failed.

- **Conformance runner: two more wire behaviours are now measured, not
  excused** (#539, #569). The bulk admin delete's subset selector is exercised
  in the repeated `?ehr_id=a&ehr_id=b` form the openEHR path template asks
  for, proving every named EHR is deleted rather than only the first; and the
  rule that a server stamps its OWN configured system identifier into a
  commit audit when the client supplies none is now checked against that
  identifier, not merely against "some non-blank value". The identifier is a
  deployment fact no openEHR operation exposes, so a conformance target
  declares it in its `ixit.json` (`"system_id": "…"`); a target that declares
  none has those cases recorded not-applicable with that citation instead of
  being checked against a guess. Both behaviours were previously carried as
  cited coverage gaps.

- **Conformance coverage: calling a resource with the wrong HTTP method is
  now measured** (#596). The openEHR REST specification says a method the
  specification recognizes but the addressed resource does not serve should be
  answered `405 Method Not Allowed`, and the HTTP standard it defers to
  requires that answer to carry an `Allow` field listing the methods the
  resource does support. The conformance suite now proves both on a real
  resource — a `DELETE` to the EHR collection, which the specification serves
  only under `POST` and `GET` — instead of recording the behaviour as an
  untestable gap. The `Allow` check asserts that both specified methods are
  listed while tolerating any order and any additional methods a server
  chooses to support.

- **Canonical XML: choose the openEHR schema namespace per request** (#196).
  openEHR publishes its XML schemas in two lineages that differ only by the
  namespace a document declares — `http://schemas.openehr.org/v1` (the stable
  release) and `http://schemas.openehr.org/v2` (the newer, trial release). You
  can now pick one with a `version` parameter on the XML media type:
  `Accept: application/xml; version=2` returns the v2 namespace, and
  `Content-Type: application/xml; version=2` declares a v2 request payload. A
  v2 response is labelled `Content-Type: application/xml; version=2`. Nothing
  changes for existing clients: omitting the parameter (or sending
  `version=1`) serves the v1 namespace under a plain
  `Content-Type: application/xml`, exactly as before, and request payloads in
  either namespace have always been accepted. Asking for a namespace the
  server does not serve is `406 Not Acceptable` on `Accept` and `415
  Unsupported Media Type` on `Content-Type`. Operational-template XML
  (`…/definition/template/adl1.4/{template_id}`) is always v1 and ignores the
  parameter. The parameter is an EHRbase-rs extension — the openEHR REST
  specification predates the two lineages and defines no way to select one.

- **Conformance coverage: the ITEM_TAG routes are now measured** (#288). All
  twenty-three released tag operations — the EHR-wide and demographic-wide
  listings, the COMPOSITION and EHR_STATUS families, and the five demographic
  party families — are enumerated by the conformance instrument for the first
  time; they have no openEHR service-model interface, so they were previously
  invisible to its coverage derivation. Thirty-two new cases turn the five tag
  laws into executed wire assertions: tag identity is the (key, target_path)
  pair, a container's tag collection and a version's tag collection are
  disjoint on read, write and delete alike, `ITEM_TAG.target` is served as the
  bare openEHR identifier, every typed tag route answers 404 for a uid of
  another kind (within the EHR space, within the demographic space and across
  the two), and the `openehr-item-tag` / `openehr-version-item-tag` request
  headers on a commit land in their own separate collections. Tag support is
  reported under a new **ItemTags** capability at the OPTIONS tier, matching
  the specification's own statement that a server need not support ITEM_TAGs.

- **Conformance coverage: the COMPOSITION, CONTRIBUTION and PARTY resources
  are now exercised in canonical XML and in the Simplified Formats, not only
  in canonical JSON** (#288). Eighteen new CNF cases drive
  `Accept: application/xml` reads and `Content-Type: application/xml` commits
  across composition create/update/latest/at-time/at-version, the
  VERSIONED_COMPOSITION container, the composition and contribution existence
  probes, and the whole PERSON create/update/read family, plus FLAT and
  STRUCTURED reads of a composition at latest and at time and FLAT/STRUCTURED
  composition updates. Each row asserts the negotiated response media type
  the specification makes a MUST, and the XML commits are compared against the
  canonical-JSON twin of the same resource, so a format-specific data loss
  shows up as a failure rather than a silent difference. One branch stays
  deliberately unexercised and is now recorded with its full derivation: the
  openEHR release declares `application/xml` for the CONTRIBUTION *commit*
  but publishes no XML form of the commit envelope, which is reported
  upstream rather than invented locally.


- **Served OpenAPI: complete documentation for the six Query operations**
  (#482). The two ad-hoc and four stored AQL executions now document what
  the wire actually does. Every `200` declares the weak RESULT_SET `ETag`
  (an identifier of the result set — ours is a deterministic content digest,
  since the released `ResultSet` schema carries no id field) and carries a
  canonical RESULT_SET example: `columns[]` with the `#N` unaliased-column
  convention, rows whose cells are JSON primitives *and* canonical
  `_type`-tagged RM objects, and the optional `meta` (`_type`,
  `_schema_version`, `_created` in extended ISO 8601, and `_executed_aql` =
  the parameter-SUBSTITUTED text, with `q` keeping the query as submitted).
  The parameters now carry the released semantics: the named-`$parameter`
  binding law and its un-prefixed rule, the `ehr_id` duality (query
  parameter or `openehr-ehr-id` header, deprecated MixedCase spelling
  accepted, a conflict 400), `offset`'s default of 0 and `fetch`'s
  implementation-defined default with the one released prohibition
  (`fetch` cannot be combined with AQL `TOP`), the qualified-query-name
  grammar including the reserved `aql`, and the version exact/prefix
  matching law. Also declared: `415` on the three POSTs, request-body
  examples, and the `Prefer`-scope reason no query response carries
  `Location` or `Preference-Applied`. Where the released text is silent the
  declarations say so explicitly — the reserved protocol keys that never
  bind as AQL parameters, REST paging composing over AQL `LIMIT`/`OFFSET`,
  the URL-vs-body precedence on the POSTs, and the `ehr_id`-scope 404.
  Document only — no wire change.

- **Served OpenAPI: complete documentation for the seven EHR ITEM_TAG
  operations** (#475). The EHR-wide read, the two per-target reads, the two
  collection replaces and the two key-scoped deletes now document what the
  wire actually does. The dual-form `uid_based_id` is spelled out with the
  released version/container sentence and the disjointness it implies (a tag
  has exactly one `target`, so container tags and version tags are separate
  collections and neither read sees the other). The `PUT` bodies are
  described as what they are — a bare JSON array of UPDATE_ITEM_TAG (`key`
  required, `value`/`target_path` optional, `target`/`owner_id`
  server-assigned from the route and ignored if sent), with `[]` quoted as
  the clear-all form, (`key`, `target_path`) as the identity, last-wins on a
  duplicate pair, an empty `target_path` normalizing to absent, and the
  200/204 `Prefer` split (204 by default, 200 carrying the full RESULTING
  list, `return=identifier` resolving to minimal because an ITEM_TAG has no
  uid). The deletes document their SET semantics (every tag under the key on
  the addressed collection) and the released third 404 trigger that makes
  them deliberately non-idempotent. Every operation now declares the target
  guard's 404s (unknown, foreign-EHR, wrong-kind or missing-version target),
  the JSON-only reality (406 for an XML `Accept`, 415 for an XML
  `Content-Type` — no ITEM_TAG type exists in the canonical XML ITS), the
  RM-invariant 422 family on the writes, the `ehr_tags_get` filter semantics
  (AND-combined, exact, case-sensitive, scalar, unbounded), and real ITEM_TAG
  examples including a VERSION-targeted tag. Also recorded: no tag route
  serves `ETag`/`Last-Modified` or accepts `If-Match` — a tag has neither a
  version nor a uid — and the released-text defects met on the way (the
  aggregate read's COMPOSITION-typed response schema, the `_updated`
  responses' copy-pasted "retrieved" wording, `tag_key` vs the `key` path
  parameter, and the "(logically) deleted" wording on a non-versioned
  resource). Document only — no wire change.

- **Served OpenAPI: complete documentation for the three CONTRIBUTION
  operations** (#464). The native change-set commit now declares the whole
  `NewContribution` envelope — `versions[]` of UPDATE_VERSION
  (`preceding_version_uid`, `signature`, `lifecycle_state`, `attestations`,
  `data`, `commit_audit`) plus the change-set `audit`, the accepted `_type`
  spellings (`UPDATE_AUDIT` / `AUDIT_DETAILS` / omitted), the server-set
  `time_committed`, the honoured-if-unused client `uid`, and the
  committer/`system_id` copy-down — with a canonical two-member example (a
  COMPOSITION creation plus an EHR_STATUS modification) and the SPECITS-84
  rule quoted: the envelope stays canonical JSON, only each
  `versions[i].data` takes the FLAT/STRUCTURED form. Every branch is
  documented: `201` with the weak `ETag` carrying the *contribution* uid (not
  a version uid), `Location`, `Preference-Applied` and the `Prefer`-conditional
  bodies (the representation lists the minted version OBJECT_REFs, the
  identifier body the contribution uid, minimal an empty `201`); `400` with
  the released first-version-of-a-MODIFICATION trigger; `404`; `406`; `409`
  (client uid in use — released — plus the non-modifiable EHR, duplicate
  singletons and an EHR_STATUS delete member, flagged as ours); `412` for a
  stale member `preceding_version_uid`; `415`; and the full `422` family
  (empty `versions`, out-of-group change types, data on a delete/attestation
  member, missing data, template and RM-invariant failures). The by-uid `GET`
  documents the plain-UUID `contribution_uid`, `Prefer: return=representation,
  resolve_refs` (members resolved to full ORIGINAL_VERSIONs, which is also
  what makes a simplified `Accept` meaningful), its `200` headers and
  canonical example (members as OBJECT_REFs, full AUDIT_DETAILS with optional
  `description`), and `400`/`404`/`406`. The contribution-list route is
  prominently flagged as our own extension with no openEHR spec behind it,
  and its `offset`/`fetch` clamping (0 / 20, capped at 100 — never a `400`)
  and row shape are now described accurately. Document only — no wire change.

- **Served OpenAPI: complete documentation for the five DIRECTORY
  operations** (#457). Every response now declares its headers (weak `ETag`,
  `Last-Modified`, `Location`, `Preference-Applied`, item-tag echoes) and the
  reads and writes carry canonical FOLDER examples (nested `folders`, `items`
  as OBJECT_REFs); the writes document the `If-Match` precondition — carried
  in the header because these routes have no version segment, so a stale
  value is `412`, never `409` — plus `Prefer`, the `openehr-version` /
  `openehr-audit-details` committal headers and the item-tag headers, and the
  canonical-JSON/XML-only request bodies (a Simplified-Format `Content-Type`
  is `415`, an unfulfillable simplified `Accept` `406`: a FOLDER is not
  templated). The `version_at_time` and `path` query parameters are described
  with the released sentence plus our register-documented resolution rules
  (root-implicit, leading-slash tolerant, folders-only, first-match; a future
  time serves the latest version, a time before the first commit is `404`),
  and every branch the wire serves is documented — the deleted-directory
  `204` on both reads, the `DELETE`'s `204` carrying the new deleted version's
  identity, the `404`s (including an EHR with no directory), the `412`s with
  the latest-uid `ETag`, `400`/`406`/`415`/`422`, and the `409`s that are our
  own design (creating a directory when one already exists, and a
  non-modifiable EHR), each flagged as such. Document only — no wire change.

- **Served OpenAPI: complete documentation for the eight COMPOSITION and
  VERSIONED_COMPOSITION operations** (#450). Every response now declares its
  headers (weak `ETag`, `Last-Modified`, `Location`, `Preference-Applied`,
  item-tag echoes) and a canonical example; the commits document the
  `openehr-version` / `openehr-audit-details` / `openehr-template-id` request
  headers and the four negotiable media types (canonical JSON/XML plus
  `application/openehr.wt.flat+json` and
  `application/openehr.wt.structured+json`); and every branch the wire
  actually serves is described — the `GET`'s deleted-version `204` for all
  addressing forms, the `DELETE` quartet (`204` carrying the NEW deleted
  version's identity, `400` already-deleted, `404`, `409` not-latest with the
  latest-uid `ETag`), `412`/`415`/`406`/`422`, and the `409`s that are our own
  design (duplicate live persistent COMPOSITION per template, and a
  non-modifiable EHR), each flagged as such. Document only — no wire change.

- **Served OpenAPI: complete documentation for the seven EHR_STATUS and
  VERSIONED_EHR_STATUS operations** (#443). Every response now declares its
  headers (weak `ETag`, `Last-Modified`, `Location`, `Preference-Applied`,
  item-tag echoes), canonical examples, and the 406/415 negotiation
  branches; the EHR_STATUS update documents the `openehr-version` /
  `openehr-audit-details` committal headers and the
  `Prefer: return=identifier` response shape. Document only — no wire
  change.

- **`Last-Modified` on VERSIONED_OBJECT container and revision-history
  reads** (#442). `GET …/versioned_ehr_status`,
  `GET …/versioned_composition/{uid}`, and both `…/revision_history` reads
  now carry `Last-Modified` derived from the newest held version's commit
  instant, alongside the existing container-uid weak `ETag` (ITS-REST
  overview *Requests and responses* §"ETag and Last-Modified": both headers
  SHOULD accompany a VERSIONED_OBJECT response).


- **`[server] system_id` — the deployment's own openEHR system identifier is
  now configurable** (#424, `EHRBASE__SERVER__SYSTEM_ID`, default unchanged at
  `ehrbase-rs.local`). The value is stamped into `EHR.system_id` at EHR
  creation (RM *EHR Information Model* §EHR Identifier Allocation: the
  identifier "that would normally be used for locally created EHRs"), into
  `AUDIT_DETAILS.system_id` whenever the client supplies none through
  `openehr-audit-details` (the REST API requires the server to "set it to its
  own configured system identifier"), and into every minted
  `OBJECT_VERSION_ID.creating_system_id`. Previously it was a hard-coded
  constant that no configuration could change. Choose it before the first EHR
  is created and keep it stable — the value is stored per EHR and per version,
  so a later change affects only newly authored data and never rewrites
  existing identifiers. It is distinct from `[server.identity]`, which is only
  the `OPTIONS` manifest's display identity. An empty value, or one containing
  the `OBJECT_VERSION_ID` separator `::`, is refused at boot.

### Removed

- **The bare-root `OPTIONS /` alias of the System API endpoint** (#420). The
  System API defines exactly one location for the Options-and-Conformance
  operation — the API base-path root (`OPTIONS {base_path}`, e.g.
  `/ehrbase/rest/openehr/v1`); the extra bare-root mount was our own
  duplication and answered identically. Clients probing `OPTIONS /` must use
  the base path.

## [3.11.0] - 2026-07-26

### Added

- **Admin console: EHR_STATUS editing and a status version history** (#306). The
  EHR detail screen's **Status** tab is no longer read-only: an **Edit status**
  card toggles `is_queryable` and `is_modifiable` and edits `other_details`
  (canonical-JSON `ITEM_STRUCTURE`; blank removes it), committing a new
  `EHR_STATUS` version conditionally on the version the screen loaded. Every
  other attribute — the subject included — is sent back exactly as the CDR served
  it, so an edit can never drop what the form does not show; a non-object
  `other_details` is refused before anything is sent, and a rejected document
  keeps the CDR's own diagnostic on screen beside the form. If another client
  committed a new status meanwhile, the write is refused rather than overwriting
  it, and the console says so with what to do next. A new **Status history** tab
  adds the versioned view: the `VERSIONED_EHR_STATUS` container plus the selected
  version's envelope facts, the revision history newest-first, a date-and-time
  lookup that resolves the version extant at that instant, and any version's
  document opened by its own `OBJECT_VERSION_ID`. A non-queryable EHR's warning
  now points at the toggle that fixes it.
- **Admin console: SMART scope previewer + effective identity** (#299). The user
  menu's "View scopes" drawer no longer prints a raw list of scope strings. It
  now states **who you are and what decides what you may do** — the
  authenticated principal and the policy source behind it (a Basic session
  replays its CDR account and carries no SMART scopes; an OIDC session's roles
  and permissions come from the same access token whose scopes are listed) — and
  renders every scope as its **parsed grant**: the compartment it delegates to
  (`patient`/`user`/`system`), the resource family and id pattern it reaches, the
  create/read/update/delete/search operations it permits, and a *broad access*
  marker on a bare `*`. Launch contexts and identity claims are labelled as such,
  and an unrecognised scope stays visible verbatim instead of vanishing. A new
  **previewer** field takes any scope string — or a whole space-separated claim —
  and renders the same reading, with an actionable explanation when a
  resource-shaped scope is malformed (a bad compartment, a missing or invalid
  `.<permission>` tail, an unknown resource). The drawer also states plainly that
  scopes **narrow** access and never grant it: the CDR remains the enforcer. The
  reading comes from the same scope grammar the CDR's own SMART gate enforces
  with, so the console's explanation cannot drift from the server's behaviour.

- **Admin console: grouped multi-series result charts** (#296). The results pane
  (both the point-and-click builder and the raw AQL editor) now charts **every**
  numeric result column instead of only the first one: one line per column, named
  by the column's own alias, with a legend whose entries switch a series on and
  off — the last visible series stays on, so the chart never empties itself. When
  a column holds ISO-8601 date/times it is offered as the **X axis** and used by
  default, giving a real time scale in which the points sit at their true
  distance apart whatever order the rows arrived in; the row order remains
  available as the fallback axis. A single numeric column still draws as one
  plain line with no legend. The **Table | Chart** toggle is now offered for
  every non-empty result set, and a result set with nothing to chart (no numeric
  column, or a single row) explains that in the chart pane instead of showing a
  blank box.
- **Admin console: EHR-detail and System-panel completions** (#315). The EHR
  detail screen now opens with a **summary header** read from the EHR resource
  itself (id, creating system, creation time, current EHR-status reference), so
  an unknown or mistyped EHR id is reported once at the top of the screen
  instead of once per tab. The **Create EHR** card takes an optional **EHR id**:
  supply a UUID to create that exact EHR (a non-UUID is refused before anything
  is sent, and an id already in use comes back as the CDR's own conflict with
  what to do next), or leave it blank as before. The composition viewer gains
  **Delete composition** — the openEHR *logical* delete of the latest version
  behind a confirmation dialog, which returns to the EHR's composition list on
  success and, if the version moved on meanwhile, says so instead of deleting
  the wrong one — and a **Versioned object** card reading the versioned
  composition and the selected version directly (lifecycle state, preceding
  version, contribution, signature, whether the version still carries content).
  The contributions tab opens with a **contribution activity** timeline of
  writes per day. On **System**, a **conformance manifest** card shows what the
  CDR advertises about itself through the openEHR System API (product, vendor,
  claimed conformance profile, and the API groups it actually mounts), and the
  served-OpenAPI card gains a **per-family document selector** whose choice
  lives in the URL (`/system?openapi=query`), so a family document is
  shareable and survives a reload.

- **Admin console: run a stored query with its parameters, at the version form
  you choose** (#295). A stored-query row now offers **Run**, which opens a
  runner screen for that query: it shows the stored AQL, prompts one field per
  `$parameter` the query declares, and executes it on the CDR as a real stored
  query (`POST /query/{name}[/{version}]` carrying `query_parameters`) rather
  than re-sending the text as an ad-hoc query. The results land in the same
  results pane as everywhere else, with paging — except when the query sets its
  own `LIMIT`/`TOP`, which the screen says instead of fighting. All three openEHR
  version-resolution forms are selectable and labelled with the exact request
  they will send: **latest** (no version), a **version prefix** like `1` or `1.2`
  (the CDR picks the latest match), or an **exact** `1.2.0`. A parameter value
  that reads as JSON is sent as that type (`38.5` as a number, `true` as a
  boolean); anything else is sent as text, and quoting forces text (`"0123"`).
  A field left blank is not sent at all.
- **Admin console: open a stored query in the query builder** (#295). Stored
  queries and the raw editor now offer **Open in builder** beside *Open in
  editor*: a query that fits the point-and-click builder's model is loaded back
  into it — template, conditions, output shape, ordering and limit — with the
  next version proposed for saving, so a stored query can be revised visually
  instead of by editing text. The load is never lossy: the builder only accepts a
  query it can reproduce **byte for byte**, and anything else (a parameterised
  query, a hand-written shape the builder has no controls for) opens with a
  notice naming exactly what it could not express and a link to work on it in the
  raw AQL editor.
- **Admin console: the stored-query and template tables are paged** (#298). Both
  listings now carry the console's shared pagination footer — which rows are on
  screen out of how many (`26–50 of 137 templates`), previous/next, and a
  rows-per-page choice (25/50/100). The page and the window size live in the
  address bar (`?page=`/`?size=`), so a page is shareable and survives a reload,
  the browser's back/forward walk the pages, and the controls work before the
  console's WebAssembly bundle has loaded. The templates filter still narrows the
  rows client-side; the footer counts what the filter left. Deleting the last row
  of the last page lands on rows rather than on a blank table, and a hand-typed
  window size is clamped to a sane range.
- **Admin console: a real document viewer** (#297). Every pane that shows a
  wire document — the composition viewer, the EHR status tab, the directory raw
  mode, a contribution, a template's OPT and example tabs, a stored query — now
  offers three views of it plus a **Copy** button. **Highlighted** (the default)
  shows the byte-exact document with JSON/XML syntax highlighting from a
  pure-Rust tokenizer (no JavaScript, no new dependency; a very large document
  is shown unstyled instead of tokenized), **Raw** shows the same text
  unstyled, and **Rendered** shows a template-free clinical reading of a
  canonical openEHR JSON document: RM section headings with their type and
  archetype node id, and one label/value row per `ELEMENT` — quantities with
  their units, coded text with its terminology code, a null-flavoured leaf
  saying so. The rendered view needs no operational template, so a composition
  whose template was since removed still reads normally; it folds away the
  bookkeeping (language, territory, category, uid) that the raw views keep in
  full, and is read-only — nothing is stored anywhere.

- **Admin console: stored-query versions are reachable** (#336). Both save
  surfaces (the point-and-click builder and the raw AQL editor) now carry an
  optional **Version** field beside the namespace and name, and state under the
  fields exactly which store a click will perform: leaving it empty stores at the
  server-assigned version and replaces what is there, while a
  `major.minor.patch` version stores a new **immutable** version and is refused
  with the CDR's own message if that pair already exists. **Open in editor** now
  keeps the version it loaded and proposes the next minor one, so editing a
  stored query publishes a new version instead of colliding with the one it came
  from, and a partial pattern (`1`, `1.0`) is refused in the save field with an
  explanation — that form selects the latest matching version when *reading* a
  query, and is not something to file a definition under.

- **Admin console: an Operations panel** (`/operations`) over the CDR's
  operational surfaces — dependency health, build and specification provenance,
  the metric registry, and runtime log control. The health card reads the public
  readiness probe (`GET /health/readiness`) and renders the aggregate plus one
  row per indicator, explaining on screen how that differs from the topbar
  status pill; the build card reports the CDR's version, git commit, `rustc`,
  PostgreSQL target and openEHR specification pins; the metrics card shows four
  headline tiles plus a browser over the whole registry, with the selected
  metric in the URL so a view is shareable; and the log card changes the live
  log filter (and resets it to the boot value) behind a confirmation dialog that
  names the consequence, re-reading the CDR's answer so the panel shows what the
  server confirmed. The screen appears in the sidebar **only when the CDR serves
  its management surface** (the console probes `GET /management/info`); a
  deployment with it switched off sees no Operations entry at all, and an
  individual endpoint left off renders as a stated absence rather than an error.
  The redacted effective configuration is deliberately not duplicated here — the
  CDR serves the same snapshot on both its management surface and its admin API,
  so the panel links to the one viewer on the System screen.
- **Admin console setting `cdr.management_base_url`**
  (`EHRBASE_ADMIN__CDR__MANAGEMENT_BASE_URL`): the CDR's management surface
  including its base path, for deployments that serve it on a separate internal
  listener (`management.port`) or under a renamed `management.base_path`.
  Unset, the console derives `{cdr.base_url}/management`.
- **The compose quickstart enables the CDR's management surface**
  (`docker/ehrbase.dev.toml`): `info`/`metrics` at `private`, `prometheus`
  `public`, `env`/`loggers` `admin_only` — so the console's Operations panel
  works out of the box on the dev stack. The surface remains off by default on
  the bare binary and in the Helm chart.

- **Admin console: delete templates, stored queries and EHRs** when the CDR's
  admin API is enabled. The Template Manager list rows and the template detail
  screen can delete an operational template; the stored-query rows can delete a
  query version from the CDR store (labelled "Delete from CDR", clearly
  separate from the console-local "Remove group"); and the EHR detail screen can
  physically delete an EHR, returning to the EHR list on success. Every action
  confirms in a modal dialog naming the exact object (the query-group removal
  now does too), and every failure names the object and the next action — a
  template still referenced by a committed version, or a session without the
  ADMIN role, is refused by the CDR and reported as such. The console first
  asks the CDR which API groups it serves (the openEHR System API conformance
  manifest, `OPTIONS` on the API base path) and renders **no** delete
  affordance at all when the admin group is not among them.
- **Public health probes (`/health/liveness`, `/health/readiness`)**: the
  server now always serves a complete health family on its main HTTP port,
  unauthenticated and independent of every configuration switch —
  `GET /health` (unchanged: constant `200 OK`, plain-text `OK`),
  `GET /health/liveness` (an identical alias under the
  orchestrator-conventional path), and `GET /health/readiness` (the
  indicator-backed probe: database ping, migrations applied and the in-memory
  component flags, `200` when the aggregate is UP/DEGRADED, `503` when a
  required component is DOWN, with the full per-indicator JSON body). They are
  mounted outside the API's authentication and overload-shedding layers, so
  they answer without credentials and are never shed on a saturated server.
  This family is now the only health surface (see **Removed**).

### Changed

- **`ETag`/`Last-Modified` on every versioned read** (#368). The
  VERSIONED_COMPOSITION and VERSIONED_EHR_STATUS container reads, the
  VERSION-by-id reads, and both revision-history reads now carry the
  versioning headers (container/version uid as the `ETag`; the commit
  instant as `Last-Modified` where the body carries one) — previously only
  the at-time variants did.
- **Unqualified stored-query names are one identity everywhere** (#366). A
  query stored without a namespace (`PUT /definition/query/my_bp/1.0.0`) now
  lands under the openEHR-assumed `misc` namespace — the same identity the
  by-name GET, the listing, the SM calls, and the admin delete address — so a
  bare-named query is no longer invisible to the admin delete (and vice
  versa). Descriptors return the canonical `misc::`-qualified name; a
  bare-name listing pattern also matches its `misc::` composition.
- **Query GETs bind the spec's named parameters** (#364). AQL `$parameter`
  binds on `GET /query/aql` and `GET /query/{name}[/{version}]` now arrive as
  ordinary named query-string parameters (`?temperature_from=36&…`), exactly
  as the REST API documents them — values are typed JSON-first with string
  fallback, a `$` prefix is tolerated, and the previous JSON-object
  `query_parameters=` form remains accepted (a named parameter wins a
  collision).

- **Version identity is the full three-part `version_uid`, compared
  case-insensitively** (#367). Deleting a composition (and reading a version
  by id) with a fabricated `creating_system_id` is now refused (409 / 404) —
  previously only the version number was compared, so a made-up system id
  could delete the latest version. Conversely, a `version_uid` or `If-Match`
  differing only in case is accepted as the same identifier, per the openEHR
  composite-identifier case rule.
- **Item tags follow the spec's identity and target model** (#365). Two tags
  sharing a key on different `target_path`s now coexist (the ITEM_TAG identity
  is the key + target_path pair, per the ITS-REST item-tag prose) instead of
  silently collapsing; a version-addressed tag (`…/composition/{version_uid}/tags`)
  now tags THAT VERSION, disjoint from the container's tags, instead of being
  folded onto the container; the tag's `target` is returned in the RM shape (a
  bare `HIER_OBJECT_ID` or `OBJECT_VERSION_ID`, replacing the former OBJECT_REF
  wrapper — the released RM wins over the stalled OAS schema); tag routes now
  404 when the addressed object is of the other kind; and the
  `openehr-item-tag` / `openehr-version-item-tag` commit headers write to their
  own distinct collections. Deleting by key removes every path under that key
  in the addressed collection (the wire has no path selector).

- **Admin console: accessibility and empty-state polish** (#302). Table header
  cells are now announced as column headers by screen readers, and every
  icon-only control in the query builder (the catalog's expand/collapse
  chevrons, the remove buttons on conditions, groups, columns and sort rules)
  plus the unlabelled column-alias, sort-path and sort-direction controls state
  what they do. Data regions that used to come back as a line of grey text —
  template usage, the served OpenAPI list, the commit-activity chart, an EHR's
  compositions, the directory version history, a version's audit, the query
  builder's conditions and result rows, and a template filter that matches
  nothing — now render the console's standard empty state: an icon, what is
  empty, and what to do about it. The user menu's popover matches the rest of
  the console's panels instead of the widget kit's stock chrome, and the modal
  backdrop is a theme token, so it dims correctly in dark mode.
- **Contribution list shows the change type's display rubric** (#304). The
  EHR contribution-list extension (`GET /ehr/{ehr_id}/contribution`) now
  carries `change_type_rubric` beside the raw `change_type` group code —
  resolved from the openEHR `audit_change_type` terminology group by the CDR
  itself, so clients never maintain a local code table. The admin console's
  contributions tab displays the rubric (code on hover). The SM-catalog
  `delete_opt` service path now also refuses with the same friendly
  409-and-reference-count as the admin template delete while committed
  versions still reference the template, instead of relying on the raw
  foreign-key error.
- **openEHR BASE spec pin refreshed** (#341). The vendored BASE 1.3.0 spec
  text and BMM codegen input now track upstream `specifications-BASE` master
  `e4879576` (24 commits: the SPECBASE-48 RESOURCE_DESCRIPTION invariants,
  the SPECAM-82 CODE_PHRASE package move into base_types, SPECPR-426/386/460
  corrections). No wire or validation behaviour changes: every
  behaviour-relevant item was verified already satisfied by the
  implementation; the regenerated crates differ only in documentation text
  and the CODE_PHRASE module location.
- **Readiness moved from `/management/health/readiness` to the public
  `/health/readiness`** (and liveness to `/health/liveness`). Database-backed
  readiness no longer hides behind `management.enabled` + a probe switch —
  nothing has to be enabled for an orchestrator to probe the server. Point
  existing probes at the new paths.
- **Helm probes use the public paths on the main HTTP port**: the chart's
  `httpGet` liveness/startup probes hit `/health/liveness` and readiness hits
  `/health/readiness` on the `http` port, with no prerequisite — the previous
  render-time failure demanding `config.management.enabled=true` +
  `config.management.probes_enabled=true` is gone. Prometheus scrape
  annotations still point at the management surface (and its separate port when
  configured), unchanged.
- **Admin console: the System screen's activity tile now links to the audit
  browser** (#301) instead of stating that the CDR exposes no audit read
  surface — it does (the IHE ITI-81 retrieval the `/audit` screen has been
  browsing all along). The tile carries a one-line description of the trail and
  an **Open audit browser** button.
- **Admin console: every write now reports its failure as prominently as its
  success** (#301). Uploading a template, creating an EHR, committing or
  updating a composition, saving a stored query or query group, and creating,
  saving, restoring or deleting a directory all raise a failure notification
  naming the object, what the CDR objected to (its diagnostic verbatim), and
  the next action to take — a stale version to reload, a role to sign in with,
  an unreachable CDR to check. Where the diagnostic is worth reading line by
  line (template validation, a rejected composition body) it also stays on
  screen beside the form as before. Previously a failed write showed a quiet
  inline message that was easy to miss after a run of green success toasts.
- **Admin console: query groups are now derived from the stored-query
  namespace** instead of being named sets kept in a console-local file. A
  stored query is identified by a qualified name — `namespace::name`, the
  namespace optional and, per the openEHR REST specification, a reverse domain
  name whose purpose is separation of stored queries by team or organisation —
  so the console groups by exactly that: the **Queries** screen's right-hand
  panel and the **Dashboard**'s cohort tiles are both derived live from
  `GET /definition/query`, and queries saved without a namespace collect under
  *unqualified*. The grouping consequently lives in the CDR: it is visible to
  every openEHR client, survives a console restart, needs no backup, and is
  identical across console replicas. The group create/edit/remove controls are
  gone — a query joins a group by being saved under that namespace — and both
  save surfaces (the query builder and the raw AQL editor) now offer a
  first-class **Namespace** field beside the query name, showing the exact
  qualified name the save will write. Existing local groups are not migrated:
  re-save a query under the namespace you want it grouped by.
- **Admin console: the EHR Directory tab creates an empty root folder**, then
  the structured tree editor builds the hierarchy. The console no longer ships
  or stores named folder shapes to start from.

### Removed

- **`GET /ehrbase/rest/status/health`** is removed. It was a third name for the
  constant liveness answer already served at `/health` and `/health/liveness`,
  with no consumer anywhere in the product (no probe, no client, no
  documentation pointed at it). Point any caller at `/health` (load balancers,
  container `HEALTHCHECK`) or `/health/liveness` (orchestrator probes);
  `GET /ehrbase/rest/status` — the product status document, a different contract
  — is unchanged.
- **`/management/health`, `/management/health/liveness`, and
  `/management/health/readiness`** are removed; the management surface is now
  ops introspection only (info, prometheus, metrics, env, loggers). The
  aggregate component view is the body of the public `/health/readiness`.
- **The `management.probes_enabled` and `management.endpoints.health`
  configuration keys** are removed. Configuration is strict, so a config file
  or `EHRBASE__MANAGEMENT__PROBES_ENABLED` / `…__ENDPOINTS__HEALTH` environment
  variable still setting them **fails at boot** with an unknown-key error —
  delete the keys; the probes are always on.
- **Admin console: folder templates are removed.** The named FOLDER-tree shapes
  the Directory tab could start from (and their `admin-ui-folder-templates.json`
  store) are gone; create the empty root and build the hierarchy in the tree
  editor, which commits it as ordinary directory versions the CDR owns.
- **Admin console: both console-local JSON stores are removed** —
  `admin-ui-groups.json` (query groups) and `admin-ui-folder-templates.json`
  (folder templates). The console now keeps **no local domain state at all**:
  it has no database and writes no files, so every fact it shows lives in the
  CDR and reads the same for every client and every replica. Delete the files;
  nothing reads them.
- **The admin console's `groups_file` configuration key**
  (`EHRBASE_ADMIN__GROUPS_FILE`) is removed. Console configuration is strict,
  so a config file or environment variable still setting it **fails at boot**
  with an unknown-key error — delete it.

### Fixed

- **The observability Compose overlay boots again** (#321). Every server
  variable in `docker-compose.observability.yml` was written in a
  single-underscore form (`EHRBASE_MANAGEMENT_*`, `EHRBASE_OTEL_*`,
  `EHRBASE_LOG_FORMAT`) that the strict boot-time sweep of the reserved
  `EHRBASE_` namespace rejects, so `docker compose -f docker-compose.yml -f
  docker-compose.observability.yml up` failed at startup with unknown-variable
  errors instead of starting the server. The overlay now uses the documented
  `EHRBASE__…` grammar (`EHRBASE__TELEMETRY__OTLP_ENDPOINT`,
  `EHRBASE__LOG__FORMAT`, `EHRBASE__MANAGEMENT__ENABLED`,
  `EHRBASE__MANAGEMENT__PORT`, `EHRBASE__MANAGEMENT__ENDPOINTS__{INFO,METRICS,PROMETHEUS}`),
  with unchanged intent: OTLP traces to the bundled collector, JSON logs, and
  the management surface public on internal port 9464 for the bundled Prometheus
  to scrape. A test now runs the real sweep over every variable the shipped
  Compose files set on the server service, so this class of drift fails in the
  test suite rather than at `docker compose up`.
- **The `compositions_committed_total` metric now counts** (#332). The counter
  was declared and scraped but never incremented, so dashboards over it
  rendered a permanently empty series. Every commit route that lands a
  COMPOSITION version — create, update, delete, and a CONTRIBUTION commit —
  now increments it once per committed version, labelled `change_type` with the
  openEHR `audit_change_type` code recorded on that version's audit
  (`249`/`251`/`523`/…). The increment happens after the transaction commits, so
  a rolled-back write is never counted. In the same audit of the metric
  registry, five metrics that were emitted but not registered
  (`version_signature_invalid_total`, `authz_cedar_decisions_total`,
  `authz_remote_pdp_calls_total`, `atna_audit_rejected_total`,
  `atna_audit_reaped_total`) now carry their `# HELP`/`# TYPE` descriptions in
  the `/management/prometheus` exposition.
- **Admin console: find-an-EHR-by-id works without JavaScript** (#301). The
  finder is now a plain `GET` form: submitting it before (or without) the
  browser app loading redirects to the EHR's detail screen server-side, and
  `/ehrs?find=<ehr_id>` is a shareable shortcut to any EHR. With the app loaded
  the lookup is unchanged — one client-side navigation, no page reload.
- **Admin console: template links and query-string values are now
  percent-encoded via the standard codec** (#293); template ids containing
  reserved characters no longer produce broken links. The console's
  hand-rolled percent encoder is gone — every internal link (the template
  detail link and its tab links, the stored-query "Open in editor" link, and
  the query builder's "Open in raw editor" link) builds its path segment and
  query-string values with the `urlencoding` crate.
- **Admin console: deleting a document reference from a directory folder no
  longer risks row state attaching to the wrong sibling** (#292). The item
  rows of the directory tree editor are now identified by a stable per-item
  identity instead of their position in the folder, so removing one reference
  leaves every remaining row bound to its own reference.

## [3.10.0] - 2026-07-25

### Added

- **CNF total wire-surface coverage gate (#271)**: a new `surface-coverage`
  machine gate in the CNF runner (`cnf-runner validate`) fails on any
  spec-defined wire behaviour with no covering case and no adjudicated
  exception — enforcing breadth, not just pass rate (`.claude/rules/testing.md`
  §CNF coverage). It measures three axes against the RELEASED spec sources only
  (the SM platform interfaces + the ITS-REST docs text, never the vendored
  OAS): (1) every SM operation of the platform interfaces has an `its-rest`
  binding or a cited boundary; (2) every realized binding's declared outcome
  and format branch is exercised by a case or excepted; (3) the cross-cutting
  wire behaviours (conditional headers `ETag`/`Location`/`Last-Modified`/
  `Prefer`/`If-Match`, JSON+XML negotiation, the 406/415 families, the error-body
  and deprecated-media families) map to covering cases or exceptions. The
  authored, spec-cited exception ledger is a new committed artifact
  (`tools/cnf-runner/artifacts/vocab/wire_surface.yaml`, with a published JSON
  Schema); `cnf-runner validate --specs …` refreshes a deterministic coverage
  report at `docs/conformance/coverage-report.md`. Coverage only ratchets up.
- **CNF catalogue content deepening (coded-text value dimension, deferred
  grounds, spec-authored corpus) (#278)**: the coded-text content cases
  (`CONT-DV_CODED_TEXT-validate_local_codes` / `-validate_ext_term`) gain an
  acceptance-direction `value` dimension — value = the bound rubric, value ≠
  rubric (an arbitrary label), and value = the raw code are all **accepted**
  (no RM invariant requires `value` to equal the coded rubric — the "must be
  the rubric" text is `dv_coded_text.adoc` Description prose, registered as
  AMB-55), while an **empty** value is rejected (the sole value invariant, RM
  `dv_text.adoc` §Invariants `Valid_value: not value.is_empty`); the
  synthesized OPTs now bind component-ontology rubrics for their local
  constraint codes. New functional coverage: a **template-example round-trip**
  (the generated example commits back cleanly) and a **deprecated-media Accept
  → 406** response-side case (the ICS-conditional companion to the existing
  request-side 415, under AMB-39). Registered as spec-silent boundaries:
  **Accept q-value negotiation strictness** (AMB-56 — ITS-REST defines only
  "unfulfillable Accept → 406", nothing about q-value weighting) and a
  **simplified-inner-data CONTRIBUTION surface** (AMB-57 — ITS-REST 1.1.0
  commits CONTRIBUTIONs canonical-only). All remaining corpus-manifest
  "structural placeholder" markers are replaced with spec-authored fixtures or
  cited boundaries.

- **Version-signature conformance breadth (CNF)**: a distinct-signature-per-
  version case (the signature is computed over the version's canonical form,
  which includes `uid` — two versions can never share a signature; RM common
  master06 §Digital Signature), backed by a new `distinct_from` fact on the
  runner's signature assertion and a `signature` capture on the
  version-envelope read binding. DIRECTORY (FOLDER) version-signature cases
  land SM-anchored as N/A-with-citation on ITS-REST 1.1.0 (no
  `versioned_directory` resource — AMB-24), activating automatically if a
  later ITS release adds the endpoint. The runner's binding-completeness gate
  now mirrors the interpreter's variant-based binding selection.

- **ADL2/OPT2 templates are full FLAT/STRUCTURED peers of OPT 1.4 (#269)**: a
  FLAT (`application/openehr.wt.flat+json`) or STRUCTURED
  (`application/openehr.wt.structured+json`) composition **commit** keyed to an
  ADL2-registered template now resolves and is validated against that template's
  archetype constraints, exactly as an ADL 1.4 commit is. Two behaviours were
  brought to parity: the am24 (OPT2) Web-Template builder now populates the
  archetype-conformance constraints (existence, cardinality, closed-attribute
  sibling sets, archetype slots, structural stubs) that composition validation
  reads — so an ADL2-template instance is archetype-constraint-checked, not only
  RM- and terminology-checked — and the runtime template resolver falls back to
  the ADL2/OPT2 store when a template id is not an ADL 1.4 template (previously a
  commit against an ADL2-registered template returned **422 "operational template
  not known"**).
- **Citation metadata (`CITATION.cff`)**: the repository is now citable in
  research papers — GitHub renders a "Cite this repository" button (APA +
  BibTeX) from the new CFF 1.2.0 file (author with ORCID, Apache-2.0,
  abstract, keywords, release version/date). A `citation-guard` CI job
  schema-validates the file and enforces that its `version` matches the
  workspace version; the release procedure bumps `version`/`date-released`
  on every cut.

### Changed

- **AQL engine: post-streaming optimization rungs** (measured, one change per
  rung): the streaming shape's dead root LATERAL is elided when the root is
  unreferenced (one fewer `pk_node` probe per version row; a bare
  `uid/value` projection now runs with zero node probes), and the
  `archetype` predicate column is case-folded at write (BASE base_types
  master05 §Composite Identifiers and Case) so archetype equality is plain
  indexed equality — `LOWER()` disappears from every containment hop.
  Measured on the seeded 10k bench: ward statement execution −11.3%,
  buffer reads −10.7%, planning −10.3%; stress knee re-measured at the
  committed 512 arrivals/s. The aql-probe instrument now attributes
  planner time per statement (`pg_stat_statements.track_planning`).


- **Docker / Compose deployment rework (#282)**: a from-the-ground-up rebuild
  of the container surface for smaller images, faster builds, and a
  production-grade posture on every build.
  - **One Dockerfile per image, two targets, zero drift**: `docker/Dockerfile`
    and `docker/admin-ui/Dockerfile` now each expose `runtime-from-source`
    (what `docker compose build` uses) and `runtime-prebuilt` (what CI uses),
    both sharing a single runtime stage — so the compose-built and published
    images can no longer diverge. The separate `*.runtime` Dockerfiles are
    removed.
  - **Faster rebuilds**: dependency compilation is split into its own
    `cargo-chef` layer, so editing application code no longer recompiles
    dependencies, and CI now reuses that layer across runs via an exported
    build cache.
  - **Debian 13 + digest pinning**: builder and runtime moved to Debian 13
    ("trixie"); the runtime is `distroless/cc-debian13` (non-root user 65532).
    Every base image is now pinned by immutable digest, and the bundled
    versions are refreshed — PostgreSQL 18.4, Keycloak 26.7.0, SeaweedFS 4.40,
    Grafana otel-lgtm 0.29.2.
  - **Compose**: the optional services are now opt-in behind profiles
    (`--profile s3` for SeaweedFS, `--profile keycloak` for Keycloak); every
    service declares memory/CPU limits mirroring the Helm chart; Keycloak has a
    real healthcheck; and there is no hard-coded project name, so the dev,
    conformance, and E2E stacks no longer collide.
  - **Build provenance** no longer reads `.git` from the build context (which
    is now excluded, shrinking the context and stabilising the cache): the
    `/management/info` commit SHA flows through the standard `REVISION` build
    argument (the same value as the `org.opencontainers.image.revision` label)
    and degrades to `unknown` when unset — never a failed build.
- **Simplified Formats folded into `openehr-its` (#268)**: the FLAT /
  STRUCTURED / Web-Template implementation moved from the standalone
  `openehr-flat` crate into `openehr-its` as the `openehr_its::flat` module,
  mirroring the openEHR ITS component decomposition (Simplified Formats is a
  STABLE ITS-REST 1.1.0 sub-specification, alongside canonical JSON, XML, and
  the REST contract this crate already houses). Pure packaging refactor — no
  change to the FLAT/STRUCTURED/Web-Template wire behaviour.

### Fixed

- **ADL2 filler-root naming in the projected WebTemplate**: a
  `use_archetype`-filled archetype root resolved its display rubric in the
  component (filled archetype) terminology first, so the template-side slot id
  could false-positively match an unrelated internal id of the constituent
  (e.g. a filled OBSERVATION surfacing as "history"). The slot rubric now
  resolves in the introducing template's own terminology first (ADL2 obliges
  the introducing artefact to define its node ids), with the component scope
  as last resort — FLAT paths over filled ADL2 templates carry the
  template-declared names.
- **CNF runner: `openehr-template-id` for in-flow-provisioned templates**: the
  simplified-format commit header now resolves from the committed data set's
  own manifest-declared `template_id` (falling back to the case's provisioned
  template list), so cases that upload their template inside the flow (the
  ADL2 FLAT pair) drive the commit correctly instead of omitting the header.

## [3.9.0] - 2026-07-24

### Added

- **Content structural conformance cases from the official schedule**: the
  master15 COMPOSITION content×context tables and the master16 ENTRY-family
  tables (OBSERVATION, HISTORY, EVENT, ITEM_STRUCTURE) are now encoded under
  their verbatim official ids, replacing the ad-hoc structural cases that
  had been authored on the false claim that those chapters were empty;
  derivable catalogue extensions beyond the official cells survive as
  flagged addition cases.

- **Dual POC measured records on the v3.8.0 build, both directions
  published**: ehrbase-rs earns class POC (normative hour at 2.03/s
  offered, worst p99 108 ms, 0 errors / 7,320 requests); upstream
  EHRbase 2.34.0 on the identical instrument, corpus, and resource floor
  does not (ward-dashboard AQL p99 10.9 s vs the 1 s ceiling, 2.4%
  errors). Comparison page and all measurement visuals derive from the
  committed runner artifacts.

### Changed

- **Version-signature read verification is now `strict` by default (#273)**:
  with signing enabled and `signing.verify_on_read` unset, the server now
  recomputes the signature of every version it served and returns a `500`
  integrity fault on a mismatch, instead of the previous silent-pass (`off`)
  default that signed every version and then never checked it. Set
  `signing.verify_on_read` explicitly to `warn` (log + meter, still serve) or
  `off` (never check) to opt out. **Client-supplied signatures** (an author's
  own signature, or one carried by an imported version) are tracked as such and
  are always stored verbatim and never re-verified, so strict-by-default never
  rejects a legitimately-stored foreign signature. Our-own-design integrity
  hardening — no openEHR spec governs server-side verify-on-read timing (RM
  common master06 §Digital Signature).

- **CNF catalogue audited case-by-case against the official spec text
  (#231)**: every case in every chapter re-verified across grounds,
  expectations, citations, fixtures, captures, and register linkage, with
  the findings applied directly to the catalogue and register (the durable
  record is the register + closed issues + git history).
  Highlights: spec-overreaching rejection rows removed (AQL TERMINOLOGY
  operation strictness; the mixed-precision interval rows now report-only
  under the SPECPR-380 openness); the SEC-BASIC proposal citations corrected;
  stale stub-era template ids fixed; the delete-latest-version OPT case
  realigned to the official version-less ground; the wrong-template update
  ground rebased onto a fixture that is valid against its own template; the
  physical-EHR-delete binding accepts the OAS-enumerated async 202; eight
  new ambiguity-register entries pin previously prose-only adjudications;
  and every phantom REQUIREMENTS.md pointer now carries its real anchor.

### Fixed

- **Conformance-runner commit provisioning fails loud**: a `requires.commit`
  key resolving to a plain composition fixture was silently skipped, leaving
  the case's committed-state precondition unestablished; a single object now
  commits as a one-item set and any other shape is a provisioning error.

- **The measured-window driver accepts the spec-legal `204 No Content`
  minimal-return form** on create-family writes (ITS-REST: with
  `Prefer: return=minimal` a service SHOULD use 204 when no body is
  returned) — previously every upstream journey commit was falsely
  counted an error; and the upstream comparison stack's database now
  gets the same `shm_size` floor as the ehrbase-rs stack (Docker's 64 MB
  default starved its PostgreSQL during maintenance settling).

## [3.8.0] - 2026-07-24

### Added

- **CNF catalogue: stored-query name-grammar cases** — three new
  `definition_query` cases pin the ITS-REST `Qualified_query_name` grammar:
  a plain unqualified name and a namespace-less dotted name (the dot is part
  of the query-name character set, not a namespace separator) both store and
  read back, and the reserved query-name `aql` is rejected case-insensitively.
- **`cnf-runner stress-compare`** — the cross-SUT stress overlay: both
  systems' latency-throughput curves on one canvas, rendered
  deterministically from the two committed `stress.json` reports (driven
  by `scripts/render/comparison.sh`); both directions on equal footing.
- **Measured runs record resource telemetry**: each measurement in
  `results.json` now carries an optional, schema-published `resources`
  block — per-container (server and database separately) CPU, resident
  memory, block-device and network I/O sampled every 10 s across the
  whole window (run-clock offsets, warmup/measured/drain phase stamps),
  plus the database volume's on-disk size at four anchors (empty → scale
  seed → ward seed → after the window) with the derived bytes per
  committed composition. Sampling is enabled by the new optional
  `containers` block in the ixit (compose container names); without it a
  run records no resources and the report says so — telemetry never
  influences a class verdict. Two new rendered assets (the resource
  time-series and the disk-growth chart) join the perf-assets family and
  the book's Performance chapter, drift-guarded in CI like every
  published number.
- **`cnf-runner aql-probe`** — the seeded-corpus AQL optimization probe:
  fires the measurement machinery's own AQL vocabulary against a freshly
  seeded server, records wire-latency percentiles per probe, and
  attributes the database-side cost per statement (`pg_stat_statements`
  through the ixit `containers` capability, degrading honestly without
  it). Report schema published (`aql-probe.schema.json`); exploration
  evidence only — never a conformance record.
- **Stress steps carry resource telemetry** — every load-ladder rung
  records the same per-container CPU/memory/I/O series as the measured
  class runs over its own warmup+hold window, so a breached rung shows
  where it saturated; the stress progress stream now logs each rung's
  verdict live (stable/BREACHED with the sustained rate, resource peaks,
  and named breaches) plus a ladder recap, and measured class runs log
  their verdict evidence at window end.
- A **diurnal day-curve** arrival option for the extended 8/12-hour
  measured holds (ITU-T E.500 busy-hour semantics: the class floor is the
  busy-hour rate).
- The conformance certificate gains a **Workload Coverage** section:
  claimed capabilities vs the set the measured hospital simulation
  actually exercised, with untouched claimed capabilities listed
  explicitly as journey-catalogue gaps.
- `scripts/generate-ckm-examples.sh` — regenerates the committed CKM
  example payload skeletons from a running SUT's example endpoint;
  `scripts/vendor/ckm-templates.sh` now vendors the runner's journey
  template pack.
- **Conformance visuals**: the capability-matrix heat grid (one cell per
  claimed capability, grouped by profile tier, evidence encoded as a
  CVD-safe color AND a glyph) and per-chapter outcome bars, rendered
  deterministically from the committed verdicts/results by the new
  `cnf-runner conformance-assets` subcommand
  (`scripts/render/conformance-assets.sh`, CI regenerate-and-diff
  guarded) and embedded on the book's conformance and comparison pages
  (both SUTs) and the landing page.

### Changed

- **`--skip-seed` and the sidecar corpus index are retired** (CLI flags on
  `perf`/`stress`, the `CONF_PERF_SKIP_SEED` pipeline variable): every
  measurement instrument now always seeds a freshly composed, empty
  server and the stack is torn down afterwards — seed reuse bred
  stale-state confusion.
- **Measurement instruments settle database maintenance
  deterministically** (`vacuumdb --analyze` through the DB container)
  after seeding and before every measured window and stress rung —
  a stale-statistics plan after the million-row seed cost a measured ~9×
  on the ward-worklist query; settling moves that debt outside every
  measured window, identically for every SUT.
- The CNF measured-performance workload is now a full **hospital
  simulation**: the class cases (`PERF-hospital_sim-*`, renamed from
  `PERF-mixed_load-*`) schedule clinical journeys — ADT
  admission/discharge, vitals rounds, the medication loop, medicines
  reconciliation, asynchronous laboratory/imaging order-to-result
  pipelines, specialist/registry reporting, public-health notifications,
  chart review, ward dashboards with a registered stored query, versioned
  corrections, contribution audit review, workflow tagging, logical
  deletion, and template polling — expanding into 22 measured operation
  kinds instead of 4, each with its own HDR-V2 record. The
  population-anchored envelope is unchanged and now validator-enforced
  (the expanded write share must reconcile to the derivation's 10:1..50:1
  read:write band); journey payloads commit against 15 COMPOSITION-rooted
  openEHR CKM templates vendored with provenance.

### Removed

- **The transitional benchmark lab** (`tools/benchmark`,
  `scripts/benchmark.sh`, `docker/benchmark/`, the manual benchmark
  workflow, and the committed `docs/benchmarks/**` artifacts): all
  measurement is native to the CNF runner — measured class runs, the
  stress ladder, the AQL probe, and the cross-SUT stress overlay — and the
  comparison page now derives its performance side from the committed
  `docs/conformance/<sut>/stress.json` reports (upstream shown as "not
  measured yet" until its report lands, never a one-sided claim).
- The completed ECC→CNF cutover comparison lane: the generated
  `docs/conformance/cnf-comparison.md`, the `cnf-runner compare-ecc`
  subcommand, the drift gate, and the preserved ECC catalogue/map (all in
  git history; the five deferred grounds are re-registered on the
  catalogue-deepening tracker). The `docs/conformance/CATALOG.md` pointer
  stub is gone with it, and the CNF 2.0 design record moved to
  `docs/conformance/cnf-design.md` as a permanent reference document.

### Fixed

- **Storing a query under the reserved name `aql` is now rejected** with
  400, case-insensitively and whether or not a namespace is supplied
  (ITS-REST `Qualified_query_name` §NOTE — the name would collide with the
  ad-hoc `/query/aql` route). A three-part `ns::aql::name` name keeps
  working: its middle segment is the formalism, not the query-name.
- **A coded value whose text is not the template-bound rubric is now
  rejected at commit** (422 naming the path, the committed value, and
  the bound rubric): RM `DV_CODED_TEXT` — "value must be the rubric from
  a controlled terminology" — enforced wherever the template itself is
  authoritative for the rubric (archetype-local at-codes and explicitly
  bound external term definitions, any bound language); `openehr`-
  terminology codes stay unchecked (the terminology ships official
  translations the template cannot enumerate), and a bound code with no
  rubric stays accepted. The once-accepted code-as-value instance is a
  pinned rejection.
- **Coded-text example values now carry the template-bound rubric**: the
  Web Template builder resolved display labels only for local at-codes,
  so an external code's rubric (OPT `term_definitions` keyed
  `TERMINOLOGY::code`, e.g. SNOMED-CT bindings) was lost and generated
  examples emitted the raw code as `DV_CODED_TEXT.value` — spec-invalid
  instance data (RM: "value must be the rubric from a controlled
  terminology"). The qualified key now resolves; the covid19 example
  regenerates with rubrics; every pack example commits clean on strict
  validators.
- **Child-assembled `DV_INTERVAL` values now carry the mandatory boundary
  flags**: an interval built from `lower`/`upper` sub-path children (the
  FLAT builder's container path — template examples included) previously
  omitted `lower_unbounded`/`upper_unbounded`/`lower_included`/
  `upper_included`, making every half-open interval spec-invalid (BASE
  `Interval`: the flags are mandatory and `Limits_consistent` is
  unevaluable against an absent bound); the flags now derive from bound
  presence, an explicit datum flag wins, and the committed CCTA example
  is regenerated. Strict validators (upstream EHRbase) rejected the old
  instances with 422.
- **Population AQL with `LIMIT` now streams instead of materializing the
  corpus**: a LIMIT-bearing, unordered, non-DISTINCT, non-aggregate
  population query lowers to a streaming FROM shape (the current-version
  spine with `LATERAL` node probes), so PostgreSQL stops at the LIMIT
  instead of building an archetype-anchor bitmap over every matching node
  first — measured on a million-composition corpus, the cross-EHR ward
  worklist drops from ~113 ms to ~2 ms per execution (~40× fewer buffer
  reads); ordered/aggregate/EHR-scoped queries keep the previous plan
  shape, and result semantics are unchanged. A version-field projection
  of `uid`/`contribution_id`/`lifecycle_state` no longer joins the audit
  table it never reads.
- **AQL cross-EHR queries with `LIMIT` no longer collapse under corpus
  scale**: predicates on multi-valued (anchored) paths now lower as
  existential semi-joins (`EXISTS` — the predicate holds when ANY matched
  node satisfies it; deterministic where the previous first-match pick was
  plan-dependent), the archetype anchor index leads with the RM type so
  the whole `CONTAINS`-class + archetype boundary is one index probe, and
  queries that never touch audit fields no longer join the audit table.
  The measured ward-dashboard profile (p99 5.8 s at class-POC scale) drops
  to milliseconds-per-request territory.
- The template **example generator no longer collapses `DV_INTERVAL`
  wrappers** onto a single constrained bound: interval-valued elements keep
  their interval identity (bounds as `/lower`/`/upper` sub-paths per the
  Simplified Formats mapping), fixing generated examples the platform's own
  validation rejected (the CKM CCTA report OPT); the CNF journey catalogue
  re-commits the CCTA imaging report.

## [3.7.0] - 2026-07-22

### Added

- The conformance pipeline assesses **upstream EHRbase** as a second
  system under test: `CONF_SUT=ehrbase scripts/conformance.sh` composes
  the official `ehrbase/ehrbase:2.34.0` + `ehrbase-v2-postgres` images on
  fresh volumes (`docker/sut-ehrbase.yml`, readiness probed externally
  — the official image carries no in-container health tooling) and runs the
  same committed catalogue with upstream's own committed party set
  (`tools/cnf-runner/party/ehrbase/`). The public comparison
  (`docs/conformance/COMPARISON.md` + the website comparison page) is fully
  generated from the two committed results/verdicts sets — profile verdicts,
  the 39-capability evidence matrix, and failure tables in both directions.
- The conformance runner performs ISO/IEC 9646-style ICS-driven test
  selection: `cnf-runner run --statement` excuses option-gated cases whose
  register branch the party statement does not declare as N/A with citation
  (previously they ran and recorded spurious failures the verdict pipeline
  then excused).
- Conformance badges carry measured amounts: per-tier badges read e.g.
  `PASS 10/10 capabilities`, the overall badge `CORE+STANDARD PASS ·
  323/323 cases` — derived from `verdicts.json` + the capability matrix,
  never hand-typed.


- Read-only role support in RBAC: a principal carrying the configured
  `authz.rbac.readonly_role` (default `READONLY`) is refused with `403` on
  every write operation — creating an EHR, committing a composition,
  uploading a template, and any update/delete — even when it also holds
  granting roles such as `ADMIN`. Reads and AQL queries stay permitted, so a
  `READONLY` account is an authenticated, view-only principal. The dev compose
  stack ships an `ehrbase-readonly` account (password `ehrbase`) for
  evaluation.
- CNF 2.0 reference runner, third increment — the executor and both verdict
  machineries: the data-driven flow interpreter under the five interpreter
  laws (per-row re-provisioning, step-mismatch row abort, errored-vs-failed
  classification, fixed temporal resolution, aggregates-after-last-row) with
  the live HTTP driver realized purely from the operation bindings, the
  reference resolver (corpus/recipes/rows/captures with normative sentinel
  semantics), the normative RESULT_SET equivalence comparator, content-case
  execution via the synthesized generate→commit→expect flow, the party
  artifacts (statement/results/ixit with schema validation and mandatory
  N/A citations), the pure verdict pipeline + deterministic
  report/statement/certificate renderers, the runner-verification pack
  (committed transcript + player: adjudicated verdicts reproduced, broken
  runners rejected), and the performance machinery (class cases with the
  published population-anchored floors, re-checkable HDR V2 measurement
  records, the earned/not-earned pure verdict). Nine published JSON-Schema
  families, drift-guarded. Live-SUT runs (the earned-class measurement and
  pack part 2) execute against a composed SUT via the new `run`/`verdicts`
  CLI once cutover lands.
- CNF 2.0 reference runner, second increment: the complete CNF 2.0 catalogue
  authored from the framework — 347 cases across every schedule chapter
  (EHR, EHR_STATUS, COMPOSITION, CONTRIBUTION, DIRECTORY, ADL 1.4 + ADL2
  definitions, stored queries, demographic, admin, messaging, AQL, content
  data-type and structural validation, simplified formats, Security
  SEC-BASIC + Signing) with 84 per-operation ITS-REST bindings (every
  status/header mapping cited to its OAS source; wire gaps are typed
  `unrealized` declarations, not silent absences), the ambiguity register
  grown to 38 adjudicated entries, and the ECC↔CNF comparison gate CLEAN:
  all 394 active rows of the old harness's catalogue adjudicated
  (350 covered, 5 deferred to the simplified-formats deepening, 18 dropped
  with justification, 9 out of scope, 12 ADL2 rows covered) in the committed
  map with the generated report at `docs/conformance/cnf-comparison.md`
  (drift-guarded). Old-harness retirement follows the owner's report review
  with the executor/emission workstreams so an acceptance instrument runs
  continuously.

- CNF 2.0 reference runner (`tools/cnf-runner`), first increment: the typed
  schedule-artifact model (case cores, per-ITS operation bindings, outcome +
  selector vocabularies, the capability→family→tier matrix, corpus manifest,
  ambiguity register — every closed vocabulary a Rust enum/newtype), a
  published JSON-Schema set for all seven artifact families (committed under
  `tools/cnf-runner/schemas/`, drift-guarded, vendorable by any runner), a
  full cross-artifact validator (id uniqueness, SM-operation and spec-ref
  resolution against the vendored specs, binding completeness, corpus
  integrity, reference/sentinel and decision-table grammars, capability-tier
  consistency), the `cnf-runner` CLI (`emit-schemas`, `validate`), and the
  eight pilot case encodings as the first schedule artifacts. The existing
  ECC (`tools/conformance`) is unchanged and remains the acceptance
  instrument until the comparison gate.
- Performance conformance, measured end to end: a `cnf-runner perf` run plays
  an open-loop offered-load schedule against a composed server at a
  population-anchored volumetric class (proof-of-concept, small, large,
  regional), records re-checkable HDR histograms into the conformance
  results, and earns — never declares — a class verdict recomputed by the
  verdict pipeline. `CONF_PERF_CLASS=<class> scripts/conformance.sh` runs it
  as a pipeline stage; the earned classes flow into the verdicts, report,
  certificate, and a performance badge. Published SVG assets (the class
  ladder and per-class latency charts) plus a generated summary are rendered
  from the committed measurement records by `scripts/render/perf-assets.sh`
  and guarded against drift in CI, and a new **Performance** chapter on the
  documentation website explains the class ladder, the floors' derivation
  from official activity statistics, how a coordinated-omission-free run
  works, and how to reproduce it.
- The sustained-window ladder: `cnf-runner perf --hours 1|2|4|6|8|12`
  (pipeline: `CONF_PERF_HOURS`) extends a class run's measured window beyond
  the normative hour — a longer hold of the same offered load is a stricter
  demonstration and persists like any measured run. There is deliberately no
  shortened run.
- A step-load **stress instrument**, distinct from conformance:
  `cnf-runner stress` climbs short intense load steps (geometric doubling,
  ~two-minute holds, bisection refinement) to the **maximum sustainable
  throughput** inside a latency budget, over the same seeded corpus and
  workload mix as the class runs. The report (`stress.json`,
  schema-published, environment-bound, per-step re-checkable histograms)
  earns no class and never touches the conformance results; the class floors
  appear as context only. A latency-throughput curve SVG renders from the
  committed report through the same drift-guarded asset pipeline, and the
  documentation's Performance chapter tells the two-instrument story.

### Changed

- The conformance acceptance instrument is now the CNF 2.0 reference runner
  (`tools/cnf-runner`) end to end: `scripts/conformance.sh` composes the SUT
  on fresh volumes, executes the committed machine-readable catalogue,
  computes verdicts through the pure pipeline, and writes
  results/verdicts/report/statement/certificate + badges per SUT. The ECC
  harness (`tools/conformance`) is retired — its final inventory is
  preserved at `tools/cnf-runner/comparison/ecc-catalog.tsv` and the
  reviewed cutover record is `docs/conformance/cnf-comparison.md`; the
  previous ehrbase comparison artifacts are frozen as historical data.
  Committed per-SUT party sets (ixit + statement) live under
  `tools/cnf-runner/party/`.
- Verdict semantics: a REQUIRED capability whose every selected case is
  excluded by a schedule-registered ambiguity (an unrealized wire on the
  technology profile, e.g. ADL 1.4 archetype provisioning under ITS-REST
  1.1.0 — AMB-41) is now recorded as an explicit `unrealized` scope
  exclusion on the certificate instead of silently failing the tier; the
  API-presence capabilities (EHR/DEFINITION/QUERY API) are evidenced by
  chapter exemplar cases.
- The benchmark harness converged onto the conformance runner's corpus,
  recipes, and ixit topology, so both instruments seed identical clinical
  documents through the public write path. The performance numbers in the
  README and on the website are no longer hand-typed: they derive from
  committed run artifacts (the benchmark comparison charts and the CNF
  measurement records), and the site stale-numbers guard now also rejects a
  hand-typed rate, latency, or footprint in the sources.


- OPT-1.4 → ADL2 conversion fidelity: `DV_ORDINAL`/`DV_QUANTITY` constraints
  now convert to real AOM2 attribute tuples (`[value, symbol]`,
  `[units, magnitude(, precision)]`) instead of loose unconstrained nodes;
  slot include/exclude assertions are carried (both retained 1.4 slots and
  the filled-slot `include` naming the embedded archetype); OPT
  `default_value`s are carried and serialized as the ADL2 `_default`
  pseudo-attribute; temporal constraints keep both the ISO8601 pattern and
  the range plus assumed values; `referenceSetUri` becomes an ac-code term
  binding; `CONSTRAINT_REF` resolves against the merged 1.4
  `constraint_definitions`/`constraint_bindings`; and everything a
  decomposed root cannot express (out-of-scope bindings, tuple assumed
  values, `DV_STATE` machines, unconvertible assertions) is reported in the
  converted archetype's `RESOURCE_DESCRIPTION.conversion_details`. The
  whole vendored OPT corpus now converts, validates and re-parses as the
  standing test gate.

### Fixed

- OPT 1.4→2 decomposition now emits phase-1-clean ADL2 sources for every
  template in the corpus: a `-`-specialised embedded root (whose
  differential lineage a flattened OPT cannot resolve) is emitted as an
  unspecialised depth-0 archetype with every dotted code renumbered into
  the flat code space, and 1.4 node codes legitimately reused across
  sibling subtrees re-mint archetype-wide-unique ADL2 ids — terminology
  definitions and bindings follow in both cases, and every remap is
  recorded in the converted archetype's `conversion_details` provenance.

- The ATNA Audit Record Repository no longer loses records under a sustained
  write load: the audit drain now takes queued events in batches and
  persists each batch in one multi-row `INSERT` (the previous per-event
  round trips saturated far below write-path rates, filling the bounded
  queue and fail-open dropping the tail). Drop warnings are rate-limited to
  one per interval carrying the count since the previous warning instead of
  one log line per dropped record (the exact count stays on the
  `atna_audit_dropped_total` metric), and the default
  `audit.queue_capacity` rises from `1024` to `8192` for burst headroom.

- Composition validation closes eight archetype-constraint enforcement gaps
  the CNF content chapter exposed: `C_STRING` list/pattern constraints on
  `DV_IDENTIFIER.issuer`/`assigner`/`type` (only `id` was checked);
  `DV_MULTIMEDIA.size` against `C_INTEGER` list and range constraints
  (previously unvalidated); `C_ATTRIBUTE` existence `1..1` on
  `OBSERVATION.state`/`protocol`, `HISTORY.summary`, and `EVENT.state` now
  rejects the absent attribute; `DV_SCALE` value/symbol value-set
  constraints (generic `C_REAL` list + `C_CODE_PHRASE` code list — AOM 1.4
  has no `C_DV_SCALE`) are enforced, including on `DV_INTERVAL` bounds;
  `timezone_validity` on `C_TIME`/`C_DATE_TIME` (mandatory and prohibited)
  is honoured; half-open (one-side-unbounded) temporal range constraints
  reject out-of-range values; a `DV_PROPORTION` of kind fraction or
  integer-fraction with a non-zero `precision` is rejected
  (`Fraction_validity`); and a partial `DV_TIME` such as `10` is no longer
  over-rejected against `HH:??:??`/`HH:XX:XX` patterns (optional and
  not-allowed fields both admit an absent field).
- A `DV_TIME`/`DV_DATE_TIME` literal carrying a fraction on the hours or
  minutes component (e.g. `10.5`, `10:05.5`) is now rejected: openEHR
  supports fractional seconds only (BASE time types §ISO 8601 semantics not
  included).
- A `DV_URI` whose value has no URI scheme (e.g. `xyz`, `www.example.org`)
  is now rejected on commit per the CNF content schedule's RFC-3986 rule;
  plain-text URI content after the scheme remains accepted per the RM's
  plain-text allowance.
- A COMPOSITION create (`201`) or update (`200`) whose response is negotiated
  as a Simplified Format (`Accept: application/openehr.wt.flat+json` or
  `…wt.structured+json`) now returns the `ETag` and `Location` headers, matching
  the canonical (`application/json`/`application/xml`) response. Previously a
  FLAT/STRUCTURED commit body omitted both version-id headers, so clients could
  not read the new version uid or resource URL from a simplified-format commit.
- Composition validation now rejects a `DV_DURATION` whose value carries a
  decimal fraction on any component other than seconds (e.g. `P1Y3M4DT2.5H` or
  `PT2H14.5M`). openEHR permits a fraction only on the seconds component
  (BASE time types: "in openEHR, only fractional seconds are supported"), so
  such a value now fails its RM `Value_valid` invariant with `422` instead of
  being accepted.
- Composition validation now enforces a `DV_QUANTITY` constraint that fixes a
  measurement `property` (with no enumerated unit list): the committed `units`
  must be a unit of that physical property (per the openEHR measurement
  property↔unit table). A quantity constrained to `length` committed with a
  mass unit such as `mg` is now rejected with `422` instead of being accepted.
- Composition validation now rejects a coded value whose terminology is
  foreign to a `C_CODE_PHRASE` constraint that explicitly binds the
  archetype-`local` terminology with a closed code list. Committing a
  `DV_CODED_TEXT` whose `defining_code` uses, e.g., SNOMED-CT against a
  `local`-scoped closed list now yields `422` instead of being accepted.
- The AQL `ehr_id` execution scope now also binds bare `FROM EHR e` sources:
  a scoped query without a CONTAINS chain previously ran over the whole
  population instead of the single EHR context the `ehr_id` parameter selects
  (ITS-REST query `Request.md` §Common Headers and Query Parameters).
- A CONTRIBUTION delete member targeting the EHR_STATUS is now refused with
  `409 Conflict`: `EHR.ehr_status` is mandatory (RM ehr, EHR class, 1..1), so
  deleting the only status would leave the EHR violating its own invariant.
- FLAT/STRUCTURED commits: spec-listed direct RM-attribute paths that an
  operational template leaves unconstrained are no longer rejected as unknown
  paths. `ACTION/ism_transition` (`current_state`/`transition`/`careflow_step`
  + `_reason:i`) and `ACTION/time`, plus `INSTRUCTION/narrative`,
  `OBSERVATION/history_origin`, `ACTIVITY/timing` + `action_archetype_id`, and
  `INTERVAL_EVENT/width` + `math_function`, are now built from their datum
  parts per the ITS-REST Simplified-Formats `master05-rm_mapping.adoc` per-type
  tables, and emitted symmetrically on the reverse (RM → FLAT) direction so
  round-trips stay lossless. Previously a client-supplied `ism_transition` was
  rejected with "unknown simplified path" and the ACTION state fell back to the
  synthesized `initial` default.
- AQL paging: the REST `fetch`/`offset` parameters now page over the result
  set the AQL `LIMIT`/`OFFSET` clauses define instead of being rejected with
  `400` when combined. Per ITS-REST query `Request.md`, only pairing `fetch`
  with the deprecated AQL `TOP` modifier is prohibited — that rejection
  remains. Negative `fetch`/`offset` values are now rejected explicitly.


- Spec version identity is now derived from the `openehr-*` crate versions
  instead of hand-typed literals, fixing the stale values those literals had
  drifted to: the startup banner advertised `ITS-REST 1.0.3` (now `1.1.0`),
  and the AQL `RESULT_SET` `meta._schema_version` was still emitted as
  `1.0.3` (now `1.1.0`, the implemented ITS-REST release). Every `openehr-*`
  spec crate exposes a `SPEC_VERSION` constant (= its crate version; the AM
  crate also exposes per-generation `am14`/`am24` constants from the BMM
  schemas), and the shared provenance constants behind the banner,
  `/status`, `OPTIONS /` (System Options), and `/management/info` read
  those, so a future pin bump propagates everywhere at compile time. The
  served `restapi_specs_version`/`openehr_rest_api_version` identity is now
  the plain version string `1.1.0` (matching the System API OAS example)
  instead of the tag-styled `Release-1.1.0`.
- SM call-status fidelity: service-layer "does not exist" failures now carry
  their granular `CALL_STATUS_TYPE` (`ehr_id_does_not_exist`,
  `composition_does_not_exist`, `template_does_not_exist`,
  `object_version_does_not_exist`, …) end-to-end instead of resurfacing as
  the generic `versioned_object_does_not_exist` after crossing the service
  boundary. HTTP status codes are unchanged (every does-not-exist status was
  and remains `404`); some `404` body messages are now the precise
  construction-site text.

## [3.5.0] - 2026-07-21

### Changed

- Conformance: zero skipped outcomes. The former 35 skips are eliminated —
  11 cases now execute against the documented ehrbase-rs extension surfaces
  (contribution listing, admin template deletion, bare stored-query
  listing), 6 more execute via new composed-stack wiring (an OpenPGP-signing
  sibling instance and a hermetic FHIR terminology fixture with fault
  injection) and loaded-database AQL golden support, and 18 native-API-only
  service operations are now first-class not-applicable verdicts carrying
  their SM citation and native-test evidence.

### Added

- ADL 2 archetype validation now enforces VETDF (external term-binding
  validity): a term bound to an external terminology (SNOMED CT, LOINC, …)
  that the configured terminology service reports as absent is rejected
  `422` with the `VETDF` rule code. Bindings the service cannot verify (no
  external provider configured, an unknown terminology, or a transport
  fault) are not raised, per the spec's "subject to tool accessibility"
  carve-out; archetype-internal (`local`/`openehr`) bindings are unaffected
  (covered by VTTBK/VTCBK key validity).
- ISO 8601 temporal ordering on the openEHR BASE time types
  (`Iso8601_date`/`_time`/`_date_time`/`_duration`): comparison with honest
  incomparability (partial-date range semantics, UTC normalization for
  zoned values, duration ordering via the spec's own `to_seconds`
  reduction with the `Time_definitions` average constants). ADL 2
  archetype validation now enforces assumed-value interval containment for
  temporal constraint types (previously undecidable and skipped); an
  incomparable pair never raises a violation.

## [3.4.0] - 2026-07-20

### Changed

- The implemented openEHR REST API is **ITS-REST Release-1.1.0** (published
  upstream 19-Jul-2026). The server was already built against the
  pre-release text of this release — the regenerated REST contract is
  byte-identical at the release tag — so wire behaviour is unchanged; the
  advertised API identity moves from 1.0.3/development to 1.1.0 everywhere
  (documentation, OpenAPI metadata, conformance artifacts), and the
  `openehr-its` spec crate is now versioned 1.1.0. Conformance reports
  state the tested edition as `release-1.1.0` (formerly `development`;
  the old label remains accepted as a CLI/config alias).

## [3.3.0] - 2026-07-20

### Added
- **ADL2 templates are now compiled and validated by the full ADL2 engine.**
  `POST /definition/template/adl2` runs the complete `openehr-adl` pipeline —
  parse, then the AOM2 validity catalogue (phase 1 basic integrity, reference-
  model conformance, and specialisation conformance against an already-loaded
  parent) — in place of the former source-subset probe. An invalid artefact is
  a **422** whose `Error.validationErrors` list the offending rule-code
  mnemonics (S-codes for an unparseable source, V-codes for a validation-phase
  failure). `GET /definition/template/adl2/{template_id}` now serves the
  `application/json` `OperationalTemplateV2` projection alongside the
  `text/plain` source, and resolves a partial `template_id` to the latest
  matching version; the previously `501` `…/{template_id}/{version}` (versioned
  get, marked deprecated in the spec) is implemented, and template list rows now
  carry `concept` and `archetype_id`. `GET …/{template_id}/example` now generates
  an example COMPOSITION from the compiled operational template (an ADL2 →
  Web Template front end feeding the shared example generator), served across the
  four `Accept_LOCATABLE` representations (canonical JSON/XML, `openehr.wt.flat`,
  `openehr.wt.structured`) with `type` (`input`/`output`) + `detail_level`
  (`required`/`medium`/`complete`) query parameters, and `400`/`404`/`406` exactly
  as the ADL 1.4 example endpoint. An `Accept` naming only `application/xml` on
  the plain template GET is a `406` (the operation declares no XML response body).
- **ADL 1.4 archetypes are now validated by the ADL 1.4 engine, and can be
  migrated to ADL 2.** An ADL 1.4 source archetype (the `I_DEFINITION_ADL14`
  archetype surface) is now parsed and validated **as ADL 1.4** by the
  `openehr-adl` engine — the subset of the phase-1 catalogue that corresponds to
  the ADL 1.4 / AOM 1.4 standalone validity rules (VARID, VARDT, VARCN, VATID,
  VDSEV/VDSIV, …), replacing the former structural probe. An invalid source is a
  **422** naming the offending rule-code mnemonic. A new service capability
  migrates a stored ADL 1.4 archetype to ADL 2 source (`adl14_convert_to_adl2`);
  no openEHR spec governs 1.4 → 2 conversion (our own design/extension) and the
  ITS-REST contract declares no conversion operation, so it is a library
  capability with no REST endpoint. The ADL 1.4 operational-template (OPT) REST
  surface (`/definition/template/adl1.4`) is unchanged.
- **RM terminology-backed invariant validation.** Composition (and any RM
  value) validation now enforces the openEHR terminology-service and code-set
  RM class invariants at the wire-boundary dispatcher, unified into a single
  hook (`openehr-its`) that every validation consumer inherits. The 30 wired
  invariants (each audited clean against the whole corpus before enforcement):
  `COMPOSITION` category/language/territory, `EVENT_CONTEXT` setting,
  `ELEMENT` null-flavour, `ISM_TRANSITION` current-state/transition,
  `PARTICIPATION` + `EXTRACT_PARTICIPATION` function/mode, `INTERVAL_EVENT`
  math-function, `TERM_MAPPING` purpose, `AUDIT_DETAILS` change-type,
  `ATTESTATION` reason, `PARTY_RELATED` relationship, `VERSION`
  lifecycle-state, `ENTRY`/`DV_TEXT` language + encoding, `DV_MULTIMEDIA`
  media-type/charset/language/compression/integrity algorithms, `DV_PARSABLE`
  charset/language, `DV_ORDERED` normal-status, and the `AUTHORED_RESOURCE` /
  `RESOURCE_DESCRIPTION_ITEM` / `TRANSLATION_DETAILS` original-language. An
  out-of-vocabulary openEHR code is a `422` naming the violated RM invariant;
  HTTP status codes are unchanged.

- Admin console: the Directory tab is now a complete directory experience —
  a structured folder-tree editor (add/rename/remove sub-folders, attach and
  remove composition item references with a picker), version history with
  read-only views and one-click restore, a `version_at_time` time-travel
  control, a sub-folder `path` query, and directory deletion with
  confirmation — on top of the existing create-from-template flow (raw JSON
  editing stays available as an advanced mode).

### Changed
- **RM validation invariant messages now carry the spec's (BMM) invariant
  names.** Three class-invariant violation messages were reconciled from their
  inherited archie spellings to the openEHR BMM invariant names, so a `422`
  validation payload reporting one of them changes text: `Accuracy_valid` →
  `Accuracy_validity` (DV_AMOUNT and its descendants — DV_QUANTITY, DV_COUNT,
  DV_DURATION, DV_PROPORTION), `Is_archetypeRoot` → `Is_archetype_root` (the
  ENTRY subtypes — OBSERVATION, EVALUATION, INSTRUCTION, ACTION, ADMIN_ENTRY),
  and `Location_validity` → `location_valid` (EVENT_CONTEXT). The check logic
  and HTTP status codes are unchanged; only the invariant name inside the
  `Invariant <name> failed on type <TYPE>` message differs.

- **Canonical-JSON codec cutover.** The openEHR spec types are now
  (de)serialized to/from canonical JSON entirely by a native emitted
  `ToJson`/`FromJson` codec in `openehr-its` — the spec types (`openehr-base`,
  `openehr-rm`, `openehr-am`, `openehr-term`, `openehr-lang`) no longer carry a
  serde derive, and the `openehr-derive` proc-macro crate is removed. The wire
  bytes are unchanged (proven by the R0 determinism manifest + the byte-hazard
  gates); the only externally visible difference is the **error-message shape on
  a malformed JSON request body** — the codec's parser reports `expected … at
  line N column M` / `missing field … on …` diagnostics instead of the previous
  serde phrasing (the HTTP status codes are unchanged: still `400`/`422`). A
  present-but-`null` array field is now rejected as a type error (was silently
  treated as an empty array), matching the strict tolerance contract.

- The served OpenAPI document now describes the COMPLETE wire for every
  operation (162 declarations across all API groups): every path/query
  parameter, request headers (`Prefer` incl. `return=identifier`, required
  `If-Match` forms, the committal headers), every reachable status code
  with its exact trigger, and the load-bearing response headers (weak
  `ETag`, `Location`, `Last-Modified`) — audited operation-by-operation
  against the vendored ITS-REST specification (both the operation
  definitions and the normative overview rules). A structural completeness
  test now gates the document.
- A disabled Admin API now answers `405 Method Not Allowed` (the status the
  ITS-REST specification declares for a disabled admin operation) instead
  of `404`.
- COMPOSITION and EHR_STATUS tag updates now honour the `Prefer` header as
  the specification defines: the default (`return=minimal`) returns
  `204 No Content`; `return=representation` returns `200` with the stored
  tag list. Previously the stored list was always returned with `200`.
- Demographic responses now carry `Last-Modified` (from the version's
  commit time) alongside the weak `ETag`; PARTY_RELATIONSHIP create/update
  honour `Prefer: return=identifier`.

### Fixed
- **Template example generation now produces fully-valid compositions.**
  `GET /definition/template/adl1.4/{template_id}/example` populated only a
  skeleton for many templates (issue #94) and could emit out-of-range or
  wrongly-typed values. The generator now synthesizes spec-valid values for
  every constrained field — quantities inside their magnitude ranges (with
  dimensionless empty units preserved), proportions satisfying their kind's
  invariants inside the archetype's numerator/denominator ranges, durations
  inside their declared range, coded text from closed value lists, URIs and
  parsables honouring their pattern constraints, and the archetype-constrained
  container/event types (`ITEM_LIST`/`ITEM_SINGLE`/`INTERVAL_EVENT`) instead
  of abstract defaults — and every generated example at the committable detail
  levels (`required`, `medium`) passes the server's own full composition
  validation. Generation is byte-deterministic.
- **Archetype-conformance validation no longer demands `archetype_node_id` on
  reference-model types that cannot carry one.** `EVENT_CONTEXT` (and any
  other non-`LOCATABLE` type) inherits `PATHABLE`, which the RM gives no
  `archetype_node_id`; a template archetyping `/context[at…]` therefore could
  never be satisfied by canonical data and such compositions were wrongly
  rejected on commit. Non-`LOCATABLE` nodes now match structurally by their
  attribute position (per the RM inheritance graph); `LOCATABLE` nodes keep
  strict node-id matching.

- Admin console: text typed into the EHR finder and create-EHR fields before
  the app finished loading is no longer silently wiped (the inputs are now
  hydration-safe, like the login form); success toasts no longer intercept
  clicks on buttons beneath them in the e2e battery.
- `GET /ehr/{ehr_id}/directory/{version_uid}` now honours the `path` query
  parameter (slash-separated FOLDER names selecting a sub-folder subtree),
  as the ITS-REST `directory_get_by_version_id` operation specifies; an
  unresolved path returns 404. Previously the parameter was accepted but
  ignored and the full tree was always returned.
- The served OpenAPI now documents the full DIRECTORY wire contract
  (`version_at_time`/`path` parameters, `Prefer` including
  `return=identifier`, `If-Match`, and the complete status ladders
  including 204/400/409/412).

## [3.2.0] - 2026-07-18

### Added
- **`GET {base}/admin/config` — the redacted effective configuration** (an
  ehrbase-rs extension; the openEHR admin API defines only EHR deletes).
  Returns the merged effective configuration (file + `EHRBASE_*` env +
  `--set` overrides) as a JSON tree with every secret-bearing value redacted
  structurally by its secret type — passwords, password hashes, HMAC/signing
  secrets, and S3 secret keys render as `***`, and connection URLs (database,
  AMQP) mask their embedded credentials while keeping host and path; non-secret
  identifiers (usernames, roles, OIDC issuer) stay visible. Shares the admin
  gate and authorization of the admin deletes (`EHRBASE__ADMIN__ENABLED=true`,
  `ADMIN` role); disabled admin API answers `404`.
- **`ehrbase-admin-ui` — the admin console**, a new standalone web
  application (its own binary and OCI image,
  `ghcr.io/rubentalstra/ehrbase-rs-admin-ui`) that manages any
  ITS-REST-1.0.3 CDR strictly over its REST API. Pure Rust end to end
  (Leptos SSR + WASM, zero hand-written JavaScript). Feature set:
  dual Basic + OIDC login (credentials held server-side in the BFF),
  a dashboard (count tiles, query-group tiles, a commit-activity trend
  chart), a Template Manager (list/filter/upload OPTs with the CDR's
  validation diagnostics verbatim; per-template path-catalog tree, raw-OPT
  view, and format-switchable generated example), an EHR browser (finder,
  status/directory/compositions/contributions, and a composition viewer
  with canonical JSON/XML + FLAT/STRUCTURED toggle, version history, and
  audit details), a **point-and-click Query Builder** that assembles the
  real AQL AST (typed per-datatype criteria from the template's
  constrained value sets, nested AND/OR/NOT groups, projection columns,
  live AQL preview) and runs it via the Query API, a raw AQL editor with
  BFF-side grammar validation and parameter bindings, stored-query
  management with console-local query groups, and a system panel (CDR
  status, SMART discovery, the served OpenAPI rendered natively).
  Configured by one `ehrbase-admin-ui.toml` (+ `EHRBASE_ADMIN__*` env);
  ships in the quickstart compose as the `ehrbase-admin-ui` service on
  port 3000. The sign-in page is served fully rendered and works with
  JavaScript disabled (the login form posts and redirects natively), and
  offers exactly the methods that can work: the console's configured login
  modes intersected with the authentication schemes the CDR advertises in
  its `WWW-Authenticate` challenge. The console received a full design
  system (semantic design tokens with lockstep light/dark theming, a teal
  brand shared by the widget kit, iconified navigation, breadcrumbed page
  headers, named table headers, empty states, and toast feedback on every
  mutation) and the complete working feature set: query result **export**
  (CSV/JSON, a plain form download that works without WebAssembly),
  **EHR creation** (empty or subject-bound) and **find-by-subject-id**,
  **composition commit** (canonical JSON/XML/FLAT with verbatim CDR
  validation diagnostics) and **edit-as-new-version** (`If-Match`
  concurrency), stored-query **open-in-editor**, shareable URL-driven tab
  state on the detail screens, a template identity card (version,
  languages, UID, archetype id), an **EHRs (cohort)** query shape
  (`SELECT DISTINCT` over the criteria tree), a **Table | Chart** toggle
  on numeric result columns, a version **timeline strip** with a
  `version_at_time` picker on the composition viewer, and a
  **contributions table** on the EHR detail screen. The Directory tab can
  now **create and edit the EHR folder directory** (spec-standard
  POST/PUT with `If-Match`), starting from console-local **folder
  templates** (two built-ins included); the System panel gained
  **repository usage** (per-template composition counts) and a read-only
  **runtime configuration** view backed by the CDR's new redacted
  `GET /admin/config` endpoint (secrets redacted structurally by their
  types — never by key matching). The E2E harness gained an image mode
  (`UI_E2E_IMAGE=1`) that runs the identical journey battery against the
  composed OCI image — including a genuinely end-to-end OIDC journey: the
  quickstart Keycloak now pins one canonical issuer and the dev CDR config
  trusts it via standard OIDC discovery, so a bearer-authenticated console
  session queries the CDR for real. Verified by a Rust-native browser E2E
  journey suite (merge-gating in CI, screenshots published as artifacts),
  including journeys over seeded clinical data and a JavaScript-disabled
  login journey.
- **`GET /ehr/{ehr_id}/contribution` — a paged contribution list** (an
  ehrbase-rs extension; the openEHR REST API defines only the by-uid read).
  Returns the EHR's contributions newest-first as
  `{ "rows": [ { uid, time_committed, committer, change_type } ], "total" }`,
  paginated with `offset` (default 0) and `fetch` (default 20, capped at
  100); **404** for an unknown EHR. Authenticated like the other EHR reads.
- **`DELETE /admin/template/{template_id}` and
  `DELETE /admin/query/{qualified_query_name}/{version}`** — admin deletes for
  operational templates and stored-query versions (ehrbase-rs extensions; the
  openEHR admin API defines only EHR deletes). Same admin gate and
  authorization as the EHR deletes: **204** on success, **404** for an unknown
  id. The template delete additionally returns **409** when a committed
  version still references the template, so a physical delete never orphans
  clinical data.

- **ATNA audit — richer DICOM records**: every audit record now carries the
  concrete operation as a DICOM `EventTypeCode` (login/logout as DCM
  110122/110123; REST operations as their ITS-REST operation id under the
  `openEHR-ITS-REST` code system), and Bearer-authenticated requests record
  the token's `jti` as the minimal token identity (token contents are never
  logged).
- **ATNA audit — FHIR R4 `AuditEvent` rendering (IHE BALP)**: every audit
  record also renders as a FHIR R4 `AuditEvent` conforming to the IHE Basic
  Audit Log Patterns (Patient\*/plain Create/Read/Update/Delete/Query
  profiles, `OAUTHaccessTokenUse.Minimal` token agent, profile claims only
  when genuinely satisfied) — the modern half of the dual ATNA format.
- **ATNA audit — local Audit Record Repository, on by default**: audit
  records are persisted in a new PostgreSQL `audit` schema (append-only;
  strictly outside the EHR content; per-sink delivery stamps; configurable
  `retention_days` with an hourly reaper). Every deployment now gets a
  queryable audit trail out of the box with nothing leaving the node.
- **ATNA audit — RESTful ATNA forwarding (ITI-20 ATX:FHIR Feed)**: opt-in
  `[audit.fhir_feed]` sink POSTs each FHIR `AuditEvent` to an external Audit
  Record Repository; with the local store on, delivery is outbox-driven — an
  ARR outage loses nothing and pending records ship on recovery.
- **ATNA audit — per-sink metrics** (`atna_audit_sent_total{sink=…}`,
  `…send_failed_total{sink=…}`, `atna_audit_rejected_total`,
  `atna_audit_reaped_total`).
- **ITI-81 Retrieve ATNA Audit Event** (`GET /fhir/r4/AuditEvent`): the
  official RESTful-ATNA retrieval — a FHIR search over the local Audit
  Record Repository returning a `searchset` Bundle of the stored `AuditEvent`
  documents. Filters: `date` (`ge`/`le`), `patient`, `agent`, `entity`,
  `outcome`, `action`, plus `_count`/`_offset` paging. Admin-only under
  RBAC; `404` when the local store is disabled.
- **Native TLS + mutual-TLS client authentication** (`[server.tls]`): the
  main listener can terminate TLS itself (TLS 1.2+ floor per IETF BCP 195)
  and demand a verified client certificate
  (`client_auth = "off" | "optional" | "required"`) against an explicit CA —
  the IHE ATNA ITI-19 node-authentication posture. The management listener
  stays plain HTTP.
- A dedicated **Audit trail (IHE ATNA)** book chapter covering the dual
  formats, the sinks, the ITI-81 retrieval, fail-mode semantics, and mTLS.
- **Admin console — the Audit log screen** (`/audit`): browse the CDR's
  ATNA security audit trail through the standard ITI-81 retrieval, with
  URL-driven filters (event-time window, patient, principal, outcome,
  action), pagination, and a per-row view of the full stored FHIR
  `AuditEvent`. Admin-only under RBAC; a disabled local audit store and a
  no-matches filter each render their own first-class state.

### Changed
- The ITS-REST template list (`GET /definition/template/adl1.4`) now reports
  the optional `version` field of each `TemplateMetadata`, derived from the
  template id's version axis (the spec documents the value as "taken from
  `template_id`"); it is omitted when the id carries no version.
- **Audit configuration redesigned: `[atna]` is now `[audit]`**, on by
  default with only the local store active, and sink-structured:
  `[audit.store]` (local repository), `[audit.syslog]` (classic
  DICOM-over-syslog feed; keys `host`/`port`/`transport`/`tls_ca_file`/
  `tls_identity_cert_file`/`tls_identity_key_file` replace the old
  `repository_host`/`repository_port`/`tls_*_path`), `[audit.fhir_feed]`
  (RESTful ATNA). `resolve_subject` now defaults to `true`. A configuration
  still using `[atna]` fails at boot with did-you-mean guidance (strict
  loader; no silent aliasing).
- **Fail-closed auditing got stronger**: with `fail_mode = "closed"` and the
  local store enabled, a store that stops accepting writes makes every
  subsequent auditable operation answer `503 Service Unavailable` until a
  write succeeds again — no un-audited PHI access.

### Fixed
- **ATNA audit — IHE/DICOM conformance corrections** (IHE ITI TF-2 ITI-20 /
  DICOM PS3.15 §A.5.1): the syslog `MSGID` is now the mandated
  `IHE+RFC-3881` (was `IHE+DICOM`); AQL query execution uses the dedicated
  DICOM EventID 110112 "Query" (was 110110); EHR-Extract communication uses
  the direction-coded EventIDs 110106 "Export" / 110107 "Import";
  authentication events (genuine logins and rejected 401/403 attempts) use
  EventID 110114 "User Authentication" with `EventTypeCode` 110122 "Login"
  (were generic Application Activity); and 1xx/3xx responses (e.g. `304 Not
  Modified`) are now recorded as success instead of minor failure.
- **Admin console — icon-only chrome and small polish**: every emoji and
  typographic glyph in the UI is replaced by a proper SVG icon (folder tree,
  status capability badges, remove buttons, disclosure carets, upload
  trigger, pagination arrows); the Audit log screen highlights its own
  navigation entry; and the documentation screenshots now cover every EHR
  detail tab — including the directory tab both before (create from a folder
  template) and after the directory exists — plus the audit raw-record view.

## [3.1.1] - 2026-07-17

### Fixed
- The release pipeline attaches the per-architecture server binary tarballs
  again: since the crate consolidation the binary is produced by the
  `ehrbase-server` package (the executable is still named `ehrbase`), but
  the release asset build still compiled the `ehrbase` platform library and
  failed — v3.1.0 published without binary assets. Container images were
  not affected. Use v3.1.1 for downloadable binaries.

## [3.1.0] - 2026-07-17

### Added
- External terminology providers cache their FHIR operation results
  (`$validate-code`/`$expand`/`$subsumes`/`$lookup`) for a configurable TTL
  (`[terminology.external.providers.<name>] cache_ttl_secs`, default 300 s,
  `0` disables; `cache_capacity`, default 10000) — a validation burst over
  the same codes costs one remote round trip per window instead of one per
  code.
- A new `atna_audit_serialize_failed_total` metric counts ATNA audit records
  dropped because the message failed to serialize, so audit loss is always
  metered.

### Changed
- The FLAT and STRUCTURED (Simplified Formats) layer was rewritten against
  the official openEHR ITS-REST Simplified Formats specification: exact
  node-id generation, per-type attribute suffixes, the full `ctx/`
  vocabulary with its documented defaults, `|raw` embedding, and the
  `|other` open-value-set rules (invalid combinations are now rejected with
  `422` instead of being silently ignored). Unknown field identifiers in a
  simplified payload are now rejected rather than dropped.
- Format selection is done exclusively via the `Accept` and `Content-Type`
  headers on every endpoint that supports the simplified media types
  (`application/openehr.wt.flat+json`, `…wt.structured+json`, and
  `application/openehr.wt+json` for template rendering), with proper
  RFC 9110 q-value negotiation, `406`/`415` answers naming the supported
  formats, and simplified support on CONTRIBUTION payloads
  (`versions[].data`) with the envelope staying canonical.
- Committing a composition in a simplified format now requires the
  `openehr-template-id` request header (`422` without it, previously `400`);
  the undocumented `template_id` query parameter is no longer read.
- Content negotiation is strict everywhere: an `Accept` header that none of
  an endpoint's supported formats can satisfy is answered with `406`
  (previously some JSON-only endpoints leniently returned JSON), and the
  server's own generated OpenAPI now advertises the simplified media types
  on the composition, contribution, and template endpoints.
- Release builds now abort on integer arithmetic overflow instead of
  silently wrapping (`overflow-checks` enabled in the release profile) — a
  corrupted-value class of fault becomes a crash-and-restart instead of
  wrong clinical data.


- The application is consolidated to two library crates plus a thin binary
  (`ehrbase` — the platform, `ehrbase-rest` — the ITS-REST adapter,
  `ehrbase-server` — the binary): the `ehrbase-sm` trait catalog is gone,
  the REST adapter calls the concrete platform service directly, and the
  full configuration tree (`[server]`, `[auth]`, `[authz]`, `[smart]`,
  `[management]`, `[tenancy]`, `[admin]`) is defined in the platform crate.
  The served wire, the `ehrbase.toml` schema, and the container entrypoint
  (`ehrbase`) are unchanged.
- Bundle-backed terminology lookups and template/query validity checks are
  now synchronous in-process calls (no behaviour change on the wire).
- Every versioned write now commits through the single folded
  audit+contribution+version statement even with digest signing enabled
  (the commit instant is read up front with the placement, so the signature
  is computed before any insert); version-tree placement is one read instead
  of three, and contribution commits batch their target pre-reads. Fewer
  round trips per write, identical wire behaviour and stored semantics.
- The OpenAPI documents (the composed `openapi.json` and the twelve Swagger
  spec-selector family documents) and the SMART `.well-known/smart-configuration`
  discovery document are now built once at server startup instead of being
  regenerated on every request. No change to the document content.

### Removed
- The `ehrbase-quirks` cargo feature and its vendor-specific behaviours
  (alternate duplicate-id spelling, the non-standard `|unit_system` /
  `|unit_display_name` quantity suffixes) — the specification-defined
  behaviour is now the only behaviour.

### Fixed
- A tenant-resolution failure (tenant registry unreachable) now fails the
  request with `503` instead of silently serving it under the default
  tenant; unknown tenant keys keep the documented unscoped behaviour and
  are negative-cached.
- Audits for authenticated writes that carry no committal headers are now
  attributed to the authenticated user (Basic username / token subject, with
  the mechanism recorded as the identifier type) instead of the generic
  system identity.
- Multi-tenant deployments now actually run on the tenant-scoped connection
  pool: with `tenancy.enabled = true` every database connection carries the
  request's tenant for the row-level-security policies. Previously the
  binary always built the plain pool, so all requests fell through to the
  default tenant regardless of configuration.
- Multi-tenancy: a connection freshly opened by the pool while serving a
  request (pool growth under load) could miss the tenant stamp and run as
  the reserved default tenant — reads returning nothing and writes landing
  outside the caller's tenant. The tenant-scoped pool now stamps
  `ehrbase.tenant_id` both when a connection is opened and on every
  checkout, so every connection carries the caller's tenant. Deployments
  with `tenancy.enabled = true` should upgrade.
- The demographic APIs (party and relationship writes) now honour the
  `openEHR-VERSION.*` / `openEHR-AUDIT_DETAILS.*` committal headers exactly
  as the EHR APIs do — a caller-supplied committer, description, and
  system id are merged into the stored version's audit.
- Direct COMPOSITION create/update/delete now honour the ITS-REST committal
  headers (`openEHR-VERSION.*` / `openEHR-AUDIT_DETAILS.*`): a
  caller-supplied committer, audit description, change type, lifecycle
  state, signature, and attestations are merged into the stored version
  exactly as on the CONTRIBUTION path (previously the direct paths discarded
  them and always committed server defaults).
- The template store no longer double-reads the OPT XML when generating an
  example for a cold template, and template upload is a single atomic
  statement (the duplicate-check race window is gone).
- The event-outbox publisher declares its AMQP topology only on connect or
  subscription change (previously every poll cycle re-declared each queue),
  and the FHIR outbound emitter parks a persistently failing row after a
  bounded retry budget instead of blocking the stream forever.
- A FLAT/STRUCTURED composition body that parses as JSON but does not conform
  to its target template now returns `422 Unprocessable Entity` instead of
  `500 Internal Server Error` — such an input is client data, not a server
  fault. Output conversion of stored compositions remains a `500` on failure.
- Panicking request handlers and audit fail-closed (`503`) responses now
  carry the standard openEHR `{ error, message }` JSON error body (the audit
  `503` also carries `Retry-After`), instead of a plain-text body.
- A malformed `If-Match` header on a state-changing request is now rejected
  with `400 Bad Request` instead of being silently ignored — an unparseable
  precondition previously ran as if no `If-Match` was sent, opening a
  lost-update window. `If-Match: *` and valid version ids are unaffected.
- Database constraint and serialization/deadlock failures now surface as
  `409 Conflict`, and connection-pool exhaustion under load as `503 Service
  Unavailable` with `Retry-After`, instead of collapsing every database error
  to `500 Internal Server Error`.
- Stored-query and template metadata list/read endpoints no longer silently
  blank a field when a database column fails to decode; a decode failure now
  surfaces as `500` with a real error instead of an empty value.

## [3.0.3] - 2026-07-16

### Changed
- The served OpenAPI documents now categorize operations the way the
  official ITS-REST reference documents do: standard-group operations are
  tagged by resource (EHR, EHR_STATUS, COMPOSITION, DIRECTORY, CONTRIBUTION,
  ITEM_TAG; PERSON, AGENT, GROUP, ORGANISATION, ROLE, VERSIONED_PARTY;
  ADL 1.4, ADL 2, Query) instead of one flat tag per API group, and the
  Swagger UI spec selector offers one document per API family — the five
  standardised openEHR groups and the seven server-extension families —
  plus the complete composed surface, all filtered from the server's own
  generated document.

### Fixed
- Duplicate-template-id fixture resolution in the validation corpus test is
  now deterministic (sorted path order) instead of OS-dependent `read_dir`
  order, fixing a Linux-only CI failure.

## [3.0.2] - 2026-07-15

### Changed
- The benchmark instrument measures both comparison stacks under a fairer,
  more deterministic protocol: the databases get a 1 GB `/dev/shm` floor
  (Docker's 64 MB default starved PostgreSQL's parallel workers mid-run),
  maintenance debt is settled with `VACUUM ANALYZE` after seeding and
  between ladder rungs (autovacuum no longer lands inside measured
  windows), the ladder drains in-flight backlog between rungs, and the
  measured cold start no longer includes building the ehrbase-rs container
  image. Ladder output prints latencies in magnitude-appropriate units
  (µs/ms/s), and the generated comparison page reports clinical events per
  minute beside request rates.
- **Configuration is now one `ehrbase.toml`.** The whole server is configured
  by a single TOML file (sections `[server]`, `[db]`, `[log]`, `[telemetry]`,
  `[auth]`, `[authz]`, `[admin]`, `[tenancy]`, `[smart]`, `[management]`,
  `[signing]`, `[query]`, `[events]`, `[fhir]`, `[terminology]`,
  `[multimedia]`, `[atna]`, `[subject_proxy]`), discovered from `--config`,
  `EHRBASE_CONFIG`, `./ehrbase.toml`, or `/etc/ehrbase/ehrbase.toml`. Every
  `EHRBASE_*` environment variable is now a mechanical per-key override:
  `EHRBASE` + the TOML path, upper-cased, with `__` between every segment
  including after the prefix
  (e.g. `EHRBASE__DB__MAX_CONNECTIONS`, `EHRBASE__AUTH__OIDC__ISSUER`). This
  replaces the previous ~14 independent per-subsystem loaders and their
  several env-name grammars. **Old spellings are not aliased** (greenfield —
  nothing is deployed to migrate): a pre-redesign variable fails at boot with
  the exact uniform replacement suggested (e.g. `EHRBASE_DB_MAX_CONNECTIONS`
  → "did you mean `EHRBASE__DB__MAX_CONNECTIONS`?"). `DATABASE_URL` and
  `RUST_LOG` remain permanent conventional aliases. New `ehrbase config
  default` prints an annotated template and `ehrbase config check` validates a
  config (and prints the effective, secret-redacted result) without a
  database. The compose stack, Helm chart, and docs all move to the new file +
  spellings; the PostgreSQL-init container variables `EHRBASE_DB_USER` /
  `_PASSWORD` / `_NAME` were renamed `PG_INIT_USER` / `_PASSWORD` / `_DB` so
  they no longer collide with the server's reserved `EHRBASE_` namespace.

### Removed
- The nine per-subsystem `EHRBASE_*_CONFIG` file pointers
  (`EHRBASE_REST_CONFIG`, `EHRBASE_AUTHZ_CONFIG`, `EHRBASE_ATNA_CONFIG`,
  `EHRBASE_SIGNING_CONFIG`, `EHRBASE_EVENTS_CONFIG`,
  `EHRBASE_FHIR_OUTBOUND_CONFIG`, `EHRBASE_MULTIMEDIA_CONFIG`,
  `EHRBASE_VALIDATION_CONFIG`, `EHRBASE_MANAGEMENT_CONFIG`,
  `EHRBASE_SUBJECT_PROXY_CONFIG`): merge each file's contents into the single
  `ehrbase.toml` under its `[section]`.
- `EHRBASE_REST_AUTH__ADMIN_SCOPE`: subsumed by `authz.rbac.admin_role`.

### Fixed
- Unknown or misspelled configuration is now rejected at boot with a
  did-you-mean suggestion (and the `file:line` for a file key) — previously a
  typo'd TOML key or `EHRBASE_*` variable was silently ignored, so a
  not-applied security setting could pass unnoticed.
- The documented `EHRBASE__SUBJECT_PROXY__SYSTEMS__<name>__BASE_URL` env form
  now actually binds — the old loader stripped the prefix such that this
  spelling was dead, so subject-proxy systems could only be set via a file.
- Unparseable `[query]` values (`query.plan_cache_capacity`, `query.timeout_ms`)
  now error at boot instead of silently falling back to defaults.
- The Swagger UI works again and now documents the **complete server
  surface** from one natively generated OpenAPI document. `…/rest/swagger-ui`
  previously entered an infinite redirect loop (the UI's trailing-slash
  redirect fought the server's path normalization) and its OpenAPI document
  was an empty stub. The UI now loads directly (documentation URL corrected to
  `/ehrbase/rest/swagger-ui`), and its spec selector has a single entry,
  `ehrbase-rest`, generated by the server itself (`utoipa-axum`, one
  `#[utoipa::path]` handler per operation, so route and documentation cannot
  drift): every ITS-REST API group (EHR, COMPOSITION, CONTRIBUTION, DIRECTORY,
  DEMOGRAPHIC, DEFINITION, QUERY, ADMIN) plus the server's own extensions
  (terminology, PARTY_RELATIONSHIP, event-subscription, multi-tenancy, FHIR
  connector) and its operational endpoints (status/health, management, SMART
  discovery, the OpenAPI endpoints). No vendored OpenAPI is served. The
  document also declares the server's **configured** authentication scheme so
  the "Authorize" dialog and per-endpoint padlocks match the running server:
  HTTP Bearer (JWT) when OIDC is configured, otherwise HTTP Basic, and none
  when authentication is disabled — never both at once.

## [3.0.1] - 2026-07-14

### Added
- The server now prints an ASCII-art startup banner to stdout before the
  structured startup logs: the `EHRbase-rs` wordmark, the running version, the
  maintainer credit (Ruben Talstra), the project URL, and the load-bearing
  spec/platform pins (openEHR RM 1.2.0 · ITS-REST 1.0.3 · AQL 1.1 ·
  PostgreSQL 18). The banner is suppressed under JSON logging
  (`EHRBASE_LOG_FORMAT=json`) so machine log consumers see only structured
  lines.
- AQL queries are now planned once and cached: a repeated ad-hoc or stored
  query text reuses its lowered plan instead of re-parsing and re-analysing on
  every execution, while per-request parameter values, `fetch`/`offset`
  paging, and EHR scope still bind independently. Queries that resolve
  terminology (`matches TERMINOLOGY(…)`) are never cached, so their expansion
  is always current. New configuration knob
  `EHRBASE_QUERY__PLAN_CACHE_CAPACITY` (default `256`; `0` disables the cache)
  bounds how many distinct plans are held, and a new `aql_plan_cache_events_total`
  metric (`event` = `hit`/`miss`) reports cache activity.


- Storage migration `0008`: a promoted `context_start timestamptz` column on
  COMPOSITION root node rows (backfilled from stored data, partially
  indexed), plus the fail-safe `ext.openehr_timestamp` conversion function.
  The AQL engine reads the indexed column for
  `ORDER BY`/`WHERE` on `c/context/start_time/value` — the measured
  patient-dashboard hot path — instead of re-extracting JSONB per candidate
  row; results are unchanged, including NULL placement and the verbatim
  projected value.
- Overload backpressure: the REST server now caps the number of API requests
  it handles concurrently and sheds the excess immediately with
  `503 Service Unavailable` + `Retry-After: 1` instead of queueing every
  request until it runs out of memory. Under sustained offered load beyond
  database capacity the server now degrades with clean errors rather than
  being killed. The cap is configurable via `EHRBASE_REST_MAX_IN_FLIGHT`
  (concurrent requests, not per second; default 256, raise for
  high-throughput deployments; `0` disables shedding). The `/status`, health,
  and discovery
  endpoints are never limited, so operators can always probe an overloaded
  server. (No openEHR spec governs overload behaviour; the `503` follows
  RFC 9110 §15.6.4.)
- Conformance framework (`tools/conformance`) redesigned and rewritten from
  the openEHR CNF component up (W-10). It now assesses **any** openEHR CDR:
  point it at a deployed server (`scripts/conformance.sh` with
  `CONF_SUT=byo CONF_BASE_URL=…`, or the CLI's `--sut byo --base-url …`) and
  receive the full spec-cited artefact set — `results.json`, a conformance
  report, a Conformance Statement, a Conformance **Certificate** (a
  machine-computed framework assessment, explicitly not an official openEHR
  certification), and badges, written per SUT. Upstream EHRbase is a
  built-in target (`CONF_SUT=ehrbase`) with a committed fairness
  register; a cross-SUT comparison matrix can be rendered from two or more
  runs (`conformance compare`). Assertions carry a **spec-edition ladder**:
  the runner tries the newest edition form first (weak `W/"…"` ETags,
  RM 1.2.0 wire) and steps down to Release-1.0.3-era forms, reporting the
  satisfied edition level per case instead of failing a CDR on edition
  deltas; ehrbase-rs CI runs stay pinned to the development edition so the
  ladder can never mask a regression.

- AQL: `OR`-combined `CONTAINS` expressions now execute (previously rejected
  as unsupported), including nested `AND`/`OR`/`NOT` containment trees, and
  `NOT CONTAINS` accepts compound operands.
- ATNA auditing: EHR-Extract export and import operations now emit audit
  events (object class `Extract`) when auditing is enabled.
- Multiple folder hierarchies per EHR (`EHR.folders`): beyond the
  `/directory` hierarchy, additional root `FOLDER`s can be committed through
  the CONTRIBUTION endpoint, each versioned independently. The EHR resource
  now carries the `folders` reference list (creation order) and `directory`
  (always its first member); EHR extract import and admin dump/load carry
  the hierarchies too. The `/directory` endpoints behave exactly as before.
- `ehr:` URI support: `DV_EHR_URI` values are parsed against the full
  openEHR `ehr:` grammar (EHR / top-level structure by uid or exact version
  id / interior item paths, absolute and relative forms), and the server can
  resolve local `ehr:` references internally (e.g. LINK targets). openEHR
  path processing now also supports `//` path patterns and 1-based
  positional predicates in stored-structure navigation (AQL is unchanged —
  its grammar defines neither).
- `EHR_ACCESS` access-control is now enforced. The spec-mandated,
  change-controlled `EHR_ACCESS` object of an EHR (RM ehr §EHR_ACCESS Class)
  is the foundational access-decision layer, evaluated after authentication
  and before dispatch on every EHR-scoped route; the enterprise RBAC/ABAC
  layers compose on top of it. Its `settings` use the
  `ehrbase.access_control.v1` scheme:
  a `default_access` (`open`/`restricted`) with a `user:`/`role:` access
  list gating the EHR, per-Composition privacy-level ceilings on Composition
  reads, and a gate-keeper that guards changes to the settings themselves
  (`403 Forbidden` on a denial). Every existing EHR keeps working — the
  default (no settings) is open.
- Client-supplied CONTRIBUTION `uid`s are honoured on commit when unused
  (`409 Conflict` when already in use; previously silently ignored).
- `Prefer: resolve_refs` is honoured on contribution reads: the
  CONTRIBUTION's `versions` are returned as full `ORIGINAL_VERSION`
  objects instead of `OBJECT_REF`s (ITS-REST representation negotiation).
- AQL single-row functions now execute: `LENGTH`, `SUBSTRING`, `POSITION`,
  the string `CONTAINS`, `CONCAT`/`CONCAT_WS`, `ABS`/`MOD`/`CEIL`/`FLOOR`/
  `ROUND`, and `CURRENT_DATE`/`CURRENT_TIME`/`CURRENT_DATE_TIME`/`NOW`/
  `CURRENT_TIMEZONE` (QUERY master03 §Functions).
- AQL `TERMINOLOGY()` Boolean value expressions
  (`TERMINOLOGY('validate'|'subsumes', …) = true`) and terminology-URI
  `matches` operands (`matches { terminology://… }`) are now evaluated
  through the terminology service (previously typed rejects).
- AQL archetype predicates now honour archetype-specialisation subsumption:
  a query naming a parent archetype (e.g.
  `[openEHR-EHR-OBSERVATION.laboratory.v1]`) also matches data created with
  any specialisation child (e.g. `…laboratory-glucose.v1`), scoped to the
  same RM entity and major version (BASE architecture_overview master10
  §Design-time Relationships; AM master07 §Querying). Non-HRID predicates
  (at/id-codes) keep exact case-folded matching.
- **Version-tree branching and merge provenance** (RM common master06
  §Version tree / §Distributed versioning / §Version Merging). Branch
  version ids (`trunk.branch.version`) are now first-class on every
  surface: modifying a version that was imported from another system forks
  a branch with the local `creating_system_id` (the spec's mandated rule
  for local modifications of copied versions) while the imported trunk
  version stays the container current; branch tips are continued,
  superseded, read, exported, and re-imported like any version; the
  container current / `LATEST_VERSION` (including in AQL) is the latest
  *trunk* version. `ORIGINAL_VERSION.preceding_version_uid` is now stored
  at commit (previously synthesized) and `other_input_version_uids` (merge
  provenance) is accepted on the CONTRIBUTION wire, preserved on import,
  and served on read. The `vo_version` storage carries the version tree in
  explicit columns with per-lineage temporal non-overlap constraints and
  the spec's global version-identity uniqueness tuple.

### Changed
- Basic-auth verification no longer re-runs the Argon2 password hash on
  every request: verified credentials are cached (as a SHA-256 digest,
  never plaintext) for `EHRBASE_REST_AUTH__VERIFIED_CACHE_TTL_SECONDS`
  (default 60 s; `0` disables), and cache misses hash on a background
  thread. At load this removes roughly a full CPU core of per-request
  hashing.
- Composition create/update responses are built from the commit result
  instead of re-reading the just-written document from the database — one
  connection acquisition and two queries fewer per write; when version
  signing is disabled the server also no longer rebuilds the full document
  it would only have signed. Response bodies and headers are unchanged.
- Storage: the version table's two GiST exclusion constraints and two
  speculative JSONB indexes on the node table (a GIN over every fragment and
  a magnitude expression index — no query the engine generates could use
  either) were removed; version-validity non-overlap is unchanged and held
  by construction (one open row per lineage via unique indexes, atomic
  close-then-insert writes, and an overlap audit on archive load). This
  removes the dominant per-commit index-maintenance and lock-contention
  costs on the write path.
- Connection-pool defaults changed: `EHRBASE_DB_MAX_CONNECTIONS` 10 → 20,
  `EHRBASE_DB_MIN_CONNECTIONS` 0 → 2, and the per-checkout liveness ping is
  disabled (a broken connection is detected by its first statement).
  `TCP_NODELAY` is now set on accepted sockets, removing Nagle-induced
  latency on small responses.
- Composition commits make fewer database round trips: the audit and
  contribution rows are written in one statement, and the create-path EHR
  existence + modifiability gates are one read instead of two. Error
  behaviour is unchanged (a missing EHR is still `404` before a
  non-modifiable `409`).
- The transactional event outbox is no longer written on every commit when no
  eventing consumer is configured. The per-commit `event_outbox` row (and its
  envelope serialization) is now written only when the AMQP publisher
  (`EHRBASE_EVENTS_ENABLED`) or the FHIR outbound emitter
  (`EHRBASE_FHIR_OUTBOUND_ENABLED`) is enabled. Consequence: the outbox
  records commits made while a consumer is enabled (at-least-once, even with
  zero bound subscribers — the gate is the boot-time config, not the current
  subscriber set); commits made while every consumer was off are not
  back-filled if eventing is later enabled.
- IHE ATNA login ("Application Activity") records now mark genuine
  authentication events rather than every authenticated request. A login
  record is emitted only when the request actually verified credentials (a
  Basic verified-credential cache miss); a cache hit continues an established
  session and a Bearer request authenticated out of band at the OIDC provider,
  so neither mints a per-request login record. Rejections (401/403) are still
  always audited, and login records remain off by default
  (`EHRBASE_ATNA_SUPPRESS_LOGIN_EVENTS`, default `true`).
- Per-EHR `EHR_ACCESS` access-settings are cached as default-open at EHR
  creation, so the access gate's first check on a freshly created EHR no
  longer costs a database lookup (a hospital-day workload creates EHRs
  constantly). Importing an `EHR_ACCESS` version into an existing EHR now
  evicts that cache entry, so the access decision reflects the imported
  policy immediately.
- Composition validation is substantially faster with identical outcomes:
  the RM-invariant pass validates each node directly against the
  spec-generated Reference Model instead of deserializing every node into
  its typed struct (falling back to the typed path for anything it cannot
  vouch for), the archetype-constraint walk reuses constraint paths parsed
  once per cached WebTemplate instead of re-parsing them on every node
  visit, and validation error messages are byte-for-byte unchanged
  (equivalence is pinned by tests across the full corpus). Measured
  end-to-end: a fully populated International Patient Summary validates in
  well under half its previous time.


- Version lifecycle states are now enforced as a state machine (RM common
  §Version Lifecycle): a commit whose `lifecycle_state` is not a legal
  transition from the preceding version's state (for example
  `incomplete` → `inactive` without completing first) is rejected `422`.
- Template identifiers now compare case-insensitively (case-preserving):
  lookups accept any casing and uploading a case-variant duplicate is a
  `409` conflict, backed by a unique index (new migration).
- AQL `MIN`/`MAX` aggregate over non-numeric leaves (text, dates, times)
  now compares type-appropriately instead of forcing a numeric cast, and
  mixed-type leaf comparison dispatches numerically for numbers.
- Contribution commits now verify the target EHR exists (`404` otherwise)
  and honour the `EHR_STATUS.is_modifiable = false` write guard and
  versioned-composition invariants on every path, including
  CONTRIBUTION-wrapped commits. Re-creating an existing directory (a folder
  hierarchy with the same root archetype and name) via a CONTRIBUTION is a
  `409` conflict; a hierarchy with a distinct root remains a new
  `EHR.folders` member.
- EHR-index errors now carry the precise SM error names
  (`ehr_id_does_not_exist`, `subject_id_does_not_exist`) instead of a
  generic not-found.
- Contribution retrieval now lists versions affected by `attestation`-only
  items alongside committed versions for demographic contributions,
  matching the EHR-scoped behaviour.
- SMART App Launch resource-server support (openEHR SMART App Launch
  framework, development edition), config-gated and off by default
  (`EHRBASE_REST_SMART__*`): the `/.well-known/smart-configuration`
  discovery document, the full resource-scope grammar
  (`compartment/resource.permission` with `*`/`**`/`ns::*` patterns), and
  scope + launch-context (`ehrId`→patient) enforcement composed after
  RBAC/ABAC.
- Subject Proxy Service completed (SM `I_SUBJECT_PROXY_SERVICE`): variables
  are now tracked over time (a persisted sample history per variable),
  `currency` freshness is evaluated (fresh samples are served without
  re-querying; data-set registration tightens currency), data-set local
  aliases resolve on reads, `using_app_ids` lifecycle drops empty data
  sets, and frames execute with primary→fallback semantics. New FHIR frame
  executor (config-gated named systems, `EHRBASE_SUBJECT_PROXY__*`) lets
  variables be populated from FHIR R4 servers; manual variables gain a
  notification input channel.
- System API `OPTIONS /` conformance manifest rebuilt: reports the live
  mounted endpoint groups, a single provenance source (the tested
  development-edition ITS-REST identity), and configurable identity fields
  (`EHRBASE_REST_SYSTEM__*`); also mounted at the API base path.
- Item tags via headers (`openehr-item-tag`/`openehr-version-item-tag`):
  accepted on EHR-group and demographic writes and echoed on responses.
- Query API: multi-EHR scoping (`ehr_ids` set), an honest
  `ehr_id_does_not_exist` (404) for a well-formed absent EHR id, a weak
  `ETag` on `RESULT_SET` responses, parameter-substituted
  `meta._executed_aql`, and an optional query execution timeout
  (`EHRBASE_QUERY__TIMEOUT_MS`) mapped to `408`.
- Definition API: template list filtering (`template_id` glob, `concept`,
  `version`) and pagination are honoured; stored-query `query_type` is
  read with an honest unsupported-formalism rejection; ADL1.4 uploads
  return the JSON `TemplateIdentifier` under `Prefer: return=identifier`.
- FLAT/STRUCTURED (Simplified Formats, now STABLE): the `_`-prefixed
  optional RM attribute family (`_uid`, `_link`, `_feeder_audit`,
  `_null_flavour`, `_mapping`, `_normal_range`, participations, work-flow
  ids, …) round-trips in both directions; `|raw` canonical-JSON embedding
  on write; complete quantity/date-time/multimedia leaf attribute tables;
  `|other` open-value-set rules enforced.
- Development-edition ITS-REST protocol adopted (the server's tested
  contract identity, now reported consistently as such): `ETag` response
  headers carry the weak `W/"…"` indicator (bare quoted values are still
  accepted on `If-Match`); committal metadata uses the lowercase
  `openehr-version` / `openehr-audit-details` value-form headers (the
  deprecated `openEHR-VERSION.*` dotted spellings remain accepted) and a
  client-supplied `system_id` is merged into the commit audit; `Location`
  is emitted only on resource creation (no longer on reads/deletes);
  `Preference-Applied` echoes the honoured `Prefer`; `405`/`501` render
  the openEHR error body.
- Demographic DELETE follows the published Demographic API: the preceding
  version id rides in the path; a stale id yields `409` (with the latest
  version `ETag`), an already-deleted party `400`.
- Admin `DELETE /admin/ehr/all` follows the published Admin API: `204`
  with no body, and an absent `ehr_id` parameter now means delete ALL
  EHRs.
- FLAT duplicate node-name suffixes default to the specification form
  (`name_1`); the Better-compatible form (`name2`) is available behind the
  `ehrbase-quirks` feature.
- The `ehrbase-rest` and `ehrbase-sm` crates were restructured
  specification-first (one folder per ITS-REST spec / SM chapter, all
  spec-silent surfaces quarantined under `extensions/`) — no route
  changes beyond those listed here.
- `PUT …/composition/{uid_based_id}` rejects a body whose
  `COMPOSITION.uid` does not identify the versioned object addressed by
  the path (`400`).
- AQL semantic analysis is stricter per QUERY master03: duplicate FROM
  variable names reject, variable references are case-insensitive,
  `LIMIT 0`/negative `OFFSET` reject, `SUM`/`AVG` over non-numeric paths
  reject, scalar-function arity is validated, and `LIKE` `\*`/`\?`
  escapes now match the literal characters.
- OPT 1.4 template upload enforces the AOM 1.4 constraint-model invariants
  (attribute existence bounds, single-attribute occurrences, archetype-id
  well-formedness and root-type match, slot identifier validity,
  internal-reference target paths, constraint-reference definedness,
  boolean satisfiability, assumed-value validity, temporal and duration
  constraint-pattern validity, duplicate code-list codes) — invalid
  templates are rejected with `400` carrying the AOM rule code.
- ADL2 artefact upload (`I_DEFINITION_ADL2`) now validates sources against
  the registration-decidable AOM2 catalogue (mandatory sections, header
  versions, root type/node-id rules, specialisation depth, terminology
  language consistency, code definedness, value-set validity, term-binding
  keys) instead of a header-only probe — invalid sources are rejected with
  `422` carrying the AOM2 rule code.
- **Stricter spec-mandated validation** on the commit path: a client
  `AUDIT_DETAILS` with an empty `system_id`, a committer
  `PARTY_IDENTIFIED`/`PARTY_RELATED` with no identity, an empty committer
  name, or a `PARTY_RELATED.relationship` outside the openEHR
  `subject_relationship` group is now rejected with 422 (previously
  accepted, or surfaced as a 500 DB error); a non-root RM node carrying
  `archetype_details` violates `LOCATABLE.Archetyped_valid` and is
  rejected; EHR-Extract `versions[]` members with a `_type` other than
  `ORIGINAL_VERSION` are rejected on import.
- AQL `VERSION` `uid` values are now built from each version's stored
  `creating_system_id` and version-tree id, not the server's live
  `system_id` configuration.
- The `ehrbase-rs-postgres` image now pre-creates the layered group roles
  (`ehrbase_migrator`, `ehrbase_app`, `ehrbase_reader`), so Compose/dev
  deployments get the same least-privilege grant topology as hardened
  deployments instead of `roles absent` startup notices. Existing data
  volumes keep working; recreate the volume (or create the roles once by
  hand) to pick the grants up.
- Public documentation website at <https://rubentalstra.github.io/ehrbase-rs/>:
  a product landing page, a versioned user guide (frozen per release, `dev`
  tracking `develop`), and an offline OpenAPI endpoint reference covering all
  seven openEHR API groups. Built from `website/` and deployed by CI, with
  link-check and OpenAPI-drift gates.

### Fixed
- The composition validator no longer falsely rejects templates that use the
  same archetype more than once under one container, differentiated by name:
  each instance is now routed to the sibling constraint whose name it
  satisfies, instead of being checked against the first same-archetype
  sibling's overlay. Cross-contaminated content (a child from one overlay
  placed in the other-named instance) is still rejected.
- Template example generation (`GET …/example`) at `detail_level=medium` and
  `complete` no longer produces an empty composition for templates whose
  content is entirely optional: `medium` now returns a fully-populated
  single-instance committable example (honouring temporal patterns,
  C_DURATION field patterns, media-type code lists, and container
  cardinality bounds), and `complete` additionally demonstrates a second
  occurrence of repeating nodes. `required` (the default) is unchanged.
- AQL `SELECT c/uid/value` (and `c/uid`) on a COMPOSITION — or any
  versioned-object root — now returns the server-assigned
  `OBJECT_VERSION_ID`, version-correct under `LATEST_VERSION` and
  `ALL_VERSIONS`. It previously returned `null` because the uid was
  injected only on REST reads, never into stored data. (QUERY master03
  lists `COMPOSITION.uid.value` as a normative identified path.)
- Composition commits against an already-seen template no longer re-read the
  stored OPT from the database on every commit — the built WebTemplate cache
  is now consulted first (measured: 10,206 redundant reads in a 120 s load
  window, the #2 database statement by total time). Deleting a template now
  also evicts it from that cache, so a commit racing a delete gets the
  correct `422` ("template not known") instead of a foreign-key `500`.


- Template example generation (`GET /definition/template/adl1.4/{id}/example`)
  now honours the template's structural constraints: a missing mandatory
  ENTRY structure (e.g. `ACTION.description`) is synthesized with the
  template's constrained node (its RM type, `archetype_node_id`, and name)
  instead of a blind `at0001` placeholder, so generated examples validate
  and commit against the same template. Surfaced by the official openEHR
  CKM **International Patient Summary** template; probed by the new
  conformance case ECC-TPL-017 (example → commit round-trip).
- Template list endpoints no longer ignore filter and pagination
  parameters.
- The conformance manifest and `/rest/status` no longer misreport the
  implemented ITS-REST edition as `1.0.3`.
- Contribution commits: a creation version against an already-existing
  object, and a modification/deletion/attestation whose
  `preceding_version_uid` names an object the server does not hold, now
  return `400` (the contract's modification-type-mismatch scope) instead of
  `422`/`404` — on `POST /ehr/{ehr_id}/contribution`, `404` is reserved for
  an unknown `ehr_id`.
- Versioned-object reads (`GET …/versioned_composition`,
  `…/versioned_ehr_status`, versioned directory) now emit the concrete RM
  class (`VERSIONED_COMPOSITION` / `VERSIONED_EHR_STATUS` /
  `VERSIONED_FOLDER`) in `_type`, not the abstract `VERSIONED_OBJECT`.
- Demographic API: `If-Match` preconditions now verify the full
  `OBJECT_VERSION_ID` (previously only the version-tree number, which
  accepted phantom versions); relationship delete now honours the same
  `If-Match` preconditions as party delete; demographic `ETag`s are emitted
  in the weak form (`W/"…"`).

## [3.0.0] - 2026-07-11

First public release of **EHRbase-rs** — a pure-Rust openEHR Clinical Data
Repository. Version numbering starts at 3.0.0: this project began as a fork
of EHRbase (Java, 2.x line) and is released as its next-generation successor;
inherited upstream tags/releases were removed from the fork. Published as a
**pre-release**: the platform is feature-complete and conformance-verified,
but has not yet run in production.

### Added
#### openEHR platform
- openEHR REST API (ITS-REST 1.0.3): EHR, EHR_STATUS, COMPOSITION,
  DIRECTORY/FOLDER, CONTRIBUTION, QUERY, DEFINITION (ADL 1.4 + ADL2), admin
  and management surfaces, with canonical JSON **and** XML content
  negotiation. The wire contract is generated from the official openEHR
  OpenAPI/BMM/XSD models with a CI drift gate.
- AQL 1.1 query engine: typed path analysis over a spec-generated Reference
  Model compiled to PostgreSQL SQL; `LATEST_VERSION` **and** `ALL_VERSIONS`;
  terminology-backed `TERMINOLOGY()` expansion; stored parameterised queries.
- Full change-control semantics: contribution-atomic commits, indelible
  temporal version history (PostgreSQL 18 `WITHOUT OVERLAPS`), logical
  delete, attestations, per-version digital signatures (RFC 8785),
  point-in-time reads.
- Templates and validation: OPT 1.4 ingestion with artefact validity
  checking (AOM2 codes), WebTemplate / FLAT / STRUCTURED simplified formats,
  deep archetype-constraint validation on every commit.
- EHR Extract and messaging (SM I_EHR_EXTRACT/I_MESSAGE/I_TDD): whole-EHR
  export/import preserving distributed version identity, EHR cloning, TDD
  import.
- Demographics: versioned party store (PERSON, ORGANISATION, GROUP, AGENT,
  ROLE) with relationships.
- Terminology: the bundled openEHR terminology plus pluggable external FHIR
  terminology servers (validate / expand / subsume).
- Conformance instrument: the ECC runner executes the full catalogue (341
  cases, JSON + XML) against the composed server and computes profile
  verdicts — **CORE: PASS · STANDARD: PASS · OPTIONS: OBTAINED**, generating
  the Conformance Statement + Certificate.

#### Integration
- Change events: transactional outbox publishing every contribution commit
  to AMQP/RabbitMQ — at-least-once, per-EHR ordered, PHI-free envelopes,
  server-side filterable subscriptions (off by default).
- FHIR R4 connectors: mapping-driven inbound ingestion (validated
  compositions with FEEDER_AUDIT provenance), a read façade over AQL, and
  event-driven outbound resource emission (off by default).
- S3 multimedia externalization: threshold-based content-addressed offload
  of DV_MULTIMEDIA to any S3-compatible store with sha-256 integrity
  verification; SeaweedFS supported out of the box (off by default).

#### Security & operations
- Authentication: HTTP Basic (argon2) and OAuth2/OIDC bearer (Keycloak,
  Active Directory, any standards-compliant IdP).
- Authorization: RBAC plus ABAC via the embedded Cedar policy engine or a
  remote PDP.
- Multi-tenancy: each tenant an isolated logical openEHR system with its own
  `system_id`, enforced by PostgreSQL row-level security (off by default —
  single-tenant mode is unchanged).
- IHE ATNA system log: DICOM audit messages over (TLS) syslog with
  build-time operation coverage.
- Observability: structured logs, OpenTelemetry traces, Prometheus metrics,
  health probes; identified data never enters telemetry.
- Layered database roles (migrator / writer / reader) with a hardened
  PostgreSQL baseline.

#### Deployment
- Docker Compose stack (server + PostgreSQL 18) with an optional Grafana
  LGTM observability overlay.
- Distroless, non-root, shell-less multi-arch container images (amd64 +
  arm64) on GHCR.
- Helm chart with security-hardened defaults (non-root, read-only rootfs,
  seccomp, default-deny NetworkPolicy) and golden-render validation.


[unreleased]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.3.2...HEAD
[4.3.2]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.3.1...v4.3.2
[4.3.1]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.3.0...v4.3.1
[4.3.0]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.2.5...v4.3.0
[4.2.5]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.2.4...v4.2.5
[4.2.4]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.2.3...v4.2.4
[4.2.3]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.2.2...v4.2.3
[4.2.2]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.2.1...v4.2.2
[4.2.1]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.2.0...v4.2.1
[4.2.0]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.1.1...v4.2.0
[4.1.1]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.1.0...v4.1.1
[4.1.0]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.18...v4.1.0
[4.0.18]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.17...v4.0.18
[4.0.17]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.16...v4.0.17
[4.0.16]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.15...v4.0.16
[4.0.15]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.13...v4.0.15
[4.0.13]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.12...v4.0.13
[4.0.12]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.11...v4.0.12
[4.0.11]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.10...v4.0.11
[4.0.10]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.9...v4.0.10
[4.0.9]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.8...v4.0.9
[4.0.8]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.7...v4.0.8
[4.0.7]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.6...v4.0.7
[4.0.6]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.6-rc3...v4.0.6
[4.0.6-rc3]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.6-rc2...v4.0.6-rc3
[4.0.6-rc2]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.5...v4.0.6-rc2
[4.0.5]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.4...v4.0.5
[4.0.4]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.3...v4.0.4
[4.0.3]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.2...v4.0.3
[4.0.2]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.1...v4.0.2
[4.0.1]: https://github.com/FerroHEALTH/FerroEHR/compare/v4.0.0...v4.0.1
[4.0.0]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.20.0...v4.0.0
[3.20.0]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.19.0...v3.20.0
[3.19.0]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.18.0...v3.19.0
[3.18.0]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.17.8...v3.18.0
[3.17.8]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.17.7...v3.17.8
[3.17.7]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.17.6...v3.17.7
[3.17.6]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.17.5...v3.17.6
[3.17.5]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.17.4...v3.17.5
[3.17.4]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.17.3...v3.17.4
[3.17.3]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.17.2...v3.17.3
[3.17.2]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.17.1...v3.17.2
[3.17.1]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.17.0...v3.17.1
[3.17.0]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.16.0...v3.17.0
[3.16.0]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.15.3...v3.16.0
[3.15.3]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.15.2...v3.15.3
[3.15.2]: https://github.com/FerroHEALTH/FerroEHR/compare/v3.15.1...v3.15.2
[3.15.1]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.15.0...v3.15.1
[3.15.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.14.0...v3.15.0
[3.14.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.13.0...v3.14.0
[3.13.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.12.0...v3.13.0
[3.12.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.11.0...v3.12.0
[3.11.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.10.0...v3.11.0
[3.10.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.9.0...v3.10.0
[3.9.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.8.0...v3.9.0
[3.8.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.7.0...v3.8.0
[3.7.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.6.0...v3.7.0
[3.6.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.5.0...v3.6.0
[3.5.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.4.0...v3.5.0
[3.4.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.3.0...v3.4.0
[3.3.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.2.0...v3.3.0
[3.2.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.1.1...v3.2.0
[3.1.1]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.1.0...v3.1.1
[3.1.0]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.0.3...v3.1.0
[3.0.3]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.0.2...v3.0.3
[3.0.2]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.0.1...v3.0.2
[3.0.1]: https://github.com/rubentalstra/ehrbase-rs/compare/v3.0.0...v3.0.1
[3.0.0]: https://github.com/rubentalstra/ehrbase-rs/releases/tag/v3.0.0
