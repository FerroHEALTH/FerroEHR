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
//! needs the declared import role, or a grant of the import operation in the
//! policy parties (`import_ehr`, `import_ehr_extract`) covering the subject
//! namespace of every EHR it touches; exporting by a system caller needs an
//! `export_ehr_extract` grant covering the EHR the same way. No openEHR spec
//! governs who may import or export: our own design (#5005, #5432).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use std::collections::BTreeSet;

use serde_json::Value;

use crate::error::ServiceError;
use crate::ids::{EhrId, VoId};
use crate::party_binding::grants::capabilities::Capabilities;
use crate::party_binding::grants::operation::Operation;
use crate::service::FerroEhrService;
use crate::versioning::Kind;
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

    /// The containers and parties of `extract`, once their import by the
    /// request's caller is admitted ([`Self::admit_import`]): an
    /// `import_ehr` into a new EHR, or an `import_ehr_extract` into the
    /// existing EHR `target`.
    ///
    /// # Errors
    /// The parse refusals of an extract and the [`Self::admit_import`] ones.
    pub(super) async fn admitted(
        &self,
        extract: &openehr_rm::prelude::Extract,
        target: Option<EhrId>,
    ) -> Result<(Vec<ImportContainer>, Vec<ImportContainer>), ServiceError> {
        let operation = match target {
            None => Operation::ImportEhr,
            Some(_) => Operation::ImportEhrExtract,
        };
        let (containers, parties) = super::import::parse_import_containers(extract)?;
        self.admit_import(&containers, &parties, operation, target)
            .await?;
        Ok((containers, parties))
    }

    /// Admits an import of `containers` and `parties`, under `operation` into
    /// the EHR `target` when it exists, by the request's caller.
    ///
    /// # Errors
    /// [`ServiceError::Forbidden`] (`403`) when the caller holds neither the
    /// import role nor a grant of `operation` covering the subject namespace
    /// of the EHR, or a wrapped committer names a local party or login from
    /// an untrusted system; a storage failure.
    pub(super) async fn admit_import(
        &self,
        containers: &[ImportContainer],
        parties: &[ImportContainer],
        operation: Operation,
        target: Option<EhrId>,
    ) -> Result<(), ServiceError> {
        let policy = &self.import_policy;
        if !policy.enforced {
            return Ok(());
        }
        if crate::versioning::committer::current_committer().is_some() {
            let role = policy.role.as_deref().is_some_and(|role| {
                crate::system_log::access_context::current_roles()
                    .iter()
                    .any(|held| held.eq_ignore_ascii_case(role))
            });
            if !role {
                let mut namespaces = imported_namespaces(containers);
                if let Some(target) = target {
                    namespaces.extend(self.subject_namespace(target).await?);
                }
                let namespaces: Vec<&str> = namespaces.iter().map(String::as_str).collect();
                if !Capabilities::current().covers(operation, &namespaces) {
                    return Err(ServiceError::not_permitted(format!(
                        "importing needs the role [authz.rbac] import_role names, or a grant of \
                         ferroehr-operations::{operation} whose subject_namespace elements \
                         name the subject namespace of the EHR"
                    )));
                }
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

    /// Admits an export of the EHR `ehr_id` by the request's caller: a system
    /// caller only under an `export_ehr_extract` grant covering the EHR's
    /// subject namespace, any other caller by the access checks it already
    /// met; then the EHR's own marks ([`Self::refuse_objected_export`]).
    ///
    /// # Errors
    /// [`ServiceError::Forbidden`] (`403`) for a system caller without such a
    /// grant; the refusals of [`Self::refuse_objected_export`]; a storage
    /// failure.
    pub(super) async fn admit_export(&self, ehr_id: EhrId) -> Result<(), ServiceError> {
        if !crate::versioning::committer::current_committer().is_some_and(|caller| caller.system) {
            return self.refuse_objected_export(ehr_id).await;
        }
        let namespace = self.subject_namespace(ehr_id).await?;
        let namespaces: Vec<&str> = namespace.iter().map(String::as_str).collect();
        if Capabilities::current().covers(Operation::ExportEhrExtract, &namespaces) {
            return self.refuse_objected_export(ehr_id).await;
        }
        Err(ServiceError::not_permitted(
            "a system caller exports an EHR only under a grant of \
             ferroehr-operations::export_ehr_extract whose subject_namespace elements name the \
             subject namespace of the EHR",
        ))
    }

    /// The subject namespace of the EHR `ehr_id`'s current `EHR_STATUS`;
    /// `None` for an EHR whose subject names none.
    async fn subject_namespace(&self, ehr_id: EhrId) -> Result<Option<String>, ServiceError> {
        let namespace: Option<Option<String>> =
            sqlx::query_scalar("SELECT subject_namespace FROM ehr WHERE id = $1")
                .bind(ehr_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(namespace.flatten())
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

/// The subject namespaces the `EHR_STATUS` versions among `containers` name,
/// `EHR_STATUS.subject.external_ref.namespace`, distinct.
fn imported_namespaces(containers: &[ImportContainer]) -> BTreeSet<String> {
    containers
        .iter()
        .filter(|container| container.kind == Kind::EhrStatus)
        .flat_map(|container| &container.versions)
        .filter_map(|version| {
            version
                .data
                .pointer("/subject/external_ref/namespace")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}
