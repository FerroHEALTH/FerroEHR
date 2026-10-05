// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy-decision-point (PDP) seam: one async trait behind which the embedded [`CedarEngine`] and the
//! [`RemotePdp`] are interchangeable.
//!
//! The engine **owns** the multi-valued fan-out semantics: given an
//! [`AuthzRequest`] whose `patient`/`template` may be sets, it evaluates every
//! [`Combination`] (the cartesian product), requires **all** to permit, and
//! short-circuits on the first deny. Errors are **fail-closed**: the PEP maps
//! any [`AuthzError`] to `500` — fail-closed: an engine that cannot decide must
//! never be read as a permit.
//!
//! [`CedarEngine`]: crate::extensions::access::authz::cedar::CedarEngine
//! [`RemotePdp`]: crate::extensions::access::authz::remote::RemotePdp
//! [`Combination`]: crate::extensions::access::authz::request::Combination

use async_trait::async_trait;

use crate::extensions::access::authz::request::{AuthzRequest, Decision};

/// A policy engine failure. Always fail-closed at the PEP (→ 500), never a
/// permit.
#[derive(Debug, thiserror::Error)]
pub enum AuthzError {
    /// The remote PDP could not be reached (connect error / timeout / transport).
    #[error("policy engine unreachable: {0}")]
    Unreachable(String),
    /// A policy set failed to load or validate (Cedar boot / hot-reload).
    #[error("policy load failed: {0}")]
    PolicyLoad(String),
    /// Policy evaluation itself failed (malformed attributes / entity build).
    #[error("policy evaluation failed: {0}")]
    Evaluation(String),
}

/// The PDP seam. Both engines implement fan-out with identical semantics so a
/// deployment can swap them without behaviour change.
#[async_trait]
pub trait PolicyEngine: Send + Sync + std::fmt::Debug {
    /// Decide a fully-resolved request. Returns [`Decision::Deny`] on the first
    /// denying [`Combination`](crate::extensions::access::authz::request::Combination); [`Decision::Permit`]
    /// only when every combination permits (a request with no combinations —
    /// an empty result set — permits vacuously). Any failure is an
    /// [`AuthzError`] (fail-closed).
    async fn decide(&self, req: &AuthzRequest<'_>) -> Result<Decision, AuthzError>;
}
