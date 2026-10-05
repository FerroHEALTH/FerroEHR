// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Demographic (ehr-less) CONTRIBUTION — commit + retrieval, per the
//! Demographic API of ITS-REST Release-1.1.0 (DEVELOPMENT lifecycle within
//! the released spec; the SM demographic service is abstract). A demographic
//! CONTRIBUTION wraps a change-set of party / relationship versions with
//! `ehr_id = None`, committed through the shared CONTRIBUTION engine
//! ([`crate::versioning::contribution::commit_version_set`]) with `party_only = true`
//! (RM common master06 §Contributions).
//!
//! The retrieval reads go through `crate::storage::version_repo` (storage owns
//! the SQL — no openEHR spec governs the storage read, our own design):
//! `crate::versioning::contribution::get_contribution` is EHR-scoped and cannot serve an ehr-less
//! contribution, so the demographic chapter reads the audit + version refs here.
//! The version-refs helper unions the versions a `666|attestation|` item
//! attested (RM common master06 §Contributions — an attestation affects an
//! existing version), so the list is the full change-set, not the narrower
//! committed-only rows.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use openehr_base::prelude::{HierObjectId, ObjectId, ObjectRef, ObjectRefData};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::status::CallStatusType;
use crate::storage::version_repo;
use crate::versioning::audit::AuditInput;
use crate::versioning::contribution::commit_version_set;
use crate::versioning::object_version_id::{TreeId, version_id};

impl FerroEhrService {
    /// Commit a demographic CONTRIBUTION (ehr-less): its versions must reference
    /// party / relationship objects (an EHR-kind type inside is rejected `422`
    /// by the engine's scope check). Returns the assembled CONTRIBUTION with its
    /// `ETag`/`Location` (the contribution uid).
    pub(super) async fn create_demographic_contribution(
        &self,
        body: Value,
    ) -> Result<ServiceResponse, ServiceError> {
        let committed = commit_version_set(self, None, &body, true).await?;
        let body = self.demographic_contribution(committed.id).await?;
        let meta = ResourceMeta::new(String::new(), committed.id.to_string())
            .with_last_modified(committed.time_committed);
        Ok(ServiceResponse::new(body, meta))
    }

    /// Retrieve a demographic (ehr-less) CONTRIBUTION by id. An EHR-scoped
    /// contribution uid here is `404` (the demographic surface only sees
    /// `ehr_id IS NULL` contributions).
    pub(super) async fn demographic_contribution(
        &self,
        contribution_id: Uuid,
    ) -> Result<Value, ServiceError> {
        let audit = version_repo::contribution::contribution_audit(
            &self.demographic_pool,
            contribution_id,
            None,
        )
        .await?
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::ContributionDoesNotExist,
                format!("demographic CONTRIBUTION {contribution_id}"),
            )
        })?;

        // The refs helper also unions the versions this contribution's
        // `666|attestation|` items attested (RM common master06 §Contributions —
        // an attestation affects an existing version), i.e. the full change-set
        // the CONTRIBUTION covers, not just the committed rows.
        let version_refs = version_repo::contribution::contribution_version_refs(
            &self.demographic_pool,
            contribution_id,
        )
        .await?;
        let mut versions: Vec<Value> = Vec::with_capacity(version_refs.len());
        for (vo_id, columns, creating_system_id, kind) in version_refs {
            let tree = TreeId::from_columns(columns.0, columns.1, columns.2);
            versions.push(openehr_its::json::to_canonical_value(
                &ObjectRef::ObjectRef(ObjectRefData {
                    namespace: "demographic".to_owned(),
                    r#type: kind.clone(),
                    id: ObjectId::ObjectVersionId(version_id(vo_id, &creating_system_id, tree)?),
                }),
            ));
        }

        let audit_details = AuditInput {
            system_id: audit.system_id,
            change_type: audit.change_type,
            description: audit
                .description
                .as_ref()
                .map(crate::versioning::audit::decode_description)
                .transpose()?,
            committer: crate::versioning::audit::party_proxy(&audit.committer)?,
            attestation: audit
                .attestation
                .as_ref()
                .map(crate::versioning::attestation::AttestationParts::decode)
                .transpose()?
                .map(Box::new),
        }
        .canonical(&audit.time_committed);
        // NOTE: a JSON-literal envelope over the already-canonical `audit`
        // fragment; every part this builder SYNTHESIZES is built from its
        // generated type.
        Ok(json!({
            "_type": "CONTRIBUTION",
            "uid": openehr_its::json::to_canonical_value(&HierObjectId::from(contribution_id)),
            "audit": audit_details,
            "versions": versions
        }))
    }
}
