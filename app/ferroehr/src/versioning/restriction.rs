// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The restriction gates every commit passes inside its transaction: GDPR
//! Art. 18 restriction of processing and the EHDS Art. 8 restriction of
//! access. No openEHR spec governs either: our own design/extension.

use sqlx::PgConnection;

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::{EhrId, VoId};

/// Refuse a write that would process an object whose processing is restricted.
///
/// GDPR Art. 18(2) leaves storage as the only processing a restricted object
/// admits (`docs/law/eu/gdpr/text.html`), so a new version of it, a logical
/// delete of it or an attestation appended to it is refused — while the rows
/// already stored stay exactly where they are. The mark is read inside the
/// commit transaction, like the `is_modifiable` gate of the commit engine, so a
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
    ensure_not_access_restricted_tx(tx, ehr_id, vo_id).await?;
    crate::versioning::access_delay::ensure_not_withheld_tx(tx, vo_id).await
}

/// Refuse a write the access evaluator refuses for the running request: as
/// though the target did not exist under an EHDS Art. 8 restriction or an
/// Art. 10 opt-out, whose fact is "not visible to healthcare providers"
/// (`docs/law/eu/ehds/text.html`), and with its `refused-*` code under every
/// other step of an EHR the request names. An EHR the request did not judge
/// yet is judged from the rows the commit's transaction reads; an object of a
/// verdict that depends on the object is judged by its head row. No openEHR
/// spec governs this: our own design (#5513).
///
/// # Errors
/// [`ServiceError::NotFound`] (`ehr_id_does_not_exist` or
/// `versioned_object_does_not_exist`) for an absent target,
/// [`ServiceError::Forbidden`] naming the refusal code, and
/// [`ServiceError::Database`] if a fact cannot be read.
async fn ensure_not_access_restricted_tx(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    vo_id: Option<VoId>,
) -> Result<(), ServiceError> {
    use crate::access_eval::procedure::Outcome;
    use crate::access_eval::scope::Item;
    let (Some(access), Some(ehr_id)) = (crate::access_eval::condition::current(), ehr_id) else {
        return Ok(());
    };
    let judged = access.verdict(ehr_id);
    let named = judged.is_some();
    let verdict = if let Some(verdict) = judged {
        verdict
    } else {
        let body =
            crate::storage::version_repo::ehr_access::current_ehr_access_body(&mut *tx, ehr_id)
                .await?;
        let settings = match body {
            Some(body) => {
                let access = serde_json::from_str::<serde_json::Value>(&body)
                    .map_err(|e| ServiceError::internal("read a stored EHR_ACCESS body", e))?;
                crate::service::ehr::access_settings::EhrAccessSettings::held_or_read(&access)
            }
            None => None,
        };
        let (subject, version) = Box::pin(subject_and_access_version(&mut *tx, ehr_id)).await?;
        access.judge_ehr(
            ehr_id,
            &crate::access_eval::condition::EhrEntry {
                version: settings.as_ref().and(version),
                settings: std::sync::Arc::new(settings),
                subject,
            },
        )
    };
    let facts = match vo_id {
        Some(vo_id) if verdict.item_dependent() => {
            crate::storage::access_facts::of_objects(&mut *tx, &[vo_id])
                .await?
                .into_iter()
                .next()
        }
        _ => None,
    };
    let item = facts.as_ref().map_or(Item::Ehr, |facts| facts.item());
    let outcome = verdict.outcome(&item);
    if matches!(outcome, Outcome::Absent(_)) || (named && matches!(outcome, Outcome::Refused(_))) {
        access.note(ehr_id, &verdict, &item);
    }
    match (outcome, vo_id) {
        (Outcome::Admit, _) => Ok(()),
        (Outcome::Absent(_), Some(vo_id)) if facts.is_some() => Err(ServiceError::not_found(
            Missing::VersionedObject,
            format!("versioned object {vo_id}"),
        )),
        (Outcome::Absent(_), _) => Err(ServiceError::not_found(
            Missing::Ehr,
            format!("EHR {ehr_id}"),
        )),
        // NOTE: our own design (#5513), a write to an EHR the request does not name, such
        // as the one it creates or imports, is held to steps 1 and 2 only, as before.
        (Outcome::Refused(_), _) if !named => Ok(()),
        (Outcome::Refused(code), _) => Err(ServiceError::not_permitted(format!(
            "{code}: the access directives of EHR {ehr_id} refuse this write"
        ))),
    }
}

/// The `PERSON` the EHR `ehr_id` names as its subject and the uid of its
/// current `EHR_ACCESS` version, read on the commit's own transaction.
///
/// # Errors
/// [`ServiceError::Database`] on a driver failure.
async fn subject_and_access_version(
    tx: &mut PgConnection,
    ehr_id: EhrId,
) -> Result<(Option<uuid::Uuid>, Option<String>), ServiceError> {
    use sqlx::Row as _;
    let row = sqlx::query(
        "SELECT e.subject_id, h.vo_id, v.creating_system_id, v.trunk_version, v.branch_number, \
                v.branch_version \
         FROM ehr e \
         LEFT JOIN vo_head h ON h.ehr_id = e.id AND h.kind = 'EHR_ACCESS' \
         LEFT JOIN version v ON v.vo_id = h.vo_id AND v.sys_version = h.trunk_head_sys_version \
         WHERE e.id = $1",
    )
    .bind(ehr_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        return Ok((None, None));
    };
    // NOTE: the local source names each party by its container's UUID, so a subject
    // id that is no UUID names no PERSON of this store.
    let subject = row
        .try_get::<Option<String>, _>("subject_id")?
        .and_then(|subject| subject.parse::<uuid::Uuid>().ok());
    let version = crate::storage::access_projection::version_of(&row)?;
    Ok((subject, version))
}
