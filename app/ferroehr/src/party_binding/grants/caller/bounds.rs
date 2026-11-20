// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's bounds on a care relationship, judged in its commit's
//! transaction: its validity, its target and its writer
//! ([`crate::access_policy::relationship`]).
//!
//! The policy is the version the request confirmed on its one indexed read
//! of the policy heads before the commit took its party connection: the
//! policy is held by the clinical domain, which no party role reaches, and no
//! connection of one pool is held while waiting for another. A relationship
//! written without an end takes the end the covering entry gives, kept in
//! `binding_care_validity` with the version uid of the relationship and of
//! that policy version, and its caller fact takes it as its upper bound. A
//! policy version committed between that read and this commit applies to the
//! writes that read it; the stored row names the version applied. Our own
//! design over the draft access control decision procedure, Writing
//! Relationships.

use openehr_rm::draft::access_control::terminology::{Confidentiality, ForeignCode};
use sqlx::PgConnection;
use uuid::Uuid;

use super::{FactValue, care_of};
use crate::access_policy::PolicyInForce;
use crate::access_policy::relationship::{self, SelfDeclaredRefusal, WriterVerdict, Written};
use crate::error::ServiceError;
use crate::ids::VoId;
use crate::party_binding::derive::Facts;
use crate::storage::error::StorageError;
use crate::versioning::change::Committed;
use crate::versioning::object_version_id::{TreeId, object_version_id};

/// The path a validity refusal names.
const VALIDITY: &str = "/time_validity";

/// The path a target refusal names.
const TARGET: &str = "/target";

/// The path a self-declared refusal names.
const BASIS: &str = "/details";

/// The trunk head version of a relationship a bound is judged for.
#[derive(Debug)]
pub(crate) struct Head {
    /// The relationship's container.
    vo_id: Uuid,
    /// The head's `OBJECT_VERSION_ID`.
    version_uid: String,
    /// The date the head was committed here, which a relationship naming no
    /// start counts from.
    committed_on: jiff::civil::Date,
}

impl From<&Committed> for Head {
    fn from(committed: &Committed) -> Self {
        Self {
            vo_id: committed.vo_id.0,
            version_uid: object_version_id(
                committed.vo_id,
                &committed.creating_system_id,
                committed.tree,
            ),
            committed_on: committed
                .time_committed
                .to_zoned(jiff::tz::TimeZone::UTC)
                .date(),
        }
    }
}

/// Judges the care relationship the stored trunk head of `vo_id` holds under
/// `facts`, as an import or an archive load lands it, against `policy`, and
/// writes the end it takes, on `tx`. An end already kept for that very
/// version stays, so a load that leaves the head as it was moves nothing.
///
/// # Errors
/// The refusals of [`judge`], and a storage failure.
pub(crate) async fn judge_stored(
    tx: &mut PgConnection,
    policy: &PolicyInForce,
    vo_id: Uuid,
    facts: Option<&Facts>,
) -> Result<(), ServiceError> {
    let row: Option<(String, i32, i32, i32, jiff_sqlx::Timestamp, Option<String>)> =
        sqlx::query_as(
            "SELECT v.creating_system_id, v.trunk_version, v.branch_number, v.branch_version, \
                    v.committed_at, c.version_uid \
             FROM vo_head h \
             JOIN version v ON v.tier = h.tier AND v.vo_id = h.vo_id \
                           AND v.sys_version = h.trunk_head_sys_version \
             LEFT JOIN binding_care_validity c ON c.vo_id = h.vo_id \
             WHERE h.vo_id = $1",
        )
        .bind(vo_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(StorageError::from)?;
    let Some((system, trunk, branch, branch_version, committed_at, kept)) = row else {
        return Ok(());
    };
    let tree = TreeId::from_columns(trunk, branch, branch_version);
    let head = Head {
        vo_id,
        version_uid: object_version_id(VoId(vo_id), &system, tree),
        committed_on: committed_at
            .to_jiff()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .date(),
    };
    let changed = kept.as_deref() != Some(head.version_uid.as_str());
    judge(tx, policy, &head, facts, changed).await
}

/// Judges the care relationship `head` holds under `facts` against `policy`
/// when `changed`, its care content differing from the version before, and
/// writes the end it takes; drops the end of a head that holds no care
/// relationship. A head whose care content did not change keeps the end it
/// took, on `tx`.
///
/// # Errors
/// `422` for a validity or a target the policy refuses, `403` for a writer it
/// does not name, and a storage failure.
pub(crate) async fn judge(
    tx: &mut PgConnection,
    policy: &PolicyInForce,
    head: &Head,
    facts: Option<&Facts>,
    changed: bool,
) -> Result<(), ServiceError> {
    let care = facts.and_then(|facts| {
        facts
            .grants
            .facts
            .iter()
            .find_map(|fact| match &fact.value {
                FactValue::Care(care) => Some((care, fact.validity.as_ref())),
                FactValue::Code(_) | FactValue::Team(_) => None,
            })
    });
    let Some((care, validity)) = care else {
        drop_end(tx, head.vo_id).await?;
        return Ok(());
    };
    if !changed {
        return Ok(());
    }
    let rules = policy.policy.as_deref();
    let provider = provider(tx, &care.provider).await?;
    relationship::target(rules, care.kind, care.carer, provider.party)
        .map_err(|refusal| refused(TARGET, &refusal))?;
    // TODO(#6057): judge the writer at the record's marking, and log an admitted write for review.
    if relationship::writer(rules, Confidentiality::Normal, |_| false) == WriterVerdict::Refused {
        return Err(ServiceError::not_permitted(
            "no writer the access policy's relationship_writers names writes this care \
             relationship",
        ));
    }
    let Some((lower, upper)) = dates(tx, validity).await? else {
        // NOTE: our own design, a validity that does not read holds no date, so it takes no end.
        drop_end(tx, head.vo_id).await?;
        return Ok(());
    };
    let written = Written {
        role: &care.role,
        provider_types: &provider.types,
        lower,
        upper,
        today: head.committed_on,
    };
    let end = relationship::effective_end(rules, &written)
        .map_err(|refusal| refused(VALIDITY, &refusal))?;
    if care.self_declared {
        let writer = writer_of(tx, head.vo_id).await?;
        let target = (care.kind, care.carer);
        relationship::self_declared(rules, &written, end.or(upper), target, writer).map_err(
            |refusal| match refusal {
                SelfDeclaredRefusal::NotTheWriter => {
                    ServiceError::not_permitted(format!("an under_care_of relationship: {refusal}"))
                }
                _ => refused(BASIS, &refusal),
            },
        )?;
    }
    match (end, policy.uid.as_deref()) {
        (Some(until), Some(policy_version)) => {
            keep_end(tx, head.vo_id, &head.version_uid, until, policy_version).await?;
        }
        _ => drop_end(tx, head.vo_id).await?,
    }
    Ok(())
}

/// A `422` naming `path` with `refusal`.
fn refused(path: &str, refusal: &dyn std::fmt::Display) -> ServiceError {
    ServiceError::ValidationFailed(vec![openehr_base::validate::InvariantViolation::at(
        path,
        format!("an under_care_of relationship: {refusal}"),
    )])
}

/// The provider of a care relationship as the party store holds it: its
/// `ORGANISATION`'s container and the types it names for itself.
#[derive(Debug, Default)]
struct Provider {
    party: Option<Uuid>,
    types: Vec<ForeignCode>,
}

/// The `ORGANISATION` whose identifier is `key` and its types, on `tx`.
async fn provider(
    tx: &mut PgConnection,
    key: &care_of::ProviderKey,
) -> Result<Provider, StorageError> {
    let rows: Vec<(Uuid, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT p.party_id, f.terminology_id, f.code_string FROM binding_provider p \
         LEFT JOIN binding_caller_fact f ON f.party_id = p.party_id \
              AND f.kind = 'organisation_type' \
         WHERE p.issuer = $1 AND p.id = $2 \
         ORDER BY p.party_id, f.terminology_id, f.code_string",
    )
    .bind(&key.issuer)
    .bind(&key.id)
    .fetch_all(&mut *tx)
    .await?;
    let mut provider = Provider::default();
    for (party, terminology, code) in rows {
        provider.party.get_or_insert(party);
        if let (Some(terminology), Some(code)) = (terminology, code) {
            provider.types.push(ForeignCode { terminology, code });
        }
    }
    Ok(provider)
}

/// The first date `validity`, a `time_validity`, holds and the first it no
/// longer holds, as the caller fact stores them, each `None` when unbounded;
/// `None` when a bound does not read and the relationship holds no date.
async fn dates(
    tx: &mut PgConnection,
    validity: Option<&serde_json::Value>,
) -> Result<Option<(Option<jiff::civil::Date>, Option<jiff::civil::Date>)>, StorageError> {
    let (empty, lower, upper): (bool, Option<jiff_sqlx::Date>, Option<jiff_sqlx::Date>) =
        sqlx::query_as(
            "SELECT isempty(r), lower(r), upper(r) \
             FROM (SELECT party.binding_validity($1::jsonb) AS r) v",
        )
        .bind(validity)
        .fetch_one(&mut *tx)
        .await?;
    Ok((!empty).then(|| {
        (
            lower.map(jiff_sqlx::Date::to_jiff),
            upper.map(jiff_sqlx::Date::to_jiff),
        )
    }))
}

/// Keeps `until`, set by the policy version `policy_version`, as the end of
/// the relationship `vo_id` at its version `version_uid`, on `tx`.
async fn keep_end(
    tx: &mut PgConnection,
    vo_id: Uuid,
    version_uid: &str,
    until: jiff::civil::Date,
    policy_version: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO binding_care_validity (vo_id, version_uid, effective_until, policy_version) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (vo_id) DO UPDATE SET version_uid = excluded.version_uid, \
             effective_until = excluded.effective_until, \
             policy_version = excluded.policy_version, written_at = now()",
    )
    .bind(vo_id)
    .bind(version_uid)
    .bind(jiff_sqlx::Date::from(until))
    .bind(policy_version)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The party that wrote the trunk head of `vo_id` ([`super::store::WRITER`]),
/// on `tx`.
async fn writer_of(tx: &mut PgConnection, vo_id: Uuid) -> Result<Option<Uuid>, StorageError> {
    let statement = format!("SELECT {}", super::store::WRITER);
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(statement))
        .bind(vo_id)
        .fetch_one(&mut *tx)
        .await?)
}

/// Drops the end the relationship `vo_id` took, on `tx`.
async fn drop_end(tx: &mut PgConnection, vo_id: Uuid) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM binding_care_validity WHERE vo_id = $1")
        .bind(vo_id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}
