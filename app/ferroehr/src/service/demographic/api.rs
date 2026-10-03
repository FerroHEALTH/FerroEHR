// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The public DEMOGRAPHIC seam on [`FerroEhrService`] — the SM
//! `I_DEMOGRAPHIC_SERVICE` / `I_PARTY` / `I_PARTY_RELATIONSHIP` operations
//! (`i_demographic_service.adoc`, `i_party.adoc`, `i_party_relationship.adoc`)
//! plus the wire-shaped party / relationship / contribution / tag calls the
//! ITS-REST adapter (`ferroehr-rest`) invokes.
//!
//! Thin adapters that parse the (kind + string) arguments the wire seams
//! supply and delegate to the sibling demographic domain modules. Party /
//! relationship ids parse through the shared BASE decoder in
//! [`crate::versioning`] (`object_version_id.rs`).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use openehr_base::v1_3::base_types::identification::lexical::composite_ids_equal;
use openehr_rm::prelude::ItemTag;
use serde_json::Value;
use uuid::Uuid;

use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::datetime::parse_at_time;
use crate::service::demographic::party::CurrentParty;
use crate::service::demographic::relationship::CurrentRelationship;
use crate::service::demographic::types::PartyKind;
use crate::service::ehr::tags::tag_target_tail;
use crate::service::error::ServiceError;
use crate::service::response::{ResourceMeta, ServiceResponse};
use crate::service::status::{CallStatusType, SmError};
use crate::service::version_update::Committal;
use crate::versioning::object_version_id::{
    components, expected_from_if_match, if_match_token, parse_uid_based_id, parse_version_uid,
};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateVersion};
use openehr_rm::prelude::{Party, PartyRelationship};

/// The `version_uid` a write produced (the new/deleted `OBJECT_VERSION_ID`),
/// pulled from the response metadata.
fn version_uid(resp: ServiceResponse) -> String {
    resp.meta.map(|m| m.uid).unwrap_or_default()
}

/// The refusal of a create or update whose version carries no data: only a
/// logical deletion has none (RM common master06 §Logical Deletion).
fn no_party_data() -> SmError {
    SmError::precondition("the version carries no data, and this operation writes content")
}

/// The [`PartyKind`] a commit envelope routes to, read off the RM `PARTY`
/// subtype it carries (`i_party.adoc`: parties are addressed by their
/// concrete RM type).
///
/// Total and infallible: `PARTY` is a closed subtype set in the generated RM,
/// so every value names exactly one resource family. [`PartyKind`] is OUR
/// route key (the `/demographic/{segment}` URL family + the stored kind
/// discriminator — `service::demographic::types`), which no openEHR spec or
/// generated type carries; this is the one mapping between the two.
fn party_kind_of(body: &Party) -> PartyKind {
    match body {
        Party::Agent(_) => PartyKind::Agent,
        Party::Group(_) => PartyKind::Group,
        Party::Organisation(_) => PartyKind::Organisation,
        Party::Person(_) => PartyKind::Person,
        Party::Role(_) => PartyKind::Role,
    }
}

/// Verifies a full-`OBJECT_VERSION_ID` `If-Match` precondition, which must
/// answer `412 Precondition Failed` when it evaluates false (ITS-REST overview
/// `Requests_and_responses.md` §"If-Match and accidental overwrites").
///
/// The precondition names the current latest version in full, the
/// `object_id :: creating_system_id :: version_tree_id` triple, and a mismatch
/// in any segment is a `412`. The comparison is case-insensitive, an
/// `OBJECT_VERSION_ID` being a composite identifier whose case-differing forms
/// "identify the same thing" (BASE `base_types`
/// `master05-identification_package.adoc` §"Composite Identifiers and Case").
/// This mirrors the EHR path's `ensure_if_match`.
///
/// The wire `ETag` syntax is decoded by the ITS-REST adapter before the value
/// reaches here; [`if_match_token`] applies the remaining quote tolerance so
/// this compare and [`expected_from_if_match`] judge the same token. Tokens that
/// are not a full `OBJECT_VERSION_ID` are not silently skipped: the RFC 9110 `*`
/// wildcard and the bare `VERSION_TREE_ID` trunk number carry no full identity
/// to compare, and every other shape is rejected as malformed by
/// [`expected_from_if_match`], which every caller invokes on the same value.
fn ensure_full_ovid_if_match(
    if_match: Option<&str>,
    current: Option<&ResourceMeta>,
) -> Result<(), SmError> {
    let Some(raw) = if_match else { return Ok(()) };
    let token = if_match_token(raw);
    if <openehr_base::prelude::ObjectVersionId as std::str::FromStr>::from_str(token).is_err() {
        return Ok(());
    }
    match current {
        Some(meta) if composite_ids_equal(&meta.uid, token) => Ok(()),
        Some(meta) => Err(SmError::version_mismatch(format!(
            "If-Match {token:?} does not match the current latest version {:?}",
            meta.uid
        ))),
        None => Ok(()),
    }
}

/// The [`Committal`] an SM `UPDATE_VERSION` envelope carries: its
/// `UPDATE_AUDIT` attributes plus its VERSION `lifecycle_state`, so an
/// SM-routed demographic commit honours the same two halves the wire's
/// committal headers do (ITS-REST overview `Requests_and_responses.md`
/// §"openehr-version and openehr-audit-details"; RM common master06 §Version
/// Lifecycle). An empty code means the caller stated none, leaving the
/// operation default.
fn envelope_committal<T>(a_version: &UpdateVersion<T>) -> Committal {
    Committal {
        audit: a_version.commit_audit.clone(),
        lifecycle_state: Some(a_version.lifecycle_state.defining_code.code_string.clone())
            .filter(|code| !code.is_empty()),
    }
}

impl FerroEhrService {
    /// Attach the party's stored `ITEM_TAG`s (RM `common.item_tag`) to a response's
    /// metadata seam ([`ResourceMeta::item_tags`]), from which the ITS-REST
    /// adapter derives the `openehr-item-tag`/`openehr-version-item-tag` response
    /// headers. A response without metadata (a deleted read → `Null` body) is
    /// left unchanged. The tags are read from the same store `party_tags_get`
    /// serves, so the header and the tags sub-resource agree.
    ///
    /// # Errors
    /// [`SmError`] on a storage/database fault reading the tag store.
    async fn attach_party_item_tags(
        &self,
        vo_id: VoId,
        resp: &mut ServiceResponse,
    ) -> Result<(), SmError> {
        if resp.meta.is_none() {
            return Ok(());
        }
        // The two response headers carry DISTINCT collections (overview
        // §"openehr-item-tag and openehr-version-item-tag"): the container's
        // set rides `openehr-item-tag`, the served VERSION's own set rides
        // `openehr-version-item-tag`. The version tail comes from the response
        // metadata's own OBJECT_VERSION_ID.
        let container = self.party_tags(vo_id, None).await?;
        let version_tail = resp
            .meta
            .as_ref()
            .and_then(|m| m.uid.split_once("::").map(|(_, tail)| tail.to_owned()));
        let version = match version_tail.as_deref() {
            Some(tail) => Some(self.party_tags(vo_id, Some(tail)).await?),
            None => None,
        };
        if let Some(meta) = resp.meta.as_mut() {
            meta.item_tags = Some(container);
            meta.version_item_tags = version;
        }
        Ok(())
    }

    // ── I_DEMOGRAPHIC_SERVICE + I_PARTY (the SM core) ───────────────────────

    /// `create_party` (`i_demographic_service.adoc`): commit the first version
    /// of a new PARTY.
    ///
    /// # Errors
    /// - [`SmError`] `content_invalid` — the payload `_type` is absent or not a
    ///   demographic party type (AGENT/GROUP/ORGANISATION/PERSON/ROLE),
    ///   `i_party.adoc`.
    /// - [`SmError`] `content_invalid` — the body fails RM validation for the
    ///   resolved party kind (`validate::party_check`).
    /// - [`SmError`] `conflict` / `service_overloaded` / `exception` — the
    ///   versioned-create transaction fails (integrity conflict, pool
    ///   exhaustion, or a storage/database fault).
    /// - [`SmError`] `precondition_violation` — the committed version uid does
    ///   not parse (defensive; the uid is server-generated).
    pub async fn create_party(&self, a_version: UpdateVersion<Party>) -> Result<VoId, SmError> {
        let kind = party_kind_of(a_version.data.as_ref().ok_or_else(no_party_data)?);
        let a_version = crate::service::ehr::canonicalize(a_version);
        let committal = envelope_committal(&a_version);
        let resp = self
            .commit_new_party(
                kind,
                a_version.data.ok_or_else(no_party_data)?,
                Some(&committal),
            )
            .await?;
        let (vo_id, _) = parse_version_uid(&version_uid(resp))?;
        Ok(vo_id)
    }

    /// True iff a *live* party of some kind exists under this versioned-object
    /// id (a logically deleted party reads `Null` → `false`).
    ///
    /// # Errors
    /// - [`SmError`] `conflict` / `service_overloaded` / `exception` — a
    ///   storage/database fault while resolving the party kind or reading its
    ///   current version. A *not-found* on either resolves to `Ok(false)`, not
    ///   an error.
    pub async fn has_party(&self, a_versioned_party_id: VoId) -> Result<bool, SmError> {
        // True iff a *live* party of some kind exists (a logically deleted party
        // reads `Null`, satisfying the delete post-condition `not has_party`).
        match self.party_kind_at(a_versioned_party_id).await {
            Ok(kind) => match self
                .read_party(kind, a_versioned_party_id, None, None)
                .await
            {
                Ok(resp) => Ok(!resp.is_empty()),
                Err(ServiceError::NotFound(_)) => Ok(false),
                Err(e) => Err(e.into()),
            },
            Err(ServiceError::NotFound(_)) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// True iff a specific party `OBJECT_VERSION_ID` exists. An unparseable id
    /// or a missing version resolve to `false`.
    ///
    /// # Errors
    /// - [`SmError`] `conflict` / `service_overloaded` / `exception` — a
    ///   storage/database fault while reading the version. A malformed id or a
    ///   *not-found* version resolves to `Ok(false)`, not an error.
    pub async fn has_party_version_id(&self, a_party_version_id: String) -> Result<bool, SmError> {
        let Ok((vo_id, tree)) = parse_version_uid(&a_party_version_id) else {
            return Ok(false);
        };
        match self.party_version(vo_id, tree).await {
            Ok(_) => Ok(true),
            Err(ServiceError::NotFound(_)) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// `get_party` (`i_party.adoc`): the current version's body.
    ///
    /// # Errors
    /// - [`SmError`] `versioned_object_does_not_exist` — no versioned party with
    ///   this id exists (`party_kind_at`), or its current version is logically
    ///   deleted / absent (the read is empty).
    /// - [`SmError`] `conflict` / `service_overloaded` / `exception` — a
    ///   storage/database fault during kind resolution or read.
    pub async fn get_party(&self, a_versioned_party_id: VoId) -> Result<Value, SmError> {
        let kind = self.party_kind_at(a_versioned_party_id).await?;
        let resp = self
            .read_party(kind, a_versioned_party_id, None, None)
            .await?;
        if resp.is_empty() {
            // Pre `has_party` failed (deleted / no current version).
            return Err(SmError::new(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("party {a_versioned_party_id} has no current version"),
            ));
        }
        Ok(resp.body)
    }

    /// `get_party_at_time` (`i_party.adoc`): the Version of a Party current at
    /// `a_time` (an extended-ISO-8601 datetime; the timezone is optional and
    /// its absence means the server's local one — `crate::service::datetime`).
    /// A deleted version at that instant reads `Null`.
    ///
    /// # Errors
    /// - [`SmError`] `versioned_object_does_not_exist` — no versioned party
    ///   with this id exists, or no version existed at `a_time`.
    /// - [`SmError`] `precondition_violation` — `a_time` does not parse as an
    ///   ISO-8601 timestamp.
    /// - [`SmError`] on a storage/database fault during kind resolution or read.
    pub async fn get_party_at_time(
        &self,
        a_versioned_party_id: VoId,
        a_time: String,
    ) -> Result<Value, SmError> {
        let kind = self.party_kind_at(a_versioned_party_id).await?;
        let at = parse_at_time(&a_time)?;
        let resp = self
            .read_party(kind, a_versioned_party_id, None, Some(at))
            .await?;
        Ok(resp.body)
    }

    /// `get_party_at_version` (`i_party.adoc`): the `ORIGINAL_VERSION` named by
    /// a full party `OBJECT_VERSION_ID`.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse as an
    ///   `OBJECT_VERSION_ID`.
    /// - [`SmError`] `object_version_does_not_exist` — the object is not a
    ///   party or holds no such version.
    /// - [`SmError`] on a storage/database fault or a signing failure while
    ///   assembling the `ORIGINAL_VERSION`.
    pub async fn get_party_at_version(&self, a_party_version_id: String) -> Result<Value, SmError> {
        let (vo_id, tree) = parse_version_uid(&a_party_version_id)?;
        // The version-addressed read carries `object_version_does_not_exist`
        // itself (`ServiceError` round-trips the granular status losslessly).
        Ok(self.party_version(vo_id, tree).await?)
    }

    /// `update_party` (`i_party.adoc`): commit a new version of an existing
    /// party; returns the new version's `OBJECT_VERSION_ID`.
    ///
    /// # Errors
    /// - [`SmError`] `content_invalid` — the payload `_type` is absent / not a
    ///   party type, or the body fails RM validation for that kind.
    /// - [`SmError`] `precondition_violation` — `preceding_version_uid` does
    ///   not parse as an `OBJECT_VERSION_ID`.
    /// - [`SmError`] `versioned_object_does_not_exist` — no live party of the
    ///   payload's kind exists under this id (unknown, wrong-kind, or deleted).
    /// - [`SmError`] `conflict` — the preceding version is stale (optimistic
    ///   concurrency), or the write transaction fails.
    pub async fn update_party(
        &self,
        a_versioned_party_id: VoId,
        a_version: UpdateVersion<Party>,
    ) -> Result<String, SmError> {
        let kind = party_kind_of(a_version.data.as_ref().ok_or_else(no_party_data)?);
        let a_version = crate::service::ehr::canonicalize(a_version);
        let expected = match &a_version.preceding_version_uid {
            Some(ovid) => Some(components(ovid)?.1),
            None => None,
        };
        let committal = envelope_committal(&a_version);
        let resp = self
            .update_party_version(
                kind,
                a_versioned_party_id,
                a_version.data.ok_or_else(no_party_data)?,
                expected,
                Some(&committal),
            )
            .await?;
        Ok(version_uid(resp))
    }

    /// `delete_party` (`i_party.adoc`): logically delete the party's current
    /// version (post `not has_party`); returns the deleted version's
    /// `OBJECT_VERSION_ID`. The SM `delete_party` has no version argument —
    /// the current version is deleted unconditionally.
    ///
    /// # Errors
    /// - [`SmError`] `versioned_object_does_not_exist` — no versioned party
    ///   with this id exists.
    /// - [`SmError`] mapped from `400` — the party is already deleted.
    /// - [`SmError`] on a storage/database fault during the delete transaction.
    pub async fn delete_party(&self, a_versioned_party_id: VoId) -> Result<String, SmError> {
        // The SM `delete_party` has no version argument — delete the current
        // version unconditionally.
        let kind = self.party_kind_at(a_versioned_party_id).await?;
        let resp = self
            .delete_party_version(kind, a_versioned_party_id, None, None)
            .await?;
        Ok(version_uid(resp))
    }

    // ── PARTY CRUD (wire seam) ────────────────────────────────────────────────

    /// Create a party of the routed [`PartyKind`] (the wire seam of
    /// `create_party`), with the party's stored `ITEM_TAG`s surfaced on the
    /// response metadata for the `openehr-item-tag` headers (a fresh party has
    /// none yet; the wire adapter persists any request-header tags and
    /// re-populates the seam afterwards).
    ///
    /// # Errors
    /// - [`SmError`] mapped from `422` — the body's `_type` mismatches the
    ///   route or fails RM validation.
    /// - [`SmError`] `precondition_violation` — the committed version uid does
    ///   not parse (defensive; server-generated).
    /// - [`SmError`] on a storage/database fault during the create transaction
    ///   or the tag read.
    pub async fn party_create(
        &self,
        kind: PartyKind,
        body: Party,
        committal: Option<Committal>,
    ) -> Result<ServiceResponse, SmError> {
        let body = openehr_its::json::to_canonical_value(&body);
        // A freshly created party has no stored ITEM_TAGs by construction, so
        // the response seam needs no tag read here; when the request carried
        // `openehr-item-tag` header tags, the wire adapter persists them after
        // the create and re-populates the seam itself (person_create.yaml).
        Ok(self
            .commit_new_party(kind, body, committal.as_ref())
            .await?)
    }

    /// Read a party of the routed [`PartyKind`] by uid-based id (bare
    /// `HIER_OBJECT_ID` or full `OBJECT_VERSION_ID`), optionally time-travelled
    /// to `version_at_time`; the stored `ITEM_TAG`s ride the metadata seam. A
    /// deleted current version reads `Null` (→ `204`).
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id or `version_at_time`
    ///   does not parse.
    /// - [`SmError`] mapped from `404` — unknown id, wrong kind for the route,
    ///   or no version at the requested version/instant.
    /// - [`SmError`] on a storage/database fault during the read.
    pub async fn party_get(
        &self,
        kind: PartyKind,
        uid_based_id: String,
        version_at_time: Option<String>,
    ) -> Result<ServiceResponse, SmError> {
        let decoded = parse_uid_based_id(&uid_based_id)?;
        let (vo_id, version) = (decoded.vo_id, decoded.tree());
        let at = version_at_time.as_deref().map(parse_at_time).transpose()?;
        let mut resp = self.read_party(kind, vo_id, version, at).await?;
        // Attach the party's stored ITEM_TAGs for the item-tag response headers
        // (person_get.yaml). A deleted read carries no metadata → no-op.
        self.attach_party_item_tags(vo_id, &mut resp).await?;
        Ok(resp)
    }

    /// Update a party of the routed [`PartyKind`] under a mandatory `If-Match`
    /// precondition (ITS-REST overview §"If-Match and accidental
    /// overwrites"). The current version is resolved ONCE (lean,
    /// kind-checked): the same handle serves the `If-Match` `ETag` compare
    /// here and the service write gate.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id or the `If-Match` token
    ///   does not parse.
    /// - [`SmError`] `version_mismatch` (`412`) — a full-OVID `If-Match` names
    ///   a version other than the current latest.
    /// - [`SmError`] mapped from `404` — unknown id, wrong kind, or a deleted
    ///   current version.
    /// - [`SmError`] mapped from `422` — the body fails RM validation.
    /// - [`SmError`] mapped from `409` — the expected version is stale, or the
    ///   write transaction fails.
    pub async fn party_update(
        &self,
        kind: PartyKind,
        uid_based_id: String,
        if_match: String,
        body: Party,
        committal: Option<Committal>,
    ) -> Result<ServiceResponse, SmError> {
        let body = openehr_its::json::to_canonical_value(&body);
        let vo_id = parse_uid_based_id(&uid_based_id)?.vo_id;
        // Resolve the current version ONCE (lean, kind-checked): the same handle
        // serves the `If-Match` `ETag` compare here and the service write gate —
        // the dispatcher never resolves and the service again.
        let current = self.party_current(kind, vo_id).await?;
        let meta = current.as_ref().map(CurrentParty::resource_meta);
        ensure_full_ovid_if_match(Some(&if_match), meta.as_ref())?;
        let expected = expected_from_if_match(&if_match)?;
        let current = current.ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("{} {vo_id}", kind.rm_type()),
            )
        })?;
        Ok(self
            .commit_party_update(current, body, expected, committal.as_ref())
            .await?)
    }

    /// Logically delete a party of the routed [`PartyKind`]
    /// (`delete_party(a_versioned_party_id: UUID)` — our own demographic
    /// design): the path
    /// carries the versioned-party id (bare `HIER_OBJECT_ID` or full
    /// `OBJECT_VERSION_ID`). The preceding trunk version for optimistic
    /// concurrency comes from `If-Match` when supplied, else the path OVID,
    /// else `None` (delete the current version unconditionally).
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse, or a
    ///   supplied `If-Match` is malformed (a malformed header is
    ///   rejected, never silently ignored).
    /// - [`SmError`] `version_mismatch` (`412`) — a full-OVID `If-Match` names
    ///   a version other than the current latest.
    /// - [`SmError`] mapped from `404` — unknown id or wrong kind for the route.
    /// - [`SmError`] mapped from `400` — the party is already deleted.
    /// - [`SmError`] mapped from `409` — the expected version is stale.
    pub async fn party_delete(
        &self,
        kind: PartyKind,
        uid_based_id: String,
        if_match: Option<String>,
        update_audit: Option<UpdateAudit>,
    ) -> Result<ServiceResponse, SmError> {
        let decoded = parse_uid_based_id(&uid_based_id)?;
        let (vo_id, path_version) = (decoded.vo_id, decoded.tree());
        // One lean resolve shared by the `If-Match` compare and the delete gate.
        let current = self.party_current(kind, vo_id).await?;
        let meta = current.as_ref().map(CurrentParty::resource_meta);
        ensure_full_ovid_if_match(if_match.as_deref(), meta.as_ref())?;
        // A malformed `If-Match` is rejected, not silently ignored;
        // an absent header falls back to the path OVID's version.
        let expected = match if_match.as_deref() {
            Some(raw) => expected_from_if_match(raw)?.or(path_version),
            None => path_version,
        };
        let current = current.ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("{} {vo_id}", kind.rm_type()),
            )
        })?;
        Ok(self
            .commit_party_delete(current, expected, update_audit.as_ref())
            .await?)
    }

    // ── VERSIONED_PARTY ──────────────────────────────────────────────────────

    /// The `VERSIONED_PARTY` wrapper for a party (any of the five kinds).
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse.
    /// - [`SmError`] mapped from `404` — the id is not a party or holds no
    ///   versions.
    /// - [`SmError`] on a storage/database fault reading the version spine.
    pub async fn versioned_party_get(
        &self,
        versioned_object_uid: String,
    ) -> Result<ServiceResponse, SmError> {
        let vo_id = parse_uid_based_id(&versioned_object_uid)?.vo_id;
        let body = self.versioned_party(vo_id).await?;
        // ITS-REST overview §"ETag and Last-Modified" derives `Last-Modified`
        // from `VERSION.commit_audit.time_committed.value`, and the container
        // body exposes no commit audit, so the instant comes from the version
        // spine rather than the body.
        let newest =
            crate::storage::version_repo::meta::all_version_meta(&self.demographic_pool, vo_id)
                .await
                .map_err(ServiceError::from)?
                .last()
                .map(|m| m.time_committed);
        let meta = ResourceMeta::new(String::new(), vo_id.to_string());
        let meta = match newest {
            Some(at) => meta.with_last_modified(at),
            None => meta,
        };
        Ok(ServiceResponse::new(body, meta))
    }

    /// The `REVISION_HISTORY` of a party (RM common master04 §Revision
    /// History).
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse.
    /// - [`SmError`] mapped from `404` — the id is not a party.
    /// - [`SmError`] on a storage/database fault reading the version spine.
    pub async fn versioned_party_revision_history(
        &self,
        versioned_object_uid: String,
    ) -> Result<ServiceResponse, SmError> {
        let vo_id = parse_uid_based_id(&versioned_object_uid)?.vo_id;
        Ok(ServiceResponse::plain(
            self.party_revision_history(vo_id).await?,
        ))
    }

    /// The party `ORIGINAL_VERSION` extant at `version_at_time` (or the latest
    /// when absent), with `ETag`/`Location` metadata.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id or `version_at_time`
    ///   does not parse.
    /// - [`SmError`] mapped from `404` — the id is not a party or no version
    ///   existed at the instant.
    /// - [`SmError`] on a storage/database fault or a signing failure.
    pub async fn versioned_party_version_get_at_time(
        &self,
        versioned_object_uid: String,
        version_at_time: Option<String>,
    ) -> Result<ServiceResponse, SmError> {
        let vo_id = parse_uid_based_id(&versioned_object_uid)?.vo_id;
        let at = version_at_time.as_deref().map(parse_at_time).transpose()?;
        Ok(self.party_version_at_time(vo_id, at).await?)
    }

    /// The party `ORIGINAL_VERSION` named by `version_uid`.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — either id does not parse.
    /// - [`SmError`] mapped from `404` — the id is not a party or holds no
    ///   such version.
    /// - [`SmError`] on a storage/database fault or a signing failure.
    pub async fn versioned_party_version_get_by_id(
        &self,
        versioned_object_uid: String,
        version_uid: String,
    ) -> Result<ServiceResponse, SmError> {
        let vo_id = parse_uid_based_id(&versioned_object_uid)?.vo_id;
        let (_, version) = parse_version_uid(&version_uid)?;
        Ok(ServiceResponse::plain(
            self.party_version(vo_id, version).await?,
        ))
    }

    // ── demographic CONTRIBUTION ─────────────────────────────────────────────

    /// Commit a demographic (ehr-less) CONTRIBUTION — a change-set of party /
    /// relationship versions (RM common master06 §Contributions).
    ///
    /// # Errors
    /// - [`SmError`] mapped from `422` — the change-set is malformed, contains
    ///   an EHR-kind version (the engine's scope check), or a version fails
    ///   validation.
    /// - [`SmError`] mapped from `409` — a preceding version is stale.
    /// - [`SmError`] on a storage/database fault during the commit or the
    ///   read-back.
    pub async fn demographic_contribution_create(
        &self,
        body: Value,
    ) -> Result<ServiceResponse, SmError> {
        Ok(self.create_demographic_contribution(body).await?)
    }

    /// Retrieve a demographic (ehr-less) CONTRIBUTION by uid.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the uid is not a UUID.
    /// - [`SmError`] mapped from `404` — unknown uid, or an EHR-scoped
    ///   contribution (the demographic surface only sees `ehr_id IS NULL`).
    /// - [`SmError`] on a storage/database fault during the read.
    #[expect(
        clippy::map_err_ignore,
        reason = "the mapped error already names the resource and echoes the \
                  rejected token; the discarded `uuid::Error` adds only its own \
                  wording, which is not part of the wire contract"
    )]
    pub async fn demographic_contribution_get(
        &self,
        contribution_uid: String,
    ) -> Result<ServiceResponse, SmError> {
        let id = Uuid::parse_str(&contribution_uid).map_err(|_| {
            SmError::precondition(format!("invalid contribution id: {contribution_uid}"))
        })?;
        Ok(ServiceResponse::plain(
            self.demographic_contribution(id).await?,
        ))
    }

    // ── demographic item tags ────────────────────────────────────────────────

    /// All demographic `ITEM_TAG`s (ehr-less), optionally filtered by
    /// key/value/target path.
    ///
    /// # Errors
    /// [`SmError`] on a storage/database fault reading the tag store.
    pub async fn demographic_tags_get(
        &self,
        tag_key: Option<String>,
        tag_value: Option<String>,
        tag_target_path: Option<String>,
    ) -> Result<Vec<ItemTag>, SmError> {
        Ok(self
            .demographic_tags(
                tag_key.as_deref(),
                tag_value.as_deref(),
                tag_target_path.as_deref(),
            )
            .await?)
    }

    /// The `ITEM_TAG`s on one party.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse.
    /// - [`SmError`] on a storage/database fault reading the tag store.
    pub async fn party_tags_get(
        &self,
        kind: PartyKind,
        uid_based_id: String,
    ) -> Result<Vec<ItemTag>, SmError> {
        let (vo_id, version) = crate::service::ehr::tags::parse_tag_target(&uid_based_id)?;
        // The released 404 trigger ("when the `uid_based_id` does not exist")
        // plus the kind-checked-routes law: the guard runs on the GET too; an
        // existing target with no tags stays an empty 200 list.
        self.ensure_party_tag_target(kind, vo_id, version.as_ref())
            .await?;
        Ok(self
            .party_tags(vo_id, tag_target_tail(version.as_ref()))
            .await?)
    }

    /// Replace the whole `ITEM_TAG` collection of a party (PUT full-collection
    /// semantics; an empty list clears all; duplicate keys are last-wins).
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse.
    /// - [`SmError`] mapped from `404` — no live party of the routed kind.
    /// - [`SmError`] mapped from `422` — a tag misses its key or violates
    ///   `Inv_key_valid`/`Inv_value_valid` (RM `common.item_tag`).
    /// - [`SmError`] on a storage/database fault during the replace.
    pub async fn party_tags_update(
        &self,
        kind: PartyKind,
        uid_based_id: String,
        body: Vec<openehr_its::rest::generated::common::UpdateItemTag>,
    ) -> Result<Vec<ItemTag>, SmError> {
        let (vo_id, version) = crate::service::ehr::tags::parse_tag_target(&uid_based_id)?;
        Ok(self
            .replace_party_tags(kind, vo_id, version.as_ref(), &body)
            .await?)
    }

    /// Delete one `ITEM_TAG` by key from a party.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse.
    /// - [`SmError`] mapped from `404` — no tag with this key exists.
    /// - [`SmError`] on a storage/database fault during the delete.
    pub async fn party_tags_delete(
        &self,
        kind: PartyKind,
        uid_based_id: String,
        key: String,
    ) -> Result<ServiceResponse, SmError> {
        let (vo_id, version) = crate::service::ehr::tags::parse_tag_target(&uid_based_id)?;
        self.delete_party_tag(kind, vo_id, version.as_ref(), &key)
            .await?;
        Ok(ServiceResponse::plain(Value::Null))
    }

    /// The current party version metadata (the latest `version_uid` for
    /// `ETag`/`Location`/`412` echoes), or `None` if unknown/wrong-kind — the
    /// lean resolve, no node reassembly.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse.
    /// - [`SmError`] on a storage/database fault during the lean read.
    pub async fn demographic_latest_meta(
        &self,
        kind: PartyKind,
        uid_based_id: String,
    ) -> Result<Option<ResourceMeta>, SmError> {
        let vo_id = parse_uid_based_id(&uid_based_id)?.vo_id;
        Ok(self.party_current_meta(kind, vo_id).await?)
    }
}

impl FerroEhrService {
    // ── I_PARTY_RELATIONSHIP + create factory (the SM core) ─────────────────

    /// `create_party_relationship` (`i_demographic_service.adoc`): commit the
    /// first version of a new `PARTY_RELATIONSHIP`.
    ///
    /// # Errors
    /// - [`SmError`] mapped from `422` — the body is not a valid
    ///   `PARTY_RELATIONSHIP` (missing/invalid `source`/`target` `PARTY_REF`s,
    ///   `validate::relationship_check`).
    /// - [`SmError`] `precondition_violation` — the committed version uid does
    ///   not parse (defensive; server-generated).
    /// - [`SmError`] on a storage/database fault during the create transaction.
    pub async fn create_party_relationship(
        &self,
        a_version: UpdateVersion<PartyRelationship>,
    ) -> Result<VoId, SmError> {
        let a_version = crate::service::ehr::canonicalize(a_version);
        let committal = envelope_committal(&a_version);
        let resp = self
            .create_relationship(a_version.data.ok_or_else(no_party_data)?, Some(&committal))
            .await?;
        let (vo_id, _) = parse_version_uid(&version_uid(resp))?;
        Ok(vo_id)
    }

    /// True iff a *live* relationship exists under this versioned-object id (a
    /// logically deleted one reads `Null` → `false`, satisfying the delete
    /// post-condition).
    ///
    /// # Errors
    /// [`SmError`] on a storage/database fault while reading the current
    /// version. A *not-found* resolves to `Ok(false)`, not an error.
    pub async fn has_party_relationship(
        &self,
        a_versioned_party_rel_id: VoId,
    ) -> Result<bool, SmError> {
        // True iff a *live* relationship exists (a logically deleted one reads
        // `Null`, satisfying the delete post-condition).
        match self
            .read_relationship(a_versioned_party_rel_id, None, None)
            .await
        {
            Ok(resp) => Ok(!resp.is_empty()),
            Err(ServiceError::NotFound(_)) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// `get_party_relationship` (`i_party_relationship.adoc`): the current
    /// version's body.
    ///
    /// # Errors
    /// - [`SmError`] `versioned_object_does_not_exist` — no relationship with
    ///   this id exists, or its current version is logically deleted / absent
    ///   (the read is empty).
    /// - [`SmError`] on a storage/database fault during the read.
    pub async fn get_party_relationship(
        &self,
        a_versioned_party_rel_id: VoId,
    ) -> Result<Value, SmError> {
        let resp = self
            .read_relationship(a_versioned_party_rel_id, None, None)
            .await?;
        if resp.is_empty() {
            return Err(SmError::new(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("party relationship {a_versioned_party_rel_id} has no current version"),
            ));
        }
        Ok(resp.body)
    }

    /// `get_party_relationship_at_time` (`i_party_relationship.adoc`): the
    /// Version current at `a_time`. A deleted version at that instant reads
    /// `Null`.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — `a_time` does not parse as an
    ///   ISO-8601 timestamp.
    /// - [`SmError`] `versioned_object_does_not_exist` — no relationship with
    ///   this id, or no version existed at `a_time`.
    /// - [`SmError`] on a storage/database fault during the read.
    pub async fn get_party_relationship_at_time(
        &self,
        a_versioned_party_rel_id: VoId,
        a_time: String,
    ) -> Result<Value, SmError> {
        let at = parse_at_time(&a_time)?;
        let resp = self
            .read_relationship(a_versioned_party_rel_id, None, Some(at))
            .await?;
        Ok(resp.body)
    }

    /// `get_party_relationship_at_version` (`i_party_relationship.adoc`): the
    /// `ORIGINAL_VERSION` named by a full relationship `OBJECT_VERSION_ID`.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse.
    /// - [`SmError`] `object_version_does_not_exist` — the object is not a
    ///   relationship or holds no such version.
    /// - [`SmError`] on a storage/database fault or a signing failure.
    pub async fn get_party_relationship_at_version(
        &self,
        a_party_rel_version_id: String,
    ) -> Result<Value, SmError> {
        let (vo_id, tree) = parse_version_uid(&a_party_rel_version_id)?;
        // The version-addressed read carries `object_version_does_not_exist`
        // itself (`ServiceError` round-trips the granular status losslessly).
        Ok(self.relationship_version(vo_id, tree).await?)
    }

    /// `update_party_relationship` (`i_party_relationship.adoc`): commit a new
    /// relationship version; returns the new version's `OBJECT_VERSION_ID`.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — `preceding_version_uid` does
    ///   not parse as an `OBJECT_VERSION_ID`.
    /// - [`SmError`] mapped from `404` — no live relationship under this id
    ///   (unknown, wrong-kind, or deleted).
    /// - [`SmError`] mapped from `422` — the body fails RM validation.
    /// - [`SmError`] mapped from `409` — the preceding version is stale.
    pub async fn update_party_relationship(
        &self,
        a_versioned_party_rel_id: VoId,
        a_version: UpdateVersion<PartyRelationship>,
    ) -> Result<String, SmError> {
        let a_version = crate::service::ehr::canonicalize(a_version);
        let expected = match &a_version.preceding_version_uid {
            Some(ovid) => Some(components(ovid)?.1),
            None => None,
        };
        let committal = envelope_committal(&a_version);
        let resp = self
            .update_relationship(
                a_versioned_party_rel_id,
                a_version.data.ok_or_else(no_party_data)?,
                expected,
                Some(&committal),
            )
            .await?;
        Ok(version_uid(resp))
    }

    /// `delete_party_relationship` (`i_party_relationship.adoc`): logically
    /// delete the relationship's current version; returns the deleted version's
    /// `OBJECT_VERSION_ID`. The SM `delete_party_relationship` has no version
    /// argument — the current version is deleted unconditionally.
    ///
    /// # Errors
    /// - [`SmError`] `versioned_object_does_not_exist` — no relationship with
    ///   this id has a current version.
    /// - [`SmError`] mapped from `400` — the relationship is already deleted.
    /// - [`SmError`] on a storage/database fault during the delete transaction.
    pub async fn delete_party_relationship(
        &self,
        a_versioned_party_rel_id: VoId,
    ) -> Result<String, SmError> {
        // The SM `delete_party_relationship` has no version argument — resolve
        // the current version once and delete it unconditionally.
        let current = self
            .relationship_current(a_versioned_party_rel_id)
            .await?
            .ok_or_else(|| {
                SmError::new(
                    CallStatusType::VersionedObjectDoesNotExist,
                    format!("party relationship {a_versioned_party_rel_id}"),
                )
            })?;
        let resp = self.commit_relationship_delete(current, None, None).await?;
        Ok(version_uid(resp))
    }

    // ── the relationship wire seam ────────────────────────────────────────────

    /// Create a `PARTY_RELATIONSHIP` (the wire seam of
    /// `create_party_relationship`).
    ///
    /// # Errors
    /// - [`SmError`] mapped from `422` — the body's `_type` is not
    ///   `PARTY_RELATIONSHIP` or fails RM validation.
    /// - [`SmError`] on a storage/database fault during the create transaction.
    pub async fn party_relationship_create(
        &self,
        body: PartyRelationship,
        committal: Option<Committal>,
    ) -> Result<ServiceResponse, SmError> {
        let body = openehr_its::json::to_canonical_value(&body);
        Ok(self.create_relationship(body, committal.as_ref()).await?)
    }

    /// Read a relationship by uid-based id, optionally time-travelled to
    /// `version_at_time`. A deleted current version reads `Null` (→ `204`).
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id or `version_at_time`
    ///   does not parse.
    /// - [`SmError`] mapped from `404` — unknown id, wrong kind, or no version
    ///   at the requested version/instant.
    /// - [`SmError`] on a storage/database fault during the read.
    pub async fn party_relationship_get(
        &self,
        uid_based_id: String,
        version_at_time: Option<String>,
    ) -> Result<ServiceResponse, SmError> {
        let decoded = parse_uid_based_id(&uid_based_id)?;
        let (vo_id, version) = (decoded.vo_id, decoded.tree());
        let at = version_at_time.as_deref().map(parse_at_time).transpose()?;
        Ok(self.read_relationship(vo_id, version, at).await?)
    }

    /// Update a relationship under a mandatory `If-Match` precondition
    /// (ITS-REST overview §"If-Match and accidental overwrites"). The current
    /// version is resolved ONCE (lean): the same handle serves the `If-Match`
    /// compare and the service write gate.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id or the `If-Match` token
    ///   does not parse.
    /// - [`SmError`] `version_mismatch` (`412`) — a full-OVID `If-Match` names
    ///   a version other than the current latest.
    /// - [`SmError`] mapped from `404` — unknown id, wrong kind, or a deleted
    ///   current version.
    /// - [`SmError`] mapped from `422` — the body fails RM validation.
    /// - [`SmError`] mapped from `409` — the expected version is stale.
    pub async fn party_relationship_update(
        &self,
        uid_based_id: String,
        if_match: String,
        body: PartyRelationship,
        committal: Option<Committal>,
    ) -> Result<ServiceResponse, SmError> {
        let body = openehr_its::json::to_canonical_value(&body);
        let vo_id = parse_uid_based_id(&uid_based_id)?.vo_id;
        let current = self.relationship_current(vo_id).await?;
        let meta = current.as_ref().map(CurrentRelationship::resource_meta);
        ensure_full_ovid_if_match(Some(&if_match), meta.as_ref())?;
        let expected = expected_from_if_match(&if_match)?;
        let current = current.ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("PARTY_RELATIONSHIP {vo_id}"),
            )
        })?;
        Ok(self
            .commit_relationship_update(current, body, expected, committal.as_ref())
            .await?)
    }

    /// Logically delete a relationship. Mirrors `party_delete`: the path
    /// carries the versioned-relationship id (bare `HIER_OBJECT_ID` or full
    /// `OBJECT_VERSION_ID`); the preceding version for optimistic concurrency
    /// comes from `If-Match` when supplied, else the path OVID, else `None`
    /// (delete the current version — ITS-REST overview §"If-Match and
    /// accidental overwrites").
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse, or a
    ///   supplied `If-Match` is malformed (rejected, never silently
    ///   ignored).
    /// - [`SmError`] `version_mismatch` (`412`) — a full-OVID `If-Match` names
    ///   a version other than the current latest.
    /// - [`SmError`] mapped from `404` — unknown id or wrong kind.
    /// - [`SmError`] mapped from `400` — the relationship is already deleted.
    /// - [`SmError`] mapped from `409` — the expected version is stale.
    pub async fn party_relationship_delete(
        &self,
        uid_based_id: String,
        if_match: Option<String>,
        update_audit: Option<UpdateAudit>,
    ) -> Result<ServiceResponse, SmError> {
        let decoded = parse_uid_based_id(&uid_based_id)?;
        let (vo_id, path_version) = (decoded.vo_id, decoded.tree());
        // One lean resolve shared by the `If-Match` compare and the delete gate.
        let current = self.relationship_current(vo_id).await?;
        let meta = current.as_ref().map(CurrentRelationship::resource_meta);
        ensure_full_ovid_if_match(if_match.as_deref(), meta.as_ref())?;
        // A malformed `If-Match` is rejected, not silently ignored;
        // an absent header falls back to the path OVID's version.
        let expected = match if_match.as_deref() {
            Some(raw) => expected_from_if_match(raw)?.or(path_version),
            None => path_version,
        };
        let current = current.ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("PARTY_RELATIONSHIP {vo_id}"),
            )
        })?;
        Ok(self
            .commit_relationship_delete(current, expected, update_audit.as_ref())
            .await?)
    }

    /// The `VERSIONED_OBJECT` wrapper for a relationship.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse.
    /// - [`SmError`] mapped from `404` — the id is not a relationship or holds
    ///   no versions.
    /// - [`SmError`] on a storage/database fault reading the version spine.
    pub async fn versioned_party_relationship_get(
        &self,
        versioned_object_uid: String,
    ) -> Result<ServiceResponse, SmError> {
        let vo_id = parse_uid_based_id(&versioned_object_uid)?.vo_id;
        Ok(ServiceResponse::plain(
            self.versioned_relationship(vo_id).await?,
        ))
    }

    /// The `REVISION_HISTORY` of a relationship (RM common master04 §Revision
    /// History).
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse.
    /// - [`SmError`] mapped from `404` — the id is not a relationship.
    /// - [`SmError`] on a storage/database fault reading the version spine.
    pub async fn party_relationship_revision_history(
        &self,
        versioned_object_uid: String,
    ) -> Result<ServiceResponse, SmError> {
        let vo_id = parse_uid_based_id(&versioned_object_uid)?.vo_id;
        Ok(ServiceResponse::plain(
            self.relationship_revision_history(vo_id).await?,
        ))
    }

    /// The relationship `ORIGINAL_VERSION` extant at `version_at_time` (or the
    /// latest when absent), with `ETag`/`Location` metadata.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id or `version_at_time`
    ///   does not parse.
    /// - [`SmError`] mapped from `404` — the id is not a relationship or no
    ///   version existed at the instant.
    /// - [`SmError`] on a storage/database fault or a signing failure.
    pub async fn party_relationship_version_get_at_time(
        &self,
        versioned_object_uid: String,
        version_at_time: Option<String>,
    ) -> Result<ServiceResponse, SmError> {
        let vo_id = parse_uid_based_id(&versioned_object_uid)?.vo_id;
        let at = version_at_time.as_deref().map(parse_at_time).transpose()?;
        Ok(self.relationship_version_at_time(vo_id, at).await?)
    }

    /// The relationship `ORIGINAL_VERSION` named by `version_uid`.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — either id does not parse.
    /// - [`SmError`] mapped from `404` — the id is not a relationship or holds
    ///   no such version.
    /// - [`SmError`] on a storage/database fault or a signing failure.
    pub async fn party_relationship_version_get_by_id(
        &self,
        versioned_object_uid: String,
        version_uid: String,
    ) -> Result<ServiceResponse, SmError> {
        let vo_id = parse_uid_based_id(&versioned_object_uid)?.vo_id;
        let (_, version) = parse_version_uid(&version_uid)?;
        Ok(ServiceResponse::plain(
            self.relationship_version(vo_id, version).await?,
        ))
    }

    /// The current relationship version metadata (the latest `version_uid` for
    /// `ETag`/`Location`/`412` echoes), or `None` if unknown/wrong-kind — the
    /// lean resolve, no node reassembly.
    ///
    /// # Errors
    /// - [`SmError`] `precondition_violation` — the id does not parse.
    /// - [`SmError`] on a storage/database fault during the lean read.
    pub async fn party_relationship_latest_meta(
        &self,
        uid_based_id: String,
    ) -> Result<Option<ResourceMeta>, SmError> {
        let vo_id = parse_uid_based_id(&uid_based_id)?.vo_id;
        Ok(self.relationship_current_meta(vo_id).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VO: &str = "8849182c-82ad-4088-a07f-48ead4180515";

    fn latest(uid: &str) -> ResourceMeta {
        ResourceMeta::new(String::new(), uid.to_owned())
    }

    /// An `If-Match` naming the current latest version in full satisfies the
    /// precondition (ITS-REST overview §"If-Match and accidental overwrites").
    #[test]
    fn matching_full_ovid_passes() {
        let uid = format!("{VO}::openEHRSys.example.com::2");
        assert!(ensure_full_ovid_if_match(Some(&uid), Some(&latest(&uid))).is_ok());
        // The quote tolerance of the library boundary.
        let quoted = format!("\"{uid}\"");
        assert!(ensure_full_ovid_if_match(Some(&quoted), Some(&latest(&uid))).is_ok());
    }

    /// `creating_system_id` is a composite identifier, so a case variant names
    /// the SAME version and must NOT raise a spurious `412` (BASE `base_types`
    /// master05 §"Composite Identifiers and Case").
    #[test]
    fn case_variant_creating_system_id_matches() {
        let stored = format!("{VO}::openEHRSys.example.com::2");
        for variant in [
            format!("{VO}::OPENEHRSYS.EXAMPLE.COM::2"),
            format!("{VO}::openehrsys.example.com::2"),
            format!("{}::openEHRSys.example.com::2", VO.to_uppercase()),
        ] {
            assert!(
                ensure_full_ovid_if_match(Some(&variant), Some(&latest(&stored))).is_ok(),
                "BASE master05 §Composite Identifiers and Case: {variant:?} names the \
                 same version as {stored:?}"
            );
        }
    }

    /// A stale full `OBJECT_VERSION_ID` — a mismatch in ANY segment — is the
    /// `412` branch.
    #[test]
    fn stale_or_foreign_ovid_is_a_version_mismatch() {
        let stored = format!("{VO}::openEHRSys.example.com::2");
        for stale in [
            // wrong version_tree_id
            format!("{VO}::openEHRSys.example.com::1"),
            // wrong creating_system_id (not a mere case variant)
            format!("{VO}::other.system::2"),
            // wrong object_id
            "00000000-0000-4000-8000-0000000000ff::openEHRSys.example.com::2".to_owned(),
        ] {
            let err = ensure_full_ovid_if_match(Some(&stale), Some(&latest(&stored)))
                .expect_err("stale precondition");
            assert_eq!(
                err.status,
                CallStatusType::VersionMismatch,
                "{stale:?} must fail the precondition, got {err:?}"
            );
        }
    }

    /// An absent header, and an object with no current version, defer to the
    /// versioning path (nothing to compare against).
    #[test]
    fn absent_header_or_no_current_version_defers() {
        let uid = format!("{VO}::openEHRSys.example.com::2");
        assert!(ensure_full_ovid_if_match(None, Some(&latest(&uid))).is_ok());
        assert!(ensure_full_ovid_if_match(Some(&uid), None).is_ok());
    }

    /// A non-OVID token carries no full identity to compare here — the RFC 9110
    /// `*` wildcard and the lenient trunk number pass through to
    /// `expected_from_if_match`, which enforces the tree precondition or rejects
    /// the value as malformed (`400`); nothing is silently skipped.
    #[test]
    fn non_ovid_tokens_defer_to_the_tree_precondition() {
        let uid = format!("{VO}::openEHRSys.example.com::2");
        assert!(ensure_full_ovid_if_match(Some("*"), Some(&latest(&uid))).is_ok());
        assert!(ensure_full_ovid_if_match(Some("2"), Some(&latest(&uid))).is_ok());
        // …and the malformed shapes the same call chain rejects downstream.
        assert!(ensure_full_ovid_if_match(Some("garbage"), Some(&latest(&uid))).is_ok());
        assert!(expected_from_if_match("garbage").is_err());
    }
}
