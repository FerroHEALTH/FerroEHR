// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The commit of a direct party or relationship write (#6257): judged on one
//! gate read of the party pool and committed in one guarded statement after
//! its access record, with no lock: the version keys and the tip the gate read
//! found arbitrate, and a moved tip is judged again. The wire is the
//! Demographic API of ITS-REST Release-1.1.0, whose status, `ETag`,
//! `If-Match` and deleted-read semantics mirror the EHR group. No openEHR
//! spec governs how a commit is run against the database: our own design.

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::precondition::Precondition;
use crate::service::demographic::validate::Container;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::version_update::WriteRequest;
use crate::service::write::{Materials, Resolved, first_attempt};
use crate::storage::version_repo::gate::{self, Prior, Target};
use crate::storage::version_repo::guarded::{Guard, PartySync};
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::body::Body;
use crate::versioning::change::{Change, Committed, Write, WriteEnvelope};
use crate::versioning::lifecycle::state::DELETED;
use crate::versioning::object_version_id::object_version_id;

impl FerroEhrService {
    /// Commits the first version of a new demographic object of `kind` from
    /// `request`, as part of `container`; `what` names it in the audit's
    /// description and `change` builds the version from the body it stores
    /// and its envelope. The committed version and the body it serves, `None`
    /// when the commit rewrites it.
    ///
    /// # Errors
    /// The checks' refusals ([`Self::demographic_write_checks`]), the
    /// offload's and the commit's.
    pub(super) async fn commit_direct_create(
        &self,
        (kind, container): (Kind, Option<Container>),
        request: WriteRequest<Body>,
        what: &str,
        change: impl FnOnce(Body, WriteEnvelope) -> Change,
    ) -> Result<(Committed, Option<Body>), ServiceError> {
        // The caller's request merges with the server rules, BOTH halves the
        // committal headers carry: the `UPDATE_AUDIT` attributes and the
        // VERSION `lifecycle_state` ("whatever is provided it MUST be merged
        // with the default VERSION and `VERSION.audit_details` attributes on
        // commit runtime", ITS-REST overview `Requests_and_responses.md`
        // §"openehr-version and openehr-audit-details").
        let description = format!("{what} creation");
        let Resolved {
            audit, envelope, ..
        } = self
            .demographic_write_checks(
                kind,
                &request,
                container,
                (change_type::CREATION, &description),
            )
            .await?;
        // The large media go to the blob store before a connection is held.
        let (body, served) = self.demographic_stored(request.data, true).await?;
        // A create judged again can only follow a moved requirement of its own new
        // object, which nothing else writes, so it keeps no copy.
        let materials = Materials::once((change(body, envelope), audit));
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
        Ok((committed, served))
    }

    /// Commits `request` as a new version of the demographic object `vo_id`
    /// of `kind`, as part of `container`: a missing object or one of another
    /// kind is `404`, then `precondition` is judged against the tip, then a
    /// deleted tip is `404`, then the body checks; `what` names it in the
    /// audit's description. The committed version and the body it serves,
    /// `None` when the commit rewrites it.
    ///
    /// # Errors
    /// The refusals in that order, the offload's and the commit's.
    pub(super) async fn commit_direct_update(
        &self,
        ((kind, vo_id), container): ((Kind, VoId), Option<Container>),
        precondition: Precondition,
        request: WriteRequest<Body>,
        what: &str,
    ) -> Result<(Committed, Option<Body>), ServiceError> {
        let description = format!("{what} update");
        let checked = self
            .demographic_write_checks(
                kind,
                &request,
                container,
                (change_type::MODIFICATION, &description),
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
                    let (target, prior) = judged(conn, (kind, vo_id), precondition).await?;
                    let tip = target
                        .placement
                        .tip
                        .as_ref()
                        .ok_or_else(|| missing(kind, vo_id))?;
                    precondition.check_tree(tip)?;
                    if tip.lifecycle_state == DELETED {
                        return Err(ServiceError::not_found(
                            Missing::VersionedObject,
                            format!("{} {vo_id} is deleted", kind.as_str()),
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
                        return Err(ServiceError::exception(format!(
                            "an unchecked {what} update reached its write"
                        )));
                    };
                    let change = Change::modify(vo_id, kind, body, None, None, envelope);
                    Ok(self.guarded(Write::new(None, change, audit), target, prior))
                }
            })
            .await?;
        Ok((committed, served))
    }

    /// Commits the logical delete of the demographic object `vo_id` of
    /// `kind` as a new `523|deleted|` version (RM common master06 §Logical
    /// Deletion); `precondition` (the `If-Match`, else the path version) is
    /// judged against the tip, and `what` names it in the audit's
    /// description. The deleted version's `ETag`/`Location` metadata.
    ///
    /// # Errors
    /// `404` for a missing object or one of another kind, the precondition's
    /// refusal, the refusal of a tip already deleted, and the commit's.
    pub(super) async fn commit_direct_delete(
        &self,
        (kind, vo_id): (Kind, VoId),
        precondition: Precondition,
        request: WriteRequest,
        what: &str,
    ) -> Result<ServiceResponse, ServiceError> {
        let description = format!("{what} delete");
        let (request, precondition, description) = (&request, &precondition, &description);
        let committed = self
            .commit_gated(&self.demographic_pool, move || {
                async move |conn| {
                    let (target, prior) = judged(conn, (kind, vo_id), precondition).await?;
                    let tip = target
                        .placement
                        .tip
                        .as_ref()
                        .ok_or_else(|| missing(kind, vo_id))?;
                    if tip.lifecycle_state == DELETED {
                        return Err(ServiceError::precondition(format!(
                            "{} {vo_id} is already deleted",
                            kind.as_str()
                        )));
                    }
                    // A stale preceding version is a VERSION MISMATCH: the wire
                    // arm answers `409` and echoes the latest `version_uid` in
                    // `ETag` (`responses/409_PERSON_with_uid_based_id.yaml`).
                    precondition.check_tree(tip)?;
                    let audit = self.commit_audit(
                        request.audit.as_ref(),
                        change_type::DELETED,
                        description,
                    )?;
                    let change = Change::delete(vo_id, kind, None);
                    Ok(self.guarded(Write::new(None, change, audit), target, prior))
                }
            })
            .await?;
        Ok(ServiceResponse::deleted(ResourceMeta::new(
            String::new(),
            object_version_id(vo_id, &committed.creating_system_id, committed.tree),
        )))
    }

    /// `write` of the existing `target` guarded, placed and marked as its
    /// gate read found it, with `prior`, what it found of the versions before.
    fn guarded(&self, write: Write, target: Target, prior: Prior) -> Write {
        write
            .guarded(self.party_guard(target.head, prior))
            .object_marked(target.restricted)
            .placed(target.placement)
    }

    /// What a direct write's guarded statement requires and takes: the head
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
            directory: None,
            party: Some(Box::new(PartySync {
                key_space: !self.binding.is_empty(),
                reverse: None,
                prior,
            })),
        }
    }
}

/// The target of a direct write of the object `vo_id` of `kind`, read on
/// `conn` with what its gate read found of the versions before it, once
/// `precondition` names its tip in full.
///
/// # Errors
/// [`ServiceError::NotFound`] as [`addressed`], the precondition's refusal
/// and the read's storage error.
async fn judged(
    conn: &mut sqlx::PgConnection,
    (kind, vo_id): (Kind, VoId),
    precondition: &Precondition,
) -> Result<(Target, Prior), ServiceError> {
    let (target, prior) = gate::party(conn, Some(vo_id)).await?.split();
    let target = addressed(target, kind, vo_id)?;
    if let Some(tip) = target.placement.tip.as_ref() {
        precondition.check_full(vo_id, tip)?;
    }
    Ok((target, prior))
}

/// The target of a direct write of `kind` its gate read found, when it is
/// an object of that kind belonging to no EHR.
///
/// # Errors
/// [`ServiceError::NotFound`] when the object holds no version, is of
/// another kind or belongs to an EHR.
fn addressed(target: Option<Target>, kind: Kind, vo_id: VoId) -> Result<Target, ServiceError> {
    target
        .filter(|target| {
            target
                .placement
                .tip
                .as_ref()
                .is_some_and(|tip| tip.ehr_id.is_none() && tip.kind == kind.as_str())
        })
        .ok_or_else(|| missing(kind, vo_id))
}

/// The `404` of a direct write whose target is no object of `kind`.
fn missing(kind: Kind, vo_id: VoId) -> ServiceError {
    ServiceError::not_found(
        Missing::VersionedObject,
        format!("{} {vo_id}", kind.as_str()),
    )
}
