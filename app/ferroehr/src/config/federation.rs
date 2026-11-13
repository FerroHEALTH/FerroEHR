// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the configuration trusts of each `federation` issuer.
//!
//! The federation agreements in the CDR are judged against it
//! ([`crate::party_binding::grants::agreement`]): whether it is a MyHealth@EU
//! contact point, where its `[[auth.federation]]` table reads a `chained`
//! request's correlation id from, and how long this node keeps its access
//! records. No openEHR spec governs configuration: our own design.

use std::collections::BTreeMap;

use ferrohealth_smart::config::{AuthSection, Resolved};
use ferrohealth_smart::federation::{Field, Source};
use ferrohealth_smart::kind::IssuerKind;

use crate::party_binding::grants::trust::{Correlation, FederationTrust, PartnerIssuer};
use crate::system_log::config::Retention;

/// The trust `auth` declares of its `federation` issuers, with this node's
/// access records kept for `holder`; none when `[auth]` does not resolve,
/// which the configuration check refuses first.
#[must_use]
pub fn trust(auth: &AuthSection, holder: Retention) -> FederationTrust {
    let Ok(resolved) = crate::config::auth::resolve(auth) else {
        return FederationTrust::new(BTreeMap::new(), holder);
    };
    let issuers = resolved
        .kinds
        .iter()
        .filter(|(_, kind)| **kind == IssuerKind::Federation)
        .map(|(issuer, _)| {
            let sources = correlation_sources(auth, issuer);
            let correlation = match sources {
                None => Correlation::None,
                Some(sources) if sources.iter().any(|s| matches!(s, Source::Constant(_))) => {
                    Correlation::Constant
                }
                Some(_) => Correlation::Token,
            };
            let partner = PartnerIssuer {
                contact_point: is_contact_point(&resolved, issuer),
                correlation,
                correlation_source: sources.map(|sources| {
                    sources
                        .iter()
                        .map(describe_source)
                        .collect::<Vec<_>>()
                        .join(" ")
                }),
            };
            (issuer.clone(), partner)
        })
        .collect();
    FederationTrust::new(issuers, holder)
}

/// Whether `issuer` is a MyHealth@EU contact point (`contact_point = true`).
fn is_contact_point(resolved: &Resolved, issuer: &str) -> bool {
    resolved.issuers.iter().any(|settings| {
        settings.issuer == issuer
            && settings
                .federation
                .as_ref()
                .is_some_and(|f| f.contact_point == Some(true))
    })
}

/// The sources the `[[auth.federation]]` table of `issuer` reads the
/// correlation id of a `chained` request from, when it maps any.
fn correlation_sources<'a>(auth: &'a AuthSection, issuer: &str) -> Option<&'a [Source]> {
    auth.federation
        .iter()
        .find(|table| table.issuer.expose() == issuer)
        .and_then(|table| table.fields.get(&Field::CorrelationId))
        .map(Vec::as_slice)
        .filter(|sources| !sources.is_empty())
}

/// How the access record names `source`: its kind and its path.
fn describe_source(source: &Source) -> String {
    match source {
        Source::Claim(path) => format!("claim:{path}"),
        Source::Constant(_) => "constant".to_owned(),
        Source::AuthorizationDetails(path) => format!("authorization_details:{path}"),
        Source::Presentation(path) => format!("presentation:{path}"),
    }
}
