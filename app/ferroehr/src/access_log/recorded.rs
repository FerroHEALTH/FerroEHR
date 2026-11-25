// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The audit record a stored entry holds: written into the entry when it is
//! built, read back from it for every projection (#5127).
//!
//! [`details`] writes the facts of an access the draft's core does not hold
//! into the entry's `other_details` ([`crate::access_log::details`]), and
//! `stored` reads the entry back as the record the renderings take: the
//! core for what it holds (the time, the request, the operation, the query,
//! the authorisations, the decision) and the details for the rest. A detail
//! the core already states as the record holds it, such as the principal the
//! credential names or the count the outcome delivered, is not written, and
//! `stored` reads it from the core (#5703). Every
//! projection reads the entry through `stored`, so the BALP `AuditEvent`,
//! the DICOM message and the EHDS access record are written from what the
//! log keeps and from nothing else. No openEHR spec governs the audit trail:
//! our own design.

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::ObjectRef;
use openehr_rm::draft::access_control::terminology::log::{Action, Outcome};
use openehr_rm::draft::access_control::terminology::{Confidentiality, Term as _};
use openehr_rm::draft::access_log::access_log::access_credential::AccessCredential;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentified;
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use openehr_rm::v1_2::data_structures::item_structure::item_structure::ItemStructure;
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;

use crate::access_log::details::{Detail, Details, DetailsError, DetailsWriter};
use crate::system_log::binding::BindingRecord;
use crate::system_log::categories::{ClassifiedContent, MapDigest};
use crate::system_log::event::{
    AccessDecision, AccessDomain, ActingMode, AuditEvent, EntryScope, EventActionCode,
    EventOutcome, EventType, QueryNaming, TokenAttributes,
};
use crate::system_log::render::StoredRecord;
use crate::system_log::store::{access_domain, object_class, resource_class};
use ferrohealth_smart::assurance::AssuranceLevel;

/// The `EventTypeCode` of a login (DICOM PS3.15 A.5.1, DCM 110122).
const LOGIN: &str = "110122";
/// The `EventTypeCode` of a logout (DICOM PS3.15 A.5.1, DCM 110123).
const LOGOUT: &str = "110123";
/// The `DV_IDENTIFIER.type` of the login the accessing party is named by.
const LOGIN_TYPE: &str = "login";
/// The type an entry that reads an EHR's whole access log names the log by.
const LOG_OBJECT: &str = "ACCESS_LOG";
/// The login an entry names when the record named none.
const UNKNOWN_LOGIN: &str = "UNKNOWN";

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
    #[error("the entry holds no access log details")]
    NoDetails,
}

/// The details tree of the access `event` records in the entry whose core is
/// `core`, its EHR belonging to the subject of care `subject` when the drain
/// found one.
///
/// A fact the core states as it is, the request id, the client address, the
/// operation, the query and the authorisations, is not repeated, nor is a
/// fact `Core` reads from the core as the record holds it; every other fact
/// the audit record holds is.
#[must_use]
pub fn details(
    event: &AuditEvent,
    subject: Option<&str>,
    core: &AccessLogEntry,
) -> Option<ItemStructure> {
    let core = Core(core);
    let token_purposes = event.token.as_ref().map_or(0, |token| token.purposes.len());
    let mut tree = DetailsWriter::default();
    if core.action() != Some(event.action) {
        tree.text(Detail::EventAction, Some(event.action.as_code()));
    }
    tree.text(
        Detail::EventType,
        match event.event_type {
            Some(EventType::Login) => Some(LOGIN),
            Some(EventType::Logout) => Some(LOGOUT),
            Some(EventType::RestOperation(_)) | None => None,
        },
    );
    tree.text(Detail::ResourceClass, Some(resource_class(event.object)));
    if core.outcome() != Some(event.outcome) {
        tree.count(
            Detail::OutcomeIndicator,
            Some(outcome_indicator(event.outcome)),
        );
    }
    credential_details(&mut tree, event, core);
    if event.domain != AccessDomain::of(event.object) {
        tree.text(Detail::Domain, Some(event.domain.as_str()));
    }
    if core.ehr_id() != event.ehr_id.as_deref() {
        tree.text(Detail::EhrId, event.ehr_id.as_deref());
    }
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
    tree.level(Detail::Review, event.review);
    // NOTE: GDPR Art 5(1)(c) (`docs/law/eu/gdpr/text.html`), a read of the log names its
    // search by digest, so the entry keeps no object the search named.
    if event.entry != EntryScope::LogRead && core.object_id() != event.object_id.as_deref() {
        tree.text(Detail::ObjectId, event.object_id.as_deref());
    }
    if core.purpose(token_purposes) != event.purpose {
        tree.text(Detail::Purpose, event.purpose.as_deref());
    }
    tree.text(Detail::LegalBasis, event.legal_basis.as_deref());
    if core.result_count() != event.result_count {
        tree.count(Detail::ResultCount, event.result_count);
    }
    origin_details(&mut tree, event, core);
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
    if core.assurance_level() != event.actor.assurance_level {
        tree.text(
            Detail::AssuranceLevel,
            event.actor.assurance_level.map(AssuranceLevel::as_str),
        );
    }
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

/// Writes into `tree` the origins of the data `event` served and their count,
/// where `core`'s item does not hold them as they are.
fn origin_details(tree: &mut DetailsWriter, event: &AuditEvent, core: Core<'_>) {
    // NOTE: the access_log_details archetype, id28 is written where no accessed item holds the
    // origins; the count is written when it is not the number the item holds (#6322).
    if core.holds_origins(&event.origins, event.origin_count) {
        if event.origin_count != core.origin_count() {
            tree.count(Detail::OriginCount, event.origin_count);
        }
    } else {
        tree.texts(Detail::Origin, event.origins.iter().map(String::as_str));
        tree.count(Detail::OriginCount, event.origin_count);
    }
}

/// Writes into `tree` the facts of the caller's credential and roles that
/// `core` does not state as `event` holds them.
fn credential_details(tree: &mut DetailsWriter, event: &AuditEvent, core: Core<'_>) {
    let token_roles = event.token.as_ref().map_or(0, |token| token.roles.len());
    if core.principal().as_deref() != Some(event.user_id.as_str()) {
        tree.text(Detail::Principal, Some(&event.user_id));
    }
    if core.issuer() != event.issuer.as_deref() {
        tree.text(Detail::Issuer, event.issuer.as_deref());
    }
    if core.client_id() != event.client_id.as_deref() {
        tree.text(Detail::ClientId, event.client_id.as_deref());
    }
    if core.token_id() != event.token_id.as_deref() {
        tree.text(Detail::TokenId, event.token_id.as_deref());
    }
    if core.roles(token_roles) != event.roles {
        tree.texts(Detail::Role, event.roles.iter().map(String::as_str));
    }
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
/// [`RecordedError::NoDetails`] for an entry without access log
/// details, [`RecordedError::Details`] when they do not read, and
/// [`RecordedError::Value`] for a value this build does not write.
pub(crate) fn stored(
    entry: &AccessLogEntry,
    chain_seq: i64,
) -> Result<StoredRecord, RecordedError> {
    let Some(tree) = entry.other_details.as_ref() else {
        return Err(RecordedError::NoDetails);
    };
    let details = Details::of(Some(tree))?;
    let core = Core(entry);
    let text = |detail| details.text(detail);
    let action = match text(Detail::EventAction)?.as_deref() {
        Some("C") => EventActionCode::Create,
        Some("R") => EventActionCode::Read,
        Some("U") => EventActionCode::Update,
        Some("D") => EventActionCode::Delete,
        Some("E") => EventActionCode::Execute,
        Some(_) => return Err(RecordedError::Value(Detail::EventAction)),
        None => core
            .action()
            .ok_or(RecordedError::Value(Detail::EventAction))?,
    };
    let outcome = match details.count(Detail::OutcomeIndicator)? {
        Some(0) => EventOutcome::Success,
        Some(4) => EventOutcome::MinorFailure,
        Some(8) => EventOutcome::SeriousFailure,
        Some(_) => return Err(RecordedError::Value(Detail::OutcomeIndicator)),
        None => core
            .outcome()
            .ok_or(RecordedError::Value(Detail::OutcomeIndicator))?,
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
    accessor_and_access(&mut event, &details, core)?;
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
    event.decided = crate::access_eval::record::Decision::of_recorded(
        entry.authorisations.as_deref().unwrap_or_default(),
    );
    event.justification = event
        .decided
        .as_ref()
        .and_then(|decided| decided.justification.clone());
    labelled(&mut event, entry);
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
fn accessor_and_access(
    event: &mut AuditEvent,
    details: &Details<'_>,
    core: Core<'_>,
) -> Result<(), RecordedError> {
    let text = |detail| details.text(detail);
    event.token = details.json::<TokenAttributes>(Detail::TokenAttributes)?;
    let token_roles = event.token.as_ref().map_or(0, |token| token.roles.len());
    let token_purposes = event.token.as_ref().map_or(0, |token| token.purposes.len());
    event.user_id = match text(Detail::Principal)? {
        Some(principal) => principal,
        None => core.principal().unwrap_or_default(),
    };
    event.issuer = text(Detail::Issuer)?.or_else(|| core.issuer().map(str::to_owned));
    event.client_id = text(Detail::ClientId)?.or_else(|| core.client_id().map(str::to_owned));
    event.token_id = text(Detail::TokenId)?.or_else(|| core.token_id().map(str::to_owned));
    let roles = details.texts(Detail::Role)?;
    event.roles = if roles.is_empty() {
        core.roles(token_roles)
    } else {
        roles
    };
    if let Some(domain) = text(Detail::Domain)? {
        event.domain = access_domain(Some(&domain)).ok_or(RecordedError::Value(Detail::Domain))?;
    }
    event.ehr_id = text(Detail::EhrId)?.or_else(|| core.ehr_id().map(str::to_owned));
    event.subject_lookup_failed = details.flag(Detail::SubjectLookupFailed)?;
    event.review = details.level(Detail::Review)?;
    event.object_id = text(Detail::ObjectId)?.or_else(|| core.object_id().map(str::to_owned));
    event.purpose = text(Detail::Purpose)?.or_else(|| core.purpose(token_purposes));
    event.legal_basis = text(Detail::LegalBasis)?;
    event.result_count = details
        .count(Detail::ResultCount)?
        .or_else(|| core.result_count());
    event.origins = details.texts(Detail::Origin)?;
    event.origin_count = details.count(Detail::OriginCount)?;
    if event.origins.is_empty() {
        event.origins = core.origins();
        event.origin_count = event.origin_count.or_else(|| core.origin_count());
    }
    event.actor.mode = text(Detail::ActingMode)?
        .map(|mode| acting_mode(&mode).ok_or(RecordedError::Value(Detail::ActingMode)))
        .transpose()?;
    event.actor.assurance_level = match text(Detail::AssuranceLevel)? {
        Some(level) => {
            Some(assurance_level(&level).ok_or(RecordedError::Value(Detail::AssuranceLevel))?)
        }
        None => core.assurance_level(),
    };
    event.actor.assurance_value = text(Detail::AssuranceValue)?;
    event.actor.professional_id = text(Detail::ProfessionalId)?;
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

/// The facts of an audit record the core of its entry states, read the same
/// way when the entry is written, to leave the detail out, and when it is
/// read back, to fill it in (#5703). No openEHR spec governs this: our own
/// design.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Core<'a>(pub(crate) &'a AccessLogEntry);

impl<'a> Core<'a> {
    /// The DICOM action the `access action` stands for when it names one
    /// alone: a create or an import is `C`, an update `U`, a delete `D`, a
    /// read, a query or an export `R`.
    fn action(self) -> Option<EventActionCode> {
        let action = &self.0.action;
        if Action::Create.is(action) || Action::Import.is(action) {
            Some(EventActionCode::Create)
        } else if Action::Update.is(action) {
            Some(EventActionCode::Update)
        } else if Action::Delete.is(action) {
            Some(EventActionCode::Delete)
        } else if Action::Read.is(action) || Action::Query.is(action) || Action::Export.is(action) {
            Some(EventActionCode::Read)
        } else {
            None
        }
    }

    /// The outcome a granted access had; any other result names none alone.
    fn outcome(self) -> Option<EventOutcome> {
        Outcome::Granted
            .is(&self.0.outcome.result)
            .then_some(EventOutcome::Success)
    }

    /// The credential the accessor presented.
    fn credential(self) -> Option<&'a AccessCredential> {
        self.0.accessor.credential.as_ref()
    }

    /// The login the credential or the accessing party names.
    fn principal(self) -> Option<String> {
        if let Some(credential) = self.credential() {
            return Some(credential.subject.clone());
        }
        let PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)) =
            &self.0.accessor.party
        else {
            return None;
        };
        party
            .identifiers
            .iter()
            .flat_map(|identifiers| identifiers.iter())
            .find(|identifier| identifier.r#type.as_deref() == Some(LOGIN_TYPE))
            .map(|identifier| identifier.id.clone())
            .filter(|login| login != UNKNOWN_LOGIN)
    }

    /// The issuer the credential names.
    fn issuer(self) -> Option<&'a str> {
        self.credential()
            .map(|credential| credential.issuer.as_str())
    }

    /// The client the credential names.
    fn client_id(self) -> Option<&'a str> {
        self.credential()?.client_id.as_deref()
    }

    /// The token the credential names.
    fn token_id(self) -> Option<&'a str> {
        self.credential()?.token_id.as_deref()
    }

    /// The assurance level the credential names.
    fn assurance_level(self) -> Option<AssuranceLevel> {
        let assurance = self.credential()?.assurance.as_ref()?;
        assurance_level(&assurance.defining_code.code_string)
    }

    /// The roles the caller held: the accessor's roles before the
    /// `token_roles` its token stated.
    fn roles(self, token_roles: usize) -> Vec<String> {
        let roles = self.0.accessor.roles.as_deref().unwrap_or_default();
        let held = roles.len().saturating_sub(token_roles);
        roles
            .iter()
            .take(held)
            .map(|role| role.defining_code.code_string.clone())
            .collect()
    }

    /// The purpose the caller declared beside the `token_purposes` its token
    /// stated: the purpose after them.
    fn purpose(self, token_purposes: usize) -> Option<String> {
        self.0
            .purposes
            .as_deref()
            .unwrap_or_default()
            .get(token_purposes)
            .map(|purpose| purpose.defining_code.code_string.clone())
    }

    /// The object the entry's item names, its version or its local
    /// reference; none for a read of the log, whose item names the log.
    fn object_id(self) -> Option<&'a str> {
        let item = self.0.items.as_deref()?.first()?;
        if item.rm_type.as_deref() == Some(crate::access_log::search::LOG_RECORD) {
            return None;
        }
        if let Some(version) = &item.version_id {
            return Some(version.value());
        }
        match item.object.as_ref()? {
            ObjectRef::ObjectRef(object) if object.r#type != LOG_OBJECT => match &object.id {
                ObjectId::HierObjectId(id) => Some(id.value()),
                ObjectId::GenericId(id) => Some(&id.value),
                _ => None,
            },
            _ => None,
        }
    }

    /// The EHR the entry names.
    fn ehr_id(self) -> Option<&'a str> {
        self.0.ehr_id.as_ref().map(HierObjectId::value)
    }

    /// The origins the entry's item holds, by system id.
    fn origins(self) -> Vec<String> {
        self.0
            .items
            .as_deref()
            .and_then(<[_]>::first)
            .and_then(|item| item.origins.as_deref())
            .unwrap_or_default()
            .iter()
            .map(|origin| origin.system_id.clone())
            .collect()
    }

    /// How many origins the entry's item holds; `None` when it holds none.
    fn origin_count(self) -> Option<u64> {
        let held = self
            .0
            .items
            .as_deref()
            .and_then(<[_]>::first)
            .and_then(|item| item.origins.as_deref())
            .map_or(0, <[_]>::len);
        (held > 0).then(|| u64::try_from(held).unwrap_or(u64::MAX))
    }

    /// Whether the entry's item holds exactly `origins`, a set with a
    /// recorded `count`, so the details need not repeat them.
    fn holds_origins(self, origins: &[String], count: Option<u64>) -> bool {
        !origins.is_empty() && count.is_some() && self.origins() == origins
    }

    /// The count the outcome delivered, or else the rows the query returned.
    fn result_count(self) -> Option<u64> {
        self.0
            .outcome
            .delivered
            .or_else(|| self.0.query.as_ref().and_then(|query| query.rows))
            .and_then(|count| u64::try_from(count).ok())
    }
}

/// Stamps `event` with the confidentiality the security label of `entry`
/// names: the one label an entry carries is the level of the record it
/// reached (#6057).
fn labelled(event: &mut AuditEvent, entry: &AccessLogEntry) {
    // NOTE: our own design, a label of another code system names no level of the record.
    let confidentiality = entry
        .security_labels
        .iter()
        .flatten()
        .find_map(|label| Confidentiality::read(label).ok());
    if confidentiality.is_some() {
        event
            .decided
            .get_or_insert_with(Box::default)
            .confidentiality = confidentiality;
    }
}
