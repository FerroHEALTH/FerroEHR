// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the federation agreements are judged against, and how one in force
//! is listed.
//!
//! [`FederationTrust`] is what the configuration trusts of each `federation`
//! issuer and how long this node keeps its access records
//! ([`crate::party_binding::grants::agreement::Agreement::refusals`]);
//! [`AgreementPosture`] is an agreement as the boot line and
//! `/management/info` state it. No openEHR spec governs a data-sharing
//! agreement: our own design.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::party_binding::grants::agreement::Accessor;
use crate::system_log::config::Retention;

/// Where a `federation` issuer's `[[auth.federation]]` table reads a
/// `chained` request's correlation id from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Correlation {
    /// It maps no `correlation_id`.
    #[default]
    None,
    /// It maps one from a constant, which no token attests.
    Constant,
    /// It reads one from the token.
    Token,
}

/// What the configuration trusts of one `federation` issuer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartnerIssuer {
    /// Whether it is a MyHealth@EU contact point (`contact_point = true`).
    pub contact_point: bool,
    /// Where it reads a `chained` request's correlation id from.
    pub correlation: Correlation,
    /// The sources of that id as the access record names them, joined by a
    /// space.
    pub correlation_source: Option<String>,
}

/// What the configuration trusts of the `federation` issuers, and how long
/// this node keeps its access records: what the agreements are judged
/// against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FederationTrust {
    issuers: BTreeMap<String, PartnerIssuer>,
    holder: Retention,
}

impl FederationTrust {
    /// The trust of `issuers`, each `federation` issuer by its URL, with the
    /// access records kept for `holder`.
    #[must_use]
    pub fn new(issuers: BTreeMap<String, PartnerIssuer>, holder: Retention) -> Self {
        Self { issuers, holder }
    }

    /// What is trusted of the `federation` issuer `issuer`.
    #[must_use]
    pub fn issuer(&self, issuer: &str) -> Option<&PartnerIssuer> {
        self.issuers.get(issuer)
    }

    /// How long this node keeps its access records.
    #[must_use]
    pub fn holder(&self) -> Retention {
        self.holder
    }
}

/// One federation agreement in force, as the boot line and
/// `/management/info` state it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgreementPosture {
    /// The partner's issuer, its URL `userinfo` masked.
    pub issuer: String,
    /// The partner's client.
    pub client_id: String,
    /// Who it admits.
    pub accessor: Accessor,
    /// Its name.
    pub name: String,
    /// Its legal ground, rendered.
    pub legal_ground: Option<String>,
}
