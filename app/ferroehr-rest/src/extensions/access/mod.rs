// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Access control for the ITS-REST surface.
//!
//! **Authentication** (who the caller is) and **authorization** (what they
//! may do), unified under one module.
//!
//! Spec grounding (`docs/specs/openehr/ITS-REST/.../Requests_and_responses.md`
//! §"Authentication and authorization"; SM `master02-overview.adoc` §General
//! Assumptions; CNF `profiles/master03-profiles.adoc`):
//!
//! - **Authentication is a `SHOULD`, not a `MUST`.** If a framework is present,
//!   the service MUST use `WWW-Authenticate` and return `401`/`403` as
//!   applicable: `401 Unauthorized` = missing/invalid credentials,
//!   `403 Forbidden` = authenticated but refused, valid credentials → the
//!   request proceeds to resource logic. That is the whole of what the released
//!   specification requires of this surface.
//! - **Fine-grained RBAC/ABAC is NOT spec-mandated.** No CNF profile
//!   (CORE/STANDARD/OPTIONS) carries any authentication/authorization/roles
//!   requirement, and the SM treats authorization as an out-of-band
//!   precondition. The RBAC and ABAC layers here are therefore **our own
//!   design**, in the vocabulary of NIST SP 800-162 (ABAC) and ANSI/INCITS
//!   359 Core RBAC, kept structurally separate from the spec-grounded
//!   authentication core so the boundary between the two is legible.
//!
//! Layout:
//! - [`authn`] — authentication: Basic (`argon2`) + OAuth2/OIDC bearer
//!   (`jsonwebtoken`/`openidconnect`), producing a [`authn::Principal`].
//! - [`ehr_access`] — the **spec-grounded** access-decision layer, built on the
//!   `EHR_ACCESS` gateway clause ("All access decisions to data in the EHR must
//!   be made in accordance with the policies and rules in this object" — RM
//!   `org.openehr.rm.ehr.ehr_access.adoc`). Always-on and foundational; runs
//!   first in the pre-dispatch chain.
//! - [`authz`] — our **own enterprise extensions** (no openEHR spec governs
//!   them; the SM places authorisation out of band, SM
//!   `openehr_platform/master02-overview.adoc` §General Assumptions): the coarse
//!   RBAC gate (runs in the authn middleware) + the fine-grained ABAC engine
//!   (Cedar / remote PDP, enforced by the PEP [`pep`]).
//! - [`pep`] — the policy-enforcement point: the ABAC gate **and** the
//!   spec-grounded **SMART** resource-scope + launch-context gate
//!   (`docs/specs/openehr/ITS-REST/docs/smart_app_launch/master08-scopes.adoc`
//!   §Resource Scopes; `master07-*.adoc` §Context Selection), the latter
//!   AND-composed after RBAC/Cedar.
//!
//! **Layering direction:** the spec-grounded [`ehr_access`] gate is the base;
//! RBAC → ABAC → the SMART scope gate compose *on top of* it as additive
//! restrictions (AND-composition — a request must clear the `EHR_ACCESS` gate
//! *and* any RBAC/ABAC policy *and* the SMART scope gate), never the reverse.
//! The specs lead; the enterprise layers build on them.
//!
//! The authn↔authz seam is the [`authn::Principal`]: authn resolves it, the `EHR_ACCESS`
//! gate + the RBAC gate + the ABAC/SMART PEP all consume it, and the single
//! 401/403 decision lives in `authn::middleware`.

pub mod authn;
pub mod authz;
pub mod ehr_access;
pub mod pep;

// The authn surface (identity + the request-scoped principal). The middleware
// + `AuthLayer` are `pub(crate)` and installed by the router via `authn::`
// directly, so they are not re-exported here.

// The spec-grounded EHR_ACCESS gate (the foundational access-decision layer).

// The authorization surface: the RBAC/ABAC handle and its config/engine seams.
