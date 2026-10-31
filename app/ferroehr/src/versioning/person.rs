// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two rules a commit by the record's person meets (EHDS Art. 5).
//!
//! Regulation (EU) 2025/327 Art. 5 lets a natural person or their
//! representative insert information in their own EHR; it "shall be clearly
//! distinguishable as having been inserted by the natural person or by his or
//! her representative", and they "shall not be able to directly alter the
//! electronic health data and related information inserted by health
//! professionals" (`docs/law/eu/ehds/text.html` Art. 5). The marker is the
//! committer `PARTY_SELF` ([`crate::service::committer::attributed`]); these are
//! the refusals. No openEHR spec governs who may change what: our own design.

use serde_json::Value;
use sqlx::PgConnection;

use crate::ids::VoId;
use crate::service::error::ServiceError;
use crate::versioning::Kind;
use crate::versioning::object_version_id::TreeId;

/// Refuses a COMPOSITION a person commits whose composer is someone else.
///
/// The composer is "the person primarily responsible for the content of the
/// Composition", and "when it is the patient, the special self instance of
/// `PARTY_PROXY` will be used" (RM `UML/classes/org.openehr.rm.composition.composition.adoc`);
/// `PARTY_RELATED` is used "whenever the relationship of the party to the record
/// subject is required" (RM common master04 §Referring to Demographic Entities),
/// which is how a representative (EHDS Art. 4(2)) is named. Refusing every other composer keeps a person from presenting what
/// they insert as a professional's (EHDS Art. 5); our own design, as no openEHR
/// spec governs it.
///
/// # Errors
/// [`ServiceError::Restricted`] (`403`) when a person commits a COMPOSITION
/// whose composer is neither `PARTY_SELF` nor `PARTY_RELATED`.
pub(crate) fn refuse_composing_for_others(
    kind: Kind,
    canonical: &Value,
) -> Result<(), ServiceError> {
    if kind != Kind::Composition || !crate::service::committer::person_commits() {
        return Ok(());
    }
    let composer = canonical
        .get("composer")
        .and_then(|composer| composer.get("_type"))
        .and_then(Value::as_str);
    match composer {
        Some("PARTY_SELF" | "PARTY_RELATED") => Ok(()),
        _ => Err(ServiceError::not_permitted(
            "a COMPOSITION a person inserts in their own EHR names the person (PARTY_SELF) \
             or their representative (PARTY_RELATED) as its composer (EHDS Art. 5)",
        )),
    }
}

/// Refuses a person's change to a version the person did not insert.
///
/// EHDS Art. 5: a natural person or their representative "shall not be able to
/// directly alter the electronic health data and related information inserted
/// by health professionals" (`docs/law/eu/ehds/text.html` Art. 5). A version
/// the person inserted is one whose committer is `PARTY_SELF`, the RM's
/// "committer is the record subject" (RM common master04 §Audit Details); any
/// other committer is held to be someone else, a professional or a system.
/// Which committers count is our own design: no openEHR spec governs it.
///
/// # Errors
/// [`ServiceError::Restricted`] (`403`) when a person changes a version whose
/// committer is not `PARTY_SELF`; the storage error of the committer read.
pub(crate) async fn refuse_altering_others(
    tx: &mut PgConnection,
    kind: Kind,
    vo_id: VoId,
    preceding: TreeId,
) -> Result<(), ServiceError> {
    if !crate::service::committer::person_commits() {
        return Ok(());
    }
    let committer =
        crate::storage::version_repo::placement::committer_type(tx, vo_id, preceding.columns())
            .await?;
    if committer.as_deref() == Some("PARTY_SELF") {
        return Ok(());
    }
    Err(ServiceError::not_permitted(format!(
        "version {preceding} of {} {vo_id} was not inserted by the person, who may not \
         alter data a health professional inserted (EHDS Art. 5)",
        kind.as_str()
    )))
}
