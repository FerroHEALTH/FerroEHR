// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one recording path's last step: the draft `ACCESS_LOG_ENTRY` an access
//! record writes (#5126).
//!
//! Every access the platform records reaches the system log as one
//! [`AuditEvent`], from the REST middleware or from the service layer, and the
//! drain turns each event that is an access to EHR data into one entry,
//! written in the transaction that stores the event's own record
//! ([`crate::system_log::store::AuditStore::insert_batch`]). An access that
//! reaches several EHRs already arrives as one event per EHR, joined by the
//! request id, so it writes one entry per EHR (IHE BALP 1:52.4.1.6,
//! `docs/specs/ihe-balp/`). A refusal is an entry like any other, and so is a
//! read of the access log itself (DICOM PS3.15 A.5.3.2 "Audit Log Used").
//!
//! An entry carries no data value of the EHR: items name objects and versions,
//! and a query is named by its stored name or the digest of its text (GDPR Art
//! 5(1)(c), `docs/law/eu/gdpr/text.html`). The draft is FerroEHR's
//! change proposal and no openEHR release, so no openEHR spec governs this
//! module: our own design.

use jiff::civil::Date;
use openehr_base::containers::NonEmptyVec;
use openehr_base::v1_3::base_types::identification::generic_id::GenericId;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::{ObjectRef, ObjectRefData};
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
use openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_ACCESS_CONTROL;
use openehr_rm::v1_2::common::generic::party_identified::{PartyIdentified, PartyIdentifiedData};
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use openehr_rm::v1_2::common::generic::party_self::PartySelf;
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date::DvDate;
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use openehr_rm::v1_2::data_types::text::dv_text::{DvText, DvTextData};

use crate::access_log::search::LOG_RECORD;
use crate::system_log::binding::{BindingCode, BindingRecord, Identified};
use crate::system_log::event::{
    AccessDecision, AccessDomain, ActingMode, AuditEvent, EntryScope, EventActionCode,
    EventOutcome, EventType, OPERATION_SYSTEM, ObjectClass,
};
#[cfg(feature = "fhir")]
use crate::system_log::message::AuditContext;
#[cfg(feature = "fhir")]
use crate::system_log::store::RecordEnd;
use crate::versioning::audit::dv_date_time;
use openehr_rm::draft::access_log::access_log::access_authorisation::AccessAuthorisation;
use openehr_rm::draft::access_log::access_log::access_channel::AccessChannel;
use openehr_rm::draft::access_log::access_log::access_credential::AccessCredential;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_link::AccessLogLink;
use openehr_rm::draft::access_log::access_log::access_origin::AccessOrigin;
use openehr_rm::draft::access_log::access_log::access_outcome::AccessOutcome;
use openehr_rm::draft::access_log::access_log::access_query::AccessQuery;
use openehr_rm::draft::access_log::access_log::access_retention::AccessRetention;
use openehr_rm::draft::access_log::access_log::accessed_item::AccessedItem;
use openehr_rm::draft::access_log::access_log::accessor::Accessor;

/// The terminology of the draft's own groups (`access action`, `access
/// outcome`, `accessor kind`, `access authorisation`), whose terms carry
/// their rubric as the code until the openEHR terminology editors assign one
/// (`openehr_rm::draft::access_log::invariants`).
const OPENEHR: &str = "openehr";
/// FerroEHR's terminology of the party binding outcomes (#5007).
pub const BINDING_TERMINOLOGY: &str = "ferroehr-party-binding";
/// FerroEHR's terminology of the retention grounds an entry states.
pub const RETENTION_TERMINOLOGY: &str = "ferroehr-access-log-retention";
/// The terminology of a code a caller declared with no system.
const LOCAL: &str = "local";
/// The class an entry that changes the access log names the entry it changed
/// by.
const LOG_ENTRY: &str = "ACCESS_LOG_ENTRY";
/// The type an entry that reads an EHR's whole access log names the log by:
/// our own design, since the draft names no class for the log itself.
const LOG: &str = "ACCESS_LOG";
/// The `ACCESS_QUERY.language` of a read of the access log through ITI-81.
pub const LOG_SEARCH: &str = "FHIR search";

/// Whether `event` writes an access-log entry: an access to EHR data, a
/// refusal no operation record of its request covers, a read of the log, or a
/// change to the log or to what it means.
#[must_use]
pub fn writes_entry(event: &AuditEvent) -> bool {
    match event.entry {
        EntryScope::ByDomain => event.domain == AccessDomain::Ehr,
        EntryScope::Refusal
        | EntryScope::LogRead
        | EntryScope::LogChange
        | EntryScope::LogMeaning => true,
        EntryScope::Written => false,
    }
}

/// The access-log entry `event` writes, recorded by `system_id` and kept
/// under `retention`; `None` when it writes none ([`writes_entry`]).
///
/// `subject` is the subject of care of the event's EHR, when the drain found
/// one; the entry keeps it with the other facts its projections carry in
/// `other_details` ([`crate::access_log::recorded::details`]). The entry's
/// link is a placeholder the store replaces, since the chain decides a
/// record's position and digests.
#[must_use]
pub fn entry_of(
    event: &AuditEvent,
    subject: Option<&str>,
    system_id: &str,
    retention: AccessRetention,
) -> Option<AccessLogEntry> {
    if !writes_entry(event) {
        return None;
    }
    let ehr_id = ehr_id(event);
    let items = ehr_id
        .as_ref()
        .and_then(|_| item(event))
        .map(|item| vec![item]);
    Some(AccessLogEntry {
        uid: HierObjectId::from(uuid::Uuid::now_v7()),
        system_id: system_id.to_owned(),
        time_recorded: dv_date_time(&event.timestamp),
        link: AccessLogLink {
            sequence: 0,
            algorithm: String::new(),
            previous_digest: String::new(),
            content_digest: String::new(),
            digest: String::new(),
            signature: None,
        },
        occurred: None,
        action: coded(OPENEHR, action(event), action(event)),
        operation: operation(event),
        outcome: outcome(event),
        accessor: accessor(event, system_id),
        ehr_id,
        items,
        query: query(event),
        purposes: nonempty(
            event
                .purpose_codings()
                .into_iter()
                .map(|purpose| {
                    let system = purpose.system.as_deref().unwrap_or(LOCAL);
                    coded(system, &purpose.code, &purpose.code)
                })
                .collect(),
        ),
        authorisations: nonempty(authorisations(event)),
        is_emergency: event.emergency_access,
        channel: channel(event),
        security_labels: None,
        retention,
        other_details: details(event, subject),
    })
}

/// The access-log entry `event` writes as its record, its EHR belonging to
/// the subject of care `subject` when the drain found one.
///
/// The audit source records it. A classified access is kept for the period
/// its categories and origins call for
/// ([`crate::access_log::retention::retention`]), any other until the end its
/// record has ([`crate::system_log::fhir::record_end`]).
#[cfg(feature = "fhir")]
#[must_use]
pub fn for_store(
    event: &AuditEvent,
    subject: Option<&str>,
    ctx: &AuditContext,
) -> Option<Box<AccessLogEntry>> {
    if !writes_entry(event) {
        return None;
    }
    let retention = match &event.category {
        Some(category) => crate::access_log::retention::retention(
            &ctx.recording.retention,
            event.timestamp,
            &category.classification,
            event.origins.iter().map(String::as_str),
        ),
        None => horizon_retention(match crate::system_log::fhir::record_end(event, ctx) {
            RecordEnd::On(date) => Some(date),
            RecordEnd::Never => None,
        }),
    };
    entry_of(event, subject, &ctx.source_id, retention).map(Box::new)
}

/// A `DV_CODED_TEXT` of `code` in `terminology`, displayed as `rubric`.
fn coded(terminology: &str, code: &str, rubric: &str) -> DvCodedText {
    DvCodedText {
        value: rubric.to_owned(),
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
        defining_code: CodePhrase {
            terminology_id: TerminologyId {
                value: terminology.to_owned(),
            },
            code_string: code.to_owned(),
            preferred_term: None,
        },
    }
}

/// `values`, or `None` when there are none, as the draft's optional lists are
/// written.
fn nonempty<T>(values: Vec<T>) -> Option<Vec<T>> {
    (!values.is_empty()).then_some(values)
}

/// The EHR the access addressed. A route segment that is no object id names
/// no EHR, so the entry names none.
fn ehr_id(event: &AuditEvent) -> Option<HierObjectId> {
    // NOTE: BASE `HIER_OBJECT_ID`, a route segment outside its form addresses
    // no EHR, which is legitimately absent.
    event
        .ehr_id
        .as_deref()
        .and_then(|id| HierObjectId::new(id).ok())
}

/// The rubric of the `access action` the event performed.
fn action(event: &AuditEvent) -> &'static str {
    if event.entry == EntryScope::LogRead {
        return "read";
    }
    match (event.object, event.action) {
        (ObjectClass::Extract, EventActionCode::Create) => "import",
        (ObjectClass::Extract, _) => "export",
        (ObjectClass::Query, EventActionCode::Read | EventActionCode::Execute) => "query",
        (_, EventActionCode::Read | EventActionCode::Execute) if event.query.is_some() => "query",
        (_, EventActionCode::Create) => "create",
        (_, EventActionCode::Update) => "update",
        (_, EventActionCode::Delete) => "delete",
        (_, EventActionCode::Read | EventActionCode::Execute) => "read",
    }
}

/// The ITS-REST operation the access ran, in the operation code system the
/// FHIR rendering names it by.
fn operation(event: &AuditEvent) -> Option<DvCodedText> {
    match &event.event_type {
        Some(EventType::RestOperation(op)) => Some(coded(OPERATION_SYSTEM, op, op)),
        Some(EventType::Login | EventType::Logout) | None => None,
    }
}

/// The access outcome: granted, refused, or failed for any other failure.
fn outcome(event: &AuditEvent) -> AccessOutcome {
    let result = if event.decision == AccessDecision::Refused {
        "refused"
    } else if event.outcome == EventOutcome::Success {
        "granted"
    } else {
        "failed"
    };
    let delivered = (event.outcome == EventOutcome::Success
        && event.decision == AccessDecision::Admitted)
        .then_some(event.result_count)
        .flatten()
        .map(|count| i32::try_from(count).unwrap_or(i32::MAX));
    let refused = event
        .decided
        .as_ref()
        .and_then(|decided| decided.refused.as_ref());
    AccessOutcome {
        result: coded(OPENEHR, result, result),
        reason: refused.map(|refused| {
            coded(
                TERMINOLOGY_ID_ACCESS_CONTROL,
                refused.code(),
                refused.code(),
            )
        }),
        description: refused.map(|refused| {
            DvText::DvText(DvTextData {
                value: refused.description(),
                hyperlink: None,
                formatting: None,
                mappings: None,
                language: None,
                encoding: None,
            })
        }),
        delivered,
    }
}

/// Who accessed, for whom, and how they authenticated.
fn accessor(event: &AuditEvent, system_id: &str) -> Accessor {
    let binding = event.binding.as_ref();
    let issuer = event.issuer.as_deref().unwrap_or(system_id);
    Accessor {
        kind: coded(OPENEHR, accessor_kind(event), accessor_kind(event)),
        party: party(event, issuer),
        provider: provider(event),
        roles: nonempty(
            event
                .role_codings()
                .into_iter()
                .map(|role| {
                    coded(
                        role.system.as_deref().unwrap_or(LOCAL),
                        &role.code,
                        &role.code,
                    )
                })
                .collect(),
        ),
        groups: binding.and_then(|binding| nonempty(groups(binding))),
        binding: binding.map(|binding| {
            coded(
                BINDING_TERMINOLOGY,
                binding.code.as_str(),
                binding.code.as_str(),
            )
        }),
        credential: credential(event),
        delegation: binding.and_then(|binding| {
            nonempty(
                binding
                    .actor_chain
                    .iter()
                    .map(|actor| AccessCredential {
                        issuer: issuer.to_owned(),
                        subject: actor.clone(),
                        client_id: None,
                        token_id: None,
                        assurance: None,
                        authentication_method: None,
                    })
                    .collect(),
            )
        }),
    }
}

/// The rubric of the `accessor kind`: from the binding when it says, else
/// from how the caller acted.
fn accessor_kind(event: &AuditEvent) -> &'static str {
    if let Some(binding) = &event.binding {
        match binding.code {
            BindingCode::SelfPatient => return "subject of care",
            BindingCode::External => return "external person",
            BindingCode::Organisation => return "organisation",
            BindingCode::System => return "system",
            _ if binding.representation.is_some() => return "representative",
            _ => {}
        }
    }
    match event.actor.mode {
        Some(ActingMode::Delegated) => "delegated person",
        Some(ActingMode::External) => "external person",
        Some(ActingMode::System | ActingMode::Client) => "system",
        Some(ActingMode::Person) | None => "person",
    }
}

/// The accessing party: the subject of care as `PARTY_SELF`, a bound party by
/// its reference, anyone else by the login the credential names.
fn party(event: &AuditEvent, issuer: &str) -> PartyProxy {
    let binding = event.binding.as_ref();
    if binding.is_some_and(|binding| binding.code == BindingCode::SelfPatient) {
        return PartyProxy::PartySelf(PartySelf { external_ref: None });
    }
    if let Some(party) = binding.and_then(|binding| binding.party.as_ref()) {
        return PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(
            PartyIdentifiedData {
                external_ref: Some(party_ref(&party.namespace, &party.class, &party.id)),
                name: None,
                identifiers: None,
            },
        ));
    }
    let login = if event.user_id.is_empty() {
        "UNKNOWN"
    } else {
        event.user_id.as_str()
    };
    PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(PartyIdentifiedData {
        external_ref: None,
        name: None,
        identifiers: Some(NonEmptyVec::of(DvIdentifier {
            issuer: Some(issuer.to_owned()),
            assigner: None,
            id: login.to_owned(),
            r#type: Some("login".to_owned()),
        })),
    }))
}

/// A `PARTY_REF` to the party `id` of class `class` in `namespace`.
fn party_ref(namespace: &str, class: &str, id: &str) -> PartyRef {
    PartyRef {
        namespace: namespace.to_owned(),
        r#type: class.to_owned(),
        id: object_id(id),
    }
}

/// `id` as a `HIER_OBJECT_ID` when it reads as one, else a local
/// `GENERIC_ID`.
fn object_id(id: &str) -> ObjectId {
    // NOTE: BASE `GENERIC_ID`, a party id outside the `HIER_OBJECT_ID` form is
    // legitimately a local generic id.
    HierObjectId::new(id).map_or_else(
        |_| {
            ObjectId::GenericId(GenericId {
                value: id.to_owned(),
                scheme: LOCAL.to_owned(),
            })
        },
        ObjectId::HierObjectId,
    )
}

/// The `GROUP` parties the binding matched.
fn groups(binding: &BindingRecord) -> Vec<PartyRef> {
    let namespace = binding
        .party
        .as_ref()
        .map_or(LOCAL, |party| party.namespace.as_str());
    binding
        .groups
        .iter()
        .map(|group| party_ref(namespace, "GROUP", group))
        .collect()
}

/// The provider the caller acted for: the binding's, else the organisation
/// the token named.
fn provider(event: &AuditEvent) -> Option<PartyIdentified> {
    let identified = |identifier: DvIdentifier| {
        PartyIdentified::PartyIdentified(PartyIdentifiedData {
            external_ref: None,
            name: None,
            identifiers: Some(NonEmptyVec::of(identifier)),
        })
    };
    if let Some(Identified { issuer, id }) = event
        .binding
        .as_ref()
        .and_then(|binding| binding.provider.as_ref())
    {
        return Some(identified(DvIdentifier {
            issuer: Some(issuer.clone()),
            assigner: None,
            id: id.clone(),
            r#type: Some("provider".to_owned()),
        }));
    }
    event.organisation.as_ref().map(|organisation| {
        PartyIdentified::PartyIdentified(PartyIdentifiedData {
            external_ref: None,
            name: Some(organisation.clone()),
            identifiers: None,
        })
    })
}

/// The credential the caller presented, named by its issuer, subject, client
/// and token id, never the token; `None` without an issuer and a subject.
fn credential(event: &AuditEvent) -> Option<AccessCredential> {
    let issuer = event
        .issuer
        .as_deref()
        .filter(|issuer| !issuer.is_empty())?;
    (!event.user_id.is_empty()).then(|| AccessCredential {
        issuer: issuer.to_owned(),
        subject: event.user_id.clone(),
        client_id: event.client_id.clone(),
        token_id: event.token_id.clone(),
        assurance: event
            .actor
            .assurance_level
            .map(|level| coded(LOCAL, level.as_str(), level.as_str())),
        authentication_method: None,
    })
}

/// The object the access addressed in its EHR, with its categories and
/// origins; `None` when the record names nothing the draft can name.
fn item(event: &AuditEvent) -> Option<AccessedItem> {
    let rm_type = match event.entry {
        EntryScope::LogChange => Some(LOG_ENTRY),
        EntryScope::LogRead => Some(LOG_RECORD),
        _ => rm_type(event.object),
    };
    let addressed = event.object_id.as_deref();
    // NOTE: BASE `OBJECT_VERSION_ID`, an id outside its form names a versioned
    // object, which is legitimately no version.
    let version_id = addressed.and_then(|id| ObjectVersionId::new(id).ok());
    let object = match (&version_id, addressed, rm_type) {
        (None, Some(id), Some(rm_type)) => Some(local_ref(rm_type, id)),
        // NOTE: our own design, a read of an EHR's whole log names the log of that EHR.
        (None, None, _) if event.entry == EntryScope::LogRead => {
            event.ehr_id.as_deref().map(|ehr| local_ref(LOG, ehr))
        }
        _ => None,
    };
    let categories = event
        .category
        .as_ref()
        .map(|category| {
            category
                .classification
                .categories()
                .keys()
                .map(|category| coded(category.system(), category.code(), category.code()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if object.is_none() && version_id.is_none() && categories.is_empty() {
        return None;
    }
    Some(AccessedItem {
        object,
        version_id,
        rm_type: rm_type.map(str::to_owned),
        archetype_id: None,
        template_id: None,
        paths: None,
        categories: nonempty(categories),
        origins: nonempty(
            event
                .origins
                .iter()
                .map(|origin| AccessOrigin {
                    system_id: origin.clone(),
                    endpoint: None,
                    location: None,
                })
                .collect(),
        ),
        was_delivered: event.outcome == EventOutcome::Success
            && event.decision == AccessDecision::Admitted
            && matches!(
                event.action,
                EventActionCode::Read | EventActionCode::Execute
            ),
    })
}

/// A local `OBJECT_REF` to the object `id` of class `class`.
fn local_ref(class: &str, id: &str) -> ObjectRef {
    ObjectRef::ObjectRef(ObjectRefData {
        namespace: LOCAL.to_owned(),
        r#type: class.to_owned(),
        id: object_id(id),
    })
}

/// The RM class of an object of `class`; `None` for one outside the EHR.
const fn rm_type(class: ObjectClass) -> Option<&'static str> {
    match class {
        ObjectClass::Ehr => Some("EHR"),
        ObjectClass::Composition => Some("COMPOSITION"),
        ObjectClass::Contribution => Some("CONTRIBUTION"),
        ObjectClass::Directory => Some("FOLDER"),
        ObjectClass::Extract => Some("EXTRACT"),
        ObjectClass::Query
        | ObjectClass::Template
        | ObjectClass::Demographic
        | ObjectClass::ApplicationActivity
        | ObjectClass::Authentication => None,
    }
}

/// The query the access ran, named and never quoted, with the rows it served.
fn query(event: &AuditEvent) -> Option<AccessQuery> {
    event.query.as_ref().map(|naming| AccessQuery {
        stored_query: naming.stored.clone(),
        language: Some(naming.language.to_owned()),
        text_digest: Some(naming.text_digest.clone()),
        text: None,
        rows: event
            .result_count
            .map(|rows| i32::try_from(rows).unwrap_or(i32::MAX)),
    })
}

/// A ground of the draft `access authorisation` group whose rubric is
/// `rubric`, naming nothing else.
#[must_use]
pub(crate) fn authorisation(rubric: &str) -> AccessAuthorisation {
    AccessAuthorisation {
        kind: coded(OPENEHR, rubric, rubric),
        policy: None,
        name: None,
        version: None,
        reference: None,
        legal_grounds: None,
    }
}

/// The authorisations the access ran under: the decision grounds the access
/// decision gave, then the agreement, the representation, the data permit and
/// the emergency the request carried, unless a decision ground marks it.
fn authorisations(event: &AuditEvent) -> Vec<AccessAuthorisation> {
    let mut authorisations = event
        .decided
        .as_ref()
        .map(|decided| decided.grounds.clone())
        .unwrap_or_default();
    let kind = authorisation;
    let binding = event.binding.as_ref();
    if let Some(agreement) = binding.and_then(|binding| binding.agreement.as_ref()) {
        authorisations.push(AccessAuthorisation {
            name: Some(agreement.name.clone()),
            version: Some(agreement.version.clone()),
            legal_grounds: agreement
                .legal_ground
                .as_ref()
                .map(|ground| vec![coded(LOCAL, ground, ground)]),
            ..kind("agreement")
        });
    }
    if let Some(representation) = binding.and_then(|binding| binding.representation.as_ref()) {
        authorisations.push(AccessAuthorisation {
            reference: Some(identifier(&representation.represented)),
            ..kind("representation")
        });
    }
    if let Some(permit) = &event.permit {
        authorisations.push(AccessAuthorisation {
            reference: Some(identifier(permit)),
            ..kind("permit")
        });
    }
    let marked = |rubric: &str| {
        authorisations
            .iter()
            .any(|ground| ground.kind.value == rubric)
    };
    if event.emergency_access && !marked("emergency") {
        authorisations.push(kind("emergency"));
    }
    authorisations
}

/// A `DV_IDENTIFIER` holding `id` alone.
fn identifier(id: &str) -> DvIdentifier {
    DvIdentifier {
        issuer: None,
        assigner: None,
        id: id.to_owned(),
        r#type: None,
    }
}

/// The request the access arrived in; `None` when the record names neither
/// its request id nor its client address.
fn channel(event: &AuditEvent) -> Option<AccessChannel> {
    (event.request_id.is_some() || event.client_ip.is_some()).then(|| AccessChannel {
        interface: None,
        request_id: event.request_id.clone(),
        correlation_id: event
            .binding
            .as_ref()
            .and_then(|binding| binding.agreement.as_ref())
            .and_then(|agreement| agreement.correlation_id.clone()),
        client_address: event.client_ip.clone(),
    })
}

/// The `ACCESS_RETENTION` of an entry the audit store's horizon keeps: kept
/// through the day before `end`, the first date it may be removed on, or
/// without end when `end` is `None`.
#[must_use]
pub fn horizon_retention(end: Option<Date>) -> AccessRetention {
    AccessRetention {
        retain_until: end.and_then(|end| end.yesterday().ok()).map(|last| DvDate {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            value: last.to_string(),
        }),
        grounds: NonEmptyVec::of(coded(
            RETENTION_TERMINOLOGY,
            "store-horizon",
            "the retention horizon of the audit store",
        )),
    }
}

/// FerroEHR's access details of the access `event` records, which its
/// projections read ([`crate::access_log::recorded::details`]).
#[cfg(feature = "fhir")]
fn details(
    event: &AuditEvent,
    subject: Option<&str>,
) -> Option<openehr_rm::v1_2::data_structures::item_structure::item_structure::ItemStructure> {
    crate::access_log::recorded::details(event, subject)
}

/// A slim build projects no entry, so its entries keep no access details.
#[cfg(not(feature = "fhir"))]
fn details(
    _event: &AuditEvent,
    _subject: Option<&str>,
) -> Option<openehr_rm::v1_2::data_structures::item_structure::item_structure::ItemStructure> {
    None
}

#[cfg(test)]
mod tests;
