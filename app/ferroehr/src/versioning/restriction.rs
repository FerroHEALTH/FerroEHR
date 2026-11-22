// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The restriction gates every commit passes inside its transaction: GDPR
//! Art. 18 restriction of processing and the EHDS Art. 8 restriction of
//! access. No openEHR spec governs either: our own design/extension.

use std::sync::Arc;

use sqlx::PgConnection;

use crate::access_eval::condition::RequestAccess;
use crate::access_eval::procedure::{Outcome, Verdict};
use crate::access_eval::scope::Item;

use crate::error::ServiceError;
use crate::error::refusal::{Missing, Refused};
use crate::ids::{EhrId, VoId};
use crate::storage::access_projection::Revision;
use crate::storage::ehr_repo::Restriction;

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
/// `known` is what the caller already read on the commit's connection
/// ([`Known`]): the EHR's mark and settings revision (#5775) and the object's
/// own mark (#6256), each taken instead of reading it again.
///
/// The answer carries the access evaluator's verdict of the EHR, which
/// [`WriteGate::admit_written`] then holds the content the write is about to
/// store to.
///
/// # Errors
/// [`ServiceError::Restricted`] (`403`) when either grain is marked;
/// [`ServiceError::Database`] if the marks cannot be read.
pub(crate) async fn ensure_not_restricted_tx(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    vo_id: Option<VoId>,
    known: &Known,
) -> Result<Option<WriteGate>, ServiceError> {
    let mut seen = Revision::default();
    if let Some(ehr_id) = ehr_id {
        let read = match known.restriction.clone() {
            Some(read) => read,
            None => crate::storage::ehr_repo::restriction_and_revision(&mut *tx, ehr_id).await?,
        };
        if let Some((restricted, revision)) = read {
            if restricted {
                return Err(ServiceError::restricted(&format!("EHR {ehr_id}")));
            }
            seen = revision;
        }
    }
    let object_restricted = match (vo_id, known.object_restricted) {
        (Some(_), Some(known)) => known,
        (Some(vo_id), None) => crate::storage::marks::is_restricted(&mut *tx, vo_id).await?,
        (None, _) => false,
    };
    if let Some(vo_id) = vo_id
        && object_restricted
    {
        return Err(ServiceError::restricted(&format!(
            "versioned object {vo_id}"
        )));
    }
    let gate = ensure_not_access_restricted_tx(tx, ehr_id, vo_id, seen).await?;
    crate::versioning::access_delay::ensure_not_withheld_tx(tx, ehr_id, vo_id).await?;
    Ok(gate)
}

/// The access evaluator's verdict of the EHR a write reaches, for the content
/// the write is about to store.
#[derive(Debug)]
pub(crate) struct WriteGate {
    access: Arc<RequestAccess>,
    ehr_id: EhrId,
    verdict: Arc<Verdict>,
    named: bool,
}

impl WriteGate {
    /// Refuse the write of `written`, the object as it will be stored, before
    /// anything is stored, as steps 3 to 8 would refuse a read of it: a
    /// directive or a controller rule whose data scope names the template,
    /// archetype or origin of a body the store does not hold yet refuses it
    /// (RM EHR IM §EHR Access; BASE Architecture Overview §Access Control
    /// decides before the access). Our own design (#5626, #5621).
    ///
    /// An opt-out and a restriction never judge the content: EHDS Art. 8
    /// restricts who may see data, not its recording, so refusing a
    /// professional's write would stop care being documented and its `404`
    /// would hint at the restriction. They decide a write for the EHR alone,
    /// and the data written stay covered by them for reading.
    ///
    /// # Errors
    /// [`ServiceError::Forbidden`] naming the refusal code under steps 3 to 8
    /// of an EHR the request names.
    pub(crate) fn admit_written(&self, written: &Item<'_>) -> Result<(), ServiceError> {
        // NOTE: EHDS Art. 8 restricts access, not recording (owner ruling on #5621), so the
        // opt-out and restriction rules leave a write's content to the other steps.
        let judged = Verdict {
            rules: self
                .verdict
                .rules
                .iter()
                .filter(|rule| !matches!(rule.then, Outcome::Absent(_)))
                .cloned()
                .collect(),
            ..(*self.verdict).clone()
        };
        let Outcome::Refused(code) = judged.outcome(written) else {
            if let Item::Object(object) = written {
                self.access.wrote(object.vo_id);
            }
            return Ok(());
        };
        if !self.named {
            return Ok(());
        }
        self.access.note(self.ehr_id, &judged, written);
        Err(ServiceError::refused(
            Refused {
                code,
                legal_grounds: self.access.refusal_grounds(Some(self.ehr_id)),
            },
            format!(
                "{code}: the access directives of EHR {} refuse this write",
                self.ehr_id
            ),
        ))
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
/// past, unless the write judged it before its transaction on that revision
/// ([`RequestAccess::judge_ahead`], #6186); an object of a verdict that
/// depends on the object is judged by its head row. No openEHR spec governs
/// this: our own design (#5513, #5589).
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
) -> Result<Option<WriteGate>, ServiceError> {
    let (Some(access), Some(ehr_id)) = (crate::access_eval::condition::current(), ehr_id) else {
        return Ok(None);
    };
    let judged = access.judgement(ehr_id);
    let named = judged.is_some();
    let verdict = if let Some(judged) = judged.filter(|judged| judged.revision == seen) {
        judged.verdict
    } else if let Some(verdict) = access.take_ahead(ehr_id, &seen) {
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
        let entry = crate::access_eval::condition::EhrEntry {
            version: settings.as_ref().and(version),
            settings: Arc::new(settings),
            subject,
            revision: seen,
        };
        Box::pin(access.judge_ehr(ehr_id, &entry)).await?
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
    // NOTE: our own design (#5627), a create is judged as the datum it writes once resolved,
    // so no step about the EHR alone decides before a step about that datum.
    let deferred = vo_id.is_none() && verdict.item_dependent();
    if matches!(outcome, Outcome::Absent(_))
        || (named && !deferred && matches!(outcome, Outcome::Refused(_)))
    {
        access.note(ehr_id, &verdict, &item);
    }
    let gate = WriteGate {
        access: Arc::clone(&access),
        ehr_id,
        verdict: Arc::clone(&verdict),
        named,
    };
    match (outcome, vo_id) {
        (Outcome::Admit, _) => Ok(Some(gate)),
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
        (Outcome::Refused(_), _) if !named => Ok(Some(gate)),
        (Outcome::Refused(_), None) if deferred => Ok(Some(gate)),
        (Outcome::Refused(code), _) => Err(ServiceError::refused(
            Refused {
                code,
                legal_grounds: access.refusal_grounds(Some(ehr_id)),
            },
            format!("{code}: the access directives of EHR {ehr_id} refuse this write"),
        )),
    }
}

/// The subject reference the EHR `ehr_id` names and the uid of its current
/// `EHR_ACCESS` version, read on the commit's own transaction.
///
/// # Errors
/// [`ServiceError::Database`] on a driver failure.
async fn subject_and_access_version(
    tx: &mut PgConnection,
    ehr_id: EhrId,
) -> Result<
    (
        Option<crate::storage::access_projection::EhrSubject>,
        Option<String>,
    ),
    ServiceError,
> {
    use sqlx::Row as _;
    let row = sqlx::query(
        "SELECT e.subject_namespace, e.subject_id, h.vo_id, v.creating_system_id, v.trunk_version, v.branch_number, \
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
    let subject = crate::storage::access_projection::EhrSubject::stored(
        row.try_get("subject_namespace")?,
        row.try_get("subject_id")?,
    );
    let version = crate::storage::access_projection::version_of(&row)?;
    Ok((subject, version))
}

/// What a write's caller already read on the commit's connection: the commit
/// instant, and the EHR's restriction mark with its settings revision, which
/// [`ensure_not_restricted_tx`] takes instead of reading them again (#5775).
#[derive(Debug, Clone, Default)]
pub(crate) struct Known {
    /// The transaction timestamp; `None` makes the write read its own.
    pub(crate) now: Option<jiff::Timestamp>,
    /// The restriction read; `None` makes the gate read its own.
    pub(crate) restriction: Option<Restriction>,
    /// The written object's own restriction mark; `None` makes the gate read
    /// it.
    pub(crate) object_restricted: Option<bool>,
}

impl Known {
    /// The commit instant `now`, and no restriction read.
    pub(crate) fn at(now: jiff::Timestamp) -> Self {
        Self {
            now: Some(now),
            restriction: None,
            object_restricted: None,
        }
    }
}

impl super::change::Write {
    /// The same write, its EHR's restriction mark and settings revision
    /// already read as `restriction` on the commit's connection.
    pub(crate) fn gated(self, restriction: Restriction) -> Self {
        Self {
            known: Known {
                restriction: Some(restriction),
                ..self.known
            },
            ..self
        }
    }

    /// The same write, the written object's own restriction mark already read
    /// as `restricted` on the commit's connection.
    pub(crate) fn object_marked(self, restricted: bool) -> Self {
        Self {
            known: Known {
                object_restricted: Some(restricted),
                ..self.known
            },
            ..self
        }
    }
}
