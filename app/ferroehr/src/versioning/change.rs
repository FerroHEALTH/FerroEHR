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

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::{AllocatedVoId, EhrId, VoId};
use crate::storage::row::NodeRow;
use crate::storage::version_repo::commit::{Folded, FoldedRows, commit_folded};
use crate::versioning::attestation::{self, PendingAttest};
use crate::versioning::audit::AuditInput;
use crate::versioning::lifecycle::{
    reject_deleted_with_data, resolve_lifecycle, validate_transition,
};
use crate::versioning::object_version_id::{TreeId, VersionIdError, object_version_id};
use crate::versioning::{Kind, SigningCtx, restriction};

mod construct;
pub(crate) mod precondition;
mod project;
pub(crate) mod resolve;
mod sign;

use resolve::{Content, Placing, resolve_create, resolve_delete, resolve_modify};
use sign::body_and_signature;

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
    /// `true` when nothing was committed: the update's content equalled the
    /// current version's and `[record.versioning] idempotent_resend` answered
    /// with that version (#4415). The fields then name the existing version.
    pub skipped: bool,
    /// SHA-256 over the canonical body with the root `uid` removed
    /// (`version.content_digest`); `None` on a logical delete and on an
    /// attestation. On a skipped re-send it is the digest the content and the
    /// existing version share.
    pub content_digest: Option<[u8; 32]>,
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
pub struct CommittedContribution {
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
        /// The id allocated before the commit; `None` mints one at the commit.
        allocated: Option<AllocatedVoId>,
        kind: Kind,
        canonical: Value,
        template_id: Option<String>,
        signature: Option<String>,
        lifecycle_state: Option<String>,
        /// Wire `UPDATE_VERSION.attestations` committed with this version
        /// (master06 §Attestation "Signing content at committal").
        attestations: Vec<attestation::AttestationInput>,
        /// `ORIGINAL_VERSION.other_input_version_uids` (master06 §Version Merging).
        other_input_version_uids: Vec<String>,
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
        other_input_version_uids: Vec<String>,
    },
    /// Logically delete an object (a content-less `deleted` version — master06
    /// §Logical Deletion).
    Delete {
        vo_id: VoId,
        kind: Kind,
        expected: Option<TreeId>,
        signature: Option<String>,
        other_input_version_uids: Vec<String>,
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
    /// The tip's storage ordinal.
    sys_version: i32,
    /// The tip's commit instant.
    committed_at: jiff::Timestamp,
    /// The tip's content digest, `None` when it has no content to share.
    content_digest: Option<Vec<u8>>,
    /// The version whose rows hold the tip's content (#4415).
    content_key: i32,
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
        sys_version: row.sys_version,
        committed_at: row.committed_at,
        content_digest: row.content_digest,
        content_key: row.content_key,
    })
}

/// The preceding tip a modify was placed after, as the shared-content rule
/// and the opt-in re-send answer need it (#4415).
#[derive(Debug, Clone)]
pub(super) struct Preceding {
    pub(super) sys_version: i32,
    pub(super) tree: TreeId,
    pub(super) creating_system_id: String,
    pub(super) committed_at: jiff::Timestamp,
    /// The tip's `content_digest`; `None` when it carries no content.
    pub(super) content_digest: Option<Vec<u8>>,
    /// The tip's `content_key`: the version whose node rows hold its content.
    pub(super) content_key: i32,
}

/// The resolved placement of a new version in the version tree.
struct NextVersion {
    ordinal: i32,
    tree: TreeId,
    /// The `preceding_version_uid` to store.
    preceding_uid: String,
    /// The preceding version's lifecycle state (the transition "from" state).
    preceding_lifecycle: String,
    /// The preceding version's place in the version tree.
    preceding_tree: TreeId,
    /// The preceding tip's identity and content key ([`Preceding`]), boxed
    /// so the placement's future stays small.
    preceding: Box<Preceding>,
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
    // ([`precondition::lock_current`]); otherwise both run here.
    let placement = if let Some(placement) = preplaced {
        placement
    } else {
        // Serialize concurrent writers of the same object. A multi-object
        // CONTRIBUTION already holds it, and a repeated request by the holding
        // session succeeds (PostgreSQL docs §13.3.5 Advisory Locks).
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
            _ => Err(ServiceError::not_found(
                Missing::VersionedObject,
                format!("{} {vo_id} in EHR {ehr_id:?}", kind.as_str()),
            )),
        };
    };
    // For an EHR-scoped object the stored owner must match; for a demographic
    // party both stored and expected owner are `None`, which compares equal.
    if tip.ehr_id != ehr_id || tip.kind != kind {
        return Err(ServiceError::not_found(
            Missing::VersionedObject,
            format!("{} {vo_id} in EHR {ehr_id:?}", kind.as_str()),
        ));
    }
    if !tip.open {
        return Err(ServiceError::version_conflict(format!(
            "expected version {} has been superseded",
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
        preceding: Box::new(Preceding {
            sys_version: tip.sys_version,
            tree: tip.tree,
            creating_system_id: tip.creating_system_id,
            committed_at: tip.committed_at,
            content_digest: tip.content_digest,
            content_key: tip.content_key,
        }),
        preceding_lifecycle: tip.lifecycle_state,
        preceding_tree: tip.tree,
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
    /// `ORIGINAL_VERSION.other_input_version_uids` (empty when not a merge).
    other_input_version_uids: Vec<String>,
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
    /// SHA-256 over the canonical body with the root `uid` removed
    /// (`version.content_digest`); `None` on a logical delete.
    content_digest: Option<[u8; 32]>,
    /// What a party body holds that the binding hooks read, derived before
    /// the body was decomposed (#5220); `None` for a logical delete and for
    /// every kind but a party.
    party_facts: Option<Box<crate::party_binding::derive::Facts>>,
    /// The version whose node rows this one reads when its content equals the
    /// preceding tip's (`version.content_sys_version`, #4415); the rows are
    /// then decomposed for the signature and not written.
    shared_from: Option<i32>,
    /// The preceding tip of a modify, which the opt-in re-send answer names
    /// instead of committing.
    preceding: Option<Box<Preceding>>,
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
/// commit ([`crate::service::contribution::commit_version_set`]) and by the
/// direct write paths: `check_versioned_composition_invariants` for the
/// `VERSIONED_COMPOSITION` cross-version invariants before a COMPOSITION modify,
/// `FerroEhrService::sync_ehr_subject` for the EHR promoted-subject-column sync
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
pub(crate) fn stamp_version_uid(
    canonical: &mut Value,
    version_uid: &str,
) -> Result<(), VersionIdError> {
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

async fn apply_change(
    tx: &mut PgConnection,
    scope: CommitScope<'_>,
    change: Change,
    skip_identical: bool,
) -> Result<(Committed, Uuid), ServiceError> {
    let CommitScope {
        ehr_id,
        contribution,
        audit,
        ctx,
        known_now,
        preplaced,
    } = scope;
    restriction::ensure_not_restricted_tx(tx, ehr_id, change.target_vo_id()).await?;
    let at = Placing {
        ehr_id,
        known_now,
        preplaced,
    };
    let access = crate::service::ehr::access_projection::AccessCommit::of(&change);
    let mut resolved = match change {
        Change::Create {
            allocated,
            kind,
            canonical,
            template_id,
            signature,
            lifecycle_state,
            attestations,
            other_input_version_uids,
        } => {
            let content = Content {
                allocated,
                kind,
                canonical,
                lifecycle_state,
                template_id,
                signature,
                attestations,
                other_input_version_uids,
            };
            Box::pin(resolve_create(tx, ctx, at, content)).await?
        }
        Change::Modify {
            vo_id,
            kind,
            canonical,
            expected,
            template_id,
            signature,
            lifecycle_state,
            attestations,
            other_input_version_uids,
        } => {
            let content = Content {
                allocated: None,
                kind,
                canonical,
                lifecycle_state,
                template_id,
                signature,
                attestations,
                other_input_version_uids,
            };
            Box::pin(resolve_modify(tx, ctx, at, vo_id, expected, content)).await?
        }
        Change::Delete {
            vo_id,
            kind,
            expected,
            signature,
            other_input_version_uids,
        } => {
            Box::pin(resolve_delete(
                tx,
                ctx,
                at,
                vo_id,
                kind,
                expected,
                signature,
                other_input_version_uids,
            ))
            .await?
        }
    };
    if skip_identical && let Some(existing) = skipped_resend(&resolved, audit) {
        return Ok((existing, Uuid::nil()));
    }
    let party_facts = resolved.party_facts.take();
    // Boxed: the resolved write and the folded commit hold the whole version in their state,
    // which would otherwise put every caller's future over the size the lint allows.
    let committed = Box::pin(commit_resolved(tx, ctx, audit, contribution, resolved)).await?;
    Box::pin(project::committed(
        tx,
        ctx,
        ehr_id,
        &committed.0,
        party_facts.as_deref(),
        access,
    ))
    .await?;
    Ok(committed)
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
        attestation::complete_accompanying(&r.attestations, &audit.system_id, r.time_committed)
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
        other_input_version_uids: &r.other_input_version_uids,
        template_id: r.template_id.as_deref(),
        signature: signature.as_deref(),
        signature_client_supplied,
        stable_compatible: r.stable_compatible,
        origins: &r.origins,
        body: r.canonical_text.as_deref(),
        time_committed: r.time_committed,
        // A version that shares its predecessor's rows writes none of its
        // own; the rows still served the signature above.
        rows: if r.shared_from.is_some() {
            &[]
        } else {
            &r.rows
        },
        content_digest: r.content_digest.as_ref().map(<[u8; 32]>::as_slice),
        content_sys_version: r.shared_from,
        // Nothing is superseded in place: the head row advancing past the
        // previous version inside the SAME statement is what makes this one
        // current (master06 §The 'Virtual Version Tree').
    };
    // The folded statements BIND `r.time_committed` (the instant the signature
    // was computed over) as the audit time and the version row's own
    // `committed_at`, so stored == signed holds by construction.
    let statement = match contribution {
        ContributionCtx::New => Folded::Standalone {
            id: contribution_id,
            version: &folded,
        },
        ContributionCtx::Existing(id) => Folded::Into {
            id,
            version: &folded,
        },
    };
    let time_committed = commit_folded(tx, &audit_row, statement)
        .await?
        .time_committed;

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
            root_archetype: root_archetype(&r.rows),
            time_committed,
            skipped: false,
            content_digest: r.content_digest,
        },
        contribution_id,
    ))
}

/// The opt-in answer to an identical re-send (#4415): the existing version
/// `r` shares its content with, and no commit, when `r` is a modify whose
/// content equalled the preceding tip's; `None` otherwise. The tip is the
/// one version the `If-Match` names, so an earlier version's content is
/// never matched. ITS-REST does not govern skipping an identical commit: our
/// own design, after what Cadasto answers.
fn skipped_resend(r: &ResolvedWrite, audit: &AuditInput) -> Option<Committed> {
    let (Some(preceding), Some(_)) = (&r.preceding, r.shared_from) else {
        return None;
    };
    Some(Committed {
        vo_id: r.vo_id,
        sys_version: preceding.sys_version,
        tree: preceding.tree,
        creating_system_id: preceding.creating_system_id.clone(),
        kind: r.kind,
        change_type: audit.change_type.clone(),
        template_id: r.template_id.clone(),
        root_archetype: root_archetype(&r.rows),
        time_committed: preceding.committed_at,
        skipped: true,
        content_digest: r.content_digest,
    })
}

/// The root's `archetype_node_id` off the decomposed rows, for the outbox
/// envelope and the audit record.
fn root_archetype(rows: &[NodeRow]) -> Option<String> {
    rows.first()
        .filter(|root| root.num == 0)
        .and_then(|root| root.data.get("archetype_node_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Serializes the accepted, uid-stamped canonical value to the body bytes a
/// point read serves verbatim (`version.body`, text taken before node
/// decomposition, so the served wire keeps the codec's `_type`-first,
/// BMM-declared field order).
///
/// # Errors
/// [`ServiceError`] when the value cannot serialize (a non-finite number —
/// unreachable for a codec-produced canonical value, refused loudly anyway).
pub(crate) fn canonical_body_text(canonical: &Value) -> Result<String, ServiceError> {
    serde_json::to_string(canonical)
        .map_err(|e| ServiceError::internal("serialize the canonical body", e))
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

/// One single-object write under its own CONTRIBUTION.
pub(crate) struct Write {
    /// The owning EHR, or `None` for an ehr-less object (a demographic party).
    pub(crate) ehr_id: Option<EhrId>,
    /// What the write commits.
    pub(crate) change: Change,
    /// This version's own `commit_audit`.
    pub(crate) audit: AuditInput,
    /// The transaction timestamp the caller already read, if any.
    pub(crate) known_now: Option<jiff::Timestamp>,
    /// A placement the caller already read under the per-object lock it holds
    /// ([`precondition::lock_current`], or the composition-update mega-read); `None` makes
    /// the write take the lock and read its own.
    pub(crate) placement: Option<crate::storage::version_repo::placement::Placement>,
}

impl Write {
    /// A write of `change` to the object in `ehr_id` (or an ehr-less one),
    /// reading its own commit instant and placement.
    pub(crate) fn new(ehr_id: Option<EhrId>, change: Change, audit: AuditInput) -> Self {
        Self {
            ehr_id,
            change,
            audit,
            known_now: None,
            placement: None,
        }
    }

    /// The same write at the commit instant `now` the caller already read.
    pub(crate) fn at(self, now: jiff::Timestamp) -> Self {
        Self {
            known_now: Some(now),
            ..self
        }
    }

    /// The same write off the placement the caller already read under the
    /// per-object lock it holds.
    pub(crate) fn placed(
        self,
        placement: crate::storage::version_repo::placement::Placement,
    ) -> Self {
        Self {
            placement: Some(placement),
            ..self
        }
    }
}

/// Commit one change under its own contribution, with its event-outbox row.
///
/// # Errors
/// [`ServiceError::NotFound`] when a modify or delete does not address a stored
/// object; [`ServiceError::VersionConflict`] when `expected` is not the open
/// lineage tip; [`ServiceError::BadRequest`] when a delete's preceding version
/// is already deleted; [`ServiceError::Unprocessable`] for an illegal lifecycle
/// state; plus the [`commit_resolved`] storage/signing errors.
pub(crate) async fn commit(
    tx: &mut PgConnection,
    write: Write,
    ctx: &SigningCtx<'_>,
) -> Result<Committed, ServiceError> {
    let Write {
        ehr_id,
        change,
        audit,
        known_now,
        placement,
    } = write;
    let references: Vec<_> = crate::versioning::party_refs::of_change(&change)
        .into_iter()
        .map(|reference| (None, reference))
        .collect();
    let (committed, contribution_id) = apply_change(
        tx,
        CommitScope {
            ehr_id,
            contribution: ContributionCtx::New,
            audit: &audit,
            ctx,
            known_now,
            preplaced: placement,
        },
        change,
        ctx.skip_identical_resend,
    )
    .await?;
    crate::versioning::party_refs::check(tx, &references).await?;
    if !committed.skipped {
        write_single_outbox(tx, ctx, contribution_id, ehr_id, &committed).await?;
    }
    Ok(committed)
}

/// Record a committed single-object write: the database-transaction counter
/// and [`meter_committed`]. Called after the transaction commits, so a
/// rolled-back write is never counted.
pub(crate) fn record_commit(committed: &Committed) {
    crate::telemetry::metrics::metrics()
        .db_transactions
        .add(1, &[opentelemetry::KeyValue::new("outcome", "commit")]);
    meter_committed(committed);
}

/// Commits a set of changes atomically under one CONTRIBUTION (RM common
/// master06 §Committal and Audits — "similar to nested transactions").
///
/// `contribution_audit` is the CONTRIBUTION's own audit; each change carries
/// its VERSION `commit_audit`. `attests` are `666|attestation|` items — new
/// `ATTESTATION`s attached to **existing** versions, committed in the same
/// transaction but adding no new version.
///
/// Every audit of the set is stamped with the one commit instant this
/// function reads (RM common master06 §Committal and Audits: `time_committed`
/// "computed on the server"); each version and attestation keeps the
/// committer its own audit states.
///
/// The per-object lock of every existing object the set modifies or deletes
/// is taken up front, in ascending `vo_id` order — the order an import takes
/// the same locks in
/// ([`crate::storage::version_repo::import::lock_containers`]) — so two
/// writers sharing several objects queue instead of deadlocking.
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
    crate::storage::version_repo::import::lock_containers(
        tx,
        changes
            .iter()
            .filter_map(|(_, change)| change.target_vo_id()),
    )
    .await?;
    // The CONTRIBUTION's own audit + contribution rows in one round trip (the
    // per-version `commit_audit`s are inserted per change below). A uid the
    // client did not supply is minted here, stamped, never by the database.
    let uid = supplied_uid.unwrap_or_else(|| ctx.stamp.mint());
    let FoldedRows {
        contribution_id,
        time_committed: contribution_time,
        ..
    } = commit_folded(
        tx,
        &contribution_audit.row(),
        Folded::Contribution { id: uid, ehr_id },
    )
    .await?;
    let references: Vec<_> = changes
        .iter()
        .enumerate()
        .flat_map(|(index, (_, change))| {
            crate::versioning::party_refs::of_change(change)
                .into_iter()
                .map(move |reference| (Some(index), reference))
        })
        .collect();
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
            false,
        )
        .await?;
        committed.push(change_committed);
    }
    crate::versioning::party_refs::check(tx, &references).await?;
    // Standalone 666 attestations of existing versions (no new version) —
    // completed with the contribution's commit-act time.
    for item in attests {
        restriction::ensure_not_restricted_tx(tx, ehr_id, Some(item.vo_id)).await?;
        let full =
            attestation::complete_attestation(&item.partial, &item.system_id, contribution_time);
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
