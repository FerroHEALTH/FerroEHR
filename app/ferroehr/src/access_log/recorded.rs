// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The audit record a stored entry holds: written into the entry when it is
//! built, read back from it for every projection (#5127).
//!
//! [`details`] writes the facts of an access the draft's core does not hold
//! into the entry's `other_details` ([`crate::access_log::details`]), and
//! [`stored`] reads the entry back as the record the renderings take: the
//! core for what it holds (the time, the request, the operation, the query,
//! the authorisations, the decision) and the details for the rest. Every
//! projection reads the entry through [`stored`], so the BALP `AuditEvent`,
//! the DICOM message and the EHDS access record are written from what the
//! log keeps and from nothing else. No openEHR spec governs the audit trail:
//! our own design.

use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::v1_2::data_structures::item_structure::item_structure::ItemStructure;
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;

use crate::access_log::details::{Detail, Details, DetailsError, DetailsWriter};
use crate::system_log::binding::BindingRecord;
use crate::system_log::categories::{ClassifiedContent, MapDigest};
use crate::system_log::event::{
    AccessDecision, AccessDomain, ActingMode, AuditEvent, EntryScope, EventActionCode,
    EventOutcome, EventType, QueryNaming,
};
use crate::system_log::render::StoredRecord;
use crate::system_log::store::{access_domain, object_class, resource_class};
use ferrohealth_smart::assurance::AssuranceLevel;

/// The `EventTypeCode` of a login (DICOM PS3.15 A.5.1, DCM 110122).
const LOGIN: &str = "110122";
/// The `EventTypeCode` of a logout (DICOM PS3.15 A.5.1, DCM 110123).
const LOGOUT: &str = "110123";

/// Why a stored entry does not read back as an audit record.
#[derive(Debug, thiserror::Error)]
pub enum RecordedError {
    /// The entry's details do not read.
    #[error(transparent)]
    Details(#[from] DetailsError),
    /// A detail holds a value this build does not write.
    #[error("the {0:?} detail holds a value this build does not write")]
    Value(Detail),
    /// The entry's `time_recorded` is no RFC 3339 instant.
    #[error("the entry's time_recorded does not read")]
    Time,
    /// The entry holds no details: it was not written by this build's
    /// recording path, such as an entry imported from another system.
    #[error("the entry holds no FerroEHR access details")]
    NoDetails,
}

/// The details tree of the access `event` records, whose EHR belongs to the
/// subject of care `subject` when the drain found one.
///
/// A fact the core states as it is, the request id, the client address, the
/// operation, the query and the authorisations, is not repeated; every other
/// fact the audit record holds is.
#[must_use]
pub fn details(event: &AuditEvent, subject: Option<&str>) -> Option<ItemStructure> {
    let mut tree = DetailsWriter::default();
    tree.text(Detail::EventAction, Some(event.action.as_code()));
    tree.text(
        Detail::EventType,
        match event.event_type {
            Some(EventType::Login) => Some(LOGIN),
            Some(EventType::Logout) => Some(LOGOUT),
            Some(EventType::RestOperation(_)) | None => None,
        },
    );
    tree.text(Detail::ResourceClass, Some(resource_class(event.object)));
    tree.count(
        Detail::OutcomeIndicator,
        Some(outcome_indicator(event.outcome)),
    );
    tree.text(Detail::Principal, Some(&event.user_id));
    tree.text(Detail::Issuer, event.issuer.as_deref());
    tree.text(Detail::ClientId, event.client_id.as_deref());
    tree.text(Detail::TokenId, event.token_id.as_deref());
    tree.text(Detail::Organisation, event.organisation.as_deref());
    tree.texts(Detail::Role, event.roles.iter().map(String::as_str));
    if event.domain != AccessDomain::of(event.object) {
        tree.text(Detail::Domain, Some(event.domain.as_str()));
    }
    tree.text(Detail::EhrId, event.ehr_id.as_deref());
    if let Some(subject) = subject {
        tree.identifier(
            Detail::Subject,
            DvIdentifier {
                issuer: event.subject_namespace.clone(),
                assigner: None,
                id: subject.to_owned(),
                r#type: None,
            },
        );
    }
    tree.flag(Detail::SubjectLookupFailed, event.subject_lookup_failed);
    // NOTE: GDPR Art 5(1)(c) (`docs/law/eu/gdpr/text.html`), a read of the log names its
    // search by digest, so the entry keeps no object the search named.
    if event.entry != EntryScope::LogRead {
        tree.text(Detail::ObjectId, event.object_id.as_deref());
    }
    tree.text(Detail::Purpose, event.purpose.as_deref());
    tree.text(Detail::LegalBasis, event.legal_basis.as_deref());
    tree.count(Detail::ResultCount, event.result_count);
    tree.texts(Detail::Origin, event.origins.iter().map(String::as_str));
    tree.count(Detail::OriginCount, event.origin_count);
    let digest = event.category_map_digest.map(|digest| digest.to_string());
    tree.text(Detail::CategoryMapDigest, digest.as_deref());
    tree.json(
        Detail::CategoryContent,
        event
            .category
            .as_ref()
            .and_then(|category| serde_json::to_string(&category.content).ok()),
    );
    tree.text(Detail::ActingMode, event.actor.mode.map(ActingMode::as_str));
    tree.text(
        Detail::AssuranceLevel,
        event.actor.assurance_level.map(AssuranceLevel::as_str),
    );
    tree.text(
        Detail::AssuranceValue,
        event.actor.assurance_value.as_deref(),
    );
    tree.text(
        Detail::ProfessionalId,
        event.actor.professional_id.as_deref(),
    );
    tree.json(
        Detail::TokenAttributes,
        event
            .token
            .as_ref()
            .and_then(|token| serde_json::to_string(token).ok()),
    );
    tree.json(
        Detail::Binding,
        event
            .binding
            .as_ref()
            .and_then(|binding| serde_json::to_string(binding).ok()),
    );
    tree.json(
        Detail::Summary,
        event
            .summary
            .as_ref()
            .and_then(|summary| serde_json::to_string(summary).ok()),
    );
    tree.finish()
}

/// The DICOM `EventOutcomeIndicator` of `outcome` (PS3.15 A.5.1).
const fn outcome_indicator(outcome: EventOutcome) -> u64 {
    match outcome {
        EventOutcome::Success => 0,
        EventOutcome::MinorFailure => 4,
        EventOutcome::SeriousFailure => 8,
    }
}

/// The audit record `entry` holds, as the renderings read it, at chain
/// position `chain_seq`.
///
/// # Errors
/// [`RecordedError::NoDetails`] for an entry without FerroEHR's access
/// details, [`RecordedError::Details`] when they do not read, and
/// [`RecordedError::Value`] for a value this build does not write.
pub(crate) fn stored(
    entry: &AccessLogEntry,
    chain_seq: i64,
) -> Result<StoredRecord, RecordedError> {
    let details = Details::of(entry.other_details.as_ref())?;
    let text = |detail| details.text(detail);
    let action = match text(Detail::EventAction)?.as_deref() {
        Some("C") => EventActionCode::Create,
        Some("R") => EventActionCode::Read,
        Some("U") => EventActionCode::Update,
        Some("D") => EventActionCode::Delete,
        Some("E") => EventActionCode::Execute,
        Some(_) => return Err(RecordedError::Value(Detail::EventAction)),
        None => return Err(RecordedError::NoDetails),
    };
    let outcome = match details.count(Detail::OutcomeIndicator)? {
        Some(0) => EventOutcome::Success,
        Some(4) => EventOutcome::MinorFailure,
        Some(8) => EventOutcome::SeriousFailure,
        _ => return Err(RecordedError::Value(Detail::OutcomeIndicator)),
    };
    let object = text(Detail::ResourceClass)?
        .as_deref()
        .and_then(object_class)
        .ok_or(RecordedError::Value(Detail::ResourceClass))?;
    let mut event = AuditEvent::new(action, object, outcome);
    event.timestamp = entry
        .time_recorded
        .value
        .parse()
        .map_err(|_unreadable| RecordedError::Time)?;
    event.event_type = match text(Detail::EventType)?.as_deref() {
        Some(LOGIN) => Some(EventType::Login),
        Some(LOGOUT) => Some(EventType::Logout),
        Some(_) => return Err(RecordedError::Value(Detail::EventType)),
        None => entry.operation.as_ref().map(|operation| {
            EventType::RestOperation(std::borrow::Cow::Owned(
                operation.defining_code.code_string.clone(),
            ))
        }),
    };
    let subject = details.identifier(Detail::Subject)?;
    event.subject_namespace = subject.and_then(|subject| subject.issuer.clone());
    let map_digest = text(Detail::CategoryMapDigest)?
        .map(|digest| digest.parse::<MapDigest>())
        .transpose()
        .map_err(|_unreadable| RecordedError::Value(Detail::CategoryMapDigest))?;
    event.category_map_digest = map_digest;
    accessor_and_access(&mut event, &details)?;
    event.request_id = entry
        .channel
        .as_ref()
        .and_then(|channel| channel.request_id.clone());
    event.client_ip = entry
        .channel
        .as_ref()
        .and_then(|channel| channel.client_address.clone());
    event.permit = entry
        .authorisations
        .iter()
        .flatten()
        .find(|authorisation| authorisation.kind.defining_code.code_string == "permit")
        .and_then(|authorisation| authorisation.reference.as_ref())
        .map(|reference| reference.id.clone());
    event.emergency_access = entry.is_emergency;
    event.query = entry.query.as_ref().and_then(|query| {
        Some(QueryNaming {
            language: query_language(query.language.as_deref()?),
            stored: query.stored_query.clone(),
            text_digest: query.text_digest.clone()?,
        })
    });
    event.decision = if entry.outcome.result.defining_code.code_string == "refused" {
        AccessDecision::Refused
    } else {
        AccessDecision::Admitted
    };
    event.entry = EntryScope::Written;
    let content = details.json::<ClassifiedContent>(Detail::CategoryContent)?;
    Ok(StoredRecord {
        chain_seq,
        event,
        subject: subject.map(|subject| subject.id.clone()),
        content,
        map_digest,
    })
}

/// Fills `event` with the facts of the accessor and the access `details`
/// hold.
///
/// # Errors
/// [`RecordedError::Details`] when a detail does not read, and
/// [`RecordedError::Value`] for a value this build does not write.
fn accessor_and_access(event: &mut AuditEvent, details: &Details<'_>) -> Result<(), RecordedError> {
    let text = |detail| details.text(detail);
    event.user_id = text(Detail::Principal)?.unwrap_or_default();
    event.issuer = text(Detail::Issuer)?;
    event.client_id = text(Detail::ClientId)?;
    event.token_id = text(Detail::TokenId)?;
    event.organisation = text(Detail::Organisation)?;
    event.roles = details.texts(Detail::Role)?;
    if let Some(domain) = text(Detail::Domain)? {
        event.domain = access_domain(Some(&domain)).ok_or(RecordedError::Value(Detail::Domain))?;
    }
    event.ehr_id = text(Detail::EhrId)?;
    event.subject_lookup_failed = details.flag(Detail::SubjectLookupFailed)?;
    event.object_id = text(Detail::ObjectId)?;
    event.purpose = text(Detail::Purpose)?;
    event.legal_basis = text(Detail::LegalBasis)?;
    event.result_count = details.count(Detail::ResultCount)?;
    event.origins = details.texts(Detail::Origin)?;
    event.origin_count = details.count(Detail::OriginCount)?;
    event.actor.mode = text(Detail::ActingMode)?
        .map(|mode| acting_mode(&mode).ok_or(RecordedError::Value(Detail::ActingMode)))
        .transpose()?;
    event.actor.assurance_level = text(Detail::AssuranceLevel)?
        .map(|level| assurance_level(&level).ok_or(RecordedError::Value(Detail::AssuranceLevel)))
        .transpose()?;
    event.actor.assurance_value = text(Detail::AssuranceValue)?;
    event.actor.professional_id = text(Detail::ProfessionalId)?;
    event.token = details.json(Detail::TokenAttributes)?;
    event.binding = details.json::<BindingRecord>(Detail::Binding)?;
    event.summary = details.json(Detail::Summary)?;
    Ok(())
}

/// The acting mode `stored` names, the inverse of [`ActingMode::as_str`].
fn acting_mode(stored: &str) -> Option<ActingMode> {
    [
        ActingMode::Person,
        ActingMode::Client,
        ActingMode::Delegated,
        ActingMode::External,
        ActingMode::System,
    ]
    .into_iter()
    .find(|mode| mode.as_str() == stored)
}

/// The assurance level `stored` names, the inverse of
/// [`AssuranceLevel::as_str`].
fn assurance_level(stored: &str) -> Option<AssuranceLevel> {
    [
        AssuranceLevel::Low,
        AssuranceLevel::Substantial,
        AssuranceLevel::High,
    ]
    .into_iter()
    .find(|level| level.as_str() == stored)
}

/// The language name of a query the recording path names, as the static
/// text [`QueryNaming`] carries.
fn query_language(stored: &str) -> &'static str {
    match stored {
        crate::access_log::entry::LOG_SEARCH => crate::access_log::entry::LOG_SEARCH,
        _ => "AQL",
    }
}
