// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Import: replaying received `ORIGINAL_VERSION`s into the local store as
//! `IMPORTED_VERSION`s.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Copying / §Committal
//! and Audits. Each received original is committed locally wrapped in an
//! `IMPORTED_VERSION`, and the two acts are stored side by side exactly as
//! §Committal and Audits requires: "Both the contribution and `commit_audit` of
//! the latter object correspond to the local act of committal, while the
//! knowledge of the original Contribution and committal are retained inside the
//! wrapped `ORIGINAL_VERSION` instance."
//!
//! Concretely, per imported version row:
//!
//! * the LOCAL act — `contribution_id` (the one fresh import CONTRIBUTION),
//!   `commit_audit_id` (its `AUDIT_DETAILS`: this server's `system_id`, the importing
//!   committer, the server-computed import instant, `249|creation|` per
//!   §Contributions "import of item"), and the wrapper's own `signature`;
//! * the FOREIGN act — the wrapped original's own `contribution` `OBJECT_REF`,
//!   `commit_audit` (with its source `time_committed`) and `signature`, held
//!   verbatim in `version.wrapped_original`, beside its unchanged 3-part
//!   identity, `lifecycle_state`, data and `attestations` ("the
//!   `ORIGINAL_VERSION` instance is never modified", §Copying).
//!
//! Keeping the local act in the row's own audit is what makes §Copying's
//! chronology rule hold here ("the commit times always reflect the local (more
//! recent) act of committal"), so `VERSIONED_OBJECT.time_created`, the
//! `Last-Modified` header and every as-of-instant read are computed from the
//! import rather than the source system's clock.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the serialized version envelope is the \
              signed artifact (RM common master06 §Digital Signature) — re-encoding breaks \
              verification"
)]

use std::collections::BTreeMap;

use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::service::error::ServiceError;
use crate::storage::codec::{decompose, reassemble};
use crate::versioning::audit::AuditInput;
use crate::versioning::object_version_id::TreeId;
use crate::versioning::{Kind, SigningCtx};

/// A lineage key: the trunk (`("", 0, 0)`) or one specific branch of one system
/// (`(creating_system_id, trunk_version, branch_number)`). Versions on the same
/// lineage supersede each other; distinct lineages coexist.
type Lineage = (String, i32, i32);

/// The current state of a to-be-imported container in the target store —
/// mapped from the storage aggregate read
/// (`crate::storage::version_repo::import::imported_container_state`).
#[derive(Debug, Clone, Default)]
struct ContainerState {
    /// The stored kind, if the `vo_id` already exists.
    kind: Option<Kind>,
    /// The owning EHR of the existing container.
    owner: Option<EhrId>,
    /// The highest trunk version currently held.
    max_trunk: i32,
    /// The highest storage ordinal currently held.
    max_ordinal: i32,
}

/// Read + map the container state through the storage row I/O; an existing
/// container with an unrecognized stored kind is a server fault.
async fn container_state(
    tx: &mut PgConnection,
    vo_id: VoId,
) -> Result<ContainerState, ServiceError> {
    let row = crate::storage::version_repo::import::imported_container_state(tx, vo_id).await?;
    let kind = match row.kind {
        None => None,
        Some(text) => Some(Kind::from_type(&text).ok_or_else(|| {
            ServiceError::exception(format!("unknown versioned-object kind {text:?}"))
        })?),
    };
    Ok(ContainerState {
        kind,
        owner: row.owner,
        max_trunk: row.max_trunk,
        max_ordinal: row.max_ordinal,
    })
}

/// One received `ORIGINAL_VERSION` to import into the local store (SM
/// `I_EHR_EXTRACT_SERVICE.import_ehr`; master06 §Copying).
#[derive(Debug)]
pub(crate) struct ImportVersion {
    /// The `version_tree_id` of the wrapped original (trunk or branch; branch
    /// import is first-class — master06 §The 'Virtual Version Tree').
    pub(crate) tree: TreeId,
    /// The wrapped original's `creating_system_id` — per VERSION, not per
    /// container: a copied version tree legitimately mixes systems (master06
    /// §Distributed Versioning).
    pub(crate) creating_system_id: String,
    /// The wrapped original's `preceding_version_uid`, preserved verbatim.
    pub(crate) preceding_version_uid: Option<String>,
    /// The wrapped original's `other_input_version_uids` (merge provenance),
    /// preserved verbatim.
    pub(crate) other_input_version_uids: Vec<String>,
    /// The wrapped original's resolved `version_lifecycle_state` code.
    pub(crate) lifecycle_state: String,
    /// The wrapped original's own `VERSION.contribution` `OBJECT_REF` (1..1) —
    /// a reference into the SOURCE system's contribution set, preserved
    /// verbatim (master06 §Committal and Audits).
    pub(crate) contribution: Value,
    /// The wrapped original's `commit_audit`, preserved verbatim as the
    /// canonical `AUDIT_DETAILS` fragment received.
    pub(crate) commit_audit: Value,
    /// The version data (`Value::Null` for a `523|deleted|` version — no nodes).
    pub(crate) data: Value,
    /// The wrapped original's `VERSION.signature` (preserved, never re-signed).
    pub(crate) signature: Option<String>,
    /// The wrapped original's `ATTESTATION`s (already full RM objects),
    /// preserved.
    pub(crate) attestations: Vec<Value>,
}

/// The `version.wrapped_original` fragment for one received original: its
/// own `contribution`, `commit_audit` and (optional) `signature`, verbatim —
/// "the knowledge of the original Contribution and committal are retained
/// inside the wrapped `ORIGINAL_VERSION` instance" (master06 §Committal and
/// Audits).
fn wrapped_fragment(version: &ImportVersion) -> Value {
    let mut fragment = serde_json::json!({
        "contribution": version.contribution,
        "commit_audit": version.commit_audit,
    });
    if let Some(signature) = &version.signature
        && let Value::Object(map) = &mut fragment
    {
        map.insert("signature".to_owned(), Value::String(signature.clone()));
    }
    fragment
}

impl ImportVersion {
    /// The lineage this version sits on.
    fn lineage(&self) -> Lineage {
        match self.tree.branch {
            None => (String::new(), 0, 0),
            Some((b, _)) => (self.creating_system_id.clone(), self.tree.trunk, b),
        }
    }
}

/// One versioned object (a source `VERSIONED_OBJECT`) to import: its cloned
/// `vo_id` (the received `uid.object_id()` — master06 §Copying: "a new
/// `VERSIONED_OBJECT` is created, with its uid set to the same value as the
/// received `VERSION._uid.object_id()`"), its kind, and its versions.
pub(crate) struct ImportContainer {
    pub(crate) vo_id: VoId,
    pub(crate) kind: Kind,
    pub(crate) versions: Vec<ImportVersion>,
}

/// Replay a set of received `ORIGINAL_VERSION`s into an EHR as
/// `IMPORTED_VERSION`s under **one** local import CONTRIBUTION (master06
/// §Copying, §Committal and Audits). The `import_audit` records the local act
/// of committal (`249|creation|`, master06 §Contributions "import of item"). Returns the
/// local import contribution id.
///
/// # Errors
/// The [`commit_import_scoped`] conflicts: a duplicated version identity within
/// the import, a container owned by another EHR, a kind mismatch against the
/// existing clone, or a re-imported (non-newer) trunk version — all
/// [`ServiceError::Conflict`]; plus decompose/storage write errors.
pub(crate) async fn commit_import(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    privacy: &crate::privacy::PrivacyPolicy,
    ehr_id: EhrId,
    import_audit: &AuditInput,
    containers: Vec<ImportContainer>,
) -> Result<Uuid, ServiceError> {
    commit_import_scoped(
        tx,
        ctx,
        privacy,
        Some(ehr_id),
        import_audit,
        containers,
        false,
    )
    .await
}

/// Land demographics-chapter parties into the demographic repository under their
/// own (ehr-less) import CONTRIBUTION (master09 §Creation Semantics — demographic
/// content is not EHR-owned). A party whose version container already exists
/// locally is SKIPPED — parties are shared continuants across extracts.
///
/// # Errors
/// The same conflicts and storage errors as [`commit_import`], restricted to
/// the containers that do not already exist locally.
pub(crate) async fn commit_demographic_import(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    privacy: &crate::privacy::PrivacyPolicy,
    import_audit: &AuditInput,
    containers: Vec<ImportContainer>,
) -> Result<(), ServiceError> {
    if containers.is_empty() {
        return Ok(());
    }
    let mut fresh = Vec::with_capacity(containers.len());
    for container in containers {
        let state = container_state(tx, container.vo_id).await?;
        if state.kind.is_none() {
            fresh.push(container);
        }
    }
    if fresh.is_empty() {
        return Ok(());
    }
    commit_import_scoped(tx, ctx, privacy, None, import_audit, fresh, true).await?;
    Ok(())
}

/// Enforce the copy closure on one received container (RM common
/// master06 §Copying L275: branch versions "cannot be copied without their
/// corresponding preceding versions on the same branch (if any) and trunk
/// versions also being copied"). The rule binds the SENDER, but a
/// receiver-side check is the only way this repository keeps the invariant
/// its own read semantics rely on (`version_at` answers from the trunk chain
/// — a branch-only container has no as-of answer at all). Each branch version
/// requires its same-branch predecessor and its fork-point trunk version, in
/// THIS import or already stored (Case-3 appends).
///
/// # Errors
/// [`ServiceError::BadRequest`] naming the missing fork-point trunk or
/// same-branch predecessor; [`ServiceError`] on a storage probe failure.
async fn enforce_copy_closure(
    tx: &mut PgConnection,
    container: &ImportContainer,
) -> Result<(), ServiceError> {
    for version in &container.versions {
        let (trunk_version, branch_number, branch_version) = version.tree.columns();
        if branch_number == 0 {
            continue;
        }
        let in_set = |sys: Option<&str>, t: i32, b: i32, bv: i32| {
            container.versions.iter().any(|v| {
                let (vt, vb, vbv) = v.tree.columns();
                sys.is_none_or(|s| v.creating_system_id == s) && vt == t && vb == b && vbv == bv
            })
        };
        // (a) The fork-point trunk version. The trunk chain is per
        // versioned object, NOT per creating system — a branch legitimately
        // forks off a trunk version another system committed (master06
        // §Distributed Versioning; §6.4.1.2 branch-on-foreign), so the trunk
        // position is matched with ANY creating system.
        if !in_set(None, trunk_version, 0, 0)
            && !crate::storage::version_repo::import::has_version_tree(
                tx,
                container.vo_id,
                None,
                trunk_version,
                0,
                0,
            )
            .await?
        {
            return Err(ServiceError::precondition(format!(
                "the extract violates the copy closure (RM common master06 §Copying): \
                 branch version {}::{}::{} arrives without its fork-point trunk \
                 version {trunk_version} (neither in the extract nor already stored)",
                container.vo_id, version.creating_system_id, version.tree
            )));
        }
        // (b) The same-branch predecessor, when one must exist — a branch
        // LINEAGE is identified per creating system (the storage lineage key
        // {vo, creating system, fork point, branch number}), so this match
        // keeps the branch's own system.
        if branch_version > 1
            && !in_set(
                Some(&version.creating_system_id),
                trunk_version,
                branch_number,
                branch_version - 1,
            )
            && !crate::storage::version_repo::import::has_version_tree(
                tx,
                container.vo_id,
                Some(&version.creating_system_id),
                trunk_version,
                branch_number,
                branch_version - 1,
            )
            .await?
        {
            return Err(ServiceError::precondition(format!(
                "the extract violates the copy closure (RM common master06 §Copying): \
                 branch version {}::{}::{} arrives without its same-branch \
                 predecessor (branch version {}) — neither in the extract nor \
                 already stored",
                container.vo_id,
                version.creating_system_id,
                version.tree,
                branch_version - 1
            )));
        }
    }
    Ok(())
}

/// The local act of committal every version of one import shares: the fresh
/// CONTRIBUTION, its audit, and the temporal base of the synthetic period
/// chain (master06 §Committal and Audits).
struct ImportAct<'a> {
    ctx: &'a SigningCtx<'a>,
    /// The clinical-side data-minimisation policy every replayed clinical body
    /// is checked against before it is stored verbatim (#3237).
    privacy: &'a crate::privacy::PrivacyPolicy,
    ehr_id: Option<EhrId>,
    contribution_id: Uuid,
    contribution_commit_audit_id: Uuid,
    /// The canonical `AUDIT_DETAILS` fragment of the local act, signed into
    /// every wrapper.
    local_commit_audit: &'a Value,
    /// The local act's `change_type` code — `249|creation|` for an import
    /// (master06 §Contributions, "import of item").
    change_type: &'a str,
    base: jiff::Timestamp,
}

/// One container being replayed: its identity and the ordered versions.
struct ContainerCursor<'a> {
    vo_id: VoId,
    kind: Kind,
    versions: &'a [ImportVersion],
}

/// Refuses a container that carries the same version identity twice.
///
/// The identity tuple is {`object_id`, `creating_system_id`,
/// `version_tree_id`} (master06 §Distributed Versioning); the caller has
/// already sorted the versions, so duplicates are adjacent.
///
/// # Errors
/// [`ServiceError::Conflict`] naming the repeated version.
fn reject_duplicate_identities(container: &ImportContainer) -> Result<(), ServiceError> {
    for pair in container.versions.windows(2) {
        let [first, second] = pair else { continue };
        if first.creating_system_id == second.creating_system_id && first.tree == second.tree {
            return Err(ServiceError::conflict(format!(
                "version {}::{}::{} appears more than once in the import",
                container.vo_id, first.creating_system_id, first.tree
            )));
        }
    }
    Ok(())
}

/// Prepares a container whose first receipt already happened (master06
/// §Copying Case 3): the kind must match, the EHR must own it, every imported
/// version must be strictly newer than the stored tip of its lineage, and a
/// still-open stored tip closes at the import base.
///
/// # Errors
/// [`ServiceError::Conflict`] on a foreign owner, a kind mismatch, or a
/// re-imported trunk or branch version; storage errors from the lineage reads.
async fn append_to_existing_clone(
    tx: &mut PgConnection,
    act: &ImportAct<'_>,
    container: &ImportContainer,
    state: &ContainerState,
    existing_kind: Kind,
) -> Result<(), ServiceError> {
    if state.owner != act.ehr_id {
        return Err(ServiceError::conflict(format!(
            "versioned object {} already exists in another EHR",
            container.vo_id
        )));
    }
    if existing_kind != container.kind {
        return Err(ServiceError::conflict(format!(
            "versioned object {} is a {}, cannot import a {}",
            container.vo_id,
            existing_kind.as_str(),
            container.kind.as_str()
        )));
    }
    if let Some(first_trunk) = container
        .versions
        .iter()
        .filter(|v| v.tree.is_trunk())
        .map(|v| v.tree.trunk)
        .min()
        && first_trunk <= state.max_trunk
    {
        return Err(ServiceError::conflict(format!(
            "versioned object {} already has trunk version {} — cannot \
             re-import trunk version {first_trunk}",
            container.vo_id, state.max_trunk
        )));
    }
    advance_existing_branches(tx, container).await
}

/// The BRANCH mirror of the trunk checks in [`append_to_existing_clone`].
///
/// A later receipt may also advance an already-held branch lineage (master06
/// §Copying — "previous copies have been made for the item"; §Semantics in
/// Distributed Systems keeps lineages coexisting). Each incoming branch
/// lineage must be strictly newer than the stored tip; the successor then
/// becomes that lineage's tip by being its highest version, with nothing to
/// close.
///
/// # Errors
/// [`ServiceError::Conflict`] on a re-imported branch version; storage errors
/// from the lineage reads.
async fn advance_existing_branches(
    tx: &mut PgConnection,
    container: &ImportContainer,
) -> Result<(), ServiceError> {
    let mut incoming: BTreeMap<Lineage, i32> = BTreeMap::new();
    for version in container.versions.iter().filter(|v| !v.tree.is_trunk()) {
        let (trunk_version, branch_number, branch_version) = version.tree.columns();
        let first = incoming
            .entry((
                version.creating_system_id.clone(),
                trunk_version,
                branch_number,
            ))
            .or_insert(branch_version);
        *first = (*first).min(branch_version);
    }
    for (lineage, first_incoming) in &incoming {
        let max_stored =
            crate::storage::version_repo::import::branch_lineage_head(tx, container.vo_id, lineage)
                .await?;
        if max_stored > 0 && *first_incoming <= max_stored {
            return Err(ServiceError::conflict(format!(
                "versioned object {} already has branch version {}.{}.{max_stored} — \
                 cannot re-import branch version {}.{}.{first_incoming}",
                container.vo_id, lineage.1, lineage.2, lineage.1, lineage.2
            )));
        }
    }
    Ok(())
}

/// Lands a container this store has never seen (master06 §Copying Case 2).
///
/// A first-received FOLDER container is a new folder hierarchy of the EHR (RM
/// ehr master04 §Folders); every other kind needs no preparation.
///
/// # Errors
/// Storage errors from the folder-rank insert.
async fn land_first_receipt(
    tx: &mut PgConnection,
    act: &ImportAct<'_>,
    container: &ImportContainer,
) -> Result<(), ServiceError> {
    if container.kind == Kind::Folder
        && let Some(ehr_id) = act.ehr_id
    {
        crate::storage::version_repo::commit::insert_ehr_folder_rank(tx, ehr_id, container.vo_id)
            .await?;
    }
    Ok(())
}

/// Replays one received original as an `IMPORTED_VERSION` row (plus its nodes
/// and attestations), returning the PHI-free outbox entry announcing it.
///
/// The wrapper is signed over the bytes a later read will serve, never over
/// the received JSON, so commit-time and read-time canonical forms are
/// identical by construction (master06 §Digital Signature).
///
/// # Errors
/// Decompose/reassemble failures on the received data, signing failures, and
/// storage errors from the row, node and attestation writes.
async fn import_one_version(
    tx: &mut PgConnection,
    act: &ImportAct<'_>,
    cursor: &ContainerCursor<'_>,
    index: usize,
    ordinal: i32,
) -> Result<Value, ServiceError> {
    let Some(version) = cursor.versions.get(index) else {
        return Err(ServiceError::exception(
            "import cursor addressed a version past the end of the container".to_owned(),
        ));
    };
    let committed_at = local_commit_instant(index, act.base);
    let (trunk_version, branch_number, branch_version) = version.tree.columns();
    // The wrapped ORIGINAL_VERSION, reproduced exactly as received: its own
    // contribution reference, commit audit (with the SOURCE `time_committed`)
    // and signature, beside the identity/lifecycle/data columns the row already
    // carries. master06 §Copying: "the `ORIGINAL_VERSION` instance is never
    // modified — it remains a faithful copy of its original".
    let wrapped_original = wrapped_fragment(version);
    // The same data-minimisation pass every clinical commit takes, over the
    // received body. A replayed record is stored verbatim, never rewritten
    // (master06 §Copying), so a refusal is the only way an identifier stays
    // out of the clinical side; the demographic kinds are the domain that
    // holds identity and take no scan (#3237).
    if !cursor.kind.is_demographic() && !version.data.is_null() {
        crate::service::ehr::validation::enforce_privacy(
            act.privacy,
            cursor.kind.as_str(),
            &version.data,
        )?;
    }
    // The content, decomposed once: the node rows to write AND — through
    // `reassemble` — the exact bytes a later read will serve.
    let rows = if version.data.is_null() {
        Vec::new()
    } else {
        decompose(version.data.clone())?
    };
    let served = if rows.is_empty() {
        Value::Null
    } else {
        reassemble(&rows)?
    };
    let item = crate::versioning::wire::build_original_version(
        &crate::versioning::wire::OriginalVersionParts {
            creating_system_id: &version.creating_system_id,
            vo_id: cursor.vo_id,
            tree: version.tree,
            preceding_version_uid: version.preceding_version_uid.as_deref(),
            other_input_version_uids: &version.other_input_version_uids,
            contribution: &version.contribution,
            commit_audit: &version.commit_audit,
            lifecycle_state: &version.lifecycle_state,
            data: &served,
            // The received original's own attestations are attributes of
            // `item`, so they ride inside the wrapper's signed form (master06
            // §Digital Signature). They are this repository's at-committal
            // attestations: the local act of importing supplies none of its own.
            attestations: &version.attestations,
            signature: version.signature.as_deref(),
        },
    )?;
    // The wrapper's own signature, "which signifies the act of importing and
    // making available locally an `ORIGINAL_VERSION` from another system"
    // (master06 §Digital Signature).
    let signature = crate::versioning::integrity::sign_imported_version(
        act.ctx,
        act.contribution_id,
        act.local_commit_audit,
        &item,
    )?;
    crate::storage::version_repo::import::insert_imported_vo_version(
        tx,
        &crate::storage::version_repo::import::ImportedVersionRow {
            vo_id: cursor.vo_id,
            kind: cursor.kind.as_str(),
            ehr_id: act.ehr_id,
            sys_version: ordinal,
            trunk_version,
            branch_number,
            branch_version,
            lifecycle_state: &version.lifecycle_state,
            creating_system_id: &version.creating_system_id,
            preceding_version_uid: version.preceding_version_uid.as_deref(),
            other_input_version_uids: &version.other_input_version_uids,
            contribution_id: act.contribution_id,
            commit_audit_id: act.contribution_commit_audit_id,
            signature: signature.as_deref(),
            wrapped_original: &wrapped_original,
            committed_at,
            body: (!served.is_null()).then_some(&served),
        },
    )
    .await?;
    // A `523|deleted|` version stores no node rows (data is Void).
    if !rows.is_empty() {
        crate::storage::node_repo::write_nodes(tx, cursor.vo_id, ordinal, act.ehr_id, &rows)
            .await?;
    }
    for attestation in &version.attestations {
        crate::storage::version_repo::attestation::insert_attestation(
            tx,
            cursor.vo_id,
            ordinal,
            act.contribution_id,
            // At committal: these rode in with the original and are inside the
            // wrapper signature computed just above.
            true,
            attestation,
        )
        .await?;
    }
    // PHI-free outbox entry: identity + provenance only; no template_id. The
    // announced `change_type` is the LOCAL act's.
    Ok(serde_json::json!({
        "vo_id": cursor.vo_id,
        "kind": cursor.kind.as_str(),
        "sys_version": ordinal,
        "version_tree_id": version.tree.to_string(),
        "change_type": act.change_type,
        "template_id": Value::Null,
    }))
}

/// The synthetic local commit instant of one imported version: a
/// strictly-increasing 1 µs step off the import base.
fn local_commit_instant(index: usize, base: jiff::Timestamp) -> jiff::Timestamp {
    base + jiff::SignedDuration::from_micros(i64::try_from(index).unwrap_or(0))
}

/// NOTE (local commit instants, master06 §Copying): all versions of an imported
/// container are committed in the single local import act, so they get a
/// synthetic strictly-increasing local `committed_at` chain (base = import
/// time, 1 µs steps), which is §Copying's rule that "the commit times always
/// reflect the local (more recent) act of committal"; the source chronology is
/// not lost but moved inside the wrapped `ORIGINAL_VERSION`, where §Committal
/// and Audits puts it.
async fn commit_import_scoped(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    privacy: &crate::privacy::PrivacyPolicy,
    ehr_id: Option<EhrId>,
    import_audit: &AuditInput,
    containers: Vec<ImportContainer>,
    skip_existing: bool,
) -> Result<Uuid, ServiceError> {
    // The DATABASE transaction timestamp anchors the whole import's temporal
    // chain, never the app clock: under skew an app-clock base could close an
    // open lineage before that row's lower bound. This ONE audit row is the
    // local act of committal for the CONTRIBUTION and every IMPORTED_VERSION it
    // carries (master06 §Committal and Audits).
    let (contribution_commit_audit_id, import_time) =
        crate::storage::version_repo::commit::insert_audit(tx, &import_audit.row()).await?;
    let base = import_time;
    let local_commit_audit = import_audit.canonical(&import_time);
    let contribution_id = crate::storage::version_repo::commit::insert_contribution(
        tx,
        ctx.stamp.mint(),
        ehr_id,
        contribution_commit_audit_id,
    )
    .await?;
    let act = ImportAct {
        ctx,
        privacy,
        ehr_id,
        contribution_id,
        contribution_commit_audit_id,
        local_commit_audit: &local_commit_audit,
        change_type: &import_audit.change_type,
        base,
    };
    let mut outbox_versions: Vec<Value> = Vec::new();

    for mut container in containers {
        if container.versions.is_empty() {
            continue;
        }
        enforce_copy_closure(tx, &container).await?;
        // Version-tree order; a duplicated version identity within one import
        // is a conflict (the identity tuple is {object_id, creating_system_id,
        // version_tree_id} — master06 §Distributed Versioning).
        container
            .versions
            .sort_by_key(|v| (v.lineage(), v.tree.columns()));
        reject_duplicate_identities(&container)?;

        let state = container_state(tx, container.vo_id).await?;
        if skip_existing && state.kind.is_some() {
            continue;
        }
        match state.kind {
            Some(existing_kind) => {
                append_to_existing_clone(tx, &act, &container, &state, existing_kind).await?;
            }
            None => {
                land_first_receipt(tx, &act, &container).await?;
            }
        }

        // Per-lineage commit chains: within a lineage each version supersedes
        // its predecessor by carrying a later instant, and lineages coexist.
        let cursor = ContainerCursor {
            vo_id: container.vo_id,
            kind: container.kind,
            versions: &container.versions,
        };
        let mut ordinal = state.max_ordinal;
        for index in 0..cursor.versions.len() {
            ordinal += 1;
            outbox_versions.push(import_one_version(tx, &act, &cursor, index, ordinal).await?);
        }
        // The head row is a pure function of the version rows, so a whole
        // container recomputes it once at the end rather than per version.
        crate::storage::version_repo::import::sync_heads(tx, &[container.vo_id]).await?;
    }
    if ctx.outbox_enabled && !outbox_versions.is_empty() {
        crate::storage::version_repo::commit::write_outbox(
            tx,
            contribution_id,
            ehr_id,
            import_time,
            outbox_versions,
        )
        .await?;
    }
    Ok(contribution_id)
}
