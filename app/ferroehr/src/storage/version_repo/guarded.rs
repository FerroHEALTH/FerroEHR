// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The guarded commit of a COMPOSITION write (#6256).
//!
//! The statement writes nothing unless what the write's gate read judged
//! still holds, and names which requirement moved when it writes nothing. No
//! openEHR spec governs
//! the SQL: our own design; the concepts the requirements keep are RM ehr
//! master04 §EHR Active Status and RM common master06 §The 'Virtual Version
//! Tree'.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::Value;

use crate::storage::error::StorageError;

/// What a guarded commit ([`super::commit::Folded::Guarded`]) still requires when it runs:
/// the facts its gate read judged the write on.
#[derive(Debug, Clone)]
pub struct Guard {
    /// The object's head ordinal the write was placed after (`None` for a
    /// create, whose object has no head yet).
    pub expected_head: Option<i32>,
    /// The policy revision the gate read returned.
    pub policy: Option<i64>,
    /// The definition revision the gate read returned.
    pub definitions: Option<i64>,
    /// The event outbox envelope, when eventing is enabled.
    pub outbox: Option<Value>,
}

/// Which of a guarded commit's requirements held when it ran, and the
/// policy and definition revisions it read; written nothing unless all held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is one requirement of the guarded statement, read off its result row"
)]
pub struct GuardFlags {
    /// The EHR still exists, is modifiable and is not restricted.
    pub ehr: bool,
    /// The object's head is still the one the write was placed after.
    pub tip: bool,
    /// The object still carries no restriction mark of its own.
    pub mark: bool,
    /// The policy and definition revisions are still those judged on.
    pub policy: bool,
    /// The write's commit instant is not earlier than the head's.
    pub order: bool,
    /// The policy revision the statement read.
    pub policy_now: Option<i64>,
    /// The definition revision the statement read.
    pub definitions_now: Option<i64>,
}

impl GuardFlags {
    /// Whether every requirement held.
    #[must_use]
    pub fn held(&self) -> bool {
        self.ehr && self.tip && self.mark && self.policy && self.order
    }
}

/// The statement text of [`super::commit::Folded::Guarded`].
///
/// The parameters of [`super::commit::Folded::Standalone`] up to `$27`, then `$28` the
/// expected head ordinal, `$29` the policy revision, `$30` the definition
/// revision and `$31` the outbox envelope, and the node arrays from `$32`.
/// The leading CTEs read what the gate read judged: the EHR row under `FOR
/// SHARE` (the lock a concurrent `EHR_STATUS` flip waits on), the object's
/// head row and the policy and definition revisions. The audit insert reads
/// from `g`, which holds a row only when every requirement still holds, and
/// every later insert reads from the audit's, so nothing is written
/// otherwise. The last row names each requirement's flag.
pub(super) fn statement() -> String {
    format!(
        "WITH e AS ( \
             SELECT COALESCE(x.is_modifiable, true) AS modifiable, \
                    x.restricted_at IS NOT NULL AS restricted \
             FROM ehr x WHERE x.id = $7 FOR SHARE \
         ), t AS ( \
             SELECT h.head_sys_version, h.restricted_at IS NOT NULL AS restricted, h.committed_at \
             FROM vo_head h WHERE h.vo_id = $8 \
         ), p AS ( \
             SELECT rr.policy, rr.definitions FROM request_revisions rr \
         ), k AS ( \
             SELECT COALESCE((SELECT e.modifiable AND NOT e.restricted FROM e), false) AS ehr_held, \
                    (SELECT t.head_sys_version FROM t) IS NOT DISTINCT FROM $28::int AS tip_held, \
                    NOT COALESCE((SELECT t.restricted FROM t), false) AS mark_held, \
                    (SELECT p.policy FROM p) IS NOT DISTINCT FROM $29::bigint \
                      AND (SELECT p.definitions FROM p) IS NOT DISTINCT FROM $30::bigint \
                      AS policy_held, \
                    COALESCE((SELECT t.committed_at <= $22::timestamptz FROM t), true) AS order_held, \
                    (SELECT p.policy FROM p) AS policy_now, \
                    (SELECT p.definitions FROM p) AS definitions_now \
         ), g AS ( \
             SELECT 1 AS held FROM k \
             WHERE k.ehr_held AND k.tip_held AND k.mark_held AND k.policy_held AND k.order_held \
         ), a AS ( \
             INSERT INTO commit_audit (system_id, change_type, description, committer, \
                                       attestation, time_committed) \
             SELECT $1, $2, $3::jsonb, $4::jsonb, $5, $22::timestamptz FROM g \
             RETURNING id, time_committed \
         ), c AS ( \
             INSERT INTO contribution (id, ehr_id, commit_audit_id) \
             SELECT $6, $7, a.id FROM a \
             ON CONFLICT (id) DO NOTHING \
             RETURNING id \
         ), v AS ( \
             INSERT INTO version \
               (vo_id, kind, ehr_id, sys_version, trunk_version, branch_number, branch_version, \
                lifecycle_state, creating_system_id, preceding_version_uid, \
                contribution_id, commit_audit_id, template_id, signature, \
                signature_client_supplied, stable_compatible, body, origins, committed_at, \
                other_input_version_uids, content_digest, content_sys_version) \
             SELECT $8, $9, $7, $10, $11, $12, $13, \
                    $14, $15, $16, c.id, a.id, $17, $18, $19, $20, $21, $23, \
                    $22::timestamptz, $24, $25, $26 \
             FROM a, c \
             RETURNING content_key \
         ), h AS ( {} \
         ), n AS ( {} \
         ), o AS ( \
             INSERT INTO event_outbox (contribution_id, ehr_id, envelope, committed_at) \
             SELECT $6, $7, $31::jsonb, $22::timestamptz \
             FROM (SELECT pg_advisory_xact_lock_shared( \
                       hashtext(current_schema() || '.event_outbox'), 0) \
                   FROM v WHERE $31::jsonb IS NOT NULL) l \
             WHERE NOT EXISTS (SELECT 1 FROM ehr m WHERE m.id = $7 \
                 AND (m.restricted_at IS NOT NULL \
                      OR (m.research_objected_at IS NOT NULL \
                          AND m.research_objection_ground IS NULL))) \
         ) \
         SELECT k.ehr_held, k.tip_held, k.mark_held, k.policy_held, k.order_held, \
                k.policy_now, k.definitions_now, \
                a.id AS commit_audit_id, a.time_committed, c.id AS contribution_id \
         FROM k LEFT JOIN a ON true LEFT JOIN c ON true",
        super::commit::HeadUpsert {
            vo: "$8",
            kind: "$9",
            ehr: "$7",
            ordinal: "$10",
            branch: "$12",
            lifecycle: "$14",
            template: "$17",
            archetype: "$27",
            committed_at: "$22",
            version: "v",
        }
        .sql(),
        crate::storage::node_write::node_insert_cte(("$8", "$10", "$7"), "v", 32)
    )
}

/// One member of a guarded CONTRIBUTION statement ([`commit_set`]): a version
/// under its own audit, and the head its object must still have.
#[derive(Debug)]
pub struct SetMember<'a> {
    /// The version's own `commit_audit`.
    pub audit: super::commit::AuditRow<'a>,
    /// The version, its head upsert and its node rows.
    pub version: super::commit::FoldedVersion<'a>,
    /// The object's head ordinal the version was placed after (`None` for a
    /// create).
    pub expected_head: Option<i32>,
}

/// The CONTRIBUTION a guarded set statement writes ([`commit_set`]).
#[derive(Debug)]
pub struct SetHead<'a> {
    /// The CONTRIBUTION uid, server-minted or client-supplied.
    pub id: uuid::Uuid,
    /// The owning EHR.
    pub ehr_id: crate::ids::EhrId,
    /// The CONTRIBUTION's own audit.
    pub audit: super::commit::AuditRow<'a>,
    /// The one commit instant of the set, stamped on every audit and version.
    pub time_committed: jiff::Timestamp,
}

/// The parameters one member binds beside its node arrays: five of its audit,
/// nineteen of its version and its expected head.
const MEMBER_SCALARS: usize = 25;

/// The statement text of a guarded CONTRIBUTION of `n` members.
///
/// `$1` the CONTRIBUTION uid, `$2` the EHR, `$3` the policy revision, `$4`
/// the definition revision, `$5` the outbox envelope, `$6` the commit instant
/// and `$7` to `$11` the CONTRIBUTION's audit; then each member's audit,
/// version, expected head and node arrays in turn. Each member repeats the
/// single write's version, head-upsert and node CTEs under its own names.
fn set_statement(n: usize) -> String {
    use std::fmt::Write as _;
    let stride = MEMBER_SCALARS + crate::storage::node_write::node_array_count();
    let base = |i: usize| 12 + i * stride;
    let mut sql = String::from(
        "WITH e AS ( \
             SELECT COALESCE(x.is_modifiable, true) AS modifiable, \
                    x.restricted_at IS NOT NULL AS restricted \
             FROM ehr x WHERE x.id = $2 FOR SHARE \
         ), p AS (SELECT rr.policy, rr.definitions FROM request_revisions rr)",
    );
    let (mut tips, mut marks, mut orders) = (
        String::from("true"),
        String::from("false"),
        String::from("true"),
    );
    for i in 0..n {
        let b = base(i);
        let _ = write!(
            sql,
            ", t{i} AS (SELECT h.head_sys_version, h.restricted_at IS NOT NULL AS restricted, \
             h.committed_at FROM vo_head h WHERE h.vo_id = ${})",
            b + 5
        );
        let _ = write!(
            tips,
            " AND (SELECT t.head_sys_version FROM t{i} t) IS NOT DISTINCT FROM ${}::int",
            b + 24
        );
        let _ = write!(
            marks,
            " OR COALESCE((SELECT t.restricted FROM t{i} t), false)"
        );
        let _ = write!(
            orders,
            " AND COALESCE((SELECT t.committed_at <= $6::timestamptz FROM t{i} t), true)"
        );
    }
    let _ = write!(
        sql,
        ", k AS ( \
             SELECT COALESCE((SELECT e.modifiable AND NOT e.restricted FROM e), false) AS ehr_held, \
                    ({tips}) AS tip_held, NOT ({marks}) AS mark_held, \
                    (SELECT p.policy FROM p) IS NOT DISTINCT FROM $3::bigint \
                      AND (SELECT p.definitions FROM p) IS NOT DISTINCT FROM $4::bigint \
                      AS policy_held, \
                    ({orders}) AS order_held, \
                    (SELECT p.policy FROM p) AS policy_now, \
                    (SELECT p.definitions FROM p) AS definitions_now \
         ), g AS ( \
             SELECT 1 AS held FROM k \
             WHERE k.ehr_held AND k.tip_held AND k.mark_held AND k.policy_held AND k.order_held \
         ), ca AS ( \
             INSERT INTO commit_audit (system_id, change_type, description, committer, \
                                       attestation, time_committed) \
             SELECT $7, $8, $9::jsonb, $10::jsonb, $11, $6::timestamptz FROM g \
             RETURNING id, time_committed \
         ), c AS ( \
             INSERT INTO contribution (id, ehr_id, commit_audit_id) \
             SELECT $1, $2, ca.id FROM ca \
             ON CONFLICT (id) DO NOTHING \
             RETURNING id \
         )"
    );
    for i in 0..n {
        member_ctes(&mut sql, i, base(i));
    }
    sql.push_str(
        ", o AS ( \
             INSERT INTO event_outbox (contribution_id, ehr_id, envelope, committed_at) \
             SELECT $1, $2, $5::jsonb, $6::timestamptz \
             FROM (SELECT pg_advisory_xact_lock_shared( \
                       hashtext(current_schema() || '.event_outbox'), 0) \
                   FROM c WHERE $5::jsonb IS NOT NULL) l \
             WHERE NOT EXISTS (SELECT 1 FROM ehr m WHERE m.id = $2 \
                 AND (m.restricted_at IS NOT NULL \
                      OR (m.research_objected_at IS NOT NULL \
                          AND m.research_objection_ground IS NULL))) \
         ) \
         SELECT k.ehr_held, k.tip_held, k.mark_held, k.policy_held, k.order_held, \
                k.policy_now, k.definitions_now, \
                ca.id AS commit_audit_id, ca.time_committed, c.id AS contribution_id \
         FROM k LEFT JOIN ca ON true LEFT JOIN c ON true",
    );
    sql
}

/// The CTEs of member `i`, whose parameters start at `b`: the single
/// write's audit, version, head-upsert and node CTEs under names of its own.
fn member_ctes(sql: &mut String, i: usize, b: usize) {
    use std::fmt::Write as _;
    let p = |offset: usize| format!("${}", b + offset);
    let _ = write!(
        sql,
        ", a{i} AS ( \
             INSERT INTO commit_audit (system_id, change_type, description, committer, \
                                       attestation, time_committed) \
             SELECT {}, {}, {}::jsonb, {}::jsonb, {}, $6::timestamptz FROM c \
             RETURNING id \
         ), v{i} AS ( \
             INSERT INTO version \
               (vo_id, kind, ehr_id, sys_version, trunk_version, branch_number, \
                branch_version, lifecycle_state, creating_system_id, preceding_version_uid, \
                contribution_id, commit_audit_id, template_id, signature, \
                signature_client_supplied, stable_compatible, body, origins, committed_at, \
                other_input_version_uids, content_digest, content_sys_version) \
             SELECT {}, {}, $2, {}, {}, {}, {}, {}, {}, {}, c.id, a{i}.id, {}, {}, {}, {}, \
                    {}, {}, $6::timestamptz, {}, {}, {} \
             FROM a{i}, c \
             RETURNING content_key \
         ), h{i} AS ( {} \
         ), n{i} AS ( {} )",
        p(0),
        p(1),
        p(2),
        p(3),
        p(4),
        p(5),
        p(6),
        p(7),
        p(8),
        p(9),
        p(10),
        p(11),
        p(12),
        p(13),
        p(14),
        p(15),
        p(16),
        p(17),
        p(18),
        p(19),
        p(20),
        p(21),
        p(22),
        super::commit::HeadUpsert {
            vo: &p(5),
            kind: &p(6),
            ehr: "$2",
            ordinal: &p(7),
            branch: &p(9),
            lifecycle: &p(11),
            template: &p(14),
            archetype: &p(23),
            committed_at: "$6",
            version: &format!("v{i}"),
        }
        .sql(),
        crate::storage::node_write::node_insert_cte(
            (&p(5), &p(7), "$2"),
            &format!("v{i}"),
            b + MEMBER_SCALARS,
        )
    );
}

/// Write a CONTRIBUTION and every one of its `members` in one guarded
/// statement ([`set_statement`]).
///
/// Nothing is written unless the EHR is still
/// modifiable and unrestricted, every member's head and mark are unchanged,
/// the revisions are those judged on and no head is later than the set's
/// instant. The members' blob references follow in the caller's transaction.
///
/// # Errors
/// [`StorageError::GuardMoved`] when a requirement no longer held,
/// [`StorageError::ContributionUidInUse`] when the CONTRIBUTION uid is in
/// use, and [`StorageError::Database`] on a driver failure (a version key
/// refusing a member whose place a concurrent commit took among them).
pub async fn commit_set(
    conn: &mut sqlx::PgConnection,
    head: SetHead<'_>,
    members: &[SetMember<'_>],
    guard: &Guard,
) -> Result<jiff::Timestamp, StorageError> {
    use sqlx::Row as _;
    let sql = set_statement(members.len());
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(head.id)
        .bind(head.ehr_id)
        .bind(guard.policy)
        .bind(guard.definitions)
        .bind(&guard.outbox)
        .bind(head.time_committed.to_string())
        .bind(head.audit.system_id)
        .bind(head.audit.change_type)
        .bind(&head.audit.description)
        .bind(&head.audit.committer)
        .bind(&head.audit.attestation);
    for member in members {
        let (a, v) = (&member.audit, &member.version);
        query = query
            .bind(a.system_id)
            .bind(a.change_type)
            .bind(&a.description)
            .bind(&a.committer)
            .bind(&a.attestation)
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
            .bind(serde_json::json!(v.origins))
            .bind(super::optional_json_array(v.other_input_version_uids))
            .bind(v.content_digest)
            .bind(v.content_sys_version)
            .bind(v.root_archetype)
            .bind(member.expected_head);
        query =
            crate::storage::node_write::bind_node_arrays(query, &v.rows.iter().collect::<Vec<_>>());
    }
    let row = query.fetch_one(&mut *conn).await?;
    let flags = GuardFlags {
        ehr: row.try_get("ehr_held")?,
        tip: row.try_get("tip_held")?,
        mark: row.try_get("mark_held")?,
        policy: row.try_get("policy_held")?,
        order: row.try_get("order_held")?,
        policy_now: row.try_get("policy_now")?,
        definitions_now: row.try_get("definitions_now")?,
    };
    if !flags.held() {
        return Err(StorageError::GuardMoved(flags));
    }
    if row
        .try_get::<Option<uuid::Uuid>, _>("contribution_id")?
        .is_none()
    {
        return Err(StorageError::ContributionUidInUse(Some(head.id)));
    }
    for member in members {
        let v = &member.version;
        crate::storage::blob_ref::record_uris(conn, v.vo_id, v.sys_version, v.blob_uris).await?;
    }
    Ok(row
        .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
        .to_jiff())
}
