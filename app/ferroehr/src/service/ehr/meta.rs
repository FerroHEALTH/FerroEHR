// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared version-metadata helpers: the glue every versioned kind
//! (`EHR_STATUS`, COMPOSITION, DIRECTORY) needs to turn a loaded
//! [`VersionRead`] into a wire [`ServiceResponse`] and its [`ResourceMeta`],
//! plus the default commit-audit builder.
//!
//! The `OBJECT_VERSION_ID` law is RM common
//! `master06-change_control_package.adoc` §Version Identification and BASE
//! `base_types/master05-identification_package.adoc` §Syntaxes; the
//! `Last-Modified`, `ETag` and `Location` derivation from a version's commit
//! audit is ITS-REST, carried in the [`ResourceMeta`] envelope. No openEHR spec
//! governs that envelope, or the `current_vo` storage seam
//! ([`crate::storage::version_repo`]) — our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use crate::ids::{EhrId, VoId};
use crate::service::response::{ResourceMeta, ServiceResponse};
use openehr_rm::prelude::{DvIdentifier, PartyIdentified, PartyIdentifiedData, PartyProxy};
use serde_json::Value;

use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::versioning::Kind;
use crate::versioning::audit::AuditInput;
use crate::versioning::object_version_id::{TreeId, VersionIdError, object_version_id, version_id};
use crate::versioning::read::VersionRead;

/// The `DV_IDENTIFIER.issuer` stamped on a committer whose credential this
/// deployment holds itself (Basic auth) — there is no external authority to
/// name, so the product identifies itself.
///
/// NOTE: no openEHR spec governs the value — our own design/extension (see
/// [`committer`]).
const LOCAL_ISSUER: &str = "ferroehr";

impl FerroEhrService {
    /// The current version `(vo_id, VERSION_TREE_ID)` of an EHR's object of a
    /// given [`Kind`], if any — the trunk head the object's head row names.
    ///
    /// The `version` current-row read is a storage seam
    /// ([`crate::storage::version_repo::meta::current_vo`]; no openEHR spec governs
    /// the SQL — our own design). The [`crate::versioning::CommitEnv`]
    /// `current_vo` hook adapts this `(Uuid, TreeId)` to its `(Uuid, i32)`
    /// (trunk) shape.
    ///
    /// # Errors
    /// [`ServiceError::Database`] if the current-row read fails.
    #[expect(
        clippy::same_name_method,
        reason = "the `CommitEnv` seam (service/commit_env.rs) deliberately \
                  mirrors these chapter method names so the versioning layer \
                  calls them by their own vocabulary; that impl disambiguates \
                  explicitly with `FerroEhrService::<name>(self, …)`"
    )]
    pub(in crate::service) async fn current_vo(
        &self,
        ehr_id: EhrId,
        kind: Kind,
    ) -> Result<Option<(VoId, TreeId)>, ServiceError> {
        Ok(
            crate::storage::version_repo::meta::current_vo(&self.pool, ehr_id, kind.as_str())
                .await?
                .map(|r| {
                    (
                        r.vo_id,
                        TreeId::from_columns(r.trunk_version, r.branch_number, r.branch_version),
                    )
                }),
        )
    }

    /// A metadata-only [`ServiceResponse`] for a write, built entirely from the
    /// commit result — the version identity and the server commit instant are
    /// already in [`Committed`](crate::versioning::change::Committed), so the write
    /// path never re-reads the row it just wrote (a representation response
    /// re-reads at the protocol layer). The body is `Value::Null` by contract:
    /// every write consumer uses only the metadata.
    pub(crate) fn committed_response(
        &self,
        ehr_id: EhrId,
        committed: &crate::versioning::change::Committed,
    ) -> ServiceResponse {
        let meta = self.version_meta(
            ehr_id,
            committed.vo_id,
            &committed.creating_system_id,
            committed.tree,
            committed.time_committed,
        );
        ServiceResponse::new(Value::Null, meta)
    }

    /// The [`ResourceMeta`] for a versioned resource: the owning EHR plus the
    /// resource `OBJECT_VERSION_ID` (the `ETag` value + `Location` tail) and
    /// its commit time (the `Last-Modified`). `creating_system_id` is the
    /// stored per-version value — never re-derived from the live config — so a
    /// version's uid stays stable across a later system-id change (RM common
    /// master06 §Distributed Versioning).
    #[expect(
        clippy::unused_self,
        reason = "call-site ergonomics: every caller already holds the service, \
                  so an inherent method reads better than a free function"
    )]
    pub(crate) fn version_meta(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        creating_system_id: &str,
        version: TreeId,
        at: jiff::Timestamp,
    ) -> ResourceMeta {
        ResourceMeta::new(
            ehr_id.to_string(),
            object_version_id(vo_id, creating_system_id, version),
        )
        .with_last_modified(at)
    }

    /// Ensure the versioned object's canonical JSON carries its `uid`
    /// (`OBJECT_VERSION_ID`, RM common master06 §Version Identification) so a
    /// bare read serves its wire identity.
    ///
    /// The write path already stamps the uid into the stored body
    /// (`crate::versioning::change` — `stamp_version_uid` runs before
    /// decomposition), so on a locally committed version this is a no-op:
    /// when the stored `uid` is an `OBJECT_VERSION_ID` whose value matches,
    /// the body passes through untouched. Only a body that lacks or mismatches
    /// it (e.g. a verbatim-imported foreign version) pays the re-stamp.
    #[expect(
        clippy::unused_self,
        reason = "call-site ergonomics: every caller already holds the service, \
                  so an inherent method reads better than a free function"
    )]
    pub(in crate::service) fn with_uid(
        &self,
        mut canonical: Value,
        vo_id: VoId,
        creating_system_id: &str,
        version: TreeId,
    ) -> Result<Value, VersionIdError> {
        if let Value::Object(map) = &mut canonical {
            let id = version_id(vo_id, creating_system_id, version)?;
            let already_stamped = map.get("uid").is_some_and(|uid| {
                uid.get("_type").and_then(Value::as_str) == Some("OBJECT_VERSION_ID")
                    && uid.get("value").and_then(Value::as_str) == Some(id.value())
            });
            if !already_stamped {
                map.insert("uid".to_owned(), openehr_its::json::to_canonical_value(&id));
            }
        }
        Ok(canonical)
    }

    /// A [`ServiceResponse`] for a loaded versioned object: its canonical body
    /// with the `uid` injected, plus the resource metadata for the wire
    /// headers.
    ///
    /// # Errors
    /// [`VersionIdError`] when the stored `creating_system_id` does not compose
    /// into a well-formed `OBJECT_VERSION_ID` (see [`Self::with_uid`]).
    pub(in crate::service) fn version_response(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        read: VersionRead,
    ) -> Result<ServiceResponse, VersionIdError> {
        let meta = self.version_meta(
            ehr_id,
            vo_id,
            &read.creating_system_id,
            read.tree,
            read.time_committed,
        );
        Ok(ServiceResponse::new(
            self.with_uid(read.canonical, vo_id, &read.creating_system_id, read.tree)?,
            meta,
        ))
    }

    /// The current version metadata of an EHR-owned object of `kind` (the
    /// latest `version_uid` a `409`/`412` must echo in `ETag`/`Location`), or
    /// `None`.
    ///
    /// Resolved and read in one metadata-only `version`⋈`audit` statement
    /// (`current_version_meta_by_kind`): the `409` and `412` paths need only the
    /// full `OBJECT_VERSION_ID` and commit instant, so this avoids the node
    /// reassembly and attestation read a full version read pays. The emitted
    /// `ETag` is still `object_id::creating_system_id::version_tree_id`, the
    /// token the `If-Match` compare uses (ITS-REST overview §Concurrency
    /// control).
    ///
    /// # Errors
    /// [`ServiceError::Database`] if the metadata read fails.
    pub(in crate::service) async fn latest_version_meta(
        &self,
        ehr_id: EhrId,
        kind: Kind,
    ) -> Result<Option<ResourceMeta>, ServiceError> {
        Ok(self
            .latest_version_meta_with_vo(ehr_id, kind)
            .await?
            .map(|(_, m)| m))
    }

    /// The current version's `vo_id` **and** its [`ResourceMeta`] for an
    /// EHR-owned object of `kind`, resolved and read in the same ONE
    /// metadata-only `version`⋈`audit` statement [`Self::latest_version_meta`]
    /// uses. Threading the `vo_id` back to the caller lets a following write
    /// skip re-resolving `(ehr_id, kind) → vo_id`, so the `If-Match` pre-read
    /// and the write share one `current_vo` resolution. The
    /// full-`OBJECT_VERSION_ID` compare (ITS-REST overview §Concurrency
    /// control) is unchanged.
    ///
    /// # Errors
    /// [`ServiceError::Database`] if the metadata read fails.
    pub(in crate::service) async fn latest_version_meta_with_vo(
        &self,
        ehr_id: EhrId,
        kind: Kind,
    ) -> Result<Option<(VoId, ResourceMeta)>, ServiceError> {
        let Some(m) = crate::storage::version_repo::meta::current_version_meta_by_kind(
            &self.pool,
            ehr_id,
            kind.as_str(),
        )
        .await?
        else {
            return Ok(None);
        };
        let tree = TreeId::from_columns(m.trunk_version, m.branch_number, m.branch_version);
        let meta = self.version_meta(
            ehr_id,
            m.vo_id,
            &m.creating_system_id,
            tree,
            m.time_committed,
        );
        Ok(Some((m.vo_id, meta)))
    }

    /// Build an [`AuditInput`] for a direct (single-object) write: the
    /// effective system id, the numeric `audit_change_type` code, a
    /// description, and the request's authenticated committer (RM common
    /// master04 §Audit Details).
    pub(in crate::service) fn audit(&self, change_type: &str, description: &str) -> AuditInput {
        AuditInput {
            system_id: self.effective_system_id(),
            change_type: change_type.to_owned(),
            description: Some(crate::versioning::audit::dv_text(description)),
            committer: committer(),
            attestation: None,
        }
    }
}

/// The committer `PARTY_PROXY` for an audit, from the authenticated committer
/// of the current request (published by the protocol adapter into the
/// [`crate::service::committer`] context). A write with no authenticated
/// principal (auth disabled, or an internal/system write) is attributed to the
/// system identity (RM common master04 `AUDIT_DETAILS.committer` 1..1).
pub(in crate::service) fn committer() -> PartyProxy {
    let party = match crate::service::committer::current_committer() {
        Some(identity) => PartyIdentifiedData {
            external_ref: None,
            name: Some(identity.subject.clone()),
            identifiers: Some(openehr_base::containers::NonEmptyVec::of(DvIdentifier {
                // DV_IDENTIFIER.issuer is the "authority which issues the kind
                // of id used in the id field of this object" (RM data_types
                // UML/classes/org.openehr.rm.data_types.dv_identifier.adoc
                // §Attributes). For a federated principal that authority is the
                // token issuer, not this server, so the identity carries it
                // through; a locally-held credential (Basic) has no other
                // authority and takes this deployment's product name.
                //
                // NOTE: no openEHR spec governs which string names the issuing
                // authority — our own design (RM common master04 §Audit Details
                // says only "in the form of a system login identifier").
                issuer: Some(identity.issuer.unwrap_or_else(|| LOCAL_ISSUER.to_owned())),
                assigner: None,
                id: identity.subject,
                r#type: Some(identity.id_type.to_owned()),
            })),
        },
        None => PartyIdentifiedData {
            external_ref: None,
            name: Some(crate::service::SYSTEM_COMMITTER_NAME.to_owned()),
            identifiers: None,
        },
    };
    PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party))
}

// ── ITS-REST version-metadata adapter (adapter-support extension) ─────────────
//
// The protocol adapter's `412`/`ETag` pre-reads. No openEHR spec governs this
// adapter seam — our own extension over the SM-governed version identities.

impl FerroEhrService {
    /// The current COMPOSITION version metadata (the latest `version_uid` a
    /// `409`/`412` must echo), or `None` if unknown or outside this EHR.
    ///
    /// # Errors
    /// [`SmError`](crate::service::status::SmError) if the metadata read fails.
    pub async fn composition_latest_meta(
        &self,
        an_ehr_id: EhrId,
        a_versioned_object_uid: VoId,
    ) -> Result<Option<ResourceMeta>, crate::service::status::SmError> {
        Ok(self
            .composition_current_meta(an_ehr_id, a_versioned_object_uid)
            .await?)
    }

    /// The current `EHR_STATUS` version metadata of an EHR, or `None` when the
    /// EHR has no current `EHR_STATUS`.
    ///
    /// # Errors
    /// [`SmError`](crate::service::status::SmError) if the metadata read fails.
    pub async fn ehr_status_latest_meta(
        &self,
        an_ehr_id: EhrId,
    ) -> Result<Option<ResourceMeta>, crate::service::status::SmError> {
        Ok(self.ehr_status_meta(an_ehr_id).await?)
    }

    /// The current directory FOLDER version metadata of an EHR, or `None` when
    /// the EHR indexes no directory hierarchy.
    ///
    /// # Errors
    /// [`SmError`](crate::service::status::SmError) if the metadata read fails.
    pub async fn directory_latest_meta(
        &self,
        an_ehr_id: EhrId,
    ) -> Result<Option<ResourceMeta>, crate::service::status::SmError> {
        Ok(self.directory_meta(an_ehr_id).await?)
    }
}

/// The [`ResourceMeta`] of a version-CONTAINER read (a `VERSIONED_OBJECT` body
/// or its `REVISION_HISTORY`): the `ETag` identity is the container uid
/// (ITS-REST overview `Requests_and_responses.md` §"`ETag` and Last-Modified"
/// names "`VERSIONED_OBJECT.uid.value`" as an `ETag` source) and the
/// `Last-Modified` instant is the newest held version's commit time (same §:
/// both headers "SHOULD be included in responses for VERSION,
/// `VERSIONED_OBJECT`, or other resources that have versioning or unique
/// state identifiers"; the value is "derived from
/// `VERSION.commit_audit.time_committed.value`").
pub(in crate::service) fn container_meta(
    ehr_id: EhrId,
    vo_id: VoId,
    last_modified: jiff::Timestamp,
) -> ResourceMeta {
    ResourceMeta::new(ehr_id.to_string(), vo_id.to_string()).with_last_modified(last_modified)
}
