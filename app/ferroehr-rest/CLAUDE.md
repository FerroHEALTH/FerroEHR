# `ferroehr-rest` — the ITS-REST protocol adapter

Implements the **generated** ITS-REST contract (server traits + DTOs from
`openehr-its`) over axum 0.8, calling the concrete `FerroEhrService` directly.
Modules (`src/`): `api` (one impl group per API area —
ehr/definition/demographic/query/admin/system, each group carrying its own
spec-flagged extension routes beside the released ones: `demographic::relationship`,
`definition::archetype`, `admin::{archive, report, dump_load}` — plus `api::message`,
a group that is extension ALL THE WAY DOWN: the release publishes no
message/extract/TDD API at all, so `message::{extract, tdd}` mount SM
`I_EHR_EXTRACT_SERVICE` / `I_TDD_SERVICE` under a `/message` resource root of
our own, on the ordinary clinical authentication class rather than the admin
gate), `router`, `state`, `formats`
(content negotiation), `overview`, `overload` (shed), `config`, `system_log`
(ATNA audit middleware), `smart` (SMART App Launch resource-server role), and
`extensions` (`access` = the RBAC/ABAC authn+authz PEP, plus health / fhir /
terminology / management / event-subscription surfaces). Entry point:
`serve_full`.

- **The health family is always-on and ungated** (`extensions::health`:
  `/health`, `/health/liveness`, `/health/readiness`), mounted outside the API
  subtree — no auth, no audit, no overload shed, no config switch — and says
  as little as possible: readiness carries each indicator's name and status,
  never its detail. **Everything reachable without a credential says only what
  a client or probe needs** (owner decision 2026-10-06): the detail lives on the
  `/management` surface behind its per-endpoint access levels —
  `/management/health` (every indicator with its detail) and
  `/management/status` (licence, deployment gaps, support period; the public
  `{rest root}/status` keeps status, versions, timestamp and the deployment
  profile only), beside info/prometheus/metrics/env/loggers/flamegraph. No
  openEHR spec governs either — our own operational surface.
  `/management/flamegraph` is the on-demand CPU profiler (pprof sampling →
  SVG; `extensions/management/flamegraph.rs`, caps in
  `ferroehr::config::management::ProfilingConfig`) — the `/flamegraph` skill
  documents all three profiling instruments.

- **The wire is the spec:** status codes, headers (`ETag`, `Location`,
  `Last-Modified`, `Prefer`, committal merge), and content negotiation
  (canonical JSON + XML via `openehr-its`) come from the ITS-REST **docs text**
  `docs/specs/openehr/ITS-REST/` — never invented. The vendored released OAS
  is subordinate (owner rulings 2026-07-24 + 2026-07-28): where it and the
  docs text disagree, the text wins; where the docs text is silent, the OAS
  grounds the behaviour (overview `Specifications.md` presents the OAS files
  as the release's computable artifacts).
  Cross-check the CNF schedule; the CNF pipeline is the acceptance instrument.
- **Implement the generated traits; never fork the contract.** If the contract
  is wrong, fix `openehr-codegen -- emit-rest` and regenerate — never hand-edit
  `src/rest/generated/` in `openehr-its`.
- **Authz/authn** live in `extensions::access` (`authn/` = Basic + JWT, plus
  `authn::assurance`, the patient-data gate over `[auth.oidc.assurance]` and
  `[auth.oidc.professional]`: RFC 9470 step-up `401`, natural-person `403`,
  judged before RBAC, and the `ActorAuthentication` the audit layer records;
  `authz/` = the RBAC/ABAC engine, incl. a `cedar` policy path; `pep.rs` = the
  policy-enforcement point; `ehr_access.rs`). Use the pinned crates
  (`jsonwebtoken`, `oauth2`, `openidconnect`, `argon2`) — never hand-rolled.
  401 (unauthenticated) vs 403 (unauthorized) per ITS-REST. Rules:
  `.claude/rules/auth.md`.
- **SMART** (`smart/`): master08 scope grammar; enforcement AND-composes onto
  the ABAC PEP and is config-gated off by default (zero wire drift when
  disabled). Spec: `docs/specs/openehr/ITS-REST/docs/smart_app_launch/`.
- **OpenAPI:** serve ONLY our own `utoipa`-generated document
  (`extensions::openapi`); the vendored OAS is the `emit-rest` codegen input
  ONLY (stalled — NOT a behavioural oracle; the ITS-REST docs text is the
  oracle), never served.
- URL/percent encoding ONLY via the `urlencoding` crate (owner hard rule).
- **One integration-test binary:** `tests/it/main.rs` + one `mod` per topic
  file, with the shared router/DB fixture in `tests/it/common/mod.rs` (topic
  modules reach it as `use crate::common;`). Add a `mod` line — never a new
  top-level `tests/*.rs`.
- Rules: `.claude/rules/rest-axum.md`, `.claude/rules/auth.md`,
  `.claude/rules/serialization.md`.
- Gates: `cargo clippy -p ferroehr-rest --all-targets` +
  `cargo nextest run -p ferroehr-rest` green; wire changes re-verified by a CNF
  pipeline run (`bash scripts/conformance.sh`) with zero drift. A red row is
  attributed spec-first (`.claude/rules/cnf-triage.md`): the wire spec decides,
  this server is never assumed correct, and the catalogue/runner are never bent
  to match it.
