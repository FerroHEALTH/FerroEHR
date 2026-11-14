// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The transport-agnostic audit **event model** for the system log.
//!
//! NOTE: no openEHR specification governs this: our own design, shaped
//! for the IHE ATNA record the platform emits.
//!
//! It is the minimal shape the ITS-REST audit middleware needs to hand a
//! resolved operation record to the platform's ATNA emitter. Nothing here
//! depends on `openehr-its`, HTTP, or DICOM; the DICOM / RFC-3881 renderings of
//! these enums live in [`super::codes`], the wire model in [`super::message`].

use std::borrow::Cow;

use jiff::Timestamp;

use ferrohealth_smart::assurance::AssuranceLevel;

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
    /// EHR-Extract communication (export/import of whole-EHR or
    /// spec-driven extracts) → "extract"; Patient-Number + object-URI
    /// participants. Extract communication carries patient-identifiable
    /// clinical data across systems and is audited for **non-repudiation** —
    /// the security chapter requires that "logging of communication of
    /// Extracts … can be used to guarantee non-repudiation of information
    /// passed between systems" (BASE
    /// `architecture_overview/master07-security.adoc` §Non-repudiation).
    // The EHR-Extract export/import path emits an
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

/// The code system of the ITS-REST operation ids a record writes as its
/// operation subtype.
// NOTE: no specification governs this: our own design; no external code system names the
// openEHR REST operations, and the DICOM rendering names them `openEHR-ITS-REST`.
pub const OPERATION_SYSTEM: &str = "urn:openehr:its-rest:operation";

/// The concrete kind of event within its `EventID` family — rendered as the
/// DICOM `EventTypeCode` (DICOM PS3.15 §A.5 `EventIdentification`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventType {
    /// DCM 110122 "Login" — a user-authentication attempt (success or failure).
    Login,
    /// DCM 110123 "Logout" — a session end (reserved; no session teardown
    /// surface emits it yet).
    Logout,
    /// The concrete ITS-REST operation, carried as the generated operation id,
    /// or as the stored text when a record is read back from the store.
    /// NOTE: no external code system governs openEHR REST operations — our own
    /// design/extension: the id is emitted under the `openEHR-ITS-REST` code
    /// system name.
    RestOperation(Cow<'static, str>),
}

impl EventType {
    /// The event of the ITS-REST operation `op`.
    #[must_use]
    pub const fn rest(op: &'static str) -> Self {
        EventType::RestOperation(Cow::Borrowed(op))
    }
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
    /// The issuer of the bearer token that authenticated the user (`iss`,
    /// RFC 7519 §4.1.1); `None` for a Basic-authenticated or anonymous caller.
    pub issuer: Option<String>,
    /// The client application the bearer token was issued to (`client_id`,
    /// RFC 9068 §2.2, or `azp`); `None` when the token names none.
    pub client_id: Option<String>,
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
    /// The client network address: the peer, or the hop a `[server]
    /// trusted_proxies` peer forwarded, when the request came over a connection.
    pub client_ip: Option<String>,
    /// The owning EHR id, for optional background subject enrichment.
    pub ehr_id: Option<String>,
    /// The namespace of the EHR subject's id, as `EHR_STATUS.subject` names
    /// it, filled by the drain beside the subject it resolves.
    pub subject_namespace: Option<String>,
    /// Whether the drain could not learn the EHR's subject: the resolver
    /// could not ask the database, or did not answer.
    pub subject_lookup_failed: bool,
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
    /// The identifier of the data permit the caller declared it acts under
    /// (`[audit] permit_header`), when it declared one.
    pub permit: Option<String>,
    /// Whether the declared purpose is one of the deployment's emergency
    /// purpose codes (`[audit] emergency_purpose_codes`): an access declared
    /// as necessary to protect the vital interests of the data subject
    /// (`docs/law/eu/ehds/text.html Art. 11(5)`), which the access log shows
    /// the person. Stamped by the sender at emit, so every emitter carries it.
    /// The mark records the declaration; it lifts no restriction.
    pub emergency_access: bool,
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
    /// The EHDS priority categories of the data the operation served or wrote
    /// (Annex II 3.2(c), #3621), classified at access through the
    /// `[audit.categories]` map; `None` for an operation that touched no
    /// clinical content.
    pub category: Option<crate::system_log::categories::CategoryRecord>,
    /// The digest of the category map in force, stamped on every record by the
    /// sender, so a record says which map classified it.
    pub category_map_digest: Option<crate::system_log::categories::MapDigest>,
    /// How the accessing person authenticated, and who that person is when
    /// the token says so (EHDS Annex II 3.1 and 3.2(b), #3622).
    pub actor: ActorAuthentication,
    /// The server's request id, which the request log names and the
    /// response's `x-request-id` carries unless the client sent its own.
    pub request_id: Option<String>,
    /// The outcome of the party binding and its fields (EHDS Annex II 3.2(a)
    /// and (b)); `None` when no caller was authenticated.
    pub binding: Option<crate::system_log::binding::BindingRecord>,
    /// For a summary record, the count of the refusals it stands for and the
    /// EHR ids they named; `None` on every other record.
    pub summary: Option<crate::system_log::binding::Summary>,
    /// What the access token states about the professional under its
    /// issuer's `[[auth.attributes]]` table: the purposes of use and the
    /// roles (IHE ITI TF-2 §3.71.4.2.2.1.1, RFC 9396 §2); `None` for a token
    /// of an issuer without one, or no token.
    pub token: Option<TokenAttributes>,
    /// Whether the access was admitted or refused (a `401`, a `403`, a
    /// caller above its rate, a security event); the access-log entry
    /// records a refusal as `refused` rather than `failed`.
    pub decision: AccessDecision,
    /// Whether, and as what, this record also writes an openEHR access-log
    /// entry (`crate::access_log::entry`).
    pub entry: EntryScope,
    /// The query the access ran, named and never quoted: a stored query by
    /// its name, an ad-hoc one by the digest of its text.
    pub query: Option<QueryNaming>,
    /// What the access evaluator decided ([`crate::access_eval::record`]):
    /// the grounds the entry's `authorisations` carry, the versions of the
    /// policies they are defined in, which the `ehds-logging` record writes
    /// after the data permit, and the refusal with its code, the absent
    /// answers of an opt-out or a restriction included; `None` for an access
    /// the evaluator did not judge.
    pub decided: Option<Box<crate::access_eval::record::Decision>>,
    /// The event time.
    pub timestamp: Timestamp,
}

/// One coded value the access token states, a FHIR `Coding` reduced to its
/// system and code.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Coding {
    /// The code system, when the token names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// The code.
    pub code: String,
}

/// The purposes of use and the professional roles an access token states,
/// as the `token_attributes` column stores them.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenAttributes {
    /// The purposes of use, sorted and without duplicates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub purposes: Vec<Coding>,
    /// The professional's roles (the IUA `subject_role`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<Coding>,
}

impl TokenAttributes {
    /// Whether the token states a purpose whose code is `code`.
    #[must_use]
    pub fn states_purpose(&self, code: &str) -> bool {
        self.purposes.iter().any(|purpose| purpose.code == code)
    }
}

/// How the credential acts: a natural person, a client application for one, a
/// verified external accessor, or a client acting for nobody.
///
/// **No openEHR spec governs this — our own design/extension.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActingMode {
    /// The token names a natural person.
    Person,
    /// The token names a client application, acting for the professional in
    /// [`ActorAuthentication::professional_id`] when it names one.
    Client,
    /// A person reaches the server through a declared client or an RFC 8693
    /// actor, named in [`ActorAuthentication::professional_id`].
    Delegated,
    /// A verified external person or organisation of a `federation` issuer.
    External,
    /// A client acting for nobody.
    System,
}

impl ActingMode {
    /// The stored spelling, matching the `ck_audit_event_acting_mode`
    /// constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ActingMode::Person => "person",
            ActingMode::Client => "client",
            ActingMode::Delegated => "delegated",
            ActingMode::External => "external",
            ActingMode::System => "system",
        }
    }
}

/// How the accessing person authenticated and who that person is, as far as
/// the credential says (EHDS Annex II 3.1 and 3.2(b),
/// `docs/law/eu/ehds/text.html Annex II 3`).
///
/// Every field is `None` when the deployment configures nothing that reads it:
/// the mode and the professional need a caller its issuer classifies as a
/// person or a delegate, the level needs `[authz.assurance] levels`. **No openEHR spec governs this — our
/// own design/extension.**
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActorAuthentication {
    /// Whether a person or a client application presented the credential.
    pub mode: Option<ActingMode>,
    /// The assurance level the credential's claim value maps to.
    pub assurance_level: Option<AssuranceLevel>,
    /// The raw claim value the level was read from (usually `acr`).
    pub assurance_value: Option<String>,
    /// The identifier of the natural person who accessed the data: the
    /// person's own claim, or the professional a client token acts for.
    pub professional_id: Option<String>,
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
    /// A crossing between the demographic and the clinical domain: a party
    /// or a named subject resolved to its EHRs.
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
    /// Whether `test` holds for the code of a purpose of use the record names:
    /// the declared one, or one the access token states.
    pub fn names_purpose(&self, mut test: impl FnMut(&str) -> bool) -> bool {
        self.purpose.as_deref().is_some_and(&mut test)
            || self
                .token
                .iter()
                .flat_map(|token| token.purposes.iter())
                .any(|purpose| test(&purpose.code))
    }

    /// The purposes of use the record renders: those the access token states,
    /// then the declared one when the token states no purpose of its code.
    #[must_use]
    pub fn purpose_codings(&self) -> Vec<Coding> {
        let mut codings: Vec<Coding> = self
            .token
            .as_ref()
            .map(|token| token.purposes.clone())
            .unwrap_or_default();
        if let Some(code) = &self.purpose
            && !codings.iter().any(|coding| &coding.code == code)
        {
            codings.push(Coding {
                system: None,
                code: code.clone(),
            });
        }
        codings
    }

    /// The policies the access was authorised under, in order: the data
    /// permit the caller declared, then the versions of the policies the
    /// decision grounds are defined in (FHIR R4 `AuditEvent.agent.policy`,
    /// "the policy or plan that authorized the activity being recorded").
    #[must_use]
    pub fn policies(&self) -> Vec<String> {
        self.permit
            .iter()
            .chain(self.decided.iter().flat_map(|decided| &decided.policies))
            .cloned()
            .collect()
    }

    /// The roles the record renders: the roles the caller held, then the
    /// professional roles the access token states.
    #[must_use]
    pub fn role_codings(&self) -> Vec<Coding> {
        let held = self.roles.iter().map(|role| Coding {
            system: None,
            code: role.clone(),
        });
        let stated = self
            .token
            .iter()
            .flat_map(|token| token.roles.iter().cloned());
        held.chain(stated).collect()
    }

    /// Whether the audited data moves from this server to the client, which
    /// makes the server the Source and the client the Destination participant.
    ///
    /// DICOM PS3.15 §A.5.2.7 gives Source Role ID to the entity the information
    /// originates from and Destination Role ID to the one it is directed to, so
    /// a read (and its Data Export, §A.5.3.4) has the server as Source. A
    /// query is the exception: §A.5.3.10 makes the process issuing it the
    /// Source, as the IHE BALP `Query` profile fixes for the client agent.
    /// Every other action moves data, or a request, from the client in.
    #[must_use]
    pub fn server_is_data_source(&self) -> bool {
        self.action == EventActionCode::Read && self.object != ObjectClass::Query
    }

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
            issuer: None,
            client_id: None,
            user_is_requestor: true,
            organisation: None,
            roles: Vec::new(),
            client_ip: None,
            ehr_id: None,
            subject_namespace: None,
            subject_lookup_failed: false,
            object_id: None,
            token_id: None,
            domain: AccessDomain::of(object),
            purpose: None,
            permit: None,
            emergency_access: false,
            legal_basis: None,
            result_count: None,
            origins: Vec::new(),
            origin_count: None,
            category: None,
            category_map_digest: None,
            actor: ActorAuthentication::default(),
            request_id: None,
            binding: None,
            summary: None,
            token: None,
            decision: AccessDecision::Admitted,
            entry: EntryScope::ByDomain,
            query: None,
            decided: None,
            timestamp: Timestamp::now(),
        }
    }
}

/// Whether the access a record describes was admitted or refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccessDecision {
    /// The access was admitted; its outcome says whether it succeeded.
    #[default]
    Admitted,
    /// The access was refused before it reached any data.
    Refused,
}

/// Whether a record also writes an openEHR access-log entry, and as what.
///
/// No openEHR spec governs which records do: our own design (#5121), every
/// access to EHR data, refusals and reads of the log included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntryScope {
    /// An entry when the record is an access to EHR data
    /// ([`AccessDomain::Ehr`]), none otherwise.
    #[default]
    ByDomain,
    /// An entry for a refusal no operation record of its request covers.
    Refusal,
    /// An entry for a read of the access log itself.
    LogRead,
    /// An entry for a change to the access log, such as an erasure, naming
    /// the entry it changed.
    LogChange,
    /// An entry for a change to what the access log means, such as a new
    /// restriction of its subject view in the access policy, naming the
    /// operation that made it; the record names the version.
    LogMeaning,
    /// No entry: the caller wrote the record's entry itself, in the
    /// transaction of the act it records, and emits the record to the
    /// forwarding sinks as the one record of the act.
    Written,
}

/// The query an access ran, named by its stored name or by the SHA-256 of its
/// text, so no entry carries the text (GDPR Art 5(1)(c),
/// `docs/law/eu/gdpr/text.html`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryNaming {
    /// The language the query is written in, such as `AQL`.
    pub language: &'static str,
    /// The stored query's qualified name, when a stored query ran.
    pub stored: Option<String>,
    /// The SHA-256 of the query text in lower-case hexadecimal.
    pub text_digest: String,
}

impl QueryNaming {
    /// The naming of a query written in `language` whose text is `text`,
    /// stored under `stored` when it is a stored query.
    #[must_use]
    pub fn of(language: &'static str, text: &str, stored: Option<String>) -> Self {
        let text_digest = crate::digest::sha256_hex(text);
        Self {
            language,
            stored,
            text_digest,
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

#[cfg(test)]
mod tests {
    use super::{AuditEvent, Coding, EventActionCode, EventOutcome, ObjectClass, TokenAttributes};

    /// A purpose of use the access token states counts as named beside the
    /// declared one, so the sender's emergency mark reads it too (EHDS
    /// Art. 11(5), #5323).
    #[test]
    fn a_purpose_the_token_states_is_named_beside_the_declared_one() {
        let mut event = AuditEvent::new(
            EventActionCode::Read,
            ObjectClass::Ehr,
            EventOutcome::Success,
        );
        assert!(!event.names_purpose(|code| code == "ETREAT"));
        event.token = Some(TokenAttributes {
            purposes: vec![Coding {
                system: Some("http://terminology.hl7.org/CodeSystem/v3-ActReason".to_owned()),
                code: "ETREAT".to_owned(),
            }],
            roles: Vec::new(),
        });
        assert!(event.names_purpose(|code| code == "ETREAT"));
        assert!(!event.names_purpose(|code| code == "TREAT"));
        event.purpose = Some("TREAT".to_owned());
        assert!(event.names_purpose(|code| code == "TREAT"));
        assert!(event.names_purpose(|code| code == "ETREAT"));
    }
}
