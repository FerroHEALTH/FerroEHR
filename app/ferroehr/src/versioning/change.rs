// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The change-set unit: `Change`, the version-tree placement **decision**,
//! and the shared commit engine `apply_change`.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Version and its
//! Subtypes / §Version Lifecycle / §Distributed Versioning / §Logical Deletion.
//! Versioning owns the *decisions* — where a new version sits in the tree
//! (trunk continue / branch fork / lineage close), which lifecycle transition
//! is legal, and what to sign — and the *builders*; all `sqlx` execution is
//! delegated to `crate::storage::version_repo` (row I/O) and
//! `crate::storage::node_repo` (canonical decompose/reassemble). No openEHR
//! spec governs the SQL — our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the serialized version envelope is the \
              signed artifact (RM common master06 §Digital Signature) — re-encoding breaks \
              verification"
)]

use openehr_base::prelude::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::lexical::composite_ids_equal;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::service::error::ServiceError;
use crate::service::status::CallStatusType;
use crate::storage::codec::{decompose, reassemble};
use crate::storage::row::NodeRow;
use crate::versioning::attestation::{self, PendingAttest};
use crate::versioning::audit::AuditInput;
use crate::versioning::lifecycle::{
    self, reject_deleted_with_data, resolve_lifecycle, validate_transition,
};
use crate::versioning::object_version_id::{TreeId, VersionIdError, object_version_id};
use crate::versioning::{Kind, SigningCtx, integrity, profile};

/// The outcome of a versioned-object write: the object id, the new version's
/// tree id, and the provenance carried into the event-outbox envelope.
#[derive(Debug, Clone)]
#[expect(
    clippy::struct_field_names,
    reason = "`time_committed` is the master06 domain term, not a suffix echo of \
              the struct name"
)]
pub struct Committed {
    /// The written object's id.
    pub vo_id: VoId,
    /// The per-vo storage commit ordinal of the written row (the node /
    /// attestation key) — NOT the wire version number.
    pub sys_version: i32,
    /// The new version's `VERSION_TREE_ID` (the wire version identity).
    pub tree: TreeId,
    /// The `creating_system_id` recorded for the new version.
    pub creating_system_id: String,
    /// Which kind of versioned object was written.
    pub kind: Kind,
    /// The numeric `audit_change_type` group code recorded for this version.
    pub change_type: String,
    /// The OPT `template_id` a COMPOSITION was committed against (`None`
    /// otherwise).
    pub template_id: Option<String>,
    /// The archetype id of the written version's root node, case-folded as the
    /// node table stores it; `None` for a logical delete, which writes no node.
    pub root_archetype: Option<String>,
    /// The server-computed commit instant (the audit `time_committed`,
    /// master06 §Committal and Audits) — the write response's `Last-Modified`, carried
    /// here so the service layer never re-reads the row it just wrote.
    pub time_committed: jiff::Timestamp,
}

/// The outcome of one CONTRIBUTION commit.
///
/// The commit instant is carried out with the id because the write response
/// needs both: ITS-REST `Requests_and_responses.md` §"`ETag` and `Last-Modified`"
/// asks for `Last-Modified` on any resource with a unique state identifier,
/// derived from the commit audit's `time_committed` — re-reading the row (or
/// taking a second clock reading) would risk an instant that is not the one
/// stored.
#[derive(Debug)]
pub(crate) struct CommittedContribution {
    /// The new CONTRIBUTION's `HIER_OBJECT_ID` uid (the `ETag`/`Location`
    /// value).
    pub id: Uuid,
    /// The CONTRIBUTION audit's server-set `time_committed` (RM common
    /// master06 §Committal and Audits) — one instant for the whole change set.
    pub time_committed: jiff::Timestamp,
    /// The versions the CONTRIBUTION landed, in commit order (attestation-only
    /// members last).
    pub versions: Vec<Committed>,
}

impl Committed {
    /// The identifiers the access record classifies this write by (EHDS Annex
    /// II 3.2(c), #3621); `None` for a version that carries no content (a
    /// logical delete), which wrote no priority-category data.
    #[must_use]
    pub fn content_ids(&self) -> Option<crate::system_log::categories::ContentIds> {
        let ids = crate::system_log::categories::ContentIds::of_kind(
            self.kind.as_str(),
            self.template_id.clone(),
            self.root_archetype.clone(),
        );
        (ids.resource_kind.is_some() || ids.template_id.is_some() || ids.archetype_id.is_some())
            .then_some(ids)
    }

    /// The committed version's full `OBJECT_VERSION_ID` (`ETag`/`Location`
    /// value — RM common master06 §Version Identification).
    #[must_use]
    pub fn version_uid(&self) -> String {
        object_version_id(self.vo_id, &self.creating_system_id, self.tree)
    }

    /// The per-version entry for the PHI-free event-outbox envelope: identity +
    /// provenance metadata only, never clinical content.
    ///
    /// NOTE: no openEHR spec governs eventing — our own extension. The
    /// outbox row is written inside the commit transaction by storage; this
    /// only builds the payload (README cross-ruling: extensions build payloads).
    pub(crate) fn envelope_entry(&self) -> Value {
        serde_json::json!({
            "vo_id": self.vo_id,
            "kind": self.kind.as_str(),
            "sys_version": self.sys_version,
            "version_tree_id": self.tree.to_string(),
            "change_type": self.change_type,
            "template_id": self.template_id,
        })
    }
}

/// Meter a just-committed version on the
/// [`COMPOSITIONS_COMMITTED`](crate::telemetry::metrics::COMPOSITIONS_COMMITTED)
/// counter; kinds other than COMPOSITION are ignored.
///
/// Every commit route calls this after its transaction has committed, so a
/// rolled-back write is never counted; the increment therefore cannot live in
/// [`apply_change`], which runs inside the commit transaction.
///
/// NOTE: no openEHR spec governs telemetry — our own design/extension. The
/// `change_type` label carries the numeric `audit_change_type` group code the RM
/// stores (`AUDIT_DETAILS.Change_type_valid`, RM common master04 §Audit
/// Details), so the series survives terminology display-text changes.
pub(crate) fn meter_committed(committed: &Committed) {
    if committed.kind == Kind::Composition {
        crate::telemetry::metrics::metrics()
            .compositions_committed
            .add(
                1,
                &[opentelemetry::KeyValue::new(
                    "change_type",
                    committed.change_type.clone(),
                )],
            );
    }
}

/// One change applied within a CONTRIBUTION — the openEHR change-set unit
/// (RM common master06 §Contributions).
///
/// `signature` carries a client-supplied `UPDATE_VERSION.signature` (master06
/// §Digital Signature): present, it is stored verbatim and the server does not
/// re-sign; absent, the server signs the assembled `ORIGINAL_VERSION` if signing
/// is enabled. The direct endpoints always pass `None`.
///
/// `lifecycle_state` on `Create`/`Modify` carries the client-supplied
/// `version_lifecycle_state` (master06 §Version Lifecycle); `None` defaults to
/// `532|complete|`, and `523|deleted|` is reserved to [`Change::Delete`].
/// [`apply_change`] checks its legality against the preceding version's state.
pub(crate) enum Change {
    /// Create a new versioned object.
    Create {
        kind: Kind,
        canonical: Value,
        template_id: Option<String>,
        signature: Option<String>,
        lifecycle_state: Option<String>,
        /// Wire `UPDATE_VERSION.attestations` committed with this version
        /// (master06 §Attestation "Signing content at committal").
        attestations: Vec<attestation::AttestationInput>,
    },
    /// Commit a new version of an existing object.
    Modify {
        vo_id: VoId,
        kind: Kind,
        canonical: Value,
        expected: Option<TreeId>,
        template_id: Option<String>,
        signature: Option<String>,
        lifecycle_state: Option<String>,
        attestations: Vec<attestation::AttestationInput>,
    },
    /// Logically delete an object (a content-less `deleted` version — master06
    /// §Logical Deletion).
    Delete {
        vo_id: VoId,
        kind: Kind,
        expected: Option<TreeId>,
        signature: Option<String>,
    },
}

impl Change {
    /// The versioned object this change writes to, when it already exists.
    ///
    /// `None` on a create: the object is minted by the change itself, so the
    /// only mark that can reach it is the EHR's.
    pub(crate) fn target_vo_id(&self) -> Option<VoId> {
        match *self {
            Change::Create { .. } => None,
            Change::Modify { vo_id, .. } | Change::Delete { vo_id, .. } => Some(vo_id),
        }
    }

    /// The versioned-object [`Kind`] this change writes.
    pub(crate) fn kind(&self) -> Kind {
        match *self {
            Change::Create { kind, .. }
            | Change::Modify { kind, .. }
            | Change::Delete { kind, .. } => kind,
        }
    }
}

/// The caller's `UPDATE_VERSION` envelope pieces a direct write threads into
/// the commit (ITS-REST committal-header merge — the attributes "MUST be
/// merged … on commit runtime"): the lifecycle state, a verbatim client
/// signature, and accompanying attestations. `Default` = the plain server
/// commit (532|complete|, server-signed, none).
#[derive(Debug, Default)]
pub(crate) struct WriteEnvelope {
    /// `UPDATE_VERSION.lifecycle_state` (None → 532|complete|).
    pub(crate) lifecycle_state: Option<String>,
    /// A client-supplied `VERSION.signature`, stored verbatim (master06
    /// §Digital Signature).
    pub(crate) signature: Option<String>,
    /// `UPDATE_VERSION.attestations` committed with the version.
    pub(crate) attestations: Vec<attestation::AttestationInput>,
}

/// The preceding lineage tip read for a tree-placement decision — mapped from
/// the storage row (`crate::storage::version_repo::placement::next_placement`).
#[derive(Debug, Clone)]
struct PrecedingTip {
    ehr_id: Option<EhrId>,
    kind: Kind,
    tree: TreeId,
    creating_system_id: String,
    /// The preceding version's lifecycle state — the "from" state of the
    /// transition.
    lifecycle_state: String,
    /// Whether this version is still the tip of its own lineage.
    open: bool,
}

/// Map a storage tip row onto the versioning value contract ([`PrecedingTip`]).
fn preceding_tip(
    row: crate::storage::version_repo::placement::TipRow,
) -> Result<PrecedingTip, ServiceError> {
    let kind = Kind::from_type(&row.kind).ok_or_else(|| {
        ServiceError::exception(format!("unknown versioned-object kind {:?}", row.kind))
    })?;
    Ok(PrecedingTip {
        ehr_id: row.ehr_id,
        kind,
        tree: TreeId::from_columns(row.trunk_version, row.branch_number, row.branch_version),
        creating_system_id: row.creating_system_id,
        lifecycle_state: row.lifecycle_state,
        open: row.open,
    })
}

/// The resolved placement of a new version in the version tree.
struct NextVersion {
    ordinal: i32,
    tree: TreeId,
    /// The `preceding_version_uid` to store.
    preceding_uid: String,
    /// The preceding version's lifecycle state (the transition "from" state).
    preceding_lifecycle: String,
    /// The transaction timestamp — the commit instant every row of this
    /// transaction stamps, carried by the merged placement read so the
    /// signature is computable before any insert.
    now: jiff::Timestamp,
}

/// Validate an update/delete target (belongs to `ehr_id`, tip is the addressed
/// open lineage tip) and resolve where the new version sits (RM common master06
/// §The 'Virtual Version Tree' / §Distributed Versioning):
///
/// - the preceding version is the current TRUNK tip when `expected` is absent,
///   or exactly the version `expected` names (trunk or branch) — which must be
///   an open lineage tip, else `VersionConflict`;
/// - a preceding version created by THIS system is continued on its lineage
///   (trunk `N` → `N+1`; branch `t.b.v` → `t.b.v+1`), superseding it;
/// - a preceding version created by ANOTHER system (an imported copy) FORKS a
///   new branch `t.(max_branch+1).1` (master06 §Subsequent Local
///   Modifications) — the preceding version stays valid.
///
/// Same-system detection is case-insensitive on `creating_system_id`
/// (composite-identifier equality; BASE master05 §Composite Identifiers
/// and Case).
///
/// # Errors
/// - [`ServiceError::NotFound`] when the object does not exist, or its stored
///   owner/kind do not match the addressed `(ehr_id, kind)`;
/// - [`ServiceError::VersionConflict`] when `expected` names a version that
///   does not exist or has been superseded (a closed lineage tip).
async fn next_version(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    vo_id: VoId,
    kind: Kind,
    expected: Option<TreeId>,
    local_system_id: &str,
    preplaced: Option<crate::storage::version_repo::placement::Placement>,
) -> Result<NextVersion, ServiceError> {
    // The caller may already hold the advisory lock and the placement read
    // (the composition-update mega-read); otherwise both run here.
    let preplaced_read = preplaced.is_some();
    let placement = if let Some(placement) = preplaced {
        placement
    } else {
        // Serialize concurrent writers of the same object.
        crate::storage::version_repo::commit::advisory_lock(tx, vo_id).await?;

        // ONE statement: preceding tip + next ordinal + the transaction
        // timestamp (the commit instant every row of this transaction stamps).
        crate::storage::version_repo::placement::next_placement(
            tx,
            vo_id,
            expected.map(TreeId::columns),
        )
        .await?
    };
    let ordinal = placement.next_ordinal;
    let now = placement.now;
    let tip = placement.tip.map(preceding_tip).transpose()?;
    let Some(tip) = tip else {
        // The object may exist with the expectation naming no stored version —
        // distinguish "no such object" (404) from "wrong version" (409): the
        // current trunk tip is the lineage tip with no expectation.
        let current = crate::storage::version_repo::placement::next_placement(tx, vo_id, None)
            .await?
            .tip
            .map(preceding_tip)
            .transpose()?;
        return match (expected, current) {
            (Some(tree), Some(current)) => Err(ServiceError::version_conflict(format!(
                "expected version {tree}, which does not exist (current is {})",
                current.tree
            ))),
            _ => Err(ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("{} {vo_id} in EHR {ehr_id:?}", kind.as_str()),
            )),
        };
    };
    // For an EHR-scoped object the stored owner must match; for a demographic
    // party both stored and expected owner are `None`, which compares equal.
    if tip.ehr_id != ehr_id || tip.kind != kind {
        return Err(ServiceError::sm(
            CallStatusType::VersionedObjectDoesNotExist,
            format!("{} {vo_id} in EHR {ehr_id:?}", kind.as_str()),
        ));
    }
    if !tip.open {
        return Err(ServiceError::version_conflict(format!(
            "expected version {} has been superseded",
            tip.tree
        )));
    }
    // A preplaced read is the CURRENT-trunk-tip form; the caller's `If-Match`
    // gate pinned `expected` to it, and this guard keeps that coupling honest
    // instead of silently continuing from a different tip.
    if preplaced_read
        && let Some(expected) = expected
        && tip.tree != expected
    {
        return Err(ServiceError::version_conflict(format!(
            "expected version {expected}, current is {}",
            tip.tree
        )));
    }
    let preceding_uid = object_version_id(vo_id, &tip.creating_system_id, tip.tree);

    let tree = if composite_ids_equal(&tip.creating_system_id, local_system_id) {
        // Continue the lineage this system owns; the preceding tip is
        // superseded by the head row advancing past it.
        match tip.tree.branch {
            None => TreeId::trunk(tip.tree.trunk + 1),
            Some((b, v)) => TreeId::branch(tip.tree.trunk, b, v + 1),
        }
    } else {
        // Local modification of a version copied from elsewhere: fork a branch
        // at the preceding version's trunk fork point (master06 §Distributed
        // Versioning); the copied version itself stays valid.
        //
        // A trunk whose creating system changes mid-line is unrepresentable
        // through this engine, but storage and the read paths tolerate one, so
        // an imported container carrying such a lineage still serves.
        // NOTE: master06 §Copying §Subsequent Local Modifications and §Moving
        // Version Containers contradict each other on the identical observable
        // state; §Copying is followed — the rule with a procedure and a wire.
        let next_branch =
            crate::storage::version_repo::placement::next_branch_number(tx, vo_id, tip.tree.trunk)
                .await?;
        TreeId::branch(tip.tree.trunk, next_branch, 1)
    };
    Ok(NextVersion {
        ordinal,
        tree,
        preceding_uid,
        preceding_lifecycle: tip.lifecycle_state,
        now,
    })
}

/// How the enclosing CONTRIBUTION is supplied to [`apply_change`].
enum ContributionCtx {
    /// A standalone single-object write (create/update/delete): the CONTRIBUTION
    /// is created in the same commit, sharing the version's `commit_audit` (a
    /// direct write is one CONTRIBUTION of one change — master06 §Committal and
    /// Audits).
    New,
    /// A change within a multi-change CONTRIBUTION already opened by
    /// [`commit_contribution`]; the version's own `commit_audit` is written here,
    /// referencing this existing `contribution_id`.
    Existing(Uuid),
}

/// A [`Change`] resolved to the concrete `version` placement + content — the
/// output of the per-arm decision (offload, lifecycle, decompose, version-tree
/// placement) and the input to the shared write ([`commit_resolved`]).
struct ResolvedWrite {
    kind: Kind,
    vo_id: VoId,
    ehr_id: Option<EhrId>,
    /// The per-vo storage commit ordinal.
    ordinal: i32,
    tree: TreeId,
    /// The `version_lifecycle_state` code (master06 §Version Lifecycle).
    lifecycle: String,
    /// `ORIGINAL_VERSION.preceding_version_uid` (`None` for a first version).
    preceding_uid: Option<String>,
    template_id: Option<String>,
    /// A client-supplied `UPDATE_VERSION.signature`, stored verbatim (master06
    /// §Digital Signature); `None` on the direct endpoints.
    client_signature: Option<String>,
    /// The decomposed node rows (empty for a logical delete — data Void).
    rows: Vec<NodeRow>,
    /// The canonical body BYTES, serialized from the accepted, uid-stamped
    /// value BEFORE decomposition — the stored `version.body` text a point
    /// read serves verbatim. `None` for a logical delete.
    canonical_text: Option<String>,
    /// Whether the RELEASED generation set can express this body — the
    /// commit-time `version.stable_compatible` stamp
    /// ([`crate::versioning::profile::stable_compatible`]).
    stable_compatible: bool,
    /// The distinct origins of the body, derived at commit
    /// ([`crate::versioning::origins`]); empty on a logical delete.
    origins: Vec<String>,
    /// `UPDATE_VERSION.attestations` committed with this version.
    attestations: Vec<attestation::AttestationInput>,
    /// A newly created FOLDER hierarchy that joins `EHR.folders` (create only).
    is_first_folder: bool,
    /// The transaction timestamp — the commit instant this transaction stamps
    /// on every row, read before any insert so the `VERSION.signature` is
    /// computable up front (RM common master06 §Digital Signature).
    time_committed: jiff::Timestamp,
}

/// The core write path shared by single-object writes and CONTRIBUTION commits:
/// resolve one [`Change`] to its version-tree placement + content and commit it
/// under the supplied [`ContributionCtx`], signing the assembled
/// `ORIGINAL_VERSION` (RM common master06 §Digital Signature) and rejecting an
/// illegal lifecycle transition. Returns the [`Committed`] outcome and the
/// enclosing `contribution_id` (for the caller's event-outbox envelope).
///
/// The cross-area hooks run AROUND this write, driven by the CONTRIBUTION
/// orchestration ([`crate::versioning::CommitEnv`], called from
/// [`super::contribution::commit_version_set`]) and by the direct write paths:
/// `CommitEnv::pre_composition_modify` for the `VERSIONED_COMPOSITION`
/// cross-version invariants before a COMPOSITION modify,
/// `CommitEnv::post_status_commit` for the EHR promoted-subject-column sync
/// after an `EHR_STATUS` version, and [`meter_committed`] for the
/// `compositions_committed_total` metric, which must count only work that
/// actually committed.
///
/// # Errors
/// The [`next_version`] placement errors (`NotFound` / `VersionConflict`) on
/// modify/delete; [`ServiceError::Unprocessable`] for an out-of-group or
/// illegal lifecycle transition, or a `523|deleted|` state on a data-carrying
/// commit ([`reject_deleted_with_data`]); [`ServiceError::Internal`] on a
/// multimedia offload failure; plus the storage/signing errors of
/// [`commit_resolved`].
/// Stamps the version's own `OBJECT_VERSION_ID` into the canonical's root `uid`
/// before decompose and sign, so the stored, signed and served bytes all carry
/// it (RM common `master03-archetyped_package.adoc` §Unique Node Identification;
/// ITS-REST `Resources.md` §Identifier types). The full three-part form is BASE
/// `architecture_overview` master09 §Levels of Identification, "populated with a
/// copy of the `OBJECT_VERSION_ID` from the containing `VERSION<X>` object".
///
/// NOTE: a client-supplied create-body `uid` is overwritten, no released
/// operation stating a body-uid semantic for the content-object creates and the
/// identifier naming a version that does not exist until this commit mints it.
///
/// The EHR Extract import path does not run through here; foreign versions keep
/// their carried bytes verbatim.
///
/// # Errors
/// [`VersionIdError`] when `version_uid` is not a well-formed
/// `OBJECT_VERSION_ID` — the stamped identity goes through the BASE
/// construction door, so a malformed one is refused before it is written,
/// signed and served.
fn stamp_version_uid(canonical: &mut Value, version_uid: &str) -> Result<(), VersionIdError> {
    if let Value::Object(map) = canonical {
        let uid =
            ObjectVersionId::new(version_uid).map_err(|source| VersionIdError::Malformed {
                raw: version_uid.to_owned(),
                source,
            })?;
        let value = openehr_its::json::to_canonical_value(&uid);
        if map.contains_key("uid") {
            // Replacing keeps the key's existing position.
            map.insert("uid".to_owned(), value);
        } else {
            // The RM BMM declares LOCATABLE as (name, archetype_node_id, uid,
            // …), and the stored body is served byte-verbatim, so a fresh uid
            // must land after `archetype_node_id` rather than at the map's end.
            let anchor = ["archetype_node_id", "name", "_type"]
                .iter()
                .find_map(|k| map.keys().position(|key| key == k));
            let at = anchor.map_or(0, |i| i + 1);
            let original_len = map.len();
            let mut rebuilt = serde_json::Map::with_capacity(original_len + 1);
            for (i, (k, v)) in std::mem::take(map).into_iter().enumerate() {
                if i == at {
                    rebuilt.insert("uid".to_owned(), value.clone());
                }
                rebuilt.insert(k, v);
            }
            if rebuilt.len() == original_len {
                // `at` sat past the last entry (the anchor was last, or the
                // map was empty) — the uid closes the map instead.
                rebuilt.insert("uid".to_owned(), value);
            }
            *map = rebuilt;
        }
    }
    Ok(())
}

/// Everything that is fixed for the enclosing commit act, as one value:
/// [`apply_change`] varies only in the [`Change`] it applies, so the rest —
/// the EHR scope, the enclosing CONTRIBUTION, the audits, the signing context
/// and the shared transaction timestamp — travels together.
///
/// A single-object write builds one of these per call
/// ([`ContributionCtx::New`], no `known_now`); a CONTRIBUTION commit builds it
/// once and applies every change of the set under it, which is what makes the
/// whole set share one contribution id and one commit instant (RM common
/// `master06-change_control_package.adoc` §Committal and Audits).
struct CommitScope<'a> {
    /// The owning EHR, or `None` for an ehr-less object (a demographic party).
    ehr_id: Option<EhrId>,
    /// How the enclosing CONTRIBUTION is supplied.
    contribution: ContributionCtx,
    /// This version's own `commit_audit`.
    audit: &'a AuditInput,
    /// System id, signer, multimedia engine and outbox switch for the write.
    ctx: &'a SigningCtx<'a>,
    /// The transaction timestamp already read by the caller (the CONTRIBUTION
    /// path reads `now()` once for the whole set); `None` makes this change
    /// read its own.
    known_now: Option<jiff::Timestamp>,
    /// A placement the caller already read under the advisory lock it already
    /// holds (the composition-update mega-read); `None` makes the Modify /
    /// Delete arm take the lock and read its own.
    preplaced: Option<crate::storage::version_repo::placement::Placement>,
}

/// Seal every configured-scheme national identifier out of a demographic body
/// before it is decomposed and signed.
///
/// Runs on the demographic kinds only: a party's identifiers are the point of
/// that domain, and clinical content carrying one is a different rule
/// altogether ([`crate::privacy`] refuses it outright rather than sealing it).
///
/// Placed before `decompose` and the signature for the same reason the
/// multimedia offload is: the stored, signed and served body must be one form,
/// which is what lets a later read verify a signature against the bytes it
/// actually holds. No openEHR spec governs identifier protection — our own
/// design/extension.
///
/// # Errors
/// [`ServiceError`] when a value cannot be sealed or stored. The commit fails
/// rather than proceeding, because proceeding would store the body with the
/// identifier still in it.
async fn seal_protected_identifiers(
    ctx: &SigningCtx<'_>,
    kind: Kind,
    vo_id: VoId,
    canonical: &mut Value,
) -> Result<(), ServiceError> {
    let Some(engine) = ctx.identifiers else {
        return Ok(());
    };
    if !kind.is_demographic() {
        return Ok(());
    }
    engine
        .externalize(vo_id.0, canonical)
        .await
        .map_err(|e| ServiceError::internal("seal a protected national identifier", e))?;
    Ok(())
}

/// Refuse a write that would process an object whose processing is restricted.
///
/// GDPR Art. 18(2) leaves storage as the only processing a restricted object
/// admits (`docs/law/eu/gdpr/text.html`), so a new version of it, a logical
/// delete of it or an attestation appended to it is refused — while the rows
/// already stored stay exactly where they are. The mark is read inside the
/// commit transaction, like the `is_modifiable` gate beside it, so a
/// concurrent lift either commits first and is seen or waits for this commit.
/// Both grains are checked: a whole-EHR restriction reaches an object that has
/// no mark of its own yet, because it is created after the restriction was
/// recorded. No openEHR spec governs restriction of processing — our own
/// design/extension.
///
/// # Errors
/// [`ServiceError::Restricted`] (`403`) when either grain is marked;
/// [`ServiceError::Database`] if the marks cannot be read.
pub(crate) async fn ensure_not_restricted_tx(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    vo_id: Option<VoId>,
) -> Result<(), ServiceError> {
    if let Some(ehr_id) = ehr_id {
        let restricted: Option<bool> =
            sqlx::query_scalar("SELECT restricted_at IS NOT NULL FROM ehr WHERE id = $1")
                .bind(ehr_id)
                .fetch_optional(&mut *tx)
                .await?;
        if restricted == Some(true) {
            return Err(ServiceError::restricted(&format!("EHR {ehr_id}")));
        }
    }
    if let Some(vo_id) = vo_id
        && crate::storage::marks::is_restricted(&mut *tx, vo_id).await?
    {
        return Err(ServiceError::restricted(&format!(
            "versioned object {vo_id}"
        )));
    }
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "the three change arms build one resolved write; splitting them \
              would hide that they are alternatives on the same transaction"
)]
async fn apply_change(
    tx: &mut PgConnection,
    scope: CommitScope<'_>,
    change: Change,
) -> Result<(Committed, Uuid), ServiceError> {
    let CommitScope {
        ehr_id,
        contribution,
        audit,
        ctx,
        known_now,
        preplaced,
    } = scope;
    ensure_not_restricted_tx(tx, ehr_id, change.target_vo_id()).await?;
    let resolved = match change {
        Change::Create {
            kind,
            mut canonical,
            template_id,
            signature,
            lifecycle_state,
            attestations,
        } => {
            // Externalize large inline DV_MULTIMEDIA before decompose/sign, so
            // the stored, served and signed form is the offloaded one.
            #[cfg(feature = "multimedia")]
            if let Some(engine) = ctx.multimedia {
                engine
                    .offload(&mut canonical)
                    .await
                    .map_err(|e| ServiceError::internal("externalize multimedia", e))?;
            }
            let lifecycle = resolve_lifecycle(lifecycle_state)?;
            // This arm commits CONTENT, so the deleted state is refused here
            // before the state machine speaks (master06 §Logical Deletion:
            // deleting the data and setting the state are one act).
            reject_deleted_with_data(&lifecycle)?;
            // a first version can only be `complete`/`incomplete`.
            validate_transition(None, &lifecycle)?;
            let vo_id = VoId::minted(ctx.stamp);
            seal_protected_identifiers(ctx, kind, vo_id, &mut canonical).await?;
            stamp_version_uid(
                &mut canonical,
                &object_version_id(vo_id, &ctx.system_id, TreeId::trunk(1)),
            )?;
            // Asked of the ACCEPTED body, before it is decomposed: the stored
            // node rows reassemble to these bytes, and the released-generation
            // reader's answer is what a later `stable` deployment reads back.
            let stable_compatible = profile::stable_compatible(ctx.spec_profile, kind, &canonical);
            let origins = crate::versioning::origins::origins(&ctx.system_id, &canonical);
            let canonical_text = Some(canonical_body_text(&canonical)?);
            let rows = decompose(canonical)?;
            let time_committed = match known_now {
                Some(ts) => ts,
                None => crate::storage::version_repo::placement::tx_now(tx).await?,
            };
            ResolvedWrite {
                kind,
                vo_id,
                ehr_id,
                ordinal: 1,
                tree: TreeId::trunk(1),
                lifecycle,
                preceding_uid: None,
                template_id,
                client_signature: signature,
                rows,
                canonical_text,
                stable_compatible,
                origins,
                attestations,
                is_first_folder: kind == Kind::Folder && ehr_id.is_some(),
                time_committed,
            }
        }
        Change::Modify {
            vo_id,
            kind,
            mut canonical,
            expected,
            template_id,
            signature,
            lifecycle_state,
            attestations,
        } => {
            #[cfg(feature = "multimedia")]
            if let Some(engine) = ctx.multimedia {
                engine
                    .offload(&mut canonical)
                    .await
                    .map_err(|e| ServiceError::internal("externalize multimedia", e))?;
            }
            let lifecycle = resolve_lifecycle(lifecycle_state)?;
            // A modification carries content, so it can never be the `deleted`
            // version (master06 §Logical Deletion) — refused before the
            // placement read, so a data-carrying `523` never reaches storage.
            reject_deleted_with_data(&lifecycle)?;
            let next =
                next_version(tx, ehr_id, vo_id, kind, expected, &ctx.system_id, preplaced).await?;
            // the transition from the preceding version's state must be
            // legal (master06 §Version Lifecycle state machine).
            validate_transition(Some(&next.preceding_lifecycle), &lifecycle)?;
            seal_protected_identifiers(ctx, kind, vo_id, &mut canonical).await?;
            stamp_version_uid(
                &mut canonical,
                &object_version_id(vo_id, &ctx.system_id, next.tree),
            )?;
            let stable_compatible = profile::stable_compatible(ctx.spec_profile, kind, &canonical);
            let origins = crate::versioning::origins::origins(&ctx.system_id, &canonical);
            let canonical_text = Some(canonical_body_text(&canonical)?);
            let rows = decompose(canonical)?;
            ResolvedWrite {
                kind,
                vo_id,
                ehr_id,
                ordinal: next.ordinal,
                tree: next.tree,
                lifecycle,
                preceding_uid: Some(next.preceding_uid),
                template_id,
                client_signature: signature,
                rows,
                canonical_text,
                stable_compatible,
                origins,
                attestations,
                is_first_folder: false,
                time_committed: known_now.unwrap_or(next.now),
            }
        }
        Change::Delete {
            vo_id,
            kind,
            expected,
            signature,
        } => {
            let next =
                next_version(tx, ehr_id, vo_id, kind, expected, &ctx.system_id, preplaced).await?;
            // A deleted version carries no data — its `ORIGINAL_VERSION.data` is
            // Void (master06 §Logical Deletion); the signature is over the
            // data-less version wrapper. Logical deletion is permitted from any
            // live state, so no transition check runs here.
            ResolvedWrite {
                kind,
                vo_id,
                ehr_id,
                ordinal: next.ordinal,
                tree: next.tree,
                lifecycle: lifecycle::state::DELETED.to_owned(),
                preceding_uid: Some(next.preceding_uid),
                template_id: None,
                client_signature: signature,
                rows: Vec::new(),
                canonical_text: None,
                // A deleted version's data is Void (master06 §Logical
                // Deletion): there is no body a generation could fail to
                // express.
                stable_compatible: true,
                origins: Vec::new(),
                attestations: Vec::new(),
                is_first_folder: false,
                time_committed: known_now.unwrap_or(next.now),
            }
        }
    };
    commit_resolved(tx, ctx, audit, contribution, resolved).await
}

/// Commit a [`ResolvedWrite`] — compute the `VERSION.signature`, then write the
/// `commit_audit` (+ `contribution` for a standalone write), the `version` row
/// and the object's head row in ONE data-modifying CTE, then the node rows,
/// folder membership and accompanying attestations.
///
/// The signature is computed over the assembled `ORIGINAL_VERSION` (RM common
/// master06 §Digital Signature), which embeds `time_committed` and
/// `contribution_id`. Both are known before any statement, the commit instant
/// being the transaction timestamp
/// ([`tx_now`](crate::storage::version_repo::placement::tx_now)) and a
/// standalone write generating its `contribution_id` here, so audit,
/// contribution, `version` and the head upsert collapse into one folded CTE.
/// Nothing is superseded in place: the store is append-only, so there is no
/// close-out statement to order before the insert. No openEHR spec governs
/// statement batching — our own design.
///
/// # Errors
/// [`ServiceError::Signing`] when the canonical form cannot be produced or the
/// signer fails; the storage errors of the folded insert and the node /
/// attestation writes; the attestation-completion `Unprocessable` rejections.
async fn commit_resolved(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    audit: &AuditInput,
    contribution: ContributionCtx,
    r: ResolvedWrite,
) -> Result<(Committed, Uuid), ServiceError> {
    let audit_row = audit.row();
    let (trunk_version, branch_number, branch_version) = r.tree.columns();

    // The enclosing CONTRIBUTION id: pre-existing for a multi-change commit,
    // generated here for a standalone write — known before the signature.
    let contribution_id = match contribution {
        ContributionCtx::New => ctx.stamp.mint(),
        ContributionCtx::Existing(cid) => cid,
    };

    // The accompanying attestations are completed ONCE before the signature —
    // master06 §Digital Signature signs "the entire Version object" — and
    // reused verbatim for the insert, so signed bytes and stored bytes are the
    // same bytes.
    let at_committal_attestations: Vec<Value> =
        attestation::complete_accompanying(&r.attestations, &ctx.system_id, r.time_committed)
            .iter()
            .map(openehr_its::json::to_canonical_value)
            .collect();

    let (signature, signature_client_supplied) =
        body_and_signature(ctx, audit, contribution_id, &r, &at_committal_attestations)?;

    let folded = crate::storage::version_repo::commit::FoldedVersion {
        vo_id: r.vo_id,
        kind: r.kind.as_str(),
        ehr_id: r.ehr_id,
        sys_version: r.ordinal,
        trunk_version,
        branch_number,
        branch_version,
        lifecycle_state: &r.lifecycle,
        creating_system_id: &ctx.system_id,
        preceding_version_uid: r.preceding_uid.as_deref(),
        template_id: r.template_id.as_deref(),
        signature: signature.as_deref(),
        signature_client_supplied,
        stable_compatible: r.stable_compatible,
        origins: &r.origins,
        body: r.canonical_text.as_deref(),
        time_committed: r.time_committed,
        rows: &r.rows,
        // Nothing is superseded in place: the head row advancing past the
        // previous version inside the SAME statement is what makes this one
        // current (master06 §The 'Virtual Version Tree').
    };
    // The folded statements BIND `r.time_committed` (the instant the signature
    // was computed over) as the audit time and the version row's own
    // `committed_at`, so stored == signed holds by construction.
    let time_committed = match contribution {
        ContributionCtx::New => {
            let (_cid, _aid, tc) = crate::storage::version_repo::commit::commit_new_version(
                tx,
                &audit_row,
                Some(contribution_id),
                &folded,
            )
            .await?;
            tc
        }
        ContributionCtx::Existing(cid) => {
            let (_aid, tc) = crate::storage::version_repo::commit::commit_version_into(
                tx, &audit_row, cid, &folded,
            )
            .await?;
            tc
        }
    };

    // A newly created FOLDER hierarchy joins `EHR.folders` as a new member (RM
    // ehr master04 §Folders). Recorded only on CREATION.
    if r.is_first_folder
        && let Some(ehr_id) = r.ehr_id
    {
        crate::storage::version_repo::commit::insert_ehr_folder_rank(tx, ehr_id, r.vo_id).await?;
    }
    attestation::insert_at_committal_attestations(
        tx,
        r.vo_id,
        r.ordinal,
        contribution_id,
        &at_committal_attestations,
    )
    .await?;

    Ok((
        Committed {
            vo_id: r.vo_id,
            sys_version: r.ordinal,
            tree: r.tree,
            creating_system_id: ctx.system_id.clone(),
            kind: r.kind,
            change_type: audit.change_type.clone(),
            template_id: r.template_id,
            root_archetype: r
                .rows
                .first()
                .filter(|root| root.num == 0)
                .and_then(|root| root.archetype.clone()),
            time_committed,
        },
        contribution_id,
    ))
}

/// Serializes the accepted, uid-stamped canonical value to the body bytes a
/// point read serves verbatim (`version.body`, text taken before node
/// decomposition, so the served wire keeps the codec's `_type`-first,
/// BMM-declared field order).
///
/// # Errors
/// [`ServiceError`] when the value cannot serialize (a non-finite number —
/// unreachable for a codec-produced canonical value, refused loudly anyway).
fn canonical_body_text(canonical: &Value) -> Result<String, ServiceError> {
    serde_json::to_string(canonical)
        .map_err(|e| ServiceError::internal("serialize the canonical body", e))
}

/// Reassemble the version body from the node rows and derive the signature
/// over it.
///
/// The signature input is the reassembled value; the stored `version.body`
/// is the pre-decomposition text ([`canonical_body_text`]). The two are the
/// same VALUE by decompose/reassemble fidelity, and the signature
/// canonicalization is RFC 8785 (key-order-insensitive), so signing the
/// reassembled form still proves the stored and served bytes. Empty rows =
/// logical delete (Void body is signed — master06 §Logical Deletion).
///
/// # Errors
/// The reassembly [`ServiceError`] or the signer's failure.
fn body_and_signature(
    ctx: &SigningCtx<'_>,
    audit: &AuditInput,
    contribution_id: Uuid,
    r: &ResolvedWrite,
    at_committal_attestations: &[Value],
) -> Result<(Option<String>, bool), ServiceError> {
    let body = if r.rows.is_empty() {
        None
    } else {
        Some(reassemble(&r.rows)?)
    };
    version_signature(
        ctx,
        audit,
        contribution_id,
        r,
        body.as_ref(),
        at_committal_attestations,
    )
}

/// The signature stored with the version, and whether it is **client-supplied**
/// (foreign — stored verbatim, never re-verified at read; master06 §Digital
/// Signature) vs server-generated. A client-supplied signature is kept verbatim;
/// otherwise, with server signing enabled, the pre-reassembled `body` is signed
/// — a logically deleted version signs Void (master06 §Logical Deletion), and a
/// content version signs the same served bytes reads return, so the digest
/// recomputes at read time. `at_committal_attestations` are the completed
/// `ATTESTATION`s committed with the version, which the signed form includes
/// (master06 §Digital Signature: "the entire Version object").
///
/// # Errors
/// [`ServiceError::Signing`] when the canonical form cannot be produced or the
/// signer fails.
fn version_signature(
    ctx: &SigningCtx<'_>,
    audit: &AuditInput,
    contribution_id: Uuid,
    r: &ResolvedWrite,
    body: Option<&Value>,
    at_committal_attestations: &[Value],
) -> Result<(Option<String>, bool), ServiceError> {
    if let Some(client) = &r.client_signature {
        return Ok((Some(client.clone()), true));
    }
    if !ctx.signer.enabled() {
        return Ok((None, false));
    }
    // The signed content is the SAME reassembled value the caller stores as
    // `version.body` — computed once, never re-reassembled here.
    let signature = integrity::sign_version(
        ctx,
        audit,
        r.time_committed,
        r.vo_id,
        r.tree,
        r.preceding_uid.as_deref(),
        contribution_id,
        &r.lifecycle,
        body.unwrap_or(&Value::Null),
        at_committal_attestations,
    )?;
    Ok((signature, false))
}

/// Write the one PHI-free event-outbox row a single-object commit announces,
/// when eventing is enabled (our own extension; no openEHR spec governs it — the
/// row is skipped entirely, envelope included, when no consumer is configured).
async fn write_single_outbox(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    contribution_id: Uuid,
    ehr_id: Option<EhrId>,
    committed: &Committed,
) -> Result<(), ServiceError> {
    if ctx.outbox_enabled {
        crate::storage::version_repo::commit::write_outbox(
            tx,
            contribution_id,
            ehr_id,
            committed.time_committed,
            vec![committed.envelope_entry()],
        )
        .await?;
    }
    Ok(())
}

/// Create the first version of a new versioned object under its own
/// contribution.
///
/// `known_now` is a database `now()` a caller already fetched on this request
/// (e.g. piggybacked on a pre-commit gate read); when `Some`, the write
/// transaction spends no extra round trip on the clock. `None` fetches it
/// in-transaction.
///
/// # Errors
/// [`ServiceError::Unprocessable`] for an out-of-group / non-first lifecycle
/// state; the [`commit_resolved`] storage/signing errors; a multimedia offload
/// failure as [`ServiceError::Internal`].
/// The `409 Conflict` for a content write to a deactivated EHR
/// (`EHR_STATUS.is_modifiable = false`) — RM ehr master04 §EHR Active Status
/// forbids writes to everything except the `EHR_STATUS` object. ITS-REST
/// 1.1.0 enumerates no status code for this outcome; `409` is the closest
/// HTTP semantics (RFC 9110 §15.5.10 — the write conflicts with the current
/// state of the target resource).
pub(crate) fn not_modifiable_error(ehr_id: EhrId) -> ServiceError {
    ServiceError::conflict(format!(
        "EHR {ehr_id} is not modifiable (EHR_STATUS.is_modifiable = false); its \
         contents cannot be created, updated or deleted (RM ehr master04 §EHR Active \
         Status). Set EHR_STATUS.is_modifiable = true to reactivate it."
    ))
}

/// The content-write gate, evaluated INSIDE the commit transaction.
///
/// Reads the promoted `ehr.is_modifiable` under a row lock
/// ([`crate::storage::ehr_repo::ehr_is_modifiable_locked`]), so a concurrent
/// `EHR_STATUS` flip either commits first and is seen or waits for this commit.
/// A missing EHR row reads as modifiable; the existence gate answers 404 on its
/// own.
///
/// NOTE: RM ehr master04 §EHR Active Status defines what the flag forbids and no
/// released text states when it is evaluated relative to a commit, so the
/// in-transaction snapshot is our own recorded semantics.
///
/// # Errors
/// [`ServiceError::Conflict`] when the EHR is not modifiable;
/// [`ServiceError::Database`] if the locked read fails.
pub(crate) async fn ensure_content_writable_tx(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    exclusive: bool,
) -> Result<(), ServiceError> {
    let modifiable = crate::storage::ehr_repo::ehr_is_modifiable_locked(tx, ehr_id, exclusive)
        .await?
        .unwrap_or(true);
    if modifiable {
        Ok(())
    } else {
        Err(not_modifiable_error(ehr_id))
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the write parameters, named individually; a parameter struct \
              would not read clearer at the service call sites"
)]
pub(crate) async fn create(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    kind: Kind,
    canonical: Value,
    template_id: Option<&str>,
    audit: &AuditInput,
    envelope: WriteEnvelope,
    ctx: &SigningCtx<'_>,
    known_now: Option<jiff::Timestamp>,
) -> Result<Committed, ServiceError> {
    let (committed, contribution_id) = apply_change(
        tx,
        CommitScope {
            ehr_id,
            contribution: ContributionCtx::New,
            audit,
            ctx,
            known_now,
            preplaced: None,
        },
        Change::Create {
            kind,
            canonical,
            template_id: template_id.map(str::to_owned),
            signature: envelope.signature,
            lifecycle_state: envelope.lifecycle_state,
            attestations: envelope.attestations,
        },
    )
    .await?;
    write_single_outbox(tx, ctx, contribution_id, ehr_id, &committed).await?;
    Ok(committed)
}

/// Commit a new version of an existing object under its own contribution.
///
/// # Errors
/// [`ServiceError::NotFound`] when `(ehr_id, kind, vo_id)` does not address a
/// stored object; [`ServiceError::VersionConflict`] when `expected` is not the
/// open lineage tip; [`ServiceError::Unprocessable`] for an illegal lifecycle
/// transition; plus the [`commit_resolved`] storage/signing errors.
#[expect(
    clippy::too_many_arguments,
    reason = "the write parameters, named individually; a parameter struct \
              would not read clearer at the service call sites"
)]
pub(crate) async fn update(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    vo_id: VoId,
    kind: Kind,
    canonical: Value,
    expected: Option<TreeId>,
    template_id: Option<&str>,
    audit: &AuditInput,
    envelope: WriteEnvelope,
    ctx: &SigningCtx<'_>,
) -> Result<Committed, ServiceError> {
    let (committed, contribution_id) = apply_change(
        tx,
        CommitScope {
            ehr_id,
            contribution: ContributionCtx::New,
            audit,
            ctx,
            known_now: None,
            preplaced: None,
        },
        Change::Modify {
            vo_id,
            kind,
            canonical,
            expected,
            template_id: template_id.map(str::to_owned),
            signature: envelope.signature,
            lifecycle_state: envelope.lifecycle_state,
            attestations: envelope.attestations,
        },
    )
    .await?;
    write_single_outbox(tx, ctx, contribution_id, ehr_id, &committed).await?;
    Ok(committed)
}

/// [`update`] with the advisory lock and placement read already taken by the
/// caller — the composition-update mega-read path.
///
/// The caller has opened the write transaction, taken the per-vo advisory
/// lock, and run the merged placement + pre-check statement
/// ([`crate::storage::version_repo::placement::update_placement`]); this
/// commits the new version off that placement without re-reading. `expected`
/// must equal the placement tip's tree id (the caller's `If-Match` gate pins
/// it; [`next_version`] re-checks).
///
/// # Errors
/// The [`update`] errors, minus the placement read's own.
#[expect(
    clippy::too_many_arguments,
    reason = "the write parameters, named individually; a parameter struct \
              would not read clearer at the service call sites"
)]
pub(crate) async fn update_with_placement(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    vo_id: VoId,
    kind: Kind,
    canonical: Value,
    expected: Option<TreeId>,
    template_id: Option<&str>,
    audit: &AuditInput,
    envelope: WriteEnvelope,
    ctx: &SigningCtx<'_>,
    placement: crate::storage::version_repo::placement::Placement,
) -> Result<Committed, ServiceError> {
    let (committed, contribution_id) = apply_change(
        tx,
        CommitScope {
            ehr_id,
            contribution: ContributionCtx::New,
            audit,
            ctx,
            known_now: None,
            preplaced: Some(placement),
        },
        Change::Modify {
            vo_id,
            kind,
            canonical,
            expected,
            template_id: template_id.map(str::to_owned),
            signature: envelope.signature,
            lifecycle_state: envelope.lifecycle_state,
            attestations: envelope.attestations,
        },
    )
    .await?;
    write_single_outbox(tx, ctx, contribution_id, ehr_id, &committed).await?;
    Ok(committed)
}

/// Logically delete an object under its own contribution (master06 §Logical
/// Deletion).
///
/// Takes only the client `signature` rather than a whole [`WriteEnvelope`]:
/// deletion fixes the lifecycle state at `523|deleted|` itself, so an
/// envelope lifecycle here would be a silently dropped instruction.
///
/// # Errors
/// [`ServiceError::NotFound`] when `(ehr_id, kind, vo_id)` does not address a
/// stored object; [`ServiceError::VersionConflict`] when `expected` is not the
/// open lineage tip; plus the [`commit_resolved`] storage/signing errors.
#[expect(
    clippy::too_many_arguments,
    reason = "the write parameters, named individually; a parameter struct \
              would not read clearer at the service call sites"
)]
pub(crate) async fn delete(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    vo_id: VoId,
    kind: Kind,
    expected: Option<TreeId>,
    audit: &AuditInput,
    signature: Option<String>,
    ctx: &SigningCtx<'_>,
) -> Result<Committed, ServiceError> {
    let (committed, contribution_id) = apply_change(
        tx,
        CommitScope {
            ehr_id,
            contribution: ContributionCtx::New,
            audit,
            ctx,
            known_now: None,
            preplaced: None,
        },
        Change::Delete {
            vo_id,
            kind,
            expected,
            signature,
        },
    )
    .await?;
    write_single_outbox(tx, ctx, contribution_id, ehr_id, &committed).await?;
    Ok(committed)
}

/// Commit a set of changes atomically under one CONTRIBUTION (RM common
/// master06 §Committal and Audits — "similar to nested transactions").
/// `contribution_audit` is the CONTRIBUTION's own audit; each change carries its
/// VERSION `commit_audit`. `attests` are `666|attestation|` items — new
/// `ATTESTATION`s attached to **existing** versions, committed in the
/// same transaction but adding no new version.
///
/// Every audit of the set is stamped with the one commit instant this
/// function reads (RM common master06 §Committal and Audits: `time_committed`
/// "computed on the server"); each version and attestation keeps the
/// committer its own audit states.
///
/// # Errors
/// The per-change [`apply_change`] errors; the attestation-completion
/// `Unprocessable` rejections and the `NotFound` of a missing attestation
/// target; the storage errors of the contribution/outbox writes.
pub(crate) async fn commit_contribution(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    supplied_uid: Option<Uuid>,
    contribution_audit: &AuditInput,
    changes: Vec<(AuditInput, Change)>,
    attests: Vec<PendingAttest>,
    ctx: &SigningCtx<'_>,
) -> Result<CommittedContribution, ServiceError> {
    // The CONTRIBUTION's own audit + contribution rows in one round trip (the
    // per-version `commit_audit`s are inserted per change below). A uid the
    // client did not supply is minted here, stamped, never by the database.
    let uid = supplied_uid.unwrap_or_else(|| ctx.stamp.mint());
    let (contribution_id, _contribution_commit_audit_id, contribution_time) =
        crate::storage::version_repo::commit::write_contribution(
            tx,
            ehr_id,
            &contribution_audit.row(),
            Some(uid),
        )
        .await?;
    let mut committed = Vec::with_capacity(changes.len() + attests.len());
    for (version_audit, change) in changes {
        // Each change writes its own `commit_audit` + `version` under the
        // shared contribution, always through the folded CTE — the commit
        // instant is the contribution's transaction timestamp (one `now()`
        // for the whole set), so the signature is computable up front.
        let (change_committed, _cid) = apply_change(
            tx,
            CommitScope {
                ehr_id,
                contribution: ContributionCtx::Existing(contribution_id),
                audit: &version_audit,
                ctx,
                known_now: Some(contribution_time),
                preplaced: None,
            },
            change,
        )
        .await?;
        committed.push(change_committed);
    }
    // Standalone 666 attestations of existing versions (no new version) —
    // completed with the contribution's commit-act time.
    for item in attests {
        ensure_not_restricted_tx(tx, ehr_id, Some(item.vo_id)).await?;
        let full =
            attestation::complete_attestation(&item.partial, &ctx.system_id, contribution_time);
        committed.push(
            attestation::attest(
                tx,
                ehr_id,
                item.vo_id,
                item.kind,
                item.expected,
                &full,
                contribution_id,
                contribution_time,
            )
            .await?,
        );
    }
    // One PHI-free outbox event for the whole CONTRIBUTION (same transaction),
    // skipped entirely — including the envelope collection — when no eventing
    // consumer is configured (our own extension; no openEHR spec governs it).
    if ctx.outbox_enabled {
        let versions = committed.iter().map(Committed::envelope_entry).collect();
        crate::storage::version_repo::commit::write_outbox(
            tx,
            contribution_id,
            ehr_id,
            contribution_time,
            versions,
        )
        .await?;
    }
    Ok(CommittedContribution {
        id: contribution_id,
        time_committed: contribution_time,
        versions: committed,
    })
}
