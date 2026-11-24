// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The party CRUD and its create factory — the party create / read / update /
//! delete domain logic, built on the shared [`crate::versioning`]
//! change-control machinery with `ehr_id = None` (no EHR scope — our own
//! design; a party has no owning EHR). The wire is the Demographic API of
//! ITS-REST Release-1.1.0 (DEVELOPMENT lifecycle within the released spec):
//! status/`ETag`/`Location`/`If-Match`/deleted-read semantics mirror the EHR
//! group by the spec's own design (module NOTE in [`super`]).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use serde_json::Value;

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::precondition::Precondition;
use crate::service::demographic::support;
use crate::service::demographic::types::PartyKind;
use crate::service::demographic::validate::Container;
use crate::service::demographic::written::demographic_answer;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::version_update::WriteRequest;
use crate::versioning::body::Body;
use crate::versioning::change::Change;
use crate::versioning::object_version_id::{TreeId, object_version_id};
use crate::versioning::read::{VersionRead, demographic_current, object_kind};

/// The current version of a demographic party, resolved in ONE lean
/// `version`⋈`audit` read (no node reassembly). Carries the kind-checked
/// identity + commit instant (the `ETag`/`If-Match` parts) and the not-deleted
/// gate, so a write path resolves the target **once** and threads it — the
/// dispatcher never resolves for `If-Match` and the service again for the
/// existence/kind gate. RM common master06 §Version Identification / §Logical
/// Deletion.
pub(super) struct CurrentParty {
    vo_id: VoId,
    pub(super) tree: TreeId,
    creating_system_id: String,
    time_committed: jiff::Timestamp,
    pub(super) deleted: bool,
}

impl CurrentParty {
    /// The current version's full `OBJECT_VERSION_ID` `{vo}::{system}::{tree}`
    /// (the `ETag` value / `Location` tail).
    fn ovid(&self) -> String {
        object_version_id(self.vo_id, &self.creating_system_id, self.tree)
    }

    /// The `ResourceMeta` a `412` (`If-Match`) echoes: the current
    /// `OBJECT_VERSION_ID` + its commit instant (empty `ehr_id` — parties are
    /// not EHR-scoped).
    pub(super) fn resource_meta(&self) -> ResourceMeta {
        ResourceMeta::new(String::new(), self.ovid()).with_last_modified(self.time_committed)
    }
}

impl FerroEhrService {
    /// Load a version of a party, verifying it is of the expected [`PartyKind`]
    /// and ehr-less. A wrong-kind or unknown id is `404`.
    async fn load_party_version(
        &self,
        kind: PartyKind,
        vo_id: VoId,
        version: Option<TreeId>,
        at: Option<jiff::Timestamp>,
    ) -> Result<VersionRead, ServiceError> {
        // The granular status: a version-addressed read that misses is
        // `object_version_does_not_exist`, a current read is the generic
        // `versioned_object_does_not_exist`.
        let miss = || support::missing(kind.rm_type(), vo_id, version.is_some() || at.is_some());
        // The stored kind (constant per versioned object) must match the
        // route; the loaded row carries it (#5776).
        support::load_ehrless(
            &self.demographic_pool,
            self.spec_profile,
            vo_id,
            version,
            at,
            Some(support::kind_of(kind)),
        )
        .await?
        .ok_or_else(miss)
    }

    /// Confirm `vo_id` is some party (any of the five kinds) — the check for the
    /// kind-agnostic `versioned_party` reads. A non-party id (COMPOSITION, …) or
    /// unknown id is `404`, reported with the caller-supplied granular
    /// status (`versioned_object_does_not_exist` for object-addressed reads,
    /// `object_version_does_not_exist` for version-addressed ones).
    pub(super) async fn ensure_any_party(
        &self,
        vo_id: VoId,
        miss: Missing,
    ) -> Result<(), ServiceError> {
        match object_kind(&self.demographic_pool, vo_id).await? {
            Some(k) if k.is_party() => Ok(()),
            _ => Err(ServiceError::not_found(
                miss,
                format!("versioned party {vo_id}"),
            )),
        }
    }

    /// Create the first version
    /// of a new PARTY (server-side `VERSIONED_OBJECT` + `ORIGINAL_VERSION` +
    /// `CONTRIBUTION`). Returns it with its `uid` set and the create-response
    /// `ETag`/`Location` metadata.
    ///
    /// The representation is built from the commit result, byte-identical to a
    /// fresh [`Self::read_party`]; a body the commit rewrites
    /// ([`support::commit_rewrites_body`]) is read back instead.
    pub(super) async fn commit_new_party(
        &self,
        kind: PartyKind,
        request: WriteRequest<Body>,
    ) -> Result<ServiceResponse, ServiceError> {
        // The id is allocated before the checks, so the relationships are
        // judged against the container the commit gives the party.
        let allocated = self.allocate_party_id();
        let container = Some(Container::New(allocated.id()));
        let (committed, served) = self
            .commit_direct_create(
                (support::kind_of(kind), container),
                request,
                "PARTY creation",
                |body, envelope| {
                    Change::create_allocated(allocated, support::kind_of(kind), body, envelope)
                },
            )
            .await?;
        let read_back = self.read_party(kind, committed.vo_id, Some(committed.tree), None);
        demographic_answer(served, &committed, read_back).await
    }

    /// Retrieve a party by its versioned-object id, optionally at a specific version (`version`) or
    /// instant (`at`; else the latest). A deleted current version resolves to
    /// `Value::Null` (→ `204`, mirroring COMPOSITION). A wrong-kind object (a
    /// PERSON under the `agent` route, or a COMPOSITION) is `404`.
    pub(super) async fn read_party(
        &self,
        kind: PartyKind,
        vo_id: VoId,
        version: Option<TreeId>,
        at: Option<jiff::Timestamp>,
    ) -> Result<ServiceResponse, ServiceError> {
        let read = self.load_party_version(kind, vo_id, version, at).await?;
        if read.deleted() {
            return Ok(ServiceResponse::plain(Value::Null));
        }
        Ok(support::version_response(vo_id, read)?)
    }

    /// [`Self::read_party`] with the body kept as the stored text where nothing
    /// needs it parsed: the read the wire serves.
    pub(super) async fn read_party_raw(
        &self,
        kind: PartyKind,
        vo_id: VoId,
        version: Option<TreeId>,
        at: Option<jiff::Timestamp>,
    ) -> Result<crate::service::response::RawServiceResponse, ServiceError> {
        self.demographic_read_raw(support::kind_of(kind), vo_id, version, at)
            .await
    }

    /// Resolve the current version of a party of the routed [`PartyKind`] in ONE
    /// lean read ([`demographic_current`]) — kind-checked and ehr-less. `None`
    /// when there is no current version, the stored kind differs (a PERSON under
    /// the `agent` route), or the id is a non-demographic / EHR-scoped object.
    /// The write paths never reassemble nodes just to gate the write.
    pub(super) async fn party_current(
        &self,
        kind: PartyKind,
        vo_id: VoId,
    ) -> Result<Option<CurrentParty>, ServiceError> {
        let Some(current) = demographic_current(&self.demographic_pool, vo_id).await? else {
            return Ok(None);
        };
        if current.kind != support::kind_of(kind) {
            return Ok(None);
        }
        Ok(Some(CurrentParty {
            vo_id,
            tree: current.tree,
            creating_system_id: current.creating_system_id,
            time_committed: current.time_committed,
            deleted: current.deleted,
        }))
    }

    /// Commit a new version of the party `vo_id` of `kind`, judged on one gate
    /// read and committed in one guarded statement after its access record
    /// (#6257): a missing or wrong-kind party is `404`, then `precondition` is
    /// judged against the tip, then a deleted tip is `404`, then the body
    /// checks. The write response is built from the commit result (see
    /// [`Self::commit_new_party`]).
    ///
    /// # Errors
    /// The refusals in that order, and the commit's.
    pub(super) async fn commit_party_update(
        &self,
        (kind, vo_id): (PartyKind, VoId),
        precondition: Precondition,
        request: WriteRequest<Body>,
    ) -> Result<ServiceResponse, ServiceError> {
        let addressed = (
            (support::kind_of(kind), vo_id),
            Some(Container::Existing(vo_id)),
        );
        let (committed, served) = self
            .commit_direct_update(addressed, precondition, request, "PARTY update")
            .await?;
        let read_back = self.read_party(kind, committed.vo_id, Some(committed.tree), None);
        demographic_answer(served, &committed, read_back).await
    }

    /// The current party version metadata (the latest `version_uid` a `412`
    /// echoes in `ETag`/`Location`), or `None` if unknown/wrong-kind — the lean
    /// resolve, no node reassembly.
    ///
    /// # Errors
    /// [`ServiceError`] on a storage/database fault during the lean read.
    pub async fn demographic_latest_meta(
        &self,
        kind: PartyKind,
        vo_id: VoId,
    ) -> Result<Option<ResourceMeta>, ServiceError> {
        Ok(self
            .party_current(kind, vo_id)
            .await?
            .map(|c| c.resource_meta()))
    }
}
