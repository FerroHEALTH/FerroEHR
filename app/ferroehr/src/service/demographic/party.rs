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
use crate::service::write::{Materials, Resolved, first_attempt};
use crate::storage::version_repo::gate::{self, Prior, Target};
use crate::storage::version_repo::guarded::{Guard, PartySync};
use crate::versioning::audit::change_type;
use crate::versioning::body::Body;
use crate::versioning::change::{Change, Write};
use crate::versioning::lifecycle::state::DELETED;
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
    /// The representation is built **from the commit result** (never a
    /// post-commit re-read): the served body is the just-written canonical with
    /// the `uid` injected, byte-identical to a fresh [`Self::read_party`] (the
    /// node codec round-trips losslessly; pinned by a test). When the commit
    /// rewrites the body ([`support::commit_rewrites_body`]: the `DV_MULTIMEDIA`
    /// offload), the in-memory input is not the stored form, so the stored
    /// version is read back instead.
    pub(super) async fn commit_new_party(
        &self,
        kind: PartyKind,
        request: WriteRequest<Body>,
    ) -> Result<ServiceResponse, ServiceError> {
        // The id is allocated before the checks, so the relationships are
        // judged against the container the commit gives the party.
        let allocated = self.allocate_party_id();
        // The caller's request merges with the server rules, BOTH halves the
        // committal headers carry: the `UPDATE_AUDIT` attributes and the
        // VERSION `lifecycle_state` ("whatever is provided it MUST be merged
        // with the default VERSION and `VERSION.audit_details` attributes on
        // commit runtime", ITS-REST overview `Requests_and_responses.md`
        // §"openehr-version and openehr-audit-details").
        let Resolved {
            audit, envelope, ..
        } = self
            .demographic_write_checks(
                support::kind_of(kind),
                &request,
                Some(Container::New(allocated.id())),
                (change_type::CREATION, "PARTY creation"),
            )
            .await?;
        // The large media go to the blob store before a connection is held.
        let (body, served) = self.demographic_stored(request.data, true).await?;
        let create = Change::create_allocated(allocated, support::kind_of(kind), body, envelope);
        // A create judged again can only follow a moved requirement of its own new
        // object, which nothing else writes, so it keeps no copy.
        let materials = Materials::once((create, audit));
        let materials = &materials;
        let committed = self
            .commit_gated(&self.demographic_pool, move || {
                async move |conn| {
                    let gate = gate::party(conn, None).await?;
                    let now = gate.now;
                    let (_, prior) = gate.split();
                    let (create, audit) = materials.take_first()?;
                    Ok(Write::new(None, create, audit)
                        .at(now)
                        .guarded(self.party_guard(None, prior)))
                }
            })
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
        let checked = self
            .demographic_write_checks(
                support::kind_of(kind),
                &request,
                Some(Container::Existing(vo_id)),
                (change_type::MODIFICATION, "PARTY update"),
            )
            .await;
        // A valid body's large media go to the blob store before a connection is held.
        let (body, served) = self
            .demographic_stored(request.data, checked.is_ok())
            .await?;
        let (resolved, unchecked) = checked.map_or_else(
            |error| (None, Some(error)),
            |resolved| (Some(resolved), None),
        );
        // An update a later attempt judges again commits on the new tip only when it
        // names no preceding version, so only such a one keeps a copy.
        let materials = Materials::new((body, resolved), precondition.names_none());
        let unchecked = std::sync::Mutex::new(unchecked);
        let (materials, unchecked, precondition) = (&materials, &unchecked, &precondition);
        let committed = self
            .commit_gated(&self.demographic_pool, move || {
                async move |conn| {
                    let (target, prior) = gate::party(conn, Some(vo_id)).await?.split();
                    let target = addressed(target, kind, vo_id)?;
                    let tip = target
                        .placement
                        .tip
                        .as_ref()
                        .ok_or_else(|| missing(kind, vo_id))?;
                    precondition.check_full(vo_id, tip)?;
                    precondition.check_tree(tip)?;
                    if tip.lifecycle_state == DELETED {
                        return Err(ServiceError::not_found(
                            Missing::VersionedObject,
                            format!("{} {vo_id} is deleted", kind.rm_type()),
                        ));
                    }
                    if let Some(error) = first_attempt(unchecked) {
                        return Err(error);
                    }
                    let (
                        body,
                        Some(Resolved {
                            audit, envelope, ..
                        }),
                    ) = materials.take()?
                    else {
                        return Err(ServiceError::exception(
                            "an unchecked PARTY update reached its write",
                        ));
                    };
                    Ok(Write::new(
                        None,
                        Change::modify(vo_id, support::kind_of(kind), body, None, None, envelope),
                        audit,
                    )
                    .guarded(self.party_guard(target.head, prior))
                    .object_marked(target.restricted)
                    .placed(target.placement))
                }
            })
            .await?;
        let read_back = self.read_party(kind, committed.vo_id, Some(committed.tree), None);
        demographic_answer(served, &committed, read_back).await
    }

    /// Commit the logical delete of the party `vo_id` of `kind`, judged on one
    /// gate read and committed in one guarded statement after its access
    /// record (#6257); `precondition` (the `If-Match`, else the path version)
    /// is judged against the tip. Returns the deleted version's
    /// `ETag`/`Location` metadata.
    ///
    /// # Errors
    /// `404` for a missing or wrong-kind party, the precondition's refusal,
    /// the refusal of a tip already deleted, and the commit's.
    pub(super) async fn commit_party_delete(
        &self,
        (kind, vo_id): (PartyKind, VoId),
        precondition: Precondition,
        request: WriteRequest,
    ) -> Result<ServiceResponse, ServiceError> {
        let (request, precondition) = (&request, &precondition);
        let committed = self
            .commit_gated(&self.demographic_pool, move || {
                async move |conn| {
                    let (target, prior) = gate::party(conn, Some(vo_id)).await?.split();
                    let target = addressed(target, kind, vo_id)?;
                    let tip = target
                        .placement
                        .tip
                        .as_ref()
                        .ok_or_else(|| missing(kind, vo_id))?;
                    precondition.check_full(vo_id, tip)?;
                    if tip.lifecycle_state == DELETED {
                        return Err(ServiceError::precondition(format!(
                            "{} {vo_id} is already deleted",
                            kind.rm_type()
                        )));
                    }
                    // A stale preceding version is a VERSION MISMATCH: the wire
                    // arm answers `409` and echoes the latest `version_uid` in
                    // `ETag` (`responses/409_PERSON_with_uid_based_id.yaml`).
                    precondition.check_tree(tip)?;
                    let audit = self.commit_audit(
                        request.audit.as_ref(),
                        change_type::DELETED,
                        "PARTY delete",
                    )?;
                    Ok(Write::new(
                        None,
                        Change::delete(vo_id, support::kind_of(kind), None),
                        audit,
                    )
                    .guarded(self.party_guard(target.head, prior))
                    .object_marked(target.restricted)
                    .placed(target.placement))
                }
            })
            .await?;
        Ok(ServiceResponse::deleted(ResourceMeta::new(
            String::new(),
            object_version_id(vo_id, &committed.creating_system_id, committed.tree),
        )))
    }

    /// What a party write's guarded statement requires and takes: the head
    /// `head` its gate read found (`None` for a create), and the key-space
    /// gate when a binding mapping is configured; `prior` is what the gate read
    /// found of the versions before it.
    fn party_guard(&self, head: Option<i32>, prior: Prior) -> Guard {
        Guard {
            expected_head: head,
            policy: None,
            definitions: None,
            outbox: None,
            reservation: None,
            status: None,
            folder: None,
            party: Some(Box::new(PartySync {
                key_space: !self.binding.is_empty(),
                reverse: None,
                prior,
            })),
        }
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

/// The target of a party write of `kind` its gate read found, when it is a
/// party of that kind.
///
/// # Errors
/// [`ServiceError::NotFound`] when the object holds no version, is of
/// another kind or belongs to an EHR.
fn addressed(target: Option<Target>, kind: PartyKind, vo_id: VoId) -> Result<Target, ServiceError> {
    target
        .filter(|target| {
            target.placement.tip.as_ref().is_some_and(|tip| {
                tip.ehr_id.is_none() && tip.kind == support::kind_of(kind).as_str()
            })
        })
        .ok_or_else(|| missing(kind, vo_id))
}

/// The `404` of a party write whose target is no party of `kind`.
fn missing(kind: PartyKind, vo_id: VoId) -> ServiceError {
    ServiceError::not_found(
        Missing::VersionedObject,
        format!("{} {vo_id}", kind.rm_type()),
    )
}
