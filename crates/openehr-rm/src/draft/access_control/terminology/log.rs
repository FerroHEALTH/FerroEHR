// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The groups of the draft access terminology the access log codes its
//! entries in: what an access did, its outcome, the accessor's kind and
//! binding, the grounds it ran under and why it was refused.

use std::fmt;

use super::{
    Group, TERMINOLOGY_ID_ACCESS, TERMINOLOGY_ID_AUDIT_EVENT_ACTION,
    TERMINOLOGY_ID_SECURITY_ROLE_TYPE, Term,
};

group! {
    /// What an access recorded in the access log did (`ACCESS_LOG_ENTRY.action`).
    Action in AccessAction {
        /// The access read data: HL7 FHIR `HL7_AuditEventAction#R`.
        Read => "R" of TERMINOLOGY_ID_AUDIT_EVENT_ACTION, "read",
        /// The access queried data.
        Query => "query",
        /// The access created data: HL7 FHIR `HL7_AuditEventAction#C`.
        Create => "C" of TERMINOLOGY_ID_AUDIT_EVENT_ACTION, "create",
        /// The access changed data: HL7 FHIR `HL7_AuditEventAction#U`.
        Update => "U" of TERMINOLOGY_ID_AUDIT_EVENT_ACTION, "update",
        /// The access deleted data: HL7 FHIR `HL7_AuditEventAction#D`.
        Delete => "D" of TERMINOLOGY_ID_AUDIT_EVENT_ACTION, "delete",
        /// The access exported data out of the system.
        Export => "export",
        /// The access imported data into the system.
        Import => "import",
    }
}

group! {
    /// The result of an access (`ACCESS_OUTCOME.result`).
    Outcome in AccessOutcome {
        /// The access was granted in full.
        Granted => "granted",
        /// The access was granted for part of what it asked.
        PartiallyGranted => "partially granted",
        /// The access was refused.
        Refused => "refused",
        /// The access failed for another reason.
        Failed => "failed",
    }
}

group! {
    /// The kind of accessor (`ACCESSOR.kind`), which
    /// `ACCESS_LOG_RESTRICTION.accessor_kinds` is coded in too.
    AccessorKind in AccessorKind {
        /// A person acting on their own account.
        Person => "person",
        /// A person acting through a client they delegated to.
        DelegatedPerson => "delegated person",
        /// The subject of care: HL7 `HL7_SecurityRoleType#datasubject`.
        SubjectOfCare => "datasubject" of TERMINOLOGY_ID_SECURITY_ROLE_TYPE, "subject of care",
        /// A representative of the subject of care.
        Representative => "representative",
        /// A person of another organisation.
        ExternalPerson => "external person",
        /// An organisation.
        Organisation => "organisation",
        /// A system acting on its own account.
        System => "system",
    }
}

group! {
    /// The kind of ground an access ran under (`ACCESS_AUTHORISATION.kind`).
    Authorisation in AccessAuthorisation {
        /// A role the accessor holds.
        Role => "role",
        /// A treatment relationship to the subject of care.
        TreatmentRelationship => "treatment relationship",
        /// An agreement between organisations.
        Agreement => "agreement",
        /// A consent decision.
        Consent => "consent",
        /// A data permit.
        Permit => "permit",
        /// An emergency override.
        Emergency => "emergency",
        /// The accessor is the subject of care.
        SubjectOfCare => "subject of care",
        /// A representation of the subject of care.
        Representation => "representation",
        /// A legal obligation.
        LegalObligation => "legal obligation",
        /// A conflict rule of the controller's policy allowed a professional
        /// in conflict with the subject.
        ConflictAllowance => "conflict allowance",
    }
}

group! {
    /// Why an access was refused, partly granted or failed
    /// (`ACCESS_OUTCOME.reason`), where no [`Refusal`](super::Refusal) code names it.
    RefusalReason in AccessRefusalReason {
        /// The caller was not authenticated.
        NotAuthenticated => "not authenticated",
        /// The caller's credential is bound to no party.
        CredentialNotBound => "credential not bound",
        /// The caller is not authorised.
        NotAuthorised => "not authorised",
        /// No consent was given.
        ConsentAbsent => "consent absent",
        /// The caller exceeded a rate limit.
        RateLimited => "rate limited",
        /// The system failed.
        SystemFailure => "system failure",
    }
}

group! {
    /// How the system bound the credential to the accessor's party, or why
    /// it could not (`ACCESSOR.binding`). Whether the access ran is
    /// `ACCESS_OUTCOME`'s, so no code carries a refused prefix.
    CredentialBinding in CredentialBinding {
        /// The credential is bound to one active party.
        Bound => "bound",
        /// The credential is bound provisionally, pending its party.
        Provisional => "provisional",
        /// The credential is the subject of care's own.
        SubjectOfCare => "subject of care",
        /// The credential is of a person of another organisation.
        External => "external",
        /// The credential is of an organisation under an agreement.
        Organisation => "organisation",
        /// The credential is of a system acting for nobody.
        System => "system",
        /// The credential is bound to no party.
        Unbound => "unbound",
        /// The credential matches more than one party.
        Ambiguous => "ambiguous",
        /// The bound party is inactive.
        PartyInactive => "party inactive",
        /// The bound party was abandoned.
        PartyAbandoned => "party abandoned",
        /// The bound party was deleted.
        PartyDeleted => "party deleted",
        /// The bound party is not of the kind the credential claims.
        PartyKindMismatch => "party kind mismatch",
        /// The credential claims a representation the system holds no
        /// record of.
        RepresentationUnrecorded => "representation unrecorded",
        /// The credential lacks the claim its party is keyed by.
        KeyClaimMissing => "key claim missing",
        /// The credential's groups could not be read.
        GroupsUnavailable => "groups unavailable",
        /// The credential is a guest's.
        Guest => "guest",
        /// The credential of a person of another organisation lacks what
        /// binding it needs.
        ExternalCredentialIncomplete => "external credential incomplete",
        /// A system credential asks for a personal scope.
        SystemPersonalScope => "system personal scope",
        /// No agreement covers the credential's organisation.
        NoAgreement => "no agreement",
        /// The credential asks for more than its agreement grants.
        ScopeBeyondAgreement => "scope beyond agreement",
        /// The agreement asks for a correlation id the request lacks.
        CorrelationMissing => "correlation missing",
        /// The request carries no treatment relationship confirmation.
        TreatmentRelationshipConfirmationMissing => "treatment relationship confirmation missing",
        /// The treatment relationship confirmation names another patient.
        TreatmentRelationshipConfirmationMismatch => "treatment relationship confirmation mismatch",
    }
}
