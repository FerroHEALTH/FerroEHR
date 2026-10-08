// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Version reads: loading stored versions into the versioning value contract
//! ([`VersionRead`]) — current, by tree id, by storage ordinal, and by instant.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Versioned Objects /
//! §Version and its Subtypes / §Logical Deletion, RM common
//! `master08-versioning.adoc` §Change Management (time-travel reads). All SQL
//! is delegated to `crate::storage::version_repo`; the canonical body comes
//! from `crate::storage::node_repo` via the storage read shape. The served
//! wire forms are built from these reads by [`super::wire`].

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the serialized version envelope is the \
              signed artifact (RM common master06 §Digital Signature) — re-encoding breaks \
              verification"
)]

use serde_json::Value;
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::service::error::{ServiceError, Violation};
use crate::versioning::Kind;
use crate::versioning::audit::AuditInput;
use crate::versioning::lifecycle;
use crate::versioning::object_version_id::TreeId;

/// The wrapper-level provenance of the `ORIGINAL_VERSION` an
/// `IMPORTED_VERSION` wraps, held as the verbatim canonical fragments the
/// source system sent (RM common master06 §Copying: "the `ORIGINAL_VERSION`
/// instance is never modified"). Its presence on a [`VersionRead`] is what
/// makes the version an `IMPORTED_VERSION` (master06 §Committal and Audits).
#[derive(Debug, Clone)]
pub(crate) struct WrappedOriginal {
    /// The wrapped original's own `VERSION.contribution` — an `OBJECT_REF` to a
    /// CONTRIBUTION in the SOURCE system, kept verbatim.
    pub(crate) contribution: Value,
    /// The wrapped original's own `VERSION.commit_audit`, kept verbatim
    /// (including its foreign `time_committed` and concrete class).
    pub(crate) commit_audit: Value,
    /// The wrapped original's own `VERSION.signature` (0..1) — foreign, stored
    /// verbatim and never re-verified (master06 §Digital Signature).
    pub(crate) signature: Option<String>,
}

impl WrappedOriginal {
    /// Decode the stored `version.wrapped_original` fragment.
    ///
    /// # Errors
    /// [`ServiceError::Unprocessable`] when the stored fragment does not carry
    /// the two mandatory `VERSION` attributes (`contribution` 1..1,
    /// `commit_audit` 1..1 — RM common `version.adoc` §Attributes).
    fn decode(vo_id: VoId, fragment: &Value) -> Result<Self, ServiceError> {
        let field = |name: &str| {
            fragment.get(name).cloned().ok_or_else(|| {
                ServiceError::content_invalid(
                    Violation::new(format!(
                        "is missing from the wrapped ORIGINAL_VERSION stored for \
                         versioned object {vo_id}"
                    ))
                    .with_path(format!("ORIGINAL_VERSION.{name}")),
                )
            })
        };
        Ok(Self {
            contribution: field("contribution")?,
            commit_audit: field("commit_audit")?,
            signature: fragment
                .get("signature")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
}

/// A loaded version: its full provenance metadata and reassembled canonical
/// JSON (with attestations attached).
#[derive(Debug, Clone)]
pub(crate) struct VersionRead {
    pub(crate) vo_id: VoId,
    pub(crate) ehr_id: Option<EhrId>,
    /// The stored RM kind of the versioned object this version belongs to —
    /// the discriminator a kind-scoped resource read refuses a wrong-kind
    /// `uid_based_id` with (ITS-REST `404_unknown_ehr_id_or_uid_based_id`:
    /// the id is not a representation of THAT resource).
    pub(crate) kind: Kind,
    pub(crate) tree: TreeId,
    pub(crate) preceding_version_uid: Option<String>,
    pub(crate) other_input_version_uids: Vec<String>,
    pub(crate) lifecycle_state: String,
    /// The immutable identity of the system that created this version (RM common
    /// master06 §Distributed Versioning), the middle part of its
    /// `OBJECT_VERSION_ID`.
    pub(crate) creating_system_id: String,
    /// The CONTRIBUTION this version was committed in — on an imported version
    /// the LOCAL import CONTRIBUTION (master06 §Committal and Audits).
    pub(crate) contribution_id: Uuid,
    /// The mandatory `VERSION.commit_audit` (1..1) — on an imported version the
    /// LOCAL act of committal.
    pub(crate) audit: AuditInput,
    /// The commit instant of [`Self::audit`] — on an imported version the local
    /// import instant, never the source's (master06 §Copying: "the commit times
    /// always reflect the local (more recent) act of committal").
    pub(crate) time_committed: jiff::Timestamp,
    /// The stored `VERSION.signature` (0..1; RM common master06 §Digital
    /// Signature), or `None` for versions committed before signing was enabled.
    /// On an imported version this is the `IMPORTED_VERSION` wrapper's own.
    pub(crate) signature: Option<String>,
    /// Whether [`Self::signature`] was supplied verbatim by the client rather
    /// than generated by this server — client-supplied signatures are never
    /// re-verified at read (master06 §Digital Signature).
    pub(crate) signature_client_supplied: bool,
    /// `Some` iff this version is an `IMPORTED_VERSION`: the wrapped
    /// `ORIGINAL_VERSION`'s own provenance (master06 §Committal and Audits).
    pub(crate) wrapped: Option<WrappedOriginal>,
    /// The commit-time origin stamp (`version.origins`), or `None` for a
    /// row nothing stamped; read through
    /// [`crate::versioning::origins::of_stored`].
    pub(crate) origins: Option<Value>,
    /// The template id and root archetype id the access record classifies
    /// this version by (EHDS Annex II 3.2(c), #3621).
    pub(crate) content: crate::system_log::categories::ContentIds,
    /// The reassembled canonical JSON, or `Value::Null` for a deleted version
    /// (a logical delete stores no node rows — master06 §Logical Deletion).
    pub(crate) canonical: Value,
    /// The `ATTESTATION`s that were on this version AT the act of committal, in
    /// commit order (RM common master06 §Attestation, "Signing content at
    /// committal"; SM `UML/classes/update_version.adoc` §Attributes
    /// `UPDATE_VERSION.attestations`). They are part of the version's signed
    /// canonical form — master06 §Digital Signature signs "the entire Version
    /// object", excluding only `signature` — so they are built into the value
    /// BEFORE signature verification.
    pub(crate) attestations_at_committal: Vec<Value>,
    /// The `ATTESTATION`s added to this version AFTER committal, in commit
    /// order (master06 §Attestation: "Attestations can be added at any time
    /// after committal of the content being attested"; §Contributions:
    /// "attestation of item: a new `ATTESTATION` is added to the attestations
    /// list of an existing `ORIGINAL_VERSION`"). They post-date the signature
    /// and are therefore appended **after** verification.
    pub(crate) attestations_after_committal: Vec<Value>,
}

impl VersionRead {
    /// Whether this version is logically deleted (`lifecycle_state` `523`).
    pub(crate) fn deleted(&self) -> bool {
        self.lifecycle_state == lifecycle::state::DELETED
    }
}

/// Compose a [`VersionRead`] from the storage read shape
/// ([`crate::storage::version_repo::read::StoredVersion`]): the tree id is rebuilt
/// from its column ints and the flattened audit becomes the `commit_audit`.
/// A locally committed deleted version (lifecycle `523`) stores no node rows, so
/// storage already yields `canonical = Value::Null`: master06 §Logical Deletion
/// states the data removal and the state change as one act, and every
/// content-carrying local route refuses the `deleted` state
/// ([`crate::versioning::lifecycle::reject_deleted_with_data`]). The EHR-Extract
/// import replay is the one exception, reproducing a foreign `ORIGINAL_VERSION`
/// verbatim (master06 §Copying), so a foreign `523` version that arrived
/// carrying data reads back with that content.
///
/// This is also the one seam a stored version body passes through on its way out
/// of `version` and `node`, so it carries two read-time gates. The first is the
/// restriction mark (`vo_head.restricted_at`): a restricted object answers
/// `403`, because GDPR Art. 18(2) leaves storage as the only processing it
/// admits (`docs/law/eu/gdpr/text.html`) — our own design/extension, no openEHR
/// spec governs restriction of processing. The second is the `spec_profile` gate
/// ([`crate::versioning::profile::gate`]): under the `stable` profile a version
/// whose body only the development generations can express is a typed refusal.
/// The gate sits here rather than per handler because every served kind reaches
/// the wire through this function. AQL reads `node` rows directly instead, so it
/// carries the same gate twice: on the query text at planning time
/// (`crate::aql::analyze`) and on every projected version body at result
/// assembly (`crate::versioning::profile::gate_result_bodies`).
///
/// # Errors
/// [`ServiceError::Unprocessable`] when an imported row's stored wrapped
/// `ORIGINAL_VERSION` fragment is not decodable, or when a stored commit-audit
/// jsonb column is not the RM value it holds
/// ([`crate::versioning::audit::AuditInput::from_meta`] carries the same
/// rejections for the metadata-only read); the `403`-class
/// [`ServiceError::Restricted`] when the object carries a restriction mark; the
/// `409`-class [`ServiceError::Conflict`] of the `spec_profile` gate.
fn version_read(
    profile: crate::config::profile::SpecProfile,
    stored: crate::storage::version_repo::read::StoredVersion,
) -> Result<VersionRead, ServiceError> {
    // The `ck_version_kind` CHECK constraint admits exactly the `Kind` set,
    // so a stored discriminator that does not classify is corrupted data, not
    // a client condition — the loud answer is a fault, never a silent skip of
    // the profile gate below.
    let kind = Kind::from_type(&stored.kind).ok_or_else(|| {
        ServiceError::exception(format!(
            "version.kind {:?} of versioned object {} is not an RM versioned type",
            stored.kind, stored.vo_id
        ))
    })?;
    let tree = TreeId::from_columns(
        stored.trunk_version,
        stored.branch_number,
        stored.branch_version,
    );
    // Restriction of processing, at the same seam and for the same reason as
    // the profile gate below: every served version body passes through here.
    // GDPR Art. 18(2) leaves storage as the only processing a restricted object
    // admits (`docs/law/eu/gdpr/text.html`), so a marked object is refused
    // rather than served. No openEHR spec governs restriction — our own
    // design/extension.
    if stored.restricted_at.is_some() {
        return Err(ServiceError::restricted(&format!(
            "versioned object {}",
            stored.vo_id
        )));
    }
    crate::versioning::profile::gate(
        profile,
        kind,
        stored.stable_compatible,
        &stored.canonical,
        &|| {
            crate::versioning::object_version_id::object_version_id(
                stored.vo_id,
                &stored.creating_system_id,
                tree,
            )
        },
    )?;
    let wrapped = stored
        .wrapped_original
        .as_ref()
        .map(|fragment| WrappedOriginal::decode(stored.vo_id, fragment))
        .transpose()?;
    let content = crate::system_log::categories::ContentIds::of_kind(
        &stored.kind,
        stored.template_id.clone(),
        stored.root_archetype.clone(),
    );
    Ok(VersionRead {
        vo_id: stored.vo_id,
        ehr_id: stored.ehr_id,
        kind,
        tree,
        preceding_version_uid: stored.preceding_version_uid,
        other_input_version_uids: stored.other_input_version_uids,
        lifecycle_state: stored.lifecycle_state,
        creating_system_id: stored.creating_system_id,
        contribution_id: stored.contribution_id,
        audit: AuditInput {
            system_id: stored.audit_system_id,
            change_type: stored.audit_change_type,
            description: stored
                .audit_description
                .as_ref()
                .map(crate::versioning::audit::decode_description)
                .transpose()?,
            committer: crate::versioning::audit::party_proxy(&stored.audit_committer)?,
            attestation: stored
                .audit_attestation
                .as_ref()
                .map(crate::versioning::attestation::AttestationParts::decode)
                .transpose()?
                .map(Box::new),
        },
        time_committed: stored.time_committed,
        signature: stored.signature,
        signature_client_supplied: stored.signature_client_supplied,
        origins: stored.origins,
        content,
        wrapped,
        canonical: stored.canonical,
        attestations_at_committal: stored.attestations_at_committal,
        attestations_after_committal: stored.attestations_after_committal,
    })
}

/// Read the current version of an object by id (any kind). `None` if it never
/// existed; a deleted current version is returned with `canonical = Null` and a
/// `523` lifecycle so callers can distinguish 404 (never existed) from a
/// deleted read (RM common master06 §Logical Deletion).
///
/// # Errors
/// The storage read error of `version_repo::read::read_current`, or the
/// `spec_profile` refusal of [`version_read`].
pub(crate) async fn read_current(
    pool: &sqlx::PgPool,
    profile: crate::config::profile::SpecProfile,
    vo_id: VoId,
) -> Result<Option<VersionRead>, ServiceError> {
    crate::storage::version_repo::read::read_current(pool, vo_id)
        .await?
        .map(|stored| version_read(profile, stored))
        .transpose()
}

/// A version read whose body may still be the stored jsonb text — the
/// JSON-accept passthrough shape.
///
/// `raw_json = Some(text)` iff the body never needed parsing: a locally
/// created version (no `IMPORTED_VERSION` wrapper) whose `spec_profile` gate
/// decided on the commit-time stamp alone; `read.canonical` is `Value::Null`
/// then. Otherwise the body was parsed into `read.canonical` and `raw_json`
/// is `None` — the ordinary [`VersionRead`] contract.
#[derive(Debug)]
pub(crate) struct RawVersionRead {
    /// The loaded version (see [`RawVersionRead::raw_json`] for its body
    /// representation).
    pub(crate) read: VersionRead,
    /// The stored body's own jsonb text, when it never needed parsing.
    pub(crate) raw_json: Option<String>,
}

/// Compose a [`RawVersionRead`]: parse the raw text back into a typed value
/// wherever one is still needed — an imported version (the uid re-stamp path),
/// a `stable`-profile read the commit-time stamp cannot decide, or a row with
/// no origin stamp whose origins the access record assesses from the body
/// (#3212) — else keep the text verbatim for the passthrough.
///
/// # Errors
/// The [`version_read`] rejections; [`ServiceError::Internal`] when the
/// stored body text does not parse (our own jsonb rendering — corrupt data,
/// never a client condition).
fn raw_version_read(
    profile: crate::config::profile::SpecProfile,
    mut stored: crate::storage::version_repo::read::StoredVersion,
) -> Result<RawVersionRead, ServiceError> {
    let needs_value = stored.wrapped_original.is_some()
        || stored.origins.is_none()
        || (profile == crate::config::profile::SpecProfile::Stable
            && stored.stable_compatible != Some(true));
    let mut raw_json = stored.canonical_text.take();
    if needs_value && let Some(text) = raw_json.as_deref() {
        stored.canonical = serde_json::from_str(text).map_err(|e| {
            ServiceError::exception(format!(
                "the stored body of versioned object {} is not decodable JSON: {e}",
                stored.vo_id
            ))
        })?;
        raw_json = None;
    }
    Ok(RawVersionRead {
        read: version_read(profile, stored)?,
        raw_json,
    })
}

/// [`read_current`] with the body kept as the stored jsonb text when nothing
/// needs it parsed — the JSON-accept passthrough read.
///
/// # Errors
/// The storage read error of `version_repo::read::read_current_raw`, or the
/// [`raw_version_read`] rejections.
pub(crate) async fn read_current_raw(
    pool: &sqlx::PgPool,
    profile: crate::config::profile::SpecProfile,
    vo_id: VoId,
) -> Result<Option<RawVersionRead>, ServiceError> {
    crate::storage::version_repo::read::read_current_raw(pool, vo_id)
        .await?
        .map(|stored| raw_version_read(profile, stored))
        .transpose()
}

/// [`read_version`] with the body kept as the stored jsonb text when nothing
/// needs it parsed — the JSON-accept passthrough read.
///
/// # Errors
/// The storage read error of `version_repo::read::read_version_raw`, or the
/// [`raw_version_read`] rejections.
pub(crate) async fn read_version_raw(
    pool: &sqlx::PgPool,
    profile: crate::config::profile::SpecProfile,
    vo_id: VoId,
    tree: TreeId,
) -> Result<Option<RawVersionRead>, ServiceError> {
    let (t, b, v) = tree.columns();
    crate::storage::version_repo::read::read_version_raw(pool, vo_id, t, b, v)
        .await?
        .map(|stored| raw_version_read(profile, stored))
        .transpose()
}

/// Read the current versions of a SET of objects in ONE statement (the
/// extract export's demographics batch), each passing the same `spec_profile`
/// gate a point read passes; keyed by `vo_id`, with absent objects simply
/// missing from the map.
///
/// # Errors
/// The storage read error of `version_repo::read::read_currents`, or the
/// `spec_profile` refusal of [`version_read`] on any member.
pub(crate) async fn read_currents(
    pool: &sqlx::PgPool,
    profile: crate::config::profile::SpecProfile,
    vo_ids: &[VoId],
) -> Result<std::collections::HashMap<VoId, VersionRead>, ServiceError> {
    let stored = crate::storage::version_repo::read::read_currents(pool, vo_ids).await?;
    let mut out = std::collections::HashMap::with_capacity(stored.len());
    for s in stored {
        let vo_id = s.vo_id;
        out.insert(vo_id, version_read(profile, s)?);
    }
    Ok(out)
}

/// Read a specific version of an object by its STORAGE ORDINAL (`sys_version`)
/// — for internal callers that key rows by ordinal (extract export
/// iteration), never for wire version ids.
///
/// # Errors
/// The storage read error of `version_repo::read::read_version_by_ordinal`, or
/// the `spec_profile` refusal of [`version_read`].
pub(crate) async fn read_version_by_ordinal(
    pool: &sqlx::PgPool,
    profile: crate::config::profile::SpecProfile,
    vo_id: VoId,
    ordinal: i32,
) -> Result<Option<VersionRead>, ServiceError> {
    crate::storage::version_repo::read::read_version_by_ordinal(pool, vo_id, ordinal)
        .await?
        .map(|stored| version_read(profile, stored))
        .transpose()
}

/// Read a specific version of an object by its `VERSION_TREE_ID`
/// (`.../version/{version_uid}` — trunk or branch).
///
/// # Errors
/// The storage read error of `version_repo::read::read_version`, or the
/// `spec_profile` refusal of [`version_read`].
pub(crate) async fn read_version(
    pool: &sqlx::PgPool,
    profile: crate::config::profile::SpecProfile,
    vo_id: VoId,
    tree: TreeId,
) -> Result<Option<VersionRead>, ServiceError> {
    let (t, b, v) = tree.columns();
    crate::storage::version_repo::read::read_version(pool, vo_id, t, b, v)
        .await?
        .map(|stored| version_read(profile, stored))
        .transpose()
}

/// Read a SET of specific versions in ONE statement (the
/// resolved-CONTRIBUTION batch), each passing the same `spec_profile` gate a
/// point read passes; keyed by `(vo_id, tree)`, with absent versions simply
/// missing from the map.
///
/// # Errors
/// The storage read error of `version_repo::read::read_versions_by_tree`, or
/// the `spec_profile` refusal of [`version_read`] on any member.
pub(crate) async fn read_versions(
    pool: &sqlx::PgPool,
    profile: crate::config::profile::SpecProfile,
    refs: &[(VoId, TreeId)],
) -> Result<std::collections::HashMap<(VoId, TreeId), VersionRead>, ServiceError> {
    let columns: Vec<(VoId, (i32, i32, i32))> = refs
        .iter()
        .map(|(id, tree)| (*id, tree.columns()))
        .collect();
    let stored = crate::storage::version_repo::read::read_versions_by_tree(pool, &columns).await?;
    let mut out = std::collections::HashMap::with_capacity(stored.len());
    for s in stored {
        let key = (
            s.vo_id,
            TreeId::from_columns(s.trunk_version, s.branch_number, s.branch_version),
        );
        out.insert(key, version_read(profile, s)?);
    }
    Ok(out)
}

/// Read the version of an object that was current at a given instant
/// (time-travel; RM common master08 §Change Management — any previous state is
/// reconstructable): the trunk row with the greatest `committed_at` at or
/// before `at`.
///
/// # Errors
/// The storage read error of `version_repo::read::version_at`, or the
/// `spec_profile` refusal of [`version_read`].
pub(crate) async fn version_at(
    pool: &sqlx::PgPool,
    profile: crate::config::profile::SpecProfile,
    vo_id: VoId,
    at: jiff::Timestamp,
) -> Result<Option<VersionRead>, ServiceError> {
    crate::storage::version_repo::read::version_at(pool, vo_id, at)
        .await?
        .map(|stored| version_read(profile, stored))
        .transpose()
}

/// Read the current version of the EHR's one container of `kind` — the
/// container resolution and the version read in ONE statement (`EHR_STATUS`;
/// see the storage fn's single-container precondition).
///
/// # Errors
/// The storage read error, or the `spec_profile` refusal of [`version_read`].
pub(crate) async fn read_current_of_kind(
    pool: &sqlx::PgPool,
    profile: crate::config::profile::SpecProfile,
    ehr_id: EhrId,
    kind: Kind,
) -> Result<Option<VersionRead>, ServiceError> {
    crate::storage::version_repo::read::read_current_of_kind(pool, ehr_id, kind.as_str())
        .await?
        .map(|stored| version_read(profile, stored))
        .transpose()
}

/// [`read_current_of_kind`]'s time-travel form (the container's TRUNK version
/// whose validity contains `at`).
///
/// # Errors
/// The storage read error, or the `spec_profile` refusal of [`version_read`].
pub(crate) async fn version_at_of_kind(
    pool: &sqlx::PgPool,
    profile: crate::config::profile::SpecProfile,
    ehr_id: EhrId,
    kind: Kind,
    at: jiff::Timestamp,
) -> Result<Option<VersionRead>, ServiceError> {
    crate::storage::version_repo::read::version_at_of_kind(pool, ehr_id, kind.as_str(), at)
        .await?
        .map(|stored| version_read(profile, stored))
        .transpose()
}

/// Read the current version of the EHR's DIRECTORY folder — the `ehr_folder`
/// slot resolution and the version read in ONE statement, with the same slot
/// choice `storage::ehr_repo::directory_vo` makes.
///
/// # Errors
/// The storage read error, or the `spec_profile` refusal of [`version_read`].
pub(crate) async fn read_current_directory(
    pool: &sqlx::PgPool,
    profile: crate::config::profile::SpecProfile,
    ehr_id: EhrId,
) -> Result<Option<VersionRead>, ServiceError> {
    crate::storage::version_repo::read::read_current_directory(pool, ehr_id)
        .await?
        .map(|stored| version_read(profile, stored))
        .transpose()
}

/// The kind of the current version of an object, or `None` if it does not
/// exist.
///
/// # Errors
/// The storage read error of `version_repo::meta::object_kind`.
pub(crate) async fn object_kind(
    pool: &sqlx::PgPool,
    vo_id: VoId,
) -> Result<Option<Kind>, ServiceError> {
    Ok(crate::storage::version_repo::meta::object_kind(pool, vo_id)
        .await?
        .and_then(|kind| Kind::from_type(&kind)))
}

/// The lean current-version handle for a demographic (ehr-less) versioned
/// object: its kind-checked identity ([`Kind`] + `VERSION_TREE_ID` +
/// `creating_system_id`, the `ETag`/`If-Match` parts), commit instant, and
/// lifecycle-derived `deleted` flag — from ONE `version`⋈`audit` read, with
/// no node reassembly or attestation load. The wire seam uses this both for the
/// `If-Match` `ETag` and the not-deleted write gate without the full
/// [`read_current`] node read (RM common master06 §Version Identification /
/// §Logical Deletion).
#[derive(Debug, Clone)]
pub(crate) struct DemographicCurrent {
    pub(crate) kind: Kind,
    pub(crate) tree: TreeId,
    pub(crate) creating_system_id: String,
    pub(crate) time_committed: jiff::Timestamp,
    pub(crate) deleted: bool,
}

/// Resolve the current trunk version of a demographic object, or `None` if it
/// has no current version (or the stored kind is unrecognized).
///
/// # Errors
/// The storage read error of `version_repo::meta::current_demographic_meta`.
pub(crate) async fn demographic_current(
    pool: &sqlx::PgPool,
    vo_id: VoId,
) -> Result<Option<DemographicCurrent>, ServiceError> {
    let Some(m) = crate::storage::version_repo::meta::current_demographic_meta(pool, vo_id).await?
    else {
        return Ok(None);
    };
    let Some(kind) = Kind::from_type(&m.kind) else {
        return Ok(None);
    };
    Ok(Some(DemographicCurrent {
        kind,
        tree: TreeId::from_columns(m.trunk_version, m.branch_number, m.branch_version),
        creating_system_id: m.creating_system_id,
        time_committed: m.time_committed,
        deleted: m.lifecycle_state == lifecycle::state::DELETED,
    }))
}
