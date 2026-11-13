// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Who may import, and which wrapped committers an import may carry.
//!
//! An `IMPORTED_VERSION` has "its own audit trail and Contribution, distinct
//! from those of the imported `ORIGINAL_VERSION`"
//! (`org.openehr.rm.common.imported_version.adoc`), and the imported original
//! "remains a faithful copy of its original" (RM common master06 §Copying), so
//! the wrapped `commit_audit` is stored as received. An import could then
//! make a version read as a local colleague's original act, so a wrapped
//! committer naming a local `VERSIONED_PARTY`, or carrying a login identifier
//! of this deployment's issuers, is refused unless the version's
//! `creating_system_id` is a system the deployment declares trusted. Importing
//! needs the declared import role, a policy grant of the import's operation,
//! or a `system/` caller. No openEHR spec governs who may import: our own
//! design (#5005, #5438).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use std::collections::BTreeSet;

use serde_json::Value;

use crate::error::ServiceError;
use crate::ids::VoId;
use crate::party_binding::grants::capabilities::Capabilities;
use crate::party_binding::grants::operation::Operation;
use crate::service::FerroEhrService;
use crate::versioning::import::ImportContainer;

/// The import rules of the deployment.
#[derive(Debug, Clone, Default)]
pub struct ImportPolicy {
    /// Whether the rules are in force; the default policy admits every
    /// import.
    enforced: bool,
    /// `[authz.rbac] import_role`.
    role: Option<String>,
    /// `[authz.import] trusted_systems`.
    trusted: BTreeSet<String>,
    /// The issuers whose logins a wrapped committer may not carry: every
    /// trusted issuer, its `identifier_issuer`, and `ferroehr`.
    issuers: BTreeSet<String>,
}

impl ImportPolicy {
    /// Returns the policy importing under `role`, trusting `trusted` systems,
    /// with `issuers` the deployment's own login issuers.
    #[must_use]
    pub fn new(
        role: Option<String>,
        trusted: impl IntoIterator<Item = String>,
        issuers: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut issuers: BTreeSet<String> = issuers.into_iter().collect();
        issuers.insert("ferroehr".to_owned());
        Self {
            enforced: true,
            role,
            trusted: trusted.into_iter().collect(),
            issuers,
        }
    }
}

impl FerroEhrService {
    /// Installs the import rules.
    #[must_use]
    pub fn with_import_policy(mut self, policy: ImportPolicy) -> Self {
        self.import_policy = std::sync::Arc::new(policy);
        self
    }

    /// The containers and parties of `extract`, once their import as
    /// `operation` by the request's caller is admitted
    /// ([`Self::admit_import`]).
    ///
    /// # Errors
    /// The parse refusals of an extract and the [`Self::admit_import`] ones.
    pub(super) async fn admitted(
        &self,
        extract: &openehr_rm::prelude::Extract,
        operation: Operation,
    ) -> Result<(Vec<ImportContainer>, Vec<ImportContainer>), ServiceError> {
        let (containers, parties) = super::import::parse_import_containers(extract)?;
        self.admit_import(&containers, &parties, operation).await?;
        Ok((containers, parties))
    }

    /// Admits an import of `containers` and `parties`, the operation
    /// `operation`, by the request's caller.
    ///
    /// # Errors
    /// [`ServiceError::Forbidden`] (`403`) when the caller holds neither the
    /// import role, a policy grant of `operation`, nor a `system/` grant, or a
    /// wrapped committer names a local party or login from an untrusted
    /// system; a storage failure.
    pub(super) async fn admit_import(
        &self,
        containers: &[ImportContainer],
        parties: &[ImportContainer],
        operation: Operation,
    ) -> Result<(), ServiceError> {
        let policy = &self.import_policy;
        if !policy.enforced {
            return Ok(());
        }
        if let Some(identity) = crate::versioning::committer::current_committer() {
            let role = policy.role.as_deref().is_some_and(|role| {
                crate::system_log::access_context::current_roles()
                    .iter()
                    .any(|held| held.eq_ignore_ascii_case(role))
            });
            let granted = Capabilities::current().holds(operation);
            if !(role || granted || identity.system) {
                return Err(ServiceError::not_permitted(format!(
                    "importing needs the role [authz.rbac] import_role names, a policy grant \
                     of {operation}, or a system/ caller"
                )));
            }
        }
        let mut committers: Vec<&Value> = Vec::new();
        for container in containers.iter().chain(parties) {
            for version in &container.versions {
                if policy.trusted.contains(&version.creating_system_id) {
                    continue;
                }
                committers.extend(version.commit_audit.get("committer"));
                for attestation in &version.attestations {
                    committers.extend(attestation.get("committer"));
                }
            }
        }
        for committer in committers {
            self.refuse_local_committer(committer).await?;
        }
        Ok(())
    }

    /// Refuses a wrapped `committer` whose `external_ref` names a local
    /// `VERSIONED_PARTY` or which carries a login of this deployment.
    async fn refuse_local_committer(&self, committer: &Value) -> Result<(), ServiceError> {
        let login = committer
            .get("identifiers")
            .and_then(Value::as_array)
            .is_some_and(|ids| {
                ids.iter().any(|id| {
                    id.get("issuer")
                        .and_then(Value::as_str)
                        .is_some_and(|issuer| self.import_policy.issuers.contains(issuer))
                })
            });
        // NOTE: a reference whose root is no UUID names no container of this store.
        let reference = committer
            .pointer("/external_ref/id/value")
            .and_then(Value::as_str)
            .and_then(|id| VoId::from_uid_head(id).ok());
        let local = match reference {
            Some(id) => {
                let found: Option<i32> = sqlx::query_scalar(
                    "SELECT 1 FROM vo_head WHERE vo_id = $1 \
                     AND kind IN ('PERSON', 'ORGANISATION', 'GROUP', 'AGENT', 'ROLE')",
                )
                .bind(id)
                .fetch_optional(&self.demographic_pool)
                .await?;
                found.is_some()
            }
            None => false,
        };
        if login || local {
            return Err(ServiceError::not_permitted(
                "a wrapped AUDIT_DETAILS.committer names a party or a login of this deployment, \
                 and its version was created by no system [authz.import] trusted_systems lists",
            ));
        }
        Ok(())
    }
}
