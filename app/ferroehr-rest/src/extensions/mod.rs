// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Extension surface — **nothing in this module is governed by ITS-REST**.
//!
//! Every module here is re-audited against the vendored openEHR specs and kept
//! only when a spec in the oracle does *not* in fact govern it. Each keeps
//! either an explicit "no openEHR spec governs this — our own design/extension"
//! flag or the precise citation of the spec that governs the *operation
//! semantics* it wraps (never a bare "extension" shrug).
//!
//! The quarantined spec-silent designs, each flagged at its own module:
//!
//! - [`access`] — authn (Basic/OAuth2/OIDC) + Cedar RBAC/ABAC authz + the PEP.
//!   ITS-REST places authorization out of band
//!   (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
//!   §Authentication and authorization); the `401`/`403`/`WWW-Authenticate`
//!   discipline there IS normative and is cited at the authn layer.
//! - [`health`] — the always-on public `/health`, `/health/liveness`,
//!   `/health/readiness` family, statuses only (no openEHR spec governs health
//!   probes; the ITS-REST System API defines only `OPTIONS /`).
//! - [`management`] — metrics/info/env/loggers/flamegraph, the health detail
//!   and the full status document, an ops-introspection surface behind
//!   per-endpoint access levels (no openEHR spec governs it). Hosts the single spec-version
//!   `provenance` source below (the module constants).
//! - [`openapi`] — serves the server's OWN `utoipa`-generated document (never a
//!   vendored OAS), not an API the spec itself defines.
//! - [`terminology`] — the `/terminology` wire: `I_TERMINOLOGY_SERVICE` is SM
//!   `master12`; the development-edition OAS set defines no terminology API, so
//!   the operation semantics are cited from SM and the wire shape is our own.
//! - [`event_subscription`] — eventing; nothing in SM/ITS-REST governs it.
//! - [`fhir`] — the RESTful-ATNA ITI-81 `AuditEvent` retrieval; the SM names
//!   `I_SYSTEM_LOG` as IHE ATNA-compliant and defines no wire.
//!
//! The ATNA audit middleware is NOT here — it realizes the SM System Log
//! component and lives at [`crate::system_log`].

pub mod access;
#[cfg(feature = "events")]
pub mod event_subscription;
#[cfg(feature = "fhir")]
pub mod fhir;
pub mod health;
pub mod management;
pub mod openapi;
pub mod terminology;

// The **single spec-version provenance source** every server *identity*
// surface reads: management `/info` ([`management::info`]), the System Options
// manifest (`OPTIONS /`) and `/status` all quote [`provenance::ITS_REST`], so
// the openEHR pins are stated once and cannot drift between endpoints. No
// openEHR spec governs an endpoint reporting the server's own spec/build
// provenance — our own operational surface. The ITS-REST identity is the
// released `Release-1.1.0`, the same tree the codegen consumes
// (`crates/openehr-its/vendor/rest-oas/PROVENANCE.md`).
