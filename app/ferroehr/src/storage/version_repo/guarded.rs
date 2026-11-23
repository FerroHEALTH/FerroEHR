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
    /// The reservation of the audit records the write holds, inserted beside
    /// its rows so it commits with them or not at all (#5762); `None` when
    /// nothing is held.
    pub reservation: Option<uuid::Uuid>,
    /// The promoted `ehr` columns an `EHR_STATUS` commit writes beside its
    /// version (#6257); `None` for any other write.
    pub status: Option<Box<StatusSync>>,
    /// Whether the version is a new FOLDER hierarchy, which joins
    /// `EHR.folders` in the same statement (RM ehr master04 §Folders, #6257).
    pub folder: bool,
    /// What a party commit's statement takes and writes beside its version
    /// (#6257); `None` for a write in the clinical domain.
    pub party: Option<Box<PartySync>>,
}

/// What a party commit's guarded statement takes and writes beside its
/// version, in the party domain, which holds no `ehr` row and no policy
/// revision (#6257).
#[derive(Debug, Clone)]
pub struct PartySync {
    /// Whether the statement takes the commit side of the key-space fill
    /// gate, held to the end of the commit's transaction, before it writes.
    pub key_space: bool,
    /// The parties the version names as relationship targets, whose reverse
    /// index the statement brings to the version; `None` leaves the index to
    /// the caller.
    pub reverse: Option<Vec<uuid::Uuid>>,
    /// What the gate read found of the versions before the commit, which the
    /// binding hooks read in place of their own reads.
    pub prior: super::gate::Prior,
}

/// What a guarded statement ([`statement`]) writes beside its version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Beside {
    /// Nothing.
    Nothing,
    /// The promoted `ehr` columns of an `EHR_STATUS` commit ([`status_ctes`]).
    Status,
    /// The `ehr_folder` membership of a new FOLDER hierarchy ([`folder_cte`]).
    Folder,
    /// A party version, in the party domain ([`party_ctes`]).
    Party,
}

/// The promoted `ehr` columns of an `EHR_STATUS` commit, written by its
/// guarded statement, which then requires no modifiable EHR: the status is
/// what sets `is_modifiable` (RM ehr master04 §EHR Active Status).
#[derive(Clone)]
pub struct StatusSync {
    /// The subject's `external_ref` id.
    pub subject_id: Option<String>,
    /// The subject's `external_ref` namespace.
    pub subject_namespace: Option<String>,
    /// `EHR_STATUS.is_queryable`.
    pub is_queryable: bool,
    /// `EHR_STATUS.is_modifiable`.
    pub is_modifiable: bool,
    /// The digest of the subject reference the care projection matches.
    pub subject_digest: Option<Vec<u8>>,
}

impl std::fmt::Debug for StatusSync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusSync")
            .field("is_queryable", &self.is_queryable)
            .field("is_modifiable", &self.is_modifiable)
            .finish_non_exhaustive()
    }
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

    /// The flags off the last row of a guarded statement.
    ///
    /// # Errors
    /// [`StorageError::Database`] when a column does not decode.
    pub(super) fn read(row: &sqlx::postgres::PgRow) -> Result<Self, StorageError> {
        use sqlx::Row as _;
        Ok(Self {
            ehr: row.try_get("ehr_held")?,
            tip: row.try_get("tip_held")?,
            mark: row.try_get("mark_held")?,
            policy: row.try_get("policy_held")?,
            order: row.try_get("order_held")?,
            policy_now: row.try_get("policy_now")?,
            definitions_now: row.try_get("definitions_now")?,
        })
    }

    /// Nothing, when every requirement held.
    ///
    /// # Errors
    /// [`StorageError::GuardMoved`] naming the flags otherwise.
    pub(super) fn require(self) -> Result<(), StorageError> {
        if self.held() {
            Ok(())
        } else {
            Err(StorageError::GuardMoved(self))
        }
    }
}

/// What a guarded statement's leading CTEs read before its targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lead {
    /// The EHR row, read `FOR SHARE`, and the policy and definition revisions.
    Content,
    /// The EHR row, read `FOR UPDATE` by the `EHR_STATUS` commit that flips it,
    /// and the revisions.
    Status,
    /// Nothing of an EHR or the policy, which the party domain does not hold;
    /// the commit side of the key-space fill gate when `$gate` holds
    /// (`party_binding::key_space`).
    Party {
        /// The parameter that says whether the gate is taken.
        gate: usize,
    },
}

/// The leading CTEs of a guarded statement and its gate `g`: per `lead`, the
/// EHR row `ehr` read `FOR SHARE` (the lock a concurrent `EHR_STATUS` flip
/// waits on), or `FOR UPDATE` by the `status` commit that flips it, which
/// alone requires no modifiable EHR, or for a party the key-space gate;
/// each target's head row (`targets`, each object with the head it must
/// still have), the policy and definition revisions against `revisions`, and
/// every head no later than the commit instant `time`. `g` holds a row only
/// when every requirement holds; `k` names each one's flag.
fn guard_ctes(
    ehr: &str,
    targets: &[(String, String)],
    time: &str,
    (revisions, lead): ((&str, &str), Lead),
) -> String {
    use std::fmt::Write as _;
    let (mut sql, writable, revised, gated) = match lead {
        Lead::Party { gate } => (
            format!(
                "WITH ks AS MATERIALIZED ( \
                     SELECT pg_advisory_xact_lock_shared({}::bigint) AS taken WHERE ${gate}::bool \
                 )",
                crate::party_binding::key_space::GATE
            ),
            "true",
            false,
            " LEFT JOIN ks ON true",
        ),
        Lead::Content | Lead::Status => {
            let (lock, writable) = if lead == Lead::Status {
                (
                    "UPDATE",
                    "COALESCE((SELECT NOT e.restricted FROM e), false)",
                )
            } else {
                (
                    "SHARE",
                    "COALESCE((SELECT e.modifiable AND NOT e.restricted FROM e), false)",
                )
            };
            (
                format!(
                    "WITH e AS ( \
                         SELECT COALESCE(x.is_modifiable, true) AS modifiable, \
                                x.restricted_at IS NOT NULL AS restricted \
                         FROM ehr x WHERE x.id = {ehr} FOR {lock} \
                     ), p AS (SELECT rr.policy, rr.definitions FROM request_revisions rr)"
                ),
                writable,
                true,
                "",
            )
        }
    };
    let (mut tips, mut marks, mut orders) = (
        String::from("true"),
        String::from("false"),
        String::from("true"),
    );
    for (i, (vo, head)) in targets.iter().enumerate() {
        let _ = write!(
            sql,
            ", t{i} AS (SELECT h.head_sys_version, h.restricted_at IS NOT NULL AS restricted, \
             h.committed_at FROM vo_head h WHERE h.vo_id = {vo})"
        );
        let _ = write!(
            tips,
            " AND (SELECT t.head_sys_version FROM t{i} t) IS NOT DISTINCT FROM {head}::int"
        );
        let _ = write!(
            marks,
            " OR COALESCE((SELECT t.restricted FROM t{i} t), false)"
        );
        let _ = write!(
            orders,
            " AND COALESCE((SELECT t.committed_at <= {time}::timestamptz FROM t{i} t), true)"
        );
    }
    let (policy, definitions) = revisions;
    let (policy_held, policy_now, definitions_now) = if revised {
        (
            format!(
                "(SELECT p.policy FROM p) IS NOT DISTINCT FROM {policy}::bigint \
                 AND (SELECT p.definitions FROM p) IS NOT DISTINCT FROM {definitions}::bigint"
            ),
            "(SELECT p.policy FROM p)".to_owned(),
            "(SELECT p.definitions FROM p)".to_owned(),
        )
    } else {
        (
            "true".to_owned(),
            format!("{policy}::bigint"),
            format!("{definitions}::bigint"),
        )
    };
    let _ = write!(
        sql,
        ", k AS ( \
             SELECT {writable} AS ehr_held, \
                    ({tips}) AS tip_held, NOT ({marks}) AS mark_held, \
                    {policy_held} AS policy_held, \
                    ({orders}) AS order_held, \
                    {policy_now} AS policy_now, \
                    {definitions_now} AS definitions_now \
         ), g AS ( \
             SELECT 1 AS held FROM k{gated} \
             WHERE k.ehr_held AND k.tip_held AND k.mark_held AND k.policy_held AND k.order_held \
         )"
    );
    sql
}

/// The outbox CTE of a guarded statement: the event of CONTRIBUTION `id` in
/// EHR `ehr` with `envelope` at `time`, written once `from` wrote, under the
/// shared outbox lock and, when `marked` holds, unless the EHR carries a legal
/// mark, as [`super::commit::write_outbox`] writes it; the party domain holds
/// no `ehr` relation to read a mark from.
fn outbox_cte(
    id: &str,
    ehr: &str,
    (envelope, time): (&str, &str),
    from: &str,
    marked: bool,
) -> String {
    let unmarked = if marked {
        format!(
            " WHERE NOT EXISTS (SELECT 1 FROM ehr m WHERE m.id = {ehr} \
                 AND (m.restricted_at IS NOT NULL \
                      OR (m.research_objected_at IS NOT NULL \
                          AND m.research_objection_ground IS NULL)))"
        )
    } else {
        String::new()
    };
    format!(
        ", o AS ( \
             INSERT INTO event_outbox (contribution_id, ehr_id, envelope, committed_at) \
             SELECT {id}, {ehr}, {envelope}::jsonb, {time}::timestamptz \
             FROM (SELECT pg_advisory_xact_lock_shared( \
                       hashtext(current_schema() || '.event_outbox'), 0) \
                   FROM {from} WHERE {envelope}::jsonb IS NOT NULL) l{unmarked} \
         )"
    )
}

/// The reverse-relationship CTEs of a party version `vo` at `time`, from
/// `$targets`, as `party_reverse::replace` writes them, once `from` wrote: the
/// open rows of the targets the version no longer names are closed at `time`
/// (removed when they opened at it), and each named target without an open
/// row gets one. The two touch disjoint targets, so their order within the
/// statement does not matter; a `NULL` `$targets` writes nothing.
fn party_ctes(vo: &str, time: &str, targets: usize, from: &str) -> String {
    format!(
        ", rc AS ( \
             UPDATE party_relationship_target t SET valid_to = {time}::timestamptz FROM {from} \
             WHERE t.source_vo_id = {vo} AND t.valid_to IS NULL \
               AND t.valid_from < {time}::timestamptz \
               AND t.target_party_id <> ALL(${targets}::uuid[]) \
         ), re AS ( \
             DELETE FROM party_relationship_target t USING {from} \
             WHERE t.source_vo_id = {vo} AND t.valid_to IS NULL \
               AND t.valid_from >= {time}::timestamptz \
               AND t.target_party_id <> ALL(${targets}::uuid[]) \
         ), ri AS ( \
             INSERT INTO party_relationship_target (target_party_id, source_vo_id, valid_from) \
             SELECT DISTINCT x.target, {vo}, {time}::timestamptz \
             FROM {from}, unnest(${targets}::uuid[]) AS x(target) \
             ON CONFLICT (target_party_id, source_vo_id) WHERE valid_to IS NULL DO NOTHING \
         )"
    )
}

/// The reservation CTE of a guarded statement: `reservation`, the audit
/// records' reservation, inserted once the CONTRIBUTION is, when one is held.
fn reservation_cte(reservation: &str) -> String {
    format!(
        ", r AS ( \
             INSERT INTO audit_reservation (id) \
             SELECT {reservation} FROM c WHERE {reservation}::uuid IS NOT NULL \
         )"
    )
}

/// The last row of a guarded statement: each requirement's flag, the
/// revisions it read, the audit `audit` and CONTRIBUTION `c` it wrote, and
/// `extra` columns.
fn flags_row(audit: &str, extra: &str) -> String {
    format!(
        " SELECT k.ehr_held, k.tip_held, k.mark_held, k.policy_held, k.order_held, \
                k.policy_now, k.definitions_now, \
                {audit}.id AS commit_audit_id, {audit}.time_committed, c.id AS contribution_id{extra} \
         FROM k LEFT JOIN {audit} ON true LEFT JOIN c ON true"
    )
}

/// The `ehr` column CTEs of an `EHR_STATUS` commit in EHR `ehr`, from `$first`
/// (the five columns, then the notification channel), written once the
/// CONTRIBUTION is, as `ehr_repo::update_promoted_columns` writes them; a
/// moved subject, its namespace or its id, notifies every replica's access
/// copy (#5513, #6238).
fn status_ctes(ehr: &str, first: usize) -> String {
    format!(
        ", es AS ( \
             UPDATE ehr SET subject_id = ${}, subject_namespace = ${}, is_queryable = ${}, \
                            is_modifiable = ${}, subject_digest = ${} \
             FROM c WHERE ehr.id = {ehr} \
             RETURNING (OLD.subject_namespace, OLD.subject_id) \
                       IS DISTINCT FROM (NEW.subject_namespace, NEW.subject_id) AS moved \
         ), en AS (SELECT pg_notify(${}, {ehr}::text) FROM es WHERE es.moved)",
        first,
        first + 1,
        first + 2,
        first + 3,
        first + 4,
        first + 5
    )
}

/// The `ehr_folder` row of a new FOLDER hierarchy `vo` in EHR `ehr`, written
/// once its version is, at the next rank as
/// `commit::insert_ehr_folder_rank` writes it: two hierarchies that take one
/// rank at once are refused by `pk_ehr_folder`.
fn folder_cte(ehr: &str, vo: &str) -> String {
    format!(
        ", fm AS ( \
             INSERT INTO ehr_folder (ehr_id, rank, vo_id) \
             SELECT {ehr}, (SELECT COALESCE(MAX(f.rank), 0) + 1 FROM ehr_folder f \
                            WHERE f.ehr_id = {ehr}), {vo} FROM v \
         )"
    )
}

/// The audit insert of a guarded statement, `name`, of the audit at `first`
/// (five parameters) at `time`, written once `from` holds a row.
fn audit_cte(name: &str, first: usize, time: &str, from: &str) -> String {
    format!(
        "{name} AS ( \
             INSERT INTO commit_audit (system_id, change_type, description, committer, \
                                       attestation, time_committed) \
             SELECT ${}, ${}, ${}::jsonb, ${}::jsonb, ${}, {time}::timestamptz FROM {from} \
             RETURNING id, time_committed \
         )",
        first,
        first + 1,
        first + 2,
        first + 3,
        first + 4
    )
}

/// The statement text of [`super::commit::Folded::Guarded`].
///
/// The parameters of [`super::commit::Folded::Standalone`] up to `$27`, then
/// `$28` the expected head ordinal, `$29` the policy revision, `$30` the
/// definition revision, `$31` the outbox envelope and `$32` the audit
/// reservation, and the node arrays from `$33`, then for a `status` commit
/// the columns of [`status_ctes`], and for a party commit the key-space gate
/// flag and the reverse targets of [`party_ctes`]; `beside` names what else
/// it writes. The
/// audit insert reads from `g`, and every later insert from the audit's, so
/// nothing is written unless every requirement holds; the last row names a
/// flag per requirement.
pub(super) fn statement(beside: Beside) -> String {
    let after_nodes = 33 + crate::storage::node_write::node_array_count();
    let (sync, notified) = match beside {
        Beside::Status => (
            status_ctes("$7", after_nodes),
            ", (SELECT count(*) FROM en) AS notified",
        ),
        Beside::Folder => (folder_cte("$7", "$8"), ""),
        Beside::Party => (party_ctes("$8", "$22", after_nodes + 1, "v"), ""),
        Beside::Nothing => (String::new(), ""),
    };
    let lead = match beside {
        Beside::Status => Lead::Status,
        Beside::Party => Lead::Party { gate: after_nodes },
        Beside::Folder | Beside::Nothing => Lead::Content,
    };
    format!(
        "{}, {}, c AS ( \
             INSERT INTO contribution (id, ehr_id, commit_audit_id) \
             SELECT $6, $7, a.id FROM a ON CONFLICT (id) DO NOTHING RETURNING id \
         ){}{}{}{}{}",
        guard_ctes(
            "$7",
            &[("$8".to_owned(), "$28".to_owned())],
            "$22",
            (("$29", "$30"), lead)
        ),
        audit_cte("a", 1, "$22", "g"),
        super::commit::version_ctes("", ("$7", "c.id", "a.id", "a, c"), 8, 33),
        sync,
        outbox_cte("$6", "$7", ("$31", "$22"), "v", beside != Beside::Party),
        reservation_cte("$32"),
        flags_row("a", notified)
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
/// twenty of its version and its expected head.
const MEMBER_SCALARS: usize = 26;

/// The statement text of a guarded CONTRIBUTION of `n` members.
///
/// `$1` the CONTRIBUTION uid, `$2` the EHR, `$3` the policy revision, `$4`
/// the definition revision, `$5` the outbox envelope, `$6` the commit instant
/// `$7` to `$11` the CONTRIBUTION's audit and `$12` the audit reservation;
/// then each member's audit,
/// version, expected head and node arrays in turn. Each member repeats the
/// single write's audit, version, head-upsert and node CTEs under its own
/// names.
fn set_statement(n: usize) -> String {
    use std::fmt::Write as _;
    let stride = MEMBER_SCALARS + crate::storage::node_write::node_array_count();
    let base = |i: usize| 13 + i * stride;
    let targets: Vec<(String, String)> = (0..n)
        .map(|i| (format!("${}", base(i) + 5), format!("${}", base(i) + 25)))
        .collect();
    let mut sql = format!(
        "{}, {}, c AS ( \
             INSERT INTO contribution (id, ehr_id, commit_audit_id) \
             SELECT $1, $2, ca.id FROM ca ON CONFLICT (id) DO NOTHING RETURNING id \
         )",
        guard_ctes("$2", &targets, "$6", (("$3", "$4"), Lead::Content)),
        audit_cte("ca", 7, "$6", "g"),
    );
    for i in 0..n {
        let b = base(i);
        let _ = write!(
            sql,
            ", {}{}",
            audit_cte(&format!("a{i}"), b, "$6", "c"),
            super::commit::version_ctes(
                &i.to_string(),
                ("$2", "c.id", &format!("a{i}.id"), &format!("a{i}, c")),
                b + 5,
                b + MEMBER_SCALARS,
            )
        );
    }
    let _ = write!(
        sql,
        "{}{}{}",
        outbox_cte("$1", "$2", ("$5", "$6"), "c", true),
        reservation_cte("$12"),
        flags_row("ca", "")
    );
    sql
}

/// Write a CONTRIBUTION and every one of its `members` in one guarded
/// statement (`set_statement`).
///
/// Nothing is written unless the EHR is still modifiable and unrestricted,
/// every member's head and mark are unchanged, the revisions are those judged
/// on and no head is later than the set's instant. The members' blob
/// references follow in the caller's transaction.
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
        .bind(&head.audit.attestation)
        .bind(guard.reservation);
    for member in members {
        let a = &member.audit;
        query = query
            .bind(a.system_id)
            .bind(a.change_type)
            .bind(&a.description)
            .bind(&a.committer)
            .bind(&a.attestation);
        query = super::commit::bind_version(query, &member.version).bind(member.expected_head);
        query = crate::storage::node_write::bind_node_arrays(
            query,
            &member.version.rows.iter().collect::<Vec<_>>(),
        );
    }
    let row = query.fetch_one(&mut *conn).await?;
    GuardFlags::read(&row)?.require()?;
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
