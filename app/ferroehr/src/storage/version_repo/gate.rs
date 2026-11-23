// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The gate read of a composition write (#6256): one statement that returns
//! what every pre-check of the write is judged on.
//!
//! The row carries the EHR's existence, its `is_modifiable` flag and
//! restriction mark with the revision of its access settings and the policy
//! and definition revisions (`access_revision!`), the database `now()`, and
//! for each target object its current trunk tip with the next storage
//! ordinal and its own restriction mark. An update reads beside them the
//! stored template and the first content version's root fields, a delete the
//! content identity of the version it deletes, and a create of a persistent
//! COMPOSITION whether a live one of its template exists. Leading
//! data-modifying CTEs bring an archived target back to the hot tier first,
//! as the placement read does. No openEHR spec governs the SQL: our own
//! design; the guarded concepts are RM ehr master04 §EHR Creation and §EHR
//! Active Status and RM common master06 §The 'Virtual Version Tree'.

use std::sync::LazyLock;

use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::storage::ehr_repo::Restriction;
use crate::storage::error::StorageError;
use crate::storage::version_repo::content::ServedContent;
use crate::storage::version_repo::placement::{Placement, TipRow};

/// What a write reads beside each target's tip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// The tip alone: a create, or a write that judges no stored content.
    Plain,
    /// The stored template and the first content version's root fields,
    /// which a COMPOSITION update is judged on.
    Update,
    /// The content identity of the tip, which a logical delete's access
    /// record is classified by (EHDS Annex II 3.2(c)).
    Delete,
    /// Both, for the members of a CONTRIBUTION that update and delete.
    Members,
    /// The tip alone of the EHR's `EHR_STATUS`, found by its kind in the EHR
    /// in place of [`Ask::targets`] (#6257).
    Status,
    /// The tip of the EHR's directory, found by its slot in place of
    /// [`Ask::targets`], which of [`Ask::items`] exist in the EHR (#6257),
    /// and the highest rank in `EHR.folders` (#6318).
    Directory,
}

impl Shape {
    /// Whether the read carries the update facts.
    fn updates(self) -> bool {
        matches!(self, Self::Update | Self::Members)
    }

    /// Whether the read carries the delete facts.
    fn deletes(self) -> bool {
        matches!(self, Self::Delete | Self::Members)
    }
}

/// One gate read: the EHR, the targets and what to read beside them.
#[derive(Debug, Clone, Copy)]
pub struct Ask<'a> {
    /// The EHR the write addresses.
    pub ehr_id: EhrId,
    /// The objects the write modifies or deletes, in the order their facts
    /// come back; empty for a create.
    pub targets: &'a [VoId],
    /// What to read beside each target's tip.
    pub shape: Shape,
    /// The persistent COMPOSITION a create commits, whose live sibling of
    /// the same template the create is refused by; `None` reads no sibling.
    pub persistent: Option<Persistent<'a>>,
    /// The `(trunk_version, branch_number, branch_version)` of the version a
    /// delete addresses, whose template its SMART scope is judged on
    /// ([`Shape::Delete`]); `None` reads none.
    pub addressed: Option<(i32, i32, i32)>,
    /// The versioned objects a FOLDER's local `items` reference, which
    /// [`Shape::Directory`] reads the existing ones of.
    pub items: &'a [Uuid],
}

/// What names a live persistent sibling of a created COMPOSITION.
#[derive(Debug, Clone, Copy)]
pub struct Persistent<'a> {
    /// The template the create commits against.
    pub template_id: &'a str,
    /// The `431|persistent|` category code.
    pub category_code: &'a str,
    /// The `523|deleted|` lifecycle state, which no live sibling carries.
    pub deleted_state: &'a str,
}

/// The EHR and the targets as one gate read found them.
#[derive(Debug)]
pub struct Gate {
    /// The promoted `is_modifiable`; `None` exactly when the EHR does not
    /// exist.
    pub is_modifiable: Option<bool>,
    /// Whether the EHR is restricted, with its settings revision; `None` when
    /// it does not exist.
    pub restriction: Restriction,
    /// The database `now()` of the statement, the commit instant of the
    /// write.
    pub now: jiff::Timestamp,
    /// The policy revision the statement read.
    pub policy: Option<i64>,
    /// The definition revision the statement read.
    pub definitions: Option<i64>,
    /// Whether a live persistent sibling [`Ask::persistent`] names exists in
    /// the EHR.
    pub persistent_exists: bool,
    /// Which of [`Ask::items`] exist as versioned objects in the EHR, in any
    /// tier and lifecycle state ([`Shape::Directory`]).
    pub known_items: Vec<Uuid>,
    /// The highest rank of the EHR's hierarchies in `EHR.folders`, `0` when
    /// it has none ([`Shape::Directory`]); a new hierarchy takes the next,
    /// so one another write took first is refused by its key.
    pub folder_rank: i32,
    /// The targets, in the order asked.
    pub targets: Vec<Target>,
}

/// One target object as the gate read found it.
#[derive(Debug)]
pub struct Target {
    /// The object.
    pub vo_id: VoId,
    /// Its current trunk tip, the next storage ordinal and the statement's
    /// `now()`.
    pub placement: Placement,
    /// Whether the object carries a restriction mark of its own.
    pub restricted: bool,
    /// The head ordinal of any lineage its head row records, which a guarded
    /// commit requires unchanged; `None` when the object has no head row.
    pub head: Option<i32>,
    /// The tip body's template id ([`Shape::Update`]).
    pub stored_template: Option<String>,
    /// The first content version's `(archetype_node_id, category code)`
    /// ([`Shape::Update`]); `None` when no content version exists.
    pub first_root: Option<(Option<String>, Option<String>)>,
    /// The tip's content identity ([`Shape::Delete`]); `None` for a tip that
    /// holds no content.
    pub content: Option<ServedContent>,
    /// The template of the version [`Ask::addressed`] names, in whichever
    /// EHR it lives ([`Shape::Delete`]); `None` when no such version exists
    /// or it names no template.
    pub addressed_template: Option<String>,
}

/// The statement text of `shape`.
///
/// `$1` the EHR, `$2` the targets, `$3` the persistent template, `$4` the
/// persistent category code, `$5` the deleted lifecycle state, `$6` to
/// `$8` the tree id of the version a delete addresses and `$9` the items a
/// FOLDER references.
fn statement(shape: Shape) -> String {
    let update = if shape.updates() {
        concat!(
            ", tb.stored_template, fv.found AS first_found, fv.ani AS first_ani, ",
            "fv.category AS first_category"
        )
    } else {
        ""
    };
    let update_join = if shape.updates() {
        concat!(
            " LEFT JOIN LATERAL ( ",
            "    SELECT (b.body)::jsonb #>> '{archetype_details,template_id,value}' ",
            "           AS stored_template ",
            "    FROM version b WHERE b.vo_id = t.vo_id AND b.sys_version = tip.sys_version ",
            "    LIMIT 1 ",
            ") tb ON true ",
            // The ORDER BY and LIMIT sit inside the subquery and the jsonb
            // extractions outside it, so they run for one row only.
            "LEFT JOIN LATERAL ( ",
            "    SELECT true AS found, (b.body)::jsonb ->> 'archetype_node_id' AS ani, ",
            "           (b.body)::jsonb #>> '{category,defining_code,code_string}' AS category ",
            "    FROM (SELECT f.body FROM version f ",
            "          WHERE f.vo_id = t.vo_id AND f.body IS NOT NULL ",
            "          ORDER BY f.sys_version LIMIT 1) b ",
            ") fv ON true"
        )
    } else {
        ""
    };
    let delete = if shape.deletes() {
        ", tip.has_body, tip.template_id AS content_template, dc.archetype AS content_archetype"
    } else {
        ""
    };
    let delete_join = if shape.deletes() {
        concat!(
            " LEFT JOIN LATERAL ( ",
            "    SELECT COALESCE(n.data->>'archetype_node_id', ",
            "                    ext.node_archetype_node_id(n.archetype, n.rm_version)) AS archetype ",
            "    FROM node n WHERE n.tier = tip.tier AND n.vo_id = t.vo_id ",
            "      AND n.sys_version = tip.content_key AND n.num = 0 ",
            ") dc ON tip.has_body"
        )
    } else {
        ""
    };
    format!(
        concat!(
            "{warm} ",
            "SELECT e.is_modifiable, e.restricted_at IS NOT NULL AS restricted, ",
            access_revision!("e.id"),
            ", now() AS ts, ",
            "$3::text IS NOT NULL AND EXISTS (SELECT 1 FROM version pv ",
            "    JOIN vo_head ph ON ph.vo_id = pv.vo_id ",
            "                   AND ph.trunk_head_sys_version = pv.sys_version ",
            "    WHERE pv.ehr_id = $1 AND pv.kind = 'COMPOSITION' AND pv.lifecycle_state <> $5 ",
            "      AND pv.template_id = $3 ",
            "      AND (pv.body)::jsonb #>> '{{category,defining_code,code_string}}' = $4) ",
            "    AS persistent_exists, ",
            "{tip_columns}{}{}{}{} ",
            "FROM (VALUES (1)) AS one (x) ",
            "LEFT JOIN ehr e ON e.id = $1 ",
            "{tip_join}{}{} ",
            "ORDER BY t.ord"
        ),
        update,
        delete,
        addressed(shape),
        known_items(shape),
        update_join,
        delete_join,
        warm = warm(targets(shape)),
        tip_columns = TIP_COLUMNS,
        tip_join = tip_join(targets(shape)),
    )
}

/// The leading data-modifying CTEs of a gate read that bring an archived
/// target of `targets` back to the hot tier first, as the placement read does.
fn warm(targets: &str) -> String {
    format!(
        "WITH tv AS (UPDATE version SET tier = 'hot' \
                     WHERE vo_id = ANY({targets}) AND tier = 'cold'), \
         tn AS (UPDATE node SET tier = 'hot' WHERE tier = 'cold' AND vo_id = ANY({targets})), \
         th AS (UPDATE vo_head SET tier = 'hot', archived_at = NULL, archive_reason = NULL \
                WHERE vo_id = ANY({targets}) AND tier = 'cold')"
    )
}

/// The columns of a gate read's target `t` and its trunk tip `tip`.
const TIP_COLUMNS: &str = concat!(
    "t.vo_id AS target, ",
    "(SELECT (COALESCE(MAX(s.sys_version), 0) + 1)::int FROM version s ",
    " WHERE s.vo_id = t.vo_id) AS next_ordinal, ",
    "COALESCE((SELECT h.restricted_at IS NOT NULL FROM vo_head h WHERE h.vo_id = t.vo_id), ",
    "         false) AS object_restricted, ",
    "(SELECT h.head_sys_version FROM vo_head h WHERE h.vo_id = t.vo_id) AS head, ",
    "tip.ehr_id, tip.kind, tip.sys_version, tip.trunk_version, tip.branch_number, ",
    "tip.branch_version, tip.creating_system_id, tip.lifecycle_state, ",
    "tip.committed_at AS tip_committed_at, tip.content_digest, tip.content_key, ",
    "true AS open"
);

/// The joins of a gate read's targets `t` from `targets` and the trunk tip
/// `tip` of each.
fn tip_join(targets: &str) -> String {
    format!(
        concat!(
            "LEFT JOIN LATERAL unnest({}) WITH ORDINALITY AS t (vo_id, ord) ON true ",
            // The trunk tip is the newest ordinal on branch 0, which no later
            // ordinal of its own branch supersedes, so it is open by definition.
            "LEFT JOIN LATERAL ( ",
            "    SELECT v.ehr_id, v.kind, v.sys_version, v.trunk_version, v.branch_number, ",
            "           v.branch_version, v.creating_system_id, v.lifecycle_state, ",
            "           v.committed_at, v.content_digest, v.content_key, v.template_id, v.tier, ",
            "           v.body IS NOT NULL AS has_body ",
            "    FROM version v WHERE v.vo_id = t.vo_id AND v.branch_number = 0 ",
            "    ORDER BY v.sys_version DESC LIMIT 1 ",
            ") tip ON true"
        ),
        targets
    )
}

/// The template of the version a delete addresses ([`Shape::Delete`]), which
/// the SMART pre-check read on its own before (#6271): by its tree id and in
/// no EHR, as `meta::template_id_of` reads it.
fn addressed(shape: Shape) -> &'static str {
    if shape == Shape::Delete {
        concat!(
            ", (SELECT a.template_id FROM version a WHERE a.vo_id = t.vo_id ",
            "   AND a.trunk_version = $6 AND a.branch_number = $7 AND a.branch_version = $8) ",
            "  AS addressed_template"
        )
    } else {
        ""
    }
}

/// Which of `$9` exist as versioned objects in the EHR, as
/// `ehr_repo::existing_vo_roots` reads them, and the EHR's highest folder
/// rank ([`Shape::Directory`]).
fn known_items(shape: Shape) -> &'static str {
    if shape == Shape::Directory {
        concat!(
            ", ARRAY(SELECT DISTINCT kv.vo_id FROM version kv ",
            "  WHERE kv.ehr_id = $1 AND kv.vo_id = ANY($9::uuid[])) AS known_items, ",
            "(SELECT COALESCE(MAX(fr.rank), 0) FROM ehr_folder fr WHERE fr.ehr_id = $1) ",
            "  AS folder_rank"
        )
    } else {
        ""
    }
}

/// The targets of `shape`'s statement: `$2`, the EHR's current `EHR_STATUS`
/// found by its kind ([`Shape::Status`]), as `meta::current_vo` finds it, or
/// its directory found by its slot ([`Shape::Directory`]), the lowest-rank
/// live hierarchy, else the lowest-rank one, as `ehr_repo::directory_vo`
/// finds it.
fn targets(shape: Shape) -> &'static str {
    match shape {
        Shape::Status => concat!(
            "ARRAY(SELECT sv.vo_id FROM version sv JOIN vo_head sh ON sh.vo_id = sv.vo_id ",
            "AND sh.trunk_head_sys_version = sv.sys_version ",
            "WHERE sv.ehr_id = $1 AND sv.kind = 'EHR_STATUS')"
        ),
        Shape::Directory => concat!(
            "ARRAY(SELECT df.vo_id FROM ehr_folder df ",
            "JOIN version dv ON dv.vo_id = df.vo_id ",
            "JOIN vo_head dh ON dh.vo_id = dv.vo_id AND dh.trunk_head_sys_version = dv.sys_version ",
            "WHERE df.ehr_id = $1 ORDER BY (dv.lifecycle_state = '523'), df.rank LIMIT 1)"
        ),
        _ => "$2::uuid[]",
    }
}

/// Runs one gate read ([`Ask`]) on `conn`.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn read(conn: &mut PgConnection, ask: Ask<'_>) -> Result<Gate, StorageError> {
    static PLAIN: LazyLock<String> = LazyLock::new(|| statement(Shape::Plain));
    static UPDATE: LazyLock<String> = LazyLock::new(|| statement(Shape::Update));
    static DELETE: LazyLock<String> = LazyLock::new(|| statement(Shape::Delete));
    static MEMBERS: LazyLock<String> = LazyLock::new(|| statement(Shape::Members));
    static STATUS: LazyLock<String> = LazyLock::new(|| statement(Shape::Status));
    static DIRECTORY: LazyLock<String> = LazyLock::new(|| statement(Shape::Directory));
    let sql = match ask.shape {
        Shape::Plain => &PLAIN,
        Shape::Update => &UPDATE,
        Shape::Delete => &DELETE,
        Shape::Members => &MEMBERS,
        Shape::Status => &STATUS,
        Shape::Directory => &DIRECTORY,
    };
    let targets: Vec<Uuid> = ask.targets.iter().map(|vo_id| vo_id.0).collect();
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(ask.ehr_id)
        .bind(&targets)
        .bind(ask.persistent.map(|p| p.template_id))
        .bind(ask.persistent.map(|p| p.category_code))
        .bind(ask.persistent.map(|p| p.deleted_state))
        .bind(ask.addressed.map(|(trunk, _, _)| trunk))
        .bind(ask.addressed.map(|(_, branch, _)| branch))
        .bind(ask.addressed.map(|(_, _, version)| version))
        .bind(ask.items)
        .fetch_all(&mut *conn)
        .await?;
    let first = rows
        .first()
        .ok_or_else(|| StorageError::InvalidRows("the gate read returned no row".to_owned()))?;
    let is_modifiable: Option<bool> = first.try_get("is_modifiable")?;
    let restriction = match first.try_get::<Option<bool>, _>("restricted")? {
        Some(restricted) if is_modifiable.is_some() => Some((
            restricted,
            crate::storage::access_projection::Revision::read(first)?,
        )),
        _ => None,
    };
    let now = first.try_get::<jiff_sqlx::Timestamp, _>("ts")?.to_jiff();
    let mut found = Vec::with_capacity(rows.len());
    for row in &rows {
        let Some(vo_id) = row.try_get::<Option<Uuid>, _>("target")? else {
            continue;
        };
        found.push(target(row, VoId(vo_id), now, ask.shape)?);
    }
    Ok(Gate {
        is_modifiable,
        restriction,
        now,
        policy: first.try_get("policy_revision")?,
        definitions: first.try_get("definition_revision")?,
        persistent_exists: first.try_get("persistent_exists")?,
        known_items: if ask.shape == Shape::Directory {
            first.try_get("known_items")?
        } else {
            Vec::new()
        },
        folder_rank: if ask.shape == Shape::Directory {
            first.try_get("folder_rank")?
        } else {
            0
        },
        targets: found,
    })
}

/// One target's facts off its row of a gate read.
fn target(
    row: &sqlx::postgres::PgRow,
    vo_id: VoId,
    now: jiff::Timestamp,
    shape: Shape,
) -> Result<Target, StorageError> {
    let tip = crate::storage::version_repo::placement::tip_from_row(row)?;
    let (stored_template, first_root) = if shape.updates() {
        let first_root = match row.try_get::<Option<bool>, _>("first_found")? {
            Some(true) => Some((row.try_get("first_ani")?, row.try_get("first_category")?)),
            _ => None,
        };
        (row.try_get("stored_template")?, first_root)
    } else {
        (None, None)
    };
    let content = match (&tip, shape) {
        (Some(tip), shape)
            if shape.deletes() && row.try_get::<Option<bool>, _>("has_body")? == Some(true) =>
        {
            Some(content_of(tip, row)?)
        }
        _ => None,
    };
    let addressed_template = if shape == Shape::Delete {
        row.try_get("addressed_template")?
    } else {
        None
    };
    Ok(Target {
        vo_id,
        placement: Placement {
            tip,
            next_ordinal: row.try_get("next_ordinal")?,
            now,
        },
        restricted: row.try_get("object_restricted")?,
        head: row.try_get("head")?,
        stored_template,
        first_root,
        content,
        addressed_template,
    })
}

/// The content identity of `tip` off its row of a delete's gate read.
fn content_of(tip: &TipRow, row: &sqlx::postgres::PgRow) -> Result<ServedContent, StorageError> {
    Ok(ServedContent {
        ehr_id: tip.ehr_id,
        kind: tip.kind.clone(),
        template_id: row.try_get("content_template")?,
        root_archetype: row.try_get("content_archetype")?,
    })
}

/// One party write's gate read, on the party pool (#6257): the database
/// `now()` of the statement, the commit instant of the write, and the target
/// party as a party commit judges it.
#[derive(Debug)]
pub struct PartyGate {
    /// The database `now()` of the statement.
    pub now: jiff::Timestamp,
    /// The target, `None` for a create.
    pub target: Option<PartyTarget>,
}

/// The target of a party write as its gate read found it.
#[derive(Debug)]
pub struct PartyTarget {
    /// Its tip, next ordinal, head and mark, as a composition write's target.
    pub target: Target,
    /// Whether the newest version, on any branch, holds a body.
    pub newest_held: bool,
    /// Its head row's lifecycle state.
    pub head_lifecycle: Option<String>,
    /// Whether its head row carries the policy-party mark
    /// (`party_binding::policy_mark`).
    pub policy_marked: bool,
    /// The newest version that holds a body: its ordinal, its lifecycle state
    /// and the stored body; `None` when no version holds one.
    pub last_body: Option<(i32, String, String)>,
}

/// What a party commit's binding hooks read of the versions before it, off its
/// gate read (#6257), so they read none of them again.
#[derive(Debug, Clone, Default)]
pub struct Prior {
    /// Whether the newest version before the commit holds a body.
    pub previous_held: bool,
    /// Whether the party carried the policy-party mark before the commit.
    pub marked: bool,
    /// The newest version holding a body before the commit: its ordinal, its
    /// lifecycle state and the stored body.
    pub last_body: Option<(i32, String, String)>,
}

impl PartyGate {
    /// The target, and what the binding hooks read of the versions before
    /// the commit: none for a create.
    #[must_use]
    pub fn split(self) -> (Option<Target>, Prior) {
        match self.target {
            None => (None, Prior::default()),
            Some(found) => (
                Some(found.target),
                Prior {
                    previous_held: found.newest_held,
                    marked: found.policy_marked,
                    last_body: found.last_body,
                },
            ),
        }
    }
}

/// The statement of [`party`]: `$1` the target, `NULL` for a create.
static PARTY: LazyLock<String> = LazyLock::new(|| {
    format!(
        concat!(
            "{warm} SELECT now() AS ts, {tip_columns}, ",
            "(SELECT n.body IS NOT NULL FROM version n WHERE n.vo_id = t.vo_id ",
            " ORDER BY n.sys_version DESC LIMIT 1) AS newest_held, ",
            "(SELECT h.lifecycle_state FROM vo_head h WHERE h.vo_id = t.vo_id) AS head_lifecycle, ",
            "COALESCE((SELECT h.policy_party_at IS NOT NULL FROM vo_head h ",
            "          WHERE h.vo_id = t.vo_id), false) AS policy_marked, ",
            "lb.sys_version AS last_body_version, lb.lifecycle_state AS last_body_lifecycle, ",
            "lb.body AS last_body ",
            "FROM (VALUES (1)) AS one (x) {tip_join} ",
            "LEFT JOIN LATERAL ( ",
            "    SELECT b.sys_version, b.lifecycle_state, b.body FROM version b ",
            "    WHERE b.vo_id = t.vo_id AND b.body IS NOT NULL ",
            "    ORDER BY b.sys_version DESC LIMIT 1 ",
            ") lb ON true"
        ),
        warm = warm(PARTY_TARGETS),
        tip_columns = TIP_COLUMNS,
        tip_join = tip_join(PARTY_TARGETS),
    )
});

/// The targets of [`PARTY`]: the one target, none for a create.
const PARTY_TARGETS: &str = "ARRAY_REMOVE(ARRAY[$1::uuid], NULL)";

/// Runs the gate read of a party write of `target` on `conn` ([`PartyGate`]).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn party(
    conn: &mut PgConnection,
    target: Option<VoId>,
) -> Result<PartyGate, StorageError> {
    let row = sqlx::query(sqlx::AssertSqlSafe(PARTY.as_str()))
        .bind(target)
        .fetch_one(&mut *conn)
        .await?;
    let now = row.try_get::<jiff_sqlx::Timestamp, _>("ts")?.to_jiff();
    let target = match row.try_get::<Option<Uuid>, _>("target")? {
        None => None,
        Some(vo_id) => {
            let last_body = match row.try_get::<Option<i32>, _>("last_body_version")? {
                Some(version) => Some((
                    version,
                    row.try_get("last_body_lifecycle")?,
                    row.try_get("last_body")?,
                )),
                None => None,
            };
            Some(PartyTarget {
                target: self::target(&row, VoId(vo_id), now, Shape::Plain)?,
                newest_held: row
                    .try_get::<Option<bool>, _>("newest_held")?
                    .unwrap_or(false),
                head_lifecycle: row.try_get("head_lifecycle")?,
                policy_marked: row.try_get("policy_marked")?,
                last_body,
            })
        }
    };
    Ok(PartyGate { now, target })
}
