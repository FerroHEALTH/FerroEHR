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
/// other step. The EHR the request did not judge yet is judged from the rows
/// the commit's transaction reads; an object of a verdict that depends on the
/// object is judged by its head row. No openEHR spec governs this: our own
/// design (#5513).
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
    let verdict = if let Some(verdict) = access.verdict(ehr_id) {
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
        let subject: Option<String> =
            sqlx::query_scalar("SELECT subject_id FROM ehr WHERE id = $1")
                .bind(ehr_id)
                .fetch_optional(&mut *tx)
                .await?
                .flatten();
        let subject = subject.and_then(|subject| subject.parse::<uuid::Uuid>().ok());
        access.judge_ehr(ehr_id, settings.as_ref(), subject)
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
    let outcome = facts.as_ref().map_or_else(
        || verdict.outcome(&Item::Ehr),
        |facts| verdict.outcome(&facts.item()),
    );
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
        (Outcome::Refused(code), _) => Err(ServiceError::not_permitted(format!(
            "{code}: the access directives of EHR {ehr_id} refuse this write"
        ))),
    }
}
