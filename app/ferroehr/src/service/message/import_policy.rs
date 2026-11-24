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
//! `creating_system_id` is a system the importing caller's grant vouches for:
//! a `creating_system` element of its grant of the import operation in
//! [`crate::party_binding::policy_mark::TERMINOLOGY`], read per request from
//! [`Capabilities::current`], and never this node's own system id. Importing
//! needs a grant of the import's operation (`import_ehr`,
//! `import_ehr_extract`) covering the subject namespace of the EHR it
//! touches, or holding `no_subject_namespace` for a subject with none;
//! exporting by a system caller needs an `export_ehr_extract` grant covering
//! the EHR the same way. No openEHR spec governs who may import or export:
//! our own design (#5005, #5432, #5438, #5439, #5521, #5530). An imported
//! `EHR_ACCESS` directive names only parties this CDR or the extract holds
//! (#5517).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use std::collections::BTreeSet;

use serde_json::Value;

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::{EhrId, VoId};
use crate::party_binding::grants::caller::FactValue;
use crate::party_binding::grants::caller::store;
use crate::party_binding::grants::capabilities::Capabilities;
use crate::party_binding::grants::care::care_of;
use crate::party_binding::grants::derive::GrantFacts;
use crate::party_binding::grants::operation::{CREATING_SYSTEM, Operation};
use crate::service::FerroEhrService;
use crate::service::ehr::access_parties::Landing;
use crate::storage::access_projection::EhrSubject;
use crate::versioning::Kind;
use crate::versioning::import::ImportContainer;

/// The import rules of the deployment.
#[derive(Debug, Clone, Default)]
pub struct ImportPolicy {
    /// Whether the rules are in force; the default policy admits every
    /// import.
    enforced: bool,
    /// The issuers whose logins a wrapped committer may not carry: every
    /// trusted issuer and its `identifier_issuer`, beside the service's own
    /// Basic issuer (`[api] basic_issuer`).
    issuers: BTreeSet<String>,
}

impl ImportPolicy {
    /// Returns the policy with `issuers` the deployment's own login issuers.
    #[must_use]
    pub fn new(issuers: impl IntoIterator<Item = String>) -> Self {
        let issuers: BTreeSet<String> = issuers.into_iter().collect();
        Self {
            enforced: true,
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
    /// request's caller as `operation` is admitted ([`Self::admit_import`]).
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
        self.check_imported_directive_parties(&containers, &parties)
            .await?;
        Ok((containers, parties))
    }

    /// Refuses an import whose head `EHR_ACCESS` version, the one the import
    /// puts in force, has a directive naming a party that neither the
    /// demographic store nor the extract's demographics chapter holds
    /// ([`Self::check_directive_parties`]). An earlier version is the record of
    /// what was in force then and decides nothing, so a party erased since
    /// never blocks a faithful import.
    ///
    /// # Errors
    /// The refusals of [`Self::check_directive_parties`].
    async fn check_imported_directive_parties(
        &self,
        containers: &[ImportContainer],
        parties: &[ImportContainer],
    ) -> Result<(), ServiceError> {
        let landing: Landing = parties
            .iter()
            .map(|party| (party.vo_id, party.kind.as_str().to_owned()))
            .collect();
        for container in containers.iter().filter(|c| c.kind == Kind::EhrAccess) {
            // NOTE: the import replays a container's versions in this order, so the last is its head.
            let head = container
                .versions
                .iter()
                .max_by_key(|v| (v.lineage(), v.tree.columns()));
            if let Some(settings) = head
                .and_then(|v| v.data.get("settings"))
                .filter(|s| !s.is_null())
            {
                self.check_directive_parties(settings, &landing).await?;
            }
        }
        Ok(())
    }

    /// Refuses an import one of whose COMPOSITION versions is placed in an
    /// episode that no care relationship of the EHR's subject names, in the
    /// demographic store or the extract's demographics chapter (draft access
    /// control package, Episodes of Care: declared episodes). The subject is
    /// the one the import's head `EHR_STATUS` names, else the target EHR's;
    /// nothing is read when no version is placed.
    ///
    /// # Errors
    /// `422` naming the element for a malformed placement or an undeclared
    /// episode; a storage failure.
    pub(super) async fn check_imported_episodes(
        &self,
        containers: &[ImportContainer],
        parties: &[ImportContainer],
        target: Option<EhrId>,
    ) -> Result<(), ServiceError> {
        let mut placed = BTreeSet::new();
        for container in containers.iter().filter(|c| c.kind == Kind::Composition) {
            for version in container.versions.iter().filter(|v| !v.data.is_null()) {
                placed.extend(crate::versioning::episode::placed(&version.data)?);
            }
        }
        if placed.is_empty() {
            return Ok(());
        }
        let imported = containers
            .iter()
            .filter(|c| c.kind == Kind::EhrStatus)
            .filter_map(|c| {
                c.versions
                    .iter()
                    .max_by_key(|v| (v.lineage(), v.tree.columns()))
            })
            .find_map(|head| {
                let at = |pointer: &str| {
                    head.data
                        .pointer(pointer)
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                };
                // NOTE: RM EHR Extract IM §Creation Semantics, a `local` reference names a
                // party of the extract's demographics chapter, held under its own container.
                let namespace = at("/subject/external_ref/namespace").map(|ns| {
                    if ns == LOCAL {
                        self.grants.namespace().to_string()
                    } else {
                        ns
                    }
                });
                EhrSubject::stored(namespace, at("/subject/external_ref/id/value"))
            });
        let subject = match (imported, target) {
            (Some(subject), _) => Some(subject),
            (None, Some(ehr_id)) => {
                crate::storage::ehr_repo::ehr_subject_columns(&self.pool, ehr_id)
                    .await?
                    .and_then(|(id, namespace)| EhrSubject::stored(namespace, id))
            }
            (None, None) => None,
        };
        let person = match subject {
            Some(subject) => self.grants.person_of(&subject).await?,
            None => None,
        };
        let mut declared = BTreeSet::new();
        if let Some(person) = person {
            let rows = {
                let mut conn = acquire(&self.demographic_pool).await?;
                store::care_of_subject(&mut conn, person).await?
            };
            declared.extend(
                care_of(&rows)
                    .iter()
                    .filter_map(|care| care.episode().map(|episode| episode.key)),
            );
            for party in parties.iter().filter(|p| p.kind == Kind::PartyRelationship) {
                let Some(head) = party
                    .versions
                    .iter()
                    .max_by_key(|v| (v.lineage(), v.tree.columns()))
                else {
                    continue;
                };
                let facts = GrantFacts::of(Kind::PartyRelationship, &head.data);
                if facts.care_source != Some(person) {
                    continue;
                }
                declared.extend(facts.facts.iter().filter_map(|fact| match &fact.value {
                    FactValue::Care(care) => care.episode.as_ref().map(|episode| episode.key),
                    FactValue::Code(_) | FactValue::Team(_) | FactValue::Kin(_) => None,
                }));
            }
        }
        crate::versioning::episode::declared(&placed, &declared)
    }

    /// The `creating_system_id`s whose wrapped committers the request's
    /// caller may import as `operation`: the `creating_system` elements of
    /// its grants of that operation, never this node's own system id.
    fn trusted_systems(&self, operation: Operation) -> BTreeSet<String> {
        let own = self.effective_system_id();
        let capabilities = Capabilities::current();
        capabilities
            .of(operation)
            .flat_map(|grant| grant.credentials().identified(CREATING_SYSTEM))
            .filter(|system| !system.is_empty() && *system != own)
            .map(str::to_owned)
            .collect()
    }

    /// Admits an import of `containers` and `parties`, the operation
    /// `operation`, by the request's caller: one holding a grant of
    /// `operation` covering the subject of each `EHR_STATUS` the import
    /// carries ([`Capabilities::covers`]). An `import_ehr` must carry one; an
    /// `import_ehr_extract` carrying none is held to its target EHR by
    /// [`Self::admit_import_target`].
    ///
    /// # Errors
    /// [`ServiceError::Forbidden`] (`403`) when the caller holds no such
    /// grant, or a wrapped committer names a local
    /// party or login from a system its grant does not vouch for; a storage
    /// failure.
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
        if crate::versioning::committer::current_committer().is_some() {
            let subjects = imported_subjects(containers);
            let subjects: Vec<Option<&str>> = subjects.iter().map(Option::as_deref).collect();
            let capabilities = Capabilities::current();
            let granted = if subjects.is_empty() && operation == Operation::ImportEhrExtract {
                capabilities.holds(operation)
            } else {
                capabilities.covers(operation, &subjects)
            };
            if !granted {
                return Err(refused_import(operation));
            }
        }
        let trusted = self.trusted_systems(operation);
        let mut committers: Vec<&Value> = Vec::new();
        for container in containers.iter().chain(parties) {
            for version in &container.versions {
                if trusted.contains(&version.creating_system_id) {
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

    /// Admits an `import_ehr_extract` into the EHR `target` by the request's
    /// caller: the EHR exists, and the caller holds an `import_ehr_extract`
    /// grant naming the target's subject namespace.
    ///
    /// # Errors
    /// `ehr_id_does_not_exist` (`404`) for no EHR `target`;
    /// [`ServiceError::Forbidden`] (`403`) for a caller without it; a storage
    /// failure.
    pub(super) async fn admit_import_target(&self, target: EhrId) -> Result<(), ServiceError> {
        let mut conn = acquire(&self.pool).await?;
        let stored: Option<Option<String>> =
            sqlx::query_scalar("SELECT subject_namespace FROM ehr WHERE id = $1")
                .bind(target)
                .fetch_optional(&mut *conn)
                .await?;
        let Some(namespace) = stored else {
            return Err(ServiceError::not_found(
                Missing::Ehr,
                format!("no EHR with id {target}"),
            ));
        };
        if !self.import_policy.enforced
            || crate::versioning::committer::current_committer().is_none()
        {
            return Ok(());
        }
        if Capabilities::current().covers(Operation::ImportEhrExtract, &[namespace.as_deref()]) {
            return Ok(());
        }
        Err(refused_import(Operation::ImportEhrExtract))
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
        if self.export_granted(ehr_id).await? {
            return self.refuse_objected_export(ehr_id).await;
        }
        Err(ServiceError::not_permitted(
            "a system caller exports an EHR only under a grant of \
             openehr_access::export_ehr_extract whose subject_namespace elements name the \
             subject namespace of the EHR, or hold no_subject_namespace for a subject with none",
        ))
    }

    /// Whether the request's caller holds an `export_ehr_extract` grant whose
    /// `subject_namespace` elements cover the EHR `ehr_id`'s subject.
    ///
    /// # Errors
    /// A storage failure of the subject read.
    pub(super) async fn export_granted(&self, ehr_id: EhrId) -> Result<bool, ServiceError> {
        let namespace = self.subject_namespace(ehr_id).await?;
        Ok(Capabilities::current().covers(Operation::ExportEhrExtract, &[namespace.as_deref()]))
    }

    /// The subject namespace of the EHR `ehr_id`'s current `EHR_STATUS`;
    /// `None` for an EHR whose subject names none.
    async fn subject_namespace(&self, ehr_id: EhrId) -> Result<Option<String>, ServiceError> {
        let mut conn = acquire(&self.pool).await?;
        let namespace: Option<Option<String>> =
            sqlx::query_scalar("SELECT subject_namespace FROM ehr WHERE id = $1")
                .bind(ehr_id)
                .fetch_optional(&mut *conn)
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
                        .is_some_and(|issuer| {
                            issuer == self.basic_issuer()
                                || self.import_policy.issuers.contains(issuer)
                        })
                })
            });
        // NOTE: a reference whose root is no UUID names no container of this store.
        let reference = committer
            .pointer("/external_ref/id/value")
            .and_then(Value::as_str)
            .and_then(|id| VoId::from_uid_head(id).ok());
        let local = match reference {
            Some(id) => {
                let mut conn = acquire(&self.demographic_pool).await?;
                let found: Option<i32> = sqlx::query_scalar(
                    "SELECT 1 FROM vo_head WHERE vo_id = $1 \
                     AND kind IN ('PERSON', 'ORGANISATION', 'GROUP', 'AGENT', 'ROLE')",
                )
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
                found.is_some()
            }
            None => false,
        };
        if login || local {
            return Err(ServiceError::not_permitted(
                "a wrapped AUDIT_DETAILS.committer names a party or a login of this deployment, \
                 and its version was created by no system a creating_system element of the \
                 caller's import grant names",
            ));
        }
        Ok(())
    }
}

/// The subjects of the `EHR_STATUS` versions among `containers`, distinct:
/// each `EHR_STATUS.subject.external_ref.namespace`, `None` for a subject
/// with no `external_ref`, such as an anonymous record's `PARTY_SELF`.
fn imported_subjects(containers: &[ImportContainer]) -> BTreeSet<Option<String>> {
    containers
        .iter()
        .filter(|container| container.kind == Kind::EhrStatus)
        .flat_map(|container| &container.versions)
        .map(|version| {
            version
                .data
                .pointer("/subject/external_ref/namespace")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

/// The namespace an EHR Extract writes the references to its demographics
/// chapter under.
const LOCAL: &str = "local";

/// The refusal of an import as `operation` by a caller holding no grant
/// covering the EHR.
fn refused_import(operation: Operation) -> ServiceError {
    ServiceError::not_permitted(format!(
        "importing needs a grant of openehr_access::{operation} whose subject_namespace \
         elements name the subject namespace of the EHR, or hold no_subject_namespace for a \
         subject with none"
    ))
}
