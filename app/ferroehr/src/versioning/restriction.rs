// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The restriction gates every commit passes inside its transaction: GDPR
//! Art. 18 restriction of processing and the EHDS Art. 8 restriction of
//! access. No openEHR spec governs either: our own design/extension.

use std::sync::Arc;

use sqlx::PgConnection;

use crate::access_eval::procedure::{Outcome, Verdict};
use crate::access_eval::scope::{Item, Object};
use crate::versioning::change::Change;

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::{EhrId, VoId};
use crate::storage::access_projection::Revision;

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
    named_verdict_tx(tx, ehr_id, vo_id).await.map(drop)
}

/// [`ensure_not_restricted_tx`] for `change` into `ehr_id`, and what its
/// content is judged by once resolved ([`Written::admit`]).
///
/// # Errors
/// As [`ensure_not_restricted_tx`].
pub(crate) async fn ensure_writable_tx(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    change: &Change,
) -> Result<Written, ServiceError> {
    let named = named_verdict_tx(tx, ehr_id, change.target_vo_id()).await?;
    Ok(Written {
        judged: named.zip(ehr_id),
        template: written_template(change),
    })
}

/// The restriction gates of [`ensure_not_restricted_tx`], answering the
/// access evaluator's verdict on the EHR when the request names it.
async fn named_verdict_tx(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    vo_id: Option<VoId>,
) -> Result<Option<Arc<Verdict>>, ServiceError> {
    let mut seen = Revision::default();
    if let Some(ehr_id) = ehr_id {
        let read = crate::storage::ehr_repo::restriction_and_revision(&mut *tx, ehr_id).await?;
        if let Some((restricted, revision)) = read {
            if restricted {
                return Err(ServiceError::restricted(&format!("EHR {ehr_id}")));
            }
            seen = revision;
        }
    }
    if let Some(vo_id) = vo_id
        && crate::storage::marks::is_restricted(&mut *tx, vo_id).await?
    {
        return Err(ServiceError::restricted(&format!(
            "versioned object {vo_id}"
        )));
    }
    let named = ensure_not_access_restricted_tx(tx, ehr_id, vo_id, seen).await?;
    crate::versioning::access_delay::ensure_not_withheld_tx(tx, vo_id).await?;
    Ok(named)
}

/// A create or a modify the access evaluator judges as the datum it writes
/// once resolved: the verdict of the EHR the request names, and the template
/// the content is of.
#[derive(Debug)]
pub(crate) struct Written {
    judged: Option<(Arc<Verdict>, EhrId)>,
    template: Option<String>,
}

impl Written {
    /// Refuse the resolved version `vo_id` whose root is of `archetype`,
    /// stamped with `origins` and committed at `committed`, when the
    /// evaluator refuses it ([`ensure_content_admitted`]).
    ///
    /// # Errors
    /// [`ServiceError::Forbidden`] naming the refusal code.
    pub(crate) fn admit(
        &self,
        vo_id: VoId,
        archetype: Option<&str>,
        origins: &[String],
        committed: jiff::Timestamp,
    ) -> Result<(), ServiceError> {
        let Some((verdict, ehr_id)) = &self.judged else {
            return Ok(());
        };
        let item = Item::Object(Object {
            vo_id: vo_id.0,
            template: self.template.as_deref(),
            archetype,
            origins: (!origins.is_empty()).then_some(origins),
            recorded: Some(committed),
        });
        ensure_content_admitted(verdict, *ehr_id, &item)
    }
}

/// The template of the content a create or a modify writes; `None` for a
/// delete.
fn written_template(change: &Change) -> Option<String> {
    let (Change::Create {
        canonical,
        template_id,
        ..
    }
    | Change::Modify {
        canonical,
        template_id,
        ..
    }) = change
    else {
        return None;
    };
    crate::versioning::system::template_of(canonical, template_id.as_deref()).map(str::to_owned)
}

/// Refuse a version whose content the access evaluator refuses for the
/// running request, judged by `verdict`, the verdict of its EHR, as the
/// datum `item`: its template, root archetype, origins and the sensitivities
/// the policy's classifications give it, through steps 3 to 8.
///
/// A restriction (EHDS Art. 8) and an opt-out (Art. 10) limit access to data
/// that exist, and writing new data is no access to them, so they decide a
/// write only for the whole EHR ([`ensure_not_restricted_tx`]); a write
/// never reveals one. Our own design (#5621, #5627).
///
/// # Errors
/// [`ServiceError::Forbidden`] naming the refusal code.
fn ensure_content_admitted(
    verdict: &Verdict,
    ehr_id: EhrId,
    item: &Item<'_>,
) -> Result<(), ServiceError> {
    let Some(access) = crate::access_eval::condition::current() else {
        return Ok(());
    };
    let written = Verdict {
        rules: verdict
            .rules
            .iter()
            .filter(|rule| !matches!(rule.then, Outcome::Absent(_)))
            .cloned()
            .collect(),
        ..verdict.clone()
    };
    match written.outcome(item) {
        Outcome::Admit | Outcome::Absent(_) => Ok(()),
        Outcome::Refused(code) => {
            access.note(ehr_id, &written, item);
            Err(ServiceError::refused(
                code,
                format!("{code}: the access evaluator refuses this version of EHR {ehr_id}"),
            ))
        }
    }
}

/// Refuse a write the access evaluator refuses for the running request: as
/// though the target did not exist under an EHDS Art. 8 restriction or an
/// Art. 10 opt-out, whose fact is "not visible to healthcare providers"
/// (`docs/law/eu/ehds/text.html`), and with its `refused-*` code under every
/// other step of an EHR the request names. An EHR the request did not judge
/// yet, or judged on a revision of its settings row other than `seen`, the
/// one the commit's transaction reads, is judged from the rows that
/// transaction reads, so a change another replica committed is never written
/// past; an object of a verdict that depends on the object is judged by its
/// head row. No openEHR spec governs this: our own design (#5513, #5589).
///
/// A create of an EHR the request names whose verdict depends on the datum
/// is judged by its content once resolved ([`ensure_content_admitted`]),
/// so no step about the EHR decides before a step about the datum. The verdict
/// of an EHR the request names is the answer.
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
    seen: Revision,
) -> Result<Option<Arc<Verdict>>, ServiceError> {
    let (Some(access), Some(ehr_id)) = (crate::access_eval::condition::current(), ehr_id) else {
        return Ok(None);
    };
    let judged = access.judgement(ehr_id);
    let named = judged.is_some();
    let verdict = if let Some(judged) = judged.filter(|judged| judged.revision == seen) {
        judged.verdict
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
                settings: Arc::new(settings),
                subject,
                revision: seen,
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
    let deferred = vo_id.is_none() && verdict.item_dependent();
    if matches!(outcome, Outcome::Absent(_))
        || (named && !deferred && matches!(outcome, Outcome::Refused(_)))
    {
        access.note(ehr_id, &verdict, &item);
    }
    let answer = named.then(|| Arc::clone(&verdict));
    match (outcome, vo_id) {
        (Outcome::Admit, _) => Ok(answer),
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
        (Outcome::Refused(_), _) if !named => Ok(None),
        // NOTE: our own design (#5627), a create is judged as the datum it writes, once resolved.
        (Outcome::Refused(_), None) if deferred => Ok(answer),
        (Outcome::Refused(code), _) => Err(ServiceError::refused(
            code,
            format!("{code}: the access directives of EHR {ehr_id} refuse this write"),
        )),
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
    let subject = crate::storage::access_projection::subject_of(
        row.try_get::<Option<String>, _>("subject_id")?.as_deref(),
    );
    let version = crate::storage::access_projection::version_of(&row)?;
    Ok((subject, version))
}
