// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The local commit write path.
//!
//! No openEHR spec governs the SQL — our own design. The change-control law realized here is RM common master06
//! (§Committal and Audits, §The 'Virtual Version Tree'); `AUDIT_DETAILS` is
//! master04.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use std::sync::LazyLock;

use serde_json::Value;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use super::audit_shared::{AuditShares, SharedPart, SharedPlan, SharedRef, shared_value};
use super::head_upsert::{HeadEpisodes, HeadUpsert, Placement};
use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// The `AUDIT_DETAILS` fields to persist: the facts of the audit's own row,
/// and its shared part (#6395).
///
/// master04 §Audit Details: `change_type` is the numeric `audit_change_type`
/// group code, never a rubric (`Change_type_valid`), and the jsonb members
/// arrive as the canonical RM fragments they store (`DV_TEXT` /
/// `PARTY_PROXY` / the `ATTESTATION`-declared attributes).
#[derive(Debug)]
pub struct AuditRow<'a> {
    /// The ids of the shared parts this process has seen.
    pub shares: &'a AuditShares,
    /// The audit's shared part: its system id, and its committer and
    /// description when no record subject can be named by them.
    pub shared: SharedPart<'a>,
    /// The numeric `audit_change_type` group code.
    pub change_type: &'a str,
    /// The canonical `DV_TEXT` JSON text of a description the shared part
    /// does not hold.
    pub description: Option<String>,
    /// The canonical `PARTY_PROXY` JSON text of a committer the shared part
    /// does not hold.
    pub committer: Option<String>,
    /// The canonical fragment of the `ATTESTATION`-declared attributes when
    /// this commit audit is an `ATTESTATION` (master06 §Attestation), else
    /// `None`.
    pub attestation: Option<Value>,
}

/// Binds the five parameters of `audit`, its first the id of its shared part
/// or, when `shared` is new, the part's JSON text.
pub(super) fn bind_audit<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    audit: &'q AuditRow<'_>,
    shared: SharedRef,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    let query = match shared {
        SharedRef::Known(id) => query.bind(id),
        SharedRef::New(_) => query.bind(audit.shared.content()),
    };
    query
        .bind(audit.change_type)
        .bind(&audit.description)
        .bind(&audit.committer)
        .bind(&audit.attestation)
}

/// Takes the per-vo transaction advisory lock that serializes concurrent
/// writers of one versioned object, so branch writers do not all contend on one
/// current row.
///
/// The versioning tree-placement decision calls this before it reads the
/// preceding version.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
// The preceding-version reads and the next-ordinal/next-branch computation that
// surround this lock are the version-tree placement DECISION — they live in the
// versioning layer (`versioning::change`), which calls this lock first.
pub async fn advisory_lock(tx: &mut PgConnection, vo_id: VoId) -> Result<(), StorageError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(vo_id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// The `version` columns for a **folded** commit.
///
/// Every content column of a stored version EXCEPT
/// `contribution_id`/`commit_audit_id`, which come from the same statement's
/// `contribution`/`commit_audit` CTEs.
///
/// `time_committed` is the caller's pre-read commit instant, a database `now()`
/// fetched earlier on this request and so still server-assigned (master06
/// §Committal and Audits). Binding it makes the stored audit time, the version row's
/// own `committed_at` and the instant the `VERSION.signature` was computed over
/// one value by construction.
///
/// Nothing is superseded in place: the store is append-only, so a commit
/// inserts its version row and updates the object's one mutable head row in the
/// same statement. Which version is current is the head row's answer
/// (master06 §The 'Virtual Version Tree'), not a validity interval.
#[derive(Debug)]
pub struct FoldedVersion<'a> {
    /// The versioned object's id.
    pub vo_id: VoId,
    /// The `version.kind` discriminator text.
    pub kind: &'a str,
    /// The owning EHR, or `None` for a demographic versioned object.
    pub ehr_id: Option<EhrId>,
    /// The per-object storage commit ordinal — NOT the wire version number.
    pub sys_version: i32,
    /// `VERSION_TREE_ID` first part.
    pub trunk_version: i32,
    /// `VERSION_TREE_ID` second part; `0` on a trunk row.
    pub branch_number: i32,
    /// `VERSION_TREE_ID` third part; `0` on a trunk row.
    pub branch_version: i32,
    /// The `version_lifecycle_state` numeric code.
    pub lifecycle_state: &'a str,
    /// The version's immutable `creating_system_id`.
    pub creating_system_id: &'a str,
    /// `ORIGINAL_VERSION.preceding_version_uid`; `None` for a first version.
    pub preceding_version_uid: Option<&'a str>,
    /// `ORIGINAL_VERSION.other_input_version_uids`; empty when not a merge.
    pub other_input_version_uids: &'a [String],
    /// The OPT `template_id` a COMPOSITION was committed against (else `None`).
    pub template_id: Option<&'a str>,
    /// `VERSION.signature` (0..1), opaque radix-64.
    pub signature: Option<&'a str>,
    /// Whether `signature` was supplied verbatim by the client (foreign — never
    /// re-verified at read; master06 §Digital Signature) vs generated by this
    /// server. `false` for a server signature or an unsigned version.
    pub signature_client_supplied: bool,
    /// Whether the RELEASED openEHR generation set can express this version's
    /// body — the `version.stable_compatible` stamp the read-time
    /// `spec_profile` gate consults. No openEHR spec governs runtime
    /// generation selection — our own design/extension.
    pub stable_compatible: bool,
    /// The distinct origins of the body (`version.origins`, a JSON array
    /// of `FEEDER_AUDIT` originating system ids, or this server's own), the
    /// access log's answer to EHDS Annex II 3.2(e) (#3212).
    pub origins: &'a [String],
    /// The canonical body bytes (`version.body`, text): the accepted,
    /// uid-stamped value serialized before node decomposition, so a point read
    /// serves the codec's field order verbatim. `None` on a logical delete (no
    /// content, RM common master06 §Logical Deletion).
    pub body: Option<&'a str>,
    /// The commit instant: the database `now()` the caller read on this
    /// request (the placement read, the writability gate, or the owning
    /// CONTRIBUTION's committal), stored as the audit `time_committed` and as
    /// the version row's own `committed_at`.
    pub time_committed: jiff::Timestamp,
    /// The decomposed node rows — inserted by the SAME statement through a
    /// node CTE ordered after the version row (empty on a logical delete and
    /// on a version that shares its predecessor's rows: the unnest yields no
    /// rows and the CTE writes nothing).
    pub rows: &'a [crate::storage::render::nodes::StoredNode],
    /// The external blob URIs the body references
    /// ([`crate::storage::render::blob_uris`]).
    pub blob_uris: &'a [String],
    /// SHA-256 over the canonical body with the root `uid` removed
    /// (`version.content_digest`); `None` on a logical delete.
    pub content_digest: Option<&'a [u8]>,
    /// The version in the same object whose node rows hold this version's
    /// content, when its content equalled the preceding tip's
    /// (`version.content_sys_version`, #4415); `None` for its own rows.
    pub content_sys_version: Option<i32>,
    /// The `archetype_node_id` of the body's root, which the head row keeps
    /// for a trunk commit (#5782); `None` on a logical delete.
    pub root_archetype: Option<&'a str>,
    /// The episodes a COMPOSITION is placed in, which the head row keeps for
    /// a trunk commit; empty when unplaced.
    pub episodes: &'a [crate::versioning::episode::EpisodeKey],
}

/// What one folded commit statement writes beside its `commit_audit` row.
///
/// Each shape is one data-modifying CTE chain, run inside the caller's
/// transaction: the round-trip-collapsed equivalent of the separate inserts,
/// byte-identical in the rows written and the values returned. No openEHR spec
/// governs statement batching: our own design.
#[derive(Debug)]
pub enum Folded<'a> {
    /// The CONTRIBUTION `id` and its own audit, the versions of its change set
    /// following under [`Folded::Into`]. `ehr_id` is `None` for a demographic
    /// CONTRIBUTION. The audit's `time_committed` is the database's.
    Contribution {
        /// The CONTRIBUTION uid, server-minted or client-supplied.
        id: Uuid,
        /// The owning EHR.
        ehr_id: Option<EhrId>,
    },
    /// A standalone version under the new CONTRIBUTION `id`, the one audit row
    /// serving both (a direct write is one CONTRIBUTION of one change, master06
    /// §Committal and Audits).
    Standalone {
        /// The CONTRIBUTION uid.
        id: Uuid,
        /// The version, its head upsert and its node rows.
        version: FoldedVersion<'a>,
    },
    /// A version within the CONTRIBUTION `id` an earlier
    /// [`Folded::Contribution`] of this transaction wrote, under the version's
    /// own `commit_audit`.
    Into {
        /// The CONTRIBUTION uid.
        id: Uuid,
        /// The version, its head upsert and its node rows.
        version: FoldedVersion<'a>,
    },
    /// A [`Folded::Standalone`] COMPOSITION write that writes nothing unless
    /// what its gate read judged still holds ([`super::guarded::Guard`]), with its event
    /// outbox row in the same statement (#6256).
    Guarded {
        /// The CONTRIBUTION uid.
        id: Uuid,
        /// The version, its head upsert and its node rows.
        version: FoldedVersion<'a>,
        /// What the gate read judged.
        guard: super::guarded::Guard,
    },
}

/// The rows one folded commit wrote.
#[derive(Debug, Clone, Copy)]
pub struct FoldedRows {
    /// The CONTRIBUTION the statement wrote or wrote into.
    pub contribution_id: Uuid,
    /// The `commit_audit` row the statement inserted.
    pub commit_audit_id: Uuid,
    /// The audit's `time_committed` as stored (master06 §Committal and Audits).
    pub time_committed: jiff::Timestamp,
    /// Whether an `EHR_STATUS` commit moved the EHR's subject (namespace or
    /// id), which the EHR's access copy is revised by.
    pub subject_moved: bool,
}

/// The three statement texts, one per [`Folded`] variant.
#[derive(Debug, Clone, Copy)]
enum Shape {
    /// [`Folded::Contribution`].
    Contribution,
    /// [`Folded::Standalone`].
    Standalone,
    /// [`Folded::Into`].
    Into,
    /// [`Folded::Guarded`].
    Guarded,
}

/// The statement text of one [`Folded`] shape, for a version of
/// `placement`.
///
/// Every shape binds the same parameters in the same order: `$1` to `$5` the
/// audit, `$6` the CONTRIBUTION uid, `$7` the owning EHR, `$8` to `$26` the
/// version, `$27` the version's root archetype, `$28` on the node arrays and
/// then a placed version's episodes, so one binder serves all three.
fn folded_statement(shape: Shape, placement: Placement, plan: &SharedPlan) -> String {
    if matches!(shape, Shape::Guarded) {
        return super::guarded::statement(super::guarded::Beside::Nothing, placement, plan);
    }
    let contribution = matches!(shape, Shape::Contribution | Shape::Standalone);
    let version = matches!(shape, Shape::Standalone | Shape::Into);
    let audit_time = if version { ", time_committed" } else { "" };
    let audit_time_value = if version { ", $22::timestamptz" } else { "" };
    let contribution_cte = if contribution {
        ", c AS ( \
             INSERT INTO contribution (id, ehr_id, commit_audit_id) \
             SELECT $6, $7, a.id FROM a \
             ON CONFLICT (id) DO NOTHING \
             RETURNING id \
         )"
    } else {
        ""
    };
    let version_cte = if version {
        let (contribution_id, from) = if contribution {
            ("c.id", "a, c")
        } else {
            ("$6", "a")
        };
        let after_nodes = 28 + crate::storage::node_write::node_array_count();
        version_ctes(
            "",
            ("$7", contribution_id, "a.id", from),
            8,
            (28, placement, after_nodes),
        )
    } else {
        String::new()
    };
    let learned = plan.learned_column();
    let select = if contribution {
        format!(
            " SELECT a.id AS commit_audit_id, a.time_committed, c.id AS contribution_id{learned} \
              FROM a LEFT JOIN c ON true"
        )
    } else {
        format!(
            " SELECT a.id AS commit_audit_id, a.time_committed, $6::uuid AS contribution_id{learned} \
              FROM a"
        )
    };
    let shared = shared_value(plan.audit(0), 1);
    let parts = plan.ctes(&[1]);
    let head = if parts.is_empty() {
        "WITH".to_owned()
    } else {
        format!("WITH {},", parts.trim_start_matches(", "))
    };
    format!(
        "{head} a AS ( \
             INSERT INTO commit_audit_facts (shared, change_type, description, committer, \
                                             attestation{audit_time}) \
             VALUES ({shared}, $2, $3::jsonb, $4::jsonb, $5{audit_time_value}) \
             RETURNING id, time_committed \
         ){contribution_cte}{version_cte}{select}"
    )
}

/// The columns a version insert names, in the order every folded statement
/// binds them.
const VERSION_COLUMNS: &str = "vo_id, kind, ehr_id, sys_version, trunk_version, branch_number, \
     branch_version, lifecycle_state, creating_system_id, preceding_version_uid, contribution_id, \
     commit_audit_id, template_id, signature, signature_client_supplied, stable_compatible, body, \
     origins, committed_at, other_input_version_uids, content_digest, content_sys_version";

/// The version-row, head-upsert and node CTEs of one version, named `v`, `h`
/// and `n` with `suffix`: the version's twenty parameters start at `first`
/// in the order [`bind_version`] binds them, its node arrays at `nodes`, its
/// episodes at `episodes` when its `placement` writes them
/// ([`bind_episodes`]), and
/// `(ehr, contribution, audit, from)` name the owning EHR, the CONTRIBUTION
/// and audit ids and the CTEs the version row is selected from.
pub(super) fn version_ctes(
    suffix: &str,
    (ehr, contribution, audit, from): (&str, &str, &str, &str),
    first: usize,
    (nodes, placement, episodes): (usize, Placement, usize),
) -> String {
    let p = |offset: usize| format!("${}", first + offset);
    let list = |offsets: &[usize]| {
        offsets
            .iter()
            .map(|offset| p(*offset))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let version = format!("v{suffix}");
    format!(
        ", {version} AS ( \
             INSERT INTO version ({VERSION_COLUMNS}) \
             SELECT {}, {ehr}, {}, {contribution}, {audit}, {}, {}::timestamptz, {} \
             FROM {from} RETURNING content_key \
         ), h{suffix} AS ( {} \
         ), n{suffix} AS ( {} )",
        list(&[0, 1]),
        list(&[2, 3, 4, 5, 6, 7, 8]),
        list(&[9, 10, 11, 12, 13, 15]),
        p(14),
        list(&[16, 17, 18]),
        HeadUpsert {
            vo: &p(0),
            kind: &p(1),
            ehr,
            ordinal: &p(2),
            branch: &p(4),
            lifecycle: &p(6),
            template: &p(9),
            archetype: &p(19),
            episodes: match placement {
                Placement::Untouched => HeadEpisodes::Untouched,
                Placement::Cleared => HeadEpisodes::Cleared,
                Placement::Placed => HeadEpisodes::Placed(episodes),
            },
            committed_at: &p(14),
            version: &version,
        }
        .sql(),
        crate::storage::node_write::node_insert_cte((&p(0), &p(2), ehr), &version, nodes)
    )
}

/// Bind the twenty parameters of `v` that [`version_ctes`] reads.
pub(super) fn bind_version<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    v: &FoldedVersion<'q>,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    query
        .bind(v.vo_id)
        .bind(v.kind)
        .bind(v.sys_version)
        .bind(v.trunk_version)
        .bind(v.branch_number)
        .bind(v.branch_version)
        .bind(v.lifecycle_state)
        .bind(v.creating_system_id)
        .bind(v.preceding_version_uid)
        .bind(v.template_id)
        .bind(v.signature)
        .bind(v.signature_client_supplied)
        .bind(v.stable_compatible)
        .bind(v.body)
        .bind(v.time_committed.to_string())
        .bind(serde_json::json!(v.origins))
        .bind(super::optional_json_array(v.other_input_version_uids))
        .bind(v.content_digest)
        .bind(v.content_sys_version)
        .bind(v.root_archetype)
}

/// Bind the episodes of a placed `v` as one `bytea[]`, the parameter
/// [`version_ctes`] reads after everything else the statement binds.
pub(super) fn bind_episodes<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    v: &FoldedVersion<'q>,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    if v.episodes.is_empty() {
        return query;
    }
    query.bind(
        v.episodes
            .iter()
            .map(|key| key.as_bytes().as_slice())
            .collect::<Vec<_>>(),
    )
}

/// The statement texts of one shape, for an audit whose shared part is known
/// and for one whose part the statement inserts.
type ByPart<T> = [T; 2];

/// The text of `texts` for an audit whose shared part the statement inserts
/// when `new`.
fn by_part<T>(texts: &ByPart<T>, new: bool) -> &T {
    let [known, inserting] = texts;
    if new { inserting } else { known }
}

/// The texts of one guarded shape `beside`, by part and placement.
fn guarded_texts(beside: super::guarded::Beside) -> ByPart<[String; 3]> {
    [false, true].map(|new| {
        Placement::ALL.map(|p| super::guarded::statement(beside, p, &SharedPlan::single(new)))
    })
}

/// The texts of one folded shape, by part and placement.
fn folded_texts(shape: Shape) -> ByPart<[String; 3]> {
    [false, true]
        .map(|new| Placement::ALL.map(|p| folded_statement(shape, p, &SharedPlan::single(new))))
}

/// The statement text `folded` runs: its shape, what it writes beside the
/// version, what the version does with its object's episodes, and whether
/// the statement inserts its audit's shared part (`new`).
fn folded_text(folded: &Folded<'_>, new: bool) -> &'static str {
    use super::guarded::Beside;
    static CONTRIBUTION: LazyLock<ByPart<String>> = LazyLock::new(|| {
        [false, true].map(|new| {
            folded_statement(
                Shape::Contribution,
                Placement::Untouched,
                &SharedPlan::single(new),
            )
        })
    });
    static STANDALONE: LazyLock<ByPart<[String; 3]>> =
        LazyLock::new(|| folded_texts(Shape::Standalone));
    static INTO: LazyLock<ByPart<[String; 3]>> = LazyLock::new(|| folded_texts(Shape::Into));
    static GUARDED: LazyLock<ByPart<[String; 3]>> = LazyLock::new(|| folded_texts(Shape::Guarded));
    static STATUS: LazyLock<ByPart<[String; 3]>> = LazyLock::new(|| guarded_texts(Beside::Status));
    static FOLDER: LazyLock<ByPart<[String; 3]>> = LazyLock::new(|| guarded_texts(Beside::Folder));
    static PARTY: LazyLock<ByPart<[String; 3]>> = LazyLock::new(|| guarded_texts(Beside::Party));
    let (texts, version): (&'static LazyLock<ByPart<[String; 3]>>, _) = match folded {
        Folded::Contribution { .. } => return by_part(&CONTRIBUTION, new).as_str(),
        Folded::Standalone { version, .. } => (&STANDALONE, version),
        Folded::Into { version, .. } => (&INTO, version),
        Folded::Guarded { version, guard, .. } if guard.status.is_some() => (&STATUS, version),
        Folded::Guarded { version, guard, .. } if guard.directory.is_some() => (&FOLDER, version),
        Folded::Guarded { version, guard, .. } if guard.party.is_some() => (&PARTY, version),
        Folded::Guarded { version, .. } => (&GUARDED, version),
    };
    Placement::of(version).pick(by_part(texts, new))
}

/// Run one folded commit statement ([`Folded`]), returning the rows it wrote.
///
/// On a CONTRIBUTION uid already in use the `contribution` CTE inserts nothing
/// (`ON CONFLICT DO NOTHING`), so the outer `LEFT JOIN` yields a NULL id; the
/// audit row is discarded when the transaction unwinds.
///
/// # Errors
/// Returns [`StorageError::ContributionUidInUse`] when a new CONTRIBUTION's uid
/// is already in use (ITS-REST `contribution_create`: a duplicate is a
/// conflict, never an overwrite), else [`StorageError::Database`] on a
/// driver/insert failure.
pub async fn commit_folded(
    tx: &mut PgConnection,
    audit: &AuditRow<'_>,
    folded: Folded<'_>,
) -> Result<FoldedRows, StorageError> {
    let status = matches!(&folded, Folded::Guarded { guard, .. } if guard.status.is_some());
    let party = match &folded {
        Folded::Contribution { ehr_id, .. } => ehr_id.is_none(),
        Folded::Standalone { version, .. }
        | Folded::Into { version, .. }
        | Folded::Guarded { version, .. } => version.ehr_id.is_none(),
    };
    let known = audit.shares.of(party);
    let shared = known
        .get(audit.shared.digest())
        .map_or(SharedRef::New(0), SharedRef::Known);
    let new = shared == SharedRef::New(0);
    let sql = folded_text(&folded, new);
    let (id, ehr_id, version, guard) = match folded {
        Folded::Contribution { id, ehr_id } => (id, ehr_id, None, None),
        Folded::Standalone { id, version } | Folded::Into { id, version } => {
            (id, version.ehr_id, Some(version), None)
        }
        Folded::Guarded { id, version, guard } => (id, version.ehr_id, Some(version), Some(guard)),
    };
    let query = bind_audit(sqlx::query(sqlx::AssertSqlSafe(sql)), audit, shared)
        .bind(id)
        .bind(ehr_id);
    let (row, blobs) = match version {
        None => (query.fetch_one(&mut *tx).await?, None),
        Some(v) => {
            let query = bind_version(query, &v);
            let query = match &guard {
                Some(guard) => query
                    .bind(guard.expected_head)
                    .bind(guard.policy)
                    .bind(guard.definitions)
                    .bind(&guard.outbox)
                    .bind(guard.reservation),
                None => query,
            };
            let query = crate::storage::node_write::bind_node_arrays(
                query,
                &v.rows.iter().collect::<Vec<_>>(),
            );
            let row = fetch_beside(tx, query, guard.as_ref(), &v).await?;
            (row, Some((v.vo_id, v.sys_version, v.blob_uris)))
        }
    };
    if new
        && let Some(Some(id)) = row
            .try_get::<Vec<Option<i32>>, _>("shared_learned")?
            .first()
    {
        known.remember(*audit.shared.digest(), *id);
    }
    if guard.is_some() {
        super::guarded::GuardFlags::read(&row)?.require()?;
    }
    let contribution_id: Option<Uuid> = row.try_get("contribution_id")?;
    let rows = FoldedRows {
        contribution_id: contribution_id.ok_or(StorageError::ContributionUidInUse(Some(id)))?,
        commit_audit_id: row.try_get("commit_audit_id")?,
        time_committed: row
            .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
            .to_jiff(),
        subject_moved: status && row.try_get::<i64, _>("notified")? > 0,
    };
    if let Some((vo_id, sys_version, uris)) = blobs {
        crate::storage::blob_ref::record_uris(tx, vo_id, sys_version, uris).await?;
    }
    Ok(rows)
}

/// Run a folded statement `query`, binding first what a guarded one takes
/// after its node arrays: an `EHR_STATUS` commit's columns, a party commit's
/// key-space gate and reverse targets, or a directory write's slot
/// ([`super::guarded::DirectorySlot`]), and last the episodes of a placed `v`.
///
/// # Errors
/// [`StorageError::SubjectInUse`] when a status commit's subject is in use,
/// else [`StorageError::Database`] on a driver failure.
async fn fetch_beside<'q>(
    tx: &mut PgConnection,
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    guard: Option<&super::guarded::Guard>,
    v: &FoldedVersion<'q>,
) -> Result<sqlx::postgres::PgRow, StorageError> {
    let party = guard.and_then(|guard| guard.party.as_deref());
    let status = guard.and_then(|guard| guard.status.as_deref());
    Ok(match (status, party) {
        (None, Some(party)) => {
            bind_episodes(
                query.bind(party.key_space).bind(party.reverse.as_deref()),
                v,
            )
            .fetch_one(&mut *tx)
            .await?
        }
        (Some(status), _) => bind_episodes(
            query
                .bind(status.subject_id.as_deref())
                .bind(status.subject_namespace.as_deref())
                .bind(status.is_queryable)
                .bind(status.is_modifiable)
                .bind(status.subject_digest.as_deref())
                .bind(crate::storage::access_projection::CHANNEL),
            v,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| subject_in_use(e, status))?,
        (None, None) => match guard.and_then(|guard| guard.directory) {
            Some(slot) => {
                bind_episodes(
                    query
                        .bind(slot.highest)
                        .bind(slot.tip.map(|(vo, _)| vo))
                        .bind(slot.tip.and_then(|(_, head)| head))
                        .bind(slot.new),
                    v,
                )
                .fetch_one(&mut *tx)
                .await?
            }
            None => bind_episodes(query, v).fetch_one(&mut *tx).await?,
        },
    })
}

/// `error` as [`StorageError::SubjectInUse`] when `uq_ehr_subject` refused
/// the subject `status` names (ITS-REST EHR API `409_EHR`).
fn subject_in_use(error: sqlx::Error, status: &super::guarded::StatusSync) -> StorageError {
    match &error {
        sqlx::Error::Database(db) if db.constraint() == Some("uq_ehr_subject") => {
            StorageError::SubjectInUse(
                status.subject_id.clone().unwrap_or_else(|| "?".to_owned()),
                status
                    .subject_namespace
                    .clone()
                    .unwrap_or_else(|| "?".to_owned()),
            )
        }
        _ => StorageError::Database(error),
    }
}

// ── folder membership ─────────────────────────────────────────────────────────

/// Append a new folder-hierarchy membership row for an EHR (RM ehr master04
/// §Folders; RM ehr EHR class `Directory_in_folders`).
///
/// `rank` is 1-based, append-only and never reused: the next rank is
/// `max(rank)+1` for this EHR. Called once per FOLDER *creation*. No openEHR
/// spec governs the `ehr_folder` storage mechanism (our own design).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver/insert failure.
pub async fn insert_ehr_folder_rank(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    vo_id: VoId,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO ehr_folder (ehr_id, rank, vo_id) VALUES \
         ($1, (SELECT COALESCE(MAX(rank), 0) + 1 FROM ehr_folder WHERE ehr_id = $1), $2)",
    )
    .bind(ehr_id)
    .bind(vo_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

// ── event outbox ──────────────────────────────────────────────────────────────

/// The fixed envelope shape the outbox feed serves around the PHI-free
/// per-version entries (`{contribution_id, ehr_id, committed_at, versions[]}`).
/// No openEHR spec governs eventing: our own extension.
#[must_use]
pub fn outbox_envelope(
    contribution_id: Uuid,
    ehr_id: Option<EhrId>,
    committed_at: jiff::Timestamp,
    versions: &[Value],
) -> Value {
    serde_json::json!({
        "contribution_id": contribution_id,
        "ehr_id": ehr_id,
        "committed_at": committed_at.to_string(),
        "versions": versions,
    })
}

/// Write the contribution-outbox event row **inside the commit transaction**
/// it announces — no commit without its event, no event without its commit.
///
/// No openEHR spec governs eventing (our own extension). The PHI-free
/// per-version entries are built by the versioning layer
/// (`Committed::envelope_entry`); this function wraps them in the fixed
/// envelope shape the outbox feed serves (`{contribution_id, ehr_id,
/// committed_at, versions[]}`). The insert takes the outbox lock shared and
/// holds it to the commit's end, so a feed read never passes a `seq` this
/// commit drew ([`crate::extensions::outbox`]).
///
/// An EHR carrying a legal mark emits no event. The `INSERT ... SELECT` writes
/// nothing when the EHR is restricted (GDPR Art. 18(2) leaves only storage) or
/// carries an unoverridden research objection (Art. 21(6)), so nothing about
/// that record reaches a downstream consumer; both are read in the same
/// statement rather than a probe before it, so a mark recorded concurrently
/// cannot slip between the two. `ehr_id IS NULL` is a demographic commit, which
/// no EHR mark reaches. `docs/law/eu/gdpr/text.html`.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver/insert failure.
pub async fn write_outbox(
    tx: &mut PgConnection,
    contribution_id: Uuid,
    ehr_id: Option<EhrId>,
    committed_at: jiff::Timestamp,
    versions: Vec<Value>,
) -> Result<(), StorageError> {
    let envelope = outbox_envelope(contribution_id, ehr_id, committed_at, &versions);
    // The party domain carries no `ehr` relation, and a demographic commit has
    // no EHR to mark, so the marked-EHR filter is on the clinical statement
    // alone: naming `ehr` in the party domain would fail at parse time.
    let sql = if ehr_id.is_some() {
        "WITH gate AS (SELECT pg_advisory_xact_lock_shared( \
             hashtext(current_schema() || '.event_outbox'), 0)) \
         INSERT INTO event_outbox (contribution_id, ehr_id, envelope, committed_at) \
         SELECT $1, $2, $3, $4::timestamptz FROM gate \
         WHERE NOT EXISTS (SELECT 1 FROM ehr e WHERE e.id = $2 \
             AND (e.restricted_at IS NOT NULL \
                  OR (e.research_objected_at IS NOT NULL \
                      AND e.research_objection_ground IS NULL)))"
    } else {
        "WITH gate AS (SELECT pg_advisory_xact_lock_shared( \
             hashtext(current_schema() || '.event_outbox'), 0)) \
         INSERT INTO event_outbox (contribution_id, ehr_id, envelope, committed_at) \
         SELECT $1, $2, $3, $4::timestamptz FROM gate"
    };
    sqlx::query(sql)
        .bind(contribution_id)
        .bind(ehr_id)
        .bind(&envelope)
        .bind(committed_at.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(())
}
