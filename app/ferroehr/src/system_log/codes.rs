// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! DICOM (DCM) / RFC-3881 code constants used in the DICOM Audit Message.
//!
//! The DICOM PS3.15 §A.5 and RFC 3881 codes come from `ihe-atna`'s code tables
//! (`ihe_atna::atna::codes`); this module names the ones the CDR emits and
//! adds what is FerroEHR's own: the per-resource `originalText` of
//! "Patient Record" and the ITS-REST operation code system.

/// A coded value: `csd-code` + `codeSystemName` + `originalText`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_field_names,
    reason = "`csd_code` and `code_system` are the DICOM PS3.15 attribute names \
              (`csd-code`, `codeSystemName`) this type serializes 1:1; renaming \
              them would diverge from the audited vocabulary"
)]
pub struct Code {
    /// The `csd-code` attribute (the numeric code within the system).
    pub csd_code: &'static str,
    /// The `codeSystemName` attribute (`DCM` or `RFC-3881`).
    pub code_system: &'static str,
    /// The `originalText` attribute (human-readable display).
    pub original_text: &'static str,
}

use ihe_atna::atna::codes::{self as atna, Triple};

impl Code {
    /// The code of an `ihe-atna` table entry.
    const fn of((csd_code, code_system, original_text): Triple) -> Self {
        Self {
            csd_code,
            code_system,
            original_text,
        }
    }
}

/// DICOM Controlled Terminology code system name.
pub const DCM: &str = atna::DCM;

// ── EventID (DICOM PS3.15 §A.5.1, `DCM`) ─────────────────────────────────────
// EHR / clinical-data ops use "Patient Record" (110110, DICOM PS3.15 §A.5.1);
// the CDR varies the `originalText` per resource
// (composition/contribution/directory/demographic) while keeping the DICOM
// `csd-code` 110110 — the varied-display pattern the DICOM EventID coding
// permits. Operations with a dedicated DICOM EventID use it: query execution
// → 110112 "Query", EHR-Extract communication → 110106 "Export" /
// 110107 "Import", user authentication → 110114 "User Authentication".

/// DICOM `EventID` `csd-code` for "Patient Record".
pub const EVENT_PATIENT_RECORD_CODE: &str = atna::EVENT_PATIENT_RECORD.0;
/// DICOM `EventID` `csd-code` for "Application Activity".
pub const EVENT_APPLICATION_ACTIVITY_CODE: &str = atna::EVENT_APPLICATION_ACTIVITY.0;
/// DICOM `EventID` `csd-code` for "Query" (110112).
pub const EVENT_QUERY_CODE: &str = atna::EVENT_QUERY.0;
/// DICOM `EventID` `csd-code` for "Export" (110106) — data leaving the system.
pub const EVENT_EXPORT_CODE: &str = atna::EVENT_EXPORT.0;
/// DICOM `EventID` `csd-code` for "Import" (110107) — data entering the system.
pub const EVENT_IMPORT_CODE: &str = atna::EVENT_IMPORT.0;
/// DICOM `EventID` `csd-code` for "User Authentication" (110114).
pub const EVENT_USER_AUTHENTICATION_CODE: &str = atna::EVENT_USER_AUTHENTICATION.0;

// ── EventTypeCode (DICOM PS3.15 §A.5.1, `DCM` / our own) ─────────────────────

/// DCM 110122 "Login" — the `EventTypeCode` of a user-authentication attempt.
pub const TYPE_LOGIN: Code = Code::of(atna::TYPE_LOGIN);
/// DCM 110123 "Logout" — the `EventTypeCode` of a session end.
pub const TYPE_LOGOUT: Code = Code::of(atna::TYPE_LOGOUT);
/// The `codeSystemName` for ITS-REST operation-id `EventTypeCode`s.
///
/// NOTE: no external code system governs openEHR REST operations — our own
/// design/extension; the generated operation id is emitted as the `csd-code`
/// under this system name.
pub const OPENEHR_ITS_REST: &str = "openEHR-ITS-REST";

// ── RoleIDCode (DICOM PS3.15 §A.5.2, `DCM`) ──────────────────────────────────

/// "Source Role ID": the participant the information originates from
/// (PS3.15 §A.5.2.7), the server on a read and the client otherwise.
pub const ROLE_SOURCE: Code = Code::of(atna::ROLE_SOURCE_ROLE_ID);
/// "Destination Role ID": the participant the information is directed to
/// (PS3.15 §A.5.2.7), the client on a read and the server otherwise.
pub const ROLE_DESTINATION: Code = Code::of(atna::ROLE_DESTINATION_ROLE_ID);

// ── ParticipantObjectIDTypeCode (RFC 3881 §5.5.1) ────────────────────────────

/// Patient Number (RFC-3881 code 2) — the EHR subject id participant.
pub const OBJ_ID_PATIENT_NUMBER: Code = Code::of(atna::OBJECT_ID_PATIENT_NUMBER);
/// Search Criteria (RFC-3881 code 10) — the AQL query participant.
pub const OBJ_ID_SEARCH_CRITERIA: Code = Code::of(atna::OBJECT_ID_SEARCH_CRITERIA);
/// URI (RFC-3881 code 12) — a clinical object's resource URI participant.
pub const OBJ_ID_URI: Code = Code::of(atna::OBJECT_ID_URI);

// ── NetworkAccessPointTypeCode (RFC 3881 §5.3) ───────────────────────────────

/// IP Address network-access-point type.
pub const NETWORK_ACCESS_POINT_IP: &str = "2";

// ── ParticipantObjectTypeCode / *Role (RFC 3881 §5.5) ────────────────────────

/// Participant object type: person (`1`).
pub const OBJECT_TYPE_PERSON: &str = "1";
/// Participant object type: system object (`2`).
pub const OBJECT_TYPE_SYSTEM: &str = "2";
/// Participant object type-code role: patient (`1`).
pub const OBJECT_ROLE_PATIENT: &str = "1";
/// Participant object type-code role: query (`24`).
pub const OBJECT_ROLE_QUERY: &str = "24";

// ── ATNA rendering of the event enums ────────────────────────────────────────
// The DICOM / RFC-3881 renderings of the event model live here, in the ATNA
// layer, one inherent impl per enum.

use super::event::{EventActionCode, EventOutcome, EventType, ObjectClass};

/// ATNA (DICOM PS3.15 §A.5.1) rendering of an [`EventType`].
impl EventType {
    /// The `EventTypeCode` coded value: DCM login/logout codes, and ITS-REST
    /// operation ids under our own `openEHR-ITS-REST` system name (see [`OPENEHR_ITS_REST`]).
    pub(crate) fn coded(&self) -> ihe_atna::atna::message::CodedValue {
        use ihe_atna::atna::message::CodedValue;
        let fixed =
            |code: Code| CodedValue::new(code.csd_code, code.code_system, code.original_text);
        match self {
            EventType::Login => fixed(TYPE_LOGIN),
            EventType::Logout => fixed(TYPE_LOGOUT),
            EventType::RestOperation(op) => CodedValue::new(op, OPENEHR_ITS_REST, op),
        }
    }
}

/// ATNA (DICOM PS3.15 §A.5.1) rendering of an [`EventActionCode`].
impl EventActionCode {
    /// The `ParticipantObjectDataLifeCycle` most fitting for the action
    /// (DICOM PS3.15 §A.5.1): create is origination, update amendment, read
    /// and execute access or use, delete logical deletion.
    pub(crate) fn data_life_cycle(self) -> &'static str {
        match self {
            EventActionCode::Create => atna::LIFE_CYCLE_ORIGINATION,
            EventActionCode::Update => atna::LIFE_CYCLE_AMENDMENT,
            EventActionCode::Read | EventActionCode::Execute => atna::LIFE_CYCLE_ACCESS,
            EventActionCode::Delete => atna::LIFE_CYCLE_LOGICAL_DELETION,
        }
    }

    /// The `EventActionCode` (`C`, `R`, `U`, `D`, `E`).
    pub(crate) fn as_code(self) -> &'static str {
        match self {
            EventActionCode::Create => "C",
            EventActionCode::Read => "R",
            EventActionCode::Update => "U",
            EventActionCode::Delete => "D",
            EventActionCode::Execute => "E",
        }
    }
}

/// ATNA (DICOM PS3.15 §A.5.1) rendering of an [`EventOutcome`].
impl EventOutcome {
    /// The `EventOutcomeIndicator` (`0`, `4`, `8`).
    pub(crate) fn as_code(self) -> &'static str {
        match self {
            EventOutcome::Success => "0",
            EventOutcome::MinorFailure => "4",
            EventOutcome::SeriousFailure => "8",
        }
    }

    /// The `EventOutcomeDescription` text.
    pub(crate) fn description(self) -> &'static str {
        match self {
            EventOutcome::Success => "Operation performed successfully",
            EventOutcome::MinorFailure | EventOutcome::SeriousFailure => "Operation failed",
        }
    }
}

/// ATNA (DICOM / RFC-3881) rendering of an [`ObjectClass`]: the `EventID`
/// (DICOM PS3.15 §A.5.1) and the participant-object shape
/// (`ParticipantObjectIdentification`, DICOM PS3.15 §A.5 / RFC 3881 §5.5).
impl ObjectClass {
    /// The DICOM `EventID` `(csd-code, originalText)` for this class
    /// (DICOM PS3.15 §A.5.1). Classes with a dedicated DICOM `EventID` use it
    /// (Query 110112, Export 110106 / Import 110107, User Authentication
    /// 110114); the clinical-record classes share Patient Record (110110)
    /// with a per-resource `originalText` — the varied-display pattern the
    /// DICOM `EventID` coding permits.
    pub(crate) fn event_id(self, action: EventActionCode) -> (&'static str, &'static str) {
        match self {
            ObjectClass::Ehr => (EVENT_PATIENT_RECORD_CODE, "Patient Record"),
            ObjectClass::Composition => (EVENT_PATIENT_RECORD_CODE, "composition"),
            ObjectClass::Contribution => (EVENT_PATIENT_RECORD_CODE, "contribution"),
            ObjectClass::Directory => (EVENT_PATIENT_RECORD_CODE, "directory"),
            // Query execution has the dedicated DICOM EventID 110112 "Query"
            // (DICOM PS3.15 §A.5.1).
            ObjectClass::Query => (EVENT_QUERY_CODE, "Query"),
            // NOTE: DICOM PS3.15 §A.5.1 lists no EventID for template
            // provisioning, so definitional metadata uses Application-Activity
            // (110100) with `originalText="template"`.
            ObjectClass::Template => (EVENT_APPLICATION_ACTIVITY_CODE, "template"),
            // NOTE: demographic parties are person-identifiable, so they
            // use the Patient-Record code (110110, DICOM PS3.15 §A.5.1) with
            // `originalText="demographic"` (same varied-display pattern).
            ObjectClass::Demographic => (EVENT_PATIENT_RECORD_CODE, "demographic"),
            // EHR-Extract communication carries patient-identifiable clinical
            // data across systems, audited for non-repudiation (BASE
            // `master07-security.adoc` §Non-repudiation). DICOM PS3.15 §A.5.1
            // codes the direction — Import (110107) in, Export (110106) out —
            // and `FerroEhrService::emit_extract_audit` carries it in the action.
            ObjectClass::Extract => match action {
                EventActionCode::Create | EventActionCode::Update => (EVENT_IMPORT_CODE, "Import"),
                EventActionCode::Read | EventActionCode::Execute | EventActionCode::Delete => {
                    (EVENT_EXPORT_CODE, "Export")
                }
            },
            ObjectClass::ApplicationActivity => {
                (EVENT_APPLICATION_ACTIVITY_CODE, "Application Activity")
            }
            // User authentication has the dedicated DICOM EventID 110114
            // "User Authentication" (DICOM PS3.15 §A.5.1); the login/logout
            // kind is the `EventTypeCode` (110122/110123).
            ObjectClass::Authentication => (EVENT_USER_AUTHENTICATION_CODE, "User Authentication"),
        }
    }

    /// Whether this class carries a patient (Patient-Number) participant object.
    pub(crate) fn is_patient_centric(self) -> bool {
        matches!(
            self,
            ObjectClass::Ehr
                | ObjectClass::Composition
                | ObjectClass::Contribution
                | ObjectClass::Directory
                | ObjectClass::Extract
        )
    }

    /// Whether this class carries an object-URI participant (in addition to the
    /// patient one, for the patient-centric classes) when an object id is known.
    ///
    /// An EHR-class record names its object when it has one, as the FHIR
    /// `AuditEvent` of the same record does: the held composition of an EHDS
    /// Art. 3(3) access delay (`access-delay:<vo>`), or the EHR a read served.
    pub(crate) fn has_object_uri(self) -> bool {
        matches!(
            self,
            ObjectClass::Ehr
                | ObjectClass::Composition
                | ObjectClass::Contribution
                | ObjectClass::Directory
                | ObjectClass::Template
                | ObjectClass::Demographic
                | ObjectClass::Extract
        )
    }

    /// Whether this class renders a Search-Criteria (query) participant.
    pub(crate) fn is_query(self) -> bool {
        matches!(self, ObjectClass::Query)
    }
}
