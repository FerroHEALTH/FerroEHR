// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The callers the configuration declares: the system clients that commit
//! (`[[authz.system_client]]`) and the agreements a verified external person
//! or organisation of a `federation` issuer is admitted under
//! (`[[authz.federation.agreement]]`), only within the agreement of its
//! issuer and accessor, and only with the scopes the agreement reads.
//!
//! Its rights come from the agreement and never from a role (GDPR Art. 5(2)
//! and Art. 30(1)(b); `docs/law/eu/gdpr/text.html`), and its reads are
//! `patient/` scopes, so the launch-context confinement of SMART on openEHR
//! `master08-scopes.adoc` §Resource Scopes holds it to its token-bound
//! patient. What an external caller may do is our own design.

use std::collections::BTreeMap;
use std::sync::Arc;

use ferroehr::config::agreement::{Accessor, Agreement};
use ferroehr::config::clients::SystemClient;
use ferroehr::versioning::committer::ExternalCaller;
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::resource::{Access, Caller};

use super::AuthError;

/// The agreement an external caller was admitted under, carried on its
/// [`Caller`] as an extension.
#[derive(Debug, Clone)]
pub(crate) struct Admitted(pub(crate) Arc<Agreement>);

/// The scope tokens an external caller may hold beside the agreement's
/// `reads`: the OpenID Connect Core §5.4 and SMART launch requests, which
/// grant no access here.
const NEUTRAL: [&str; 5] = ["openid", "profile", "fhirUser", "online_access", "launch"];

/// The `[[authz.system_client]]` tables and the
/// `[[authz.federation.agreement]]` tables, by issuer and accessor.
#[derive(Debug, Default)]
pub(super) struct Declared {
    /// The system clients.
    pub(super) system_clients: Vec<Arc<SystemClient>>,
    /// The agreements, each with the jurisdiction it holds for; boot has
    /// refused a table without an accessor.
    agreements: BTreeMap<(String, Accessor), Arc<Agreement>>,
}

impl Declared {
    /// Returns the tables `config` declares.
    pub(super) fn of(config: &crate::config::AppConfig) -> Self {
        Self {
            system_clients: config
                .system_clients
                .iter()
                .cloned()
                .map(Arc::new)
                .collect(),
            agreements: config
                .agreements
                .iter()
                .filter_map(|table| {
                    let accessor = table.accessor?;
                    let held = Agreement {
                        jurisdiction: table.jurisdiction_of(&config.auth.shared),
                        ..table.clone()
                    };
                    Some(((table.issuer.clone(), accessor), Arc::new(held)))
                })
                .collect(),
        }
    }

    /// Admits the caller of `access`: an external caller only under the
    /// agreement of its issuer and accessor, holding no scope beyond its
    /// `reads`, returned carrying [`Admitted`]; any other caller unchanged.
    ///
    /// # Errors
    /// [`AuthError::KindRefused`] (`403`) for an external caller without an
    /// agreement for its accessor kind, or holding a scope the agreement does
    /// not read.
    pub(super) fn admit(&self, access: Access) -> Result<Access, AuthError> {
        let Some(caller) = access.caller() else {
            return Ok(access);
        };
        let accessor = match caller.kind() {
            CallerKind::ExternalPerson(_) => Accessor::Person,
            CallerKind::ExternalOrganisation(_) => Accessor::Organisation,
            _ => return Ok(access),
        };
        let agreement = caller
            .issuer()
            .and_then(|issuer| self.agreements.get(&(issuer.to_owned(), accessor)))
            .ok_or_else(|| AuthError::KindRefused {
                code: "refused-no-agreement",
                message: format!(
                    "no [[authz.federation.agreement]] of this issuer admits the accessor \
                     {accessor}"
                ),
                principal: None,
            })?;
        let beyond = caller.scope().iter().any(|scope| {
            !NEUTRAL.contains(&scope.as_str())
                && !scope.starts_with("launch/")
                && !agreement.reads.iter().any(|read| read == scope)
        });
        if beyond {
            return Err(AuthError::KindRefused {
                code: "refused-scope-beyond-agreement",
                message: "an external caller holds only the patient/ scopes its agreement reads"
                    .to_owned(),
                principal: None,
            });
        }
        Ok(Access::Caller(Arc::new(
            Caller::clone(caller).with_extension(Admitted(Arc::clone(agreement))),
        )))
    }
}

/// The agreement the caller of `access` was admitted under, when it is an
/// external caller.
pub(crate) fn admitted(access: &Access) -> Option<&Arc<Agreement>> {
    access
        .caller()
        .and_then(Caller::extension::<Admitted>)
        .map(|admitted| &admitted.0)
}

/// What the external caller of `access` may commit, when it is one.
pub(super) fn committing(access: &Access) -> Option<ExternalCaller> {
    let agreement = admitted(access)?;
    Some(match agreement.accessor {
        Some(Accessor::Person) => ExternalCaller::Person {
            writes: agreement.writes.clone(),
        },
        _ => ExternalCaller::Organisation,
    })
}
