---
paths: ["app/ferroehr-rest/**", "app/ferroehr/src/config/**", "app/ferroehr/src/**/security/**"]
---

# Authentication & authorization (rewritten 2026-07-13 — authn + access module are SHIPPED)

**State:** Basic and OAuth 2.0 bearer authentication run on
`ferrohealth-smart` (the crate every FerroHEALTH product shares; it checks
tokens and never issues them). Authorization lives in the
**`ferroehr-rest::access` module** (the RBAC/ABAC policy-enforcement point),
with **SMART on openEHR scope enforcement** (`ferroehr-rest::smart`)
AND-composed onto that PEP and always on while `[auth]` is on. This rule
governs maintenance and extension, not initial build.

## Spec authority

- ITS-REST §Authentication + SM `master02` place authorization largely **out
  of band** — fine-grained authz is our own extension where the spec is
  silent; flag it as such in comments ("no openEHR spec governs this"),
  never cite an internal doc (spec-adherence.md).
- **401 vs 403 discipline:** unauthenticated → 401, authenticated-but-
  unauthorized → 403, per the ITS-REST text — verified by the CNF security
  chapter (the `SEC-*` cases in the pinned instrument's catalogue), not by comparison with any other
  server. Public endpoints (`/rest/status`, health, discovery documents
  incl. `/.well-known/smart-configuration`) stay outside the auth layer.
- SMART scope grammar/enforcement: the vendored
  `docs/specs/openehr/ITS-REST/docs/smart_app_launch/` text (master08 is the
  load-bearing scope grammar). EHR-level access: the
  `ferroehr.access_control.v1` scheme implemented in
  `app/ferroehr/src/service/ehr/access_types.rs` (no openEHR spec defines a
  concrete ACCESS_CONTROL_SETTINGS scheme — our own design).

## Rules

- **Use `ferrohealth-smart`, never hand-roll crypto or token parsing:** its
  `Gate` checks bearer tokens (issuer metadata, JWKS, RFC 9068 profile) and
  Basic credentials (Argon2id), its `Requirement` and `openehr` module carry
  the SMART on openEHR grammar, and its `TokenSource` fetches outbound OAuth
  2.0 client-credentials tokens. Machine-to-machine authentication is HTTP
  Basic or the client credentials grant, never a static bearer token. A gap
  in the crate is fixed in the crate, never copied into FerroEHR.
  `secrecy`/`zeroize` for secret material.
- **Auth is a `tower`/axum middleware + extractor** on the generated router
  — one place, never per-handler. The authenticated principal goes into
  request extensions; the `access` PEP consumes it.
- **Config-driven** (one TOML file, `ferroehr.toml`, with environment-variable
  overrides): the crate's `[auth]` section (issuers, audience, the Basic user
  store, the product scope), FerroEHR's `[authz]` and `[smart]`.
  Config changes are user-visible → same-PR website-book + changelog entries.
- **Layer order is load-bearing:** authn → `access` (RBAC/ABAC) → SMART
  scopes (AND-composed — SMART can only narrow, never widen). `[smart]
  enabled` switches only the discovery document.
- Errors map through `openehr-its::rest::runtime::ApiError`; never invent
  an error body shape.
- Every auth change lands with tests (401/403 paths, valid/expired/wrong-
  audience tokens, Basic + bearer, scope-narrowing cases) and a CNF pipeline
  run showing zero drift — the SEC area covers this surface. A red SEC row is
  attributed spec-first (`.claude/rules/cnf-triage.md`); the server is never
  assumed correct.
