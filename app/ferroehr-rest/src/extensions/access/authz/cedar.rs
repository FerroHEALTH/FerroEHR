// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The embedded Cedar engine over the Cedar admin register.
//!
//! The policy set is the Cedar register in force
//! ([`ferroehr::policy_register`]), which the request path reads again before
//! every operation, so a new version decides the next request. A set is
//! parsed and validated against the engine's schema when it is written
//! ([`check`]), so a set that does not hold never reaches this engine; a
//! stored version that stops validating under a later schema leaves the set
//! in force, and is logged. No openEHR spec governs authorization: our own
//! design.

use std::sync::{Arc, PoisonError, RwLock};

use ferroehr::policy_register::{CedarCheck, LiveRegisters};
use ferrohealth_authz::cedar::CedarEngine;
use ferrohealth_authz::engine::{AuthzError, PolicyEngine};
use ferrohealth_authz::request::{Decision, Request};

use crate::extensions::access::authz::request;

/// The check a Cedar policy set passes before the register stores it: it
/// parses and validates strictly against the schema of [`request::model`].
#[must_use]
pub fn check() -> CedarCheck {
    Arc::new(|policies: &str| {
        CedarEngine::from_source(request::model(), policies)
            .map(drop)
            .map_err(|error| chain(&error))
    })
}

/// `error` and every cause under it, as one line.
fn chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        text.push_str(": ");
        text.push_str(&next.to_string());
        cause = next.source();
    }
    text
}

/// The engine of the Cedar register in force, rebuilt when the register gains
/// a version.
#[derive(Debug)]
pub struct RegisterCedar {
    registers: Arc<LiveRegisters>,
    /// The engine built, with the register version it was built from.
    built: RwLock<(i64, Arc<CedarEngine>)>,
}

impl RegisterCedar {
    /// The engine of the Cedar register `registers` holds in force.
    ///
    /// # Errors
    /// [`AuthzError::Load`] when that set does not validate.
    pub fn new(registers: Arc<LiveRegisters>) -> Result<Self, AuthzError> {
        let cedar = registers.cedar();
        let engine = CedarEngine::from_source(request::model(), &cedar.policies.policies)?;
        Ok(Self {
            registers,
            built: RwLock::new((cedar.version, Arc::new(engine))),
        })
    }

    /// The engine of the register version in force.
    fn engine(&self) -> Arc<CedarEngine> {
        let cedar = self.registers.cedar();
        let (version, engine) = self
            .built
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if version == cedar.version {
            return engine;
        }
        let engine = match CedarEngine::from_source(request::model(), &cedar.policies.policies) {
            Ok(rebuilt) => Arc::new(rebuilt),
            Err(error) => {
                tracing::error!(
                    version = cedar.version,
                    error = %chain(&error),
                    "the Cedar register version does not validate; the policy set in force stays"
                );
                engine
            }
        };
        *self.built.write().unwrap_or_else(PoisonError::into_inner) =
            (cedar.version, Arc::clone(&engine));
        engine
    }
}

#[async_trait::async_trait]
impl PolicyEngine for RegisterCedar {
    async fn decide(&self, request: &Request<'_>) -> Result<Decision, AuthzError> {
        self.engine().decide(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_that_does_not_parse_or_validate_is_refused() {
        let check = check();
        assert_eq!(check(""), Ok(()));
        assert!(check("this is not a cedar policy").is_err());
        let unknown = check("permit(principal, action == Action::\"no_such_action\", resource);")
            .expect_err("an undeclared action does not validate");
        assert_ne!(unknown, "", "the refusal says why");
    }
}
