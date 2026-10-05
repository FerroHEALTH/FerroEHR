// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The transport-agnostic audit **event model** for the SM System Log
//! component (`I_SYSTEM_LOG`).
//!
//! NOTE: the vendored SM `I_SYSTEM_LOG` interface is an empty stub
//! (`docs/specs/openehr/SM/docs/UML/classes/i_system_log.adoc` names it with no
//! methods and no description), the only normative statement being the platform
//! overview's "System Log | IHE ATNA-compliant system log"
//! (`docs/specs/openehr/SM/docs/openehr_platform/master02-overview.adoc`
//! §Overview), so this event model is entirely our own design.
//!
//! It is the minimal shape the ITS-REST audit middleware needs to hand a
//! resolved operation record to the platform's ATNA emitter. Nothing here
//! depends on `openehr-its`, HTTP, or DICOM; the DICOM / RFC-3881 renderings of
//! these enums live in [`super::codes`], the wire model in [`super::message`].

use jiff::Timestamp;

/// DICOM `EventActionCode` (PS3.15 §A.5.1): the CRUD/execute class of the op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventActionCode {
    /// `C` — create.
    Create,
    /// `R` — read.
    Read,
    /// `U` — update.
    Update,
    /// `D` — delete.
    Delete,
    /// `E` — execute (query).
    Execute,
}

/// DICOM `EventOutcomeIndicator` (PS3.15 §A.5.1), derived from the HTTP status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOutcome {
    /// `0` — success.
    Success,
    /// `4` — minor failure (the action failed; e.g. 4xx incl. 401/403).
    MinorFailure,
    /// `8` — serious failure (the action was not performed; e.g. 5xx).
    SeriousFailure,
    /// `12` — major failure (the node/service is compromised/unavailable).
    MajorFailure,
}

/// The class of resource an operation touches — determines the DICOM `EventID`
/// and the participant-object rendering (see [`super::codes`] and
/// [`super::message`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectClass {
    /// EHR / `EHR_STATUS` → "Patient Record"; a Patient-Number participant.
    Ehr,
    /// Composition → "composition"; Patient-Number + object-URI participants.
    Composition,
    /// Contribution → "contribution"; Patient-Number + object-URI participants.
    Contribution,
    /// Directory (FOLDER) → "directory"; Patient-Number + object-URI participants.
    Directory,
    /// Ad-hoc / stored query execution → "query"; a Search-Criteria participant.
    Query,
    /// Operational template provisioning (DEFINITION `adl1.4`/`adl2` templates)
    /// → "template"; an object-URI participant (the template id).
    Template,
    /// Demographic party (PERSON / AGENT / ORGANISATION / GROUP / ROLE /
    /// versioned party) → "demographic"; an object-URI participant (the party
    /// uid). Demographic data is person-identifiable, so it is audited under
    /// the Patient-Record `EventID` family.
    Demographic,
    /// EHR-Extract communication (SM-5 export/import of whole-EHR or
    /// spec-driven extracts) → "extract"; Patient-Number + object-URI
    /// participants. Extract communication carries patient-identifiable
    /// clinical data across systems and is audited for **non-repudiation** —
    /// the security chapter requires that "logging of communication of
    /// Extracts … can be used to guarantee non-repudiation of information
    /// passed between systems" (BASE
    /// `architecture_overview/master07-security.adoc` §Non-repudiation).
    // The SM-5 EHR-Extract export/import path emits an
    // `AuditEvent { object: Extract, .. }` on completion
    // (`FerroEhrService::emit_extract_audit`); this variant is its resource class.
    Extract,
    /// Generic application activity (unclassified extension operations) →
    /// "Application Activity"; no clinical object.
    ApplicationActivity,
    /// A user-authentication event (a genuine login, or a rejected 401/403
    /// access attempt) → DICOM "User Authentication" (110114); no clinical
    /// object.
    Authentication,
}

/// The concrete kind of event within its `EventID` family — rendered as the
/// DICOM `EventTypeCode` (DICOM PS3.15 §A.5 `EventIdentification`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    /// DCM 110122 "Login" — a user-authentication attempt (success or failure).
    Login,
    /// DCM 110123 "Logout" — a session end (reserved; no session teardown
    /// surface emits it yet).
    Logout,
    /// The concrete ITS-REST operation, carried as the generated operation id.
    /// NOTE: no external code system governs openEHR REST operations — our own
    /// design/extension: the id is emitted under the `openEHR-ITS-REST` code
    /// system name.
    RestOperation(&'static str),
}

/// A fully-resolved audit event, ready to be rendered into a DICOM
/// `AuditMessage` ([`super::message`]).
#[derive(Debug, Clone)]
pub struct AuditEvent {
    /// The CRUD/execute class.
    pub action: EventActionCode,
    /// The resource class (drives `EventID` + participant objects).
    pub object: ObjectClass,
    /// The concrete event kind within the `EventID` family (the DICOM
    /// `EventTypeCode`): the login marker for authentication records, the
    /// ITS-REST operation id for operation records. `None` for emitters with
    /// no finer classification (e.g. the service-layer extract audit).
    pub event_type: Option<EventType>,
    /// The response outcome.
    pub outcome: EventOutcome,
    /// The requesting user (Basic username / OAuth `sub`; `UNKNOWN` when absent).
    pub user_id: String,
    /// Whether the source participant is the requestor (always true here).
    pub user_is_requestor: bool,
    /// The organisation the requesting user acted for, when the deployment
    /// resolves one from the caller's identity token.
    ///
    /// The ACCESSING organisation, not the reporting site: the audit source's
    /// enterprise site id ([`super::message::AuditContext`]) names the node
    /// that emitted the record and answers a different question. `None` when
    /// no organisation was resolved — never a guess.
    pub organisation: Option<String>,
    /// The roles the requesting user held when the access happened (#3239):
    /// the RFC 9068 §2.2.3.1 claim carriers for a bearer, the user definition
    /// for Basic. NEN 7513 asks for the role or authority under which the
    /// person accessed the record. Empty when no principal was authenticated;
    /// the store records that as NULL, and an authenticated caller with no
    /// roles as the empty array.
    pub roles: Vec<String>,
    /// The client network address (`X-Forwarded-For` first hop / peer), if known.
    pub client_ip: Option<String>,
    /// The owning EHR id, for optional background subject enrichment.
    pub ehr_id: Option<String>,
    /// The resolved object identifier (resource URI / version uid / query name).
    pub object_id: Option<String>,
    /// The bearer token's `jti` claim when the request was Bearer-authenticated
    /// — the minimal token identity the FHIR `AuditEvent` rendering records
    /// (never the token itself; token contents are never logged).
    pub token_id: Option<String>,
    /// Which pseudonymisation domain the operation read or wrote.
    ///
    /// Derived from [`Self::object`] by [`AccessDomain::of`], so a new resource
    /// class cannot reach the trail without an answer.
    pub domain: AccessDomain,
    /// The purpose-of-use code the caller declared, when the deployment
    /// collects one.
    pub purpose: Option<String>,
    /// The legal-basis code the deployment attributes to this access.
    pub legal_basis: Option<String>,
    /// How many records the operation served: AQL result rows, search entries.
    /// `None` for an operation that serves no countable set.
    pub result_count: Option<u64>,
    /// The distinct origins of the data the operation served (EHDS Annex II
    /// 3.2(e), #3212): the `FEEDER_AUDIT` originating systems of the version
    /// bodies it touched, or the server that created them; capped at the
    /// record cap (`versioning::origins::RECORD_CAP`), with
    /// [`Self::origin_count`] carrying the true distinct number. Empty when
    /// nothing was served.
    pub origins: Vec<String>,
    /// How many distinct origins the served data had; exceeds `origins.len()`
    /// when the set was capped. `None` when nothing was served.
    pub origin_count: Option<u64>,
    /// The request correlation id, matching the response's `x-request-id`.
    pub request_id: Option<String>,
    /// The event time.
    pub timestamp: Timestamp,
}

/// The pseudonymisation domain an audited operation touched.
///
/// The clinical and demographic domains are separate schemas under separate
/// database roles, so an access record that does not say which one it read
/// cannot answer "who saw this person's identifying data" separately from "who
/// read this record". NEN 7513 and EHDS Art. 9 both ask the question per
/// record; the domain is what makes the answer separable.
///
/// **No openEHR spec governs this — our own design/extension.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessDomain {
    /// The `clinical` schema: EHRs, compositions, folders, contributions,
    /// extracts and the queries that read them.
    Ehr,
    /// The `party` schema: parties and their change control.
    Demographic,
    /// The `linkage` schema: the party-to-EHR resolve map.
    Linkage,
    /// An operation that touches no pseudonymisation domain: authentication,
    /// template provisioning, node management.
    System,
}

impl AccessDomain {
    /// The domain an operation on this resource class reads or writes.
    ///
    /// Exhaustive over [`ObjectClass`] on purpose: adding a resource class is
    /// then a compile error here, which is where the question belongs.
    #[must_use]
    pub fn of(object: ObjectClass) -> Self {
        match object {
            ObjectClass::Ehr
            | ObjectClass::Composition
            | ObjectClass::Contribution
            | ObjectClass::Directory
            | ObjectClass::Query
            | ObjectClass::Extract => AccessDomain::Ehr,
            ObjectClass::Demographic => AccessDomain::Demographic,
            ObjectClass::Template
            | ObjectClass::ApplicationActivity
            | ObjectClass::Authentication => AccessDomain::System,
        }
    }

    /// The stored spelling, matching the `ck_audit_event_domain` constraint.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AccessDomain::Ehr => "clinical",
            AccessDomain::Demographic => "party",
            AccessDomain::Linkage => "linkage",
            AccessDomain::System => "system",
        }
    }
}

impl AuditEvent {
    /// Record the origins of the served data, capped for the record with the
    /// true distinct count beside the set (`total` when the caller already
    /// aggregated a wider set, else the set's own distinct size).
    pub fn record_origins(&mut self, origins: Vec<String>, total: Option<u64>) {
        if origins.is_empty() {
            return;
        }
        let (set, distinct) = crate::versioning::origins::recorded(origins);
        self.origin_count = Some(total.map_or(distinct, |t| t.max(distinct)));
        self.origins = set;
    }

    /// Construct an event with the given class + outcome, defaulting the runtime
    /// fields (the caller fills user/ip/ids). `timestamp` is set to now.
    #[must_use]
    pub fn new(action: EventActionCode, object: ObjectClass, outcome: EventOutcome) -> Self {
        Self {
            action,
            object,
            event_type: None,
            outcome,
            user_id: String::new(),
            user_is_requestor: true,
            organisation: None,
            roles: Vec::new(),
            client_ip: None,
            ehr_id: None,
            object_id: None,
            token_id: None,
            domain: AccessDomain::of(object),
            purpose: None,
            legal_basis: None,
            result_count: None,
            origins: Vec::new(),
            origin_count: None,
            request_id: None,
            timestamp: Timestamp::now(),
        }
    }
}

/// The result of enqueuing an event onto the system log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitOutcome {
    /// Successfully enqueued for the drain task.
    Enqueued,
    /// Dropped (queue full / drain gone) under `fail_mode=open`.
    Dropped,
    /// Rejected (queue full / drain gone) under `fail_mode=closed` — the REST
    /// layer must return `503`.
    Rejected,
}
