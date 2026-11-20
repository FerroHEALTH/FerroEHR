// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The HL7 code systems the draft access terminology takes codes from, by
//! the canonical URL each terminology identifier names.

/// The canonical URL of the HL7 code system `terminology` names.
///
/// It follows the draft Terminology page's table of HL7 code systems,
/// through which an exporter to FHIR maps the identifier and keeps the code;
/// `None` for any other identifier.
#[must_use]
pub fn canonical_url(terminology: &str) -> Option<&'static str> {
    const URLS: [(&str, &str); 12] = [
        (
            "HL7_ActCode",
            "http://terminology.hl7.org/CodeSystem/v3-ActCode",
        ),
        (
            "HL7_ActReason",
            "http://terminology.hl7.org/CodeSystem/v3-ActReason",
        ),
        (
            "HL7_AuditEventAction",
            "http://hl7.org/fhir/audit-event-action",
        ),
        (
            "HL7_AuditEventOutcome",
            "http://terminology.hl7.org/CodeSystem/audit-event-outcome",
        ),
        (
            "HL7_Confidentiality",
            "http://terminology.hl7.org/CodeSystem/v3-Confidentiality",
        ),
        (
            "HL7_ConsentActionCodes",
            "http://terminology.hl7.org/CodeSystem/consentaction",
        ),
        (
            "HL7_ConsentProvisionType",
            "http://hl7.org/fhir/consent-provision-type",
        ),
        (
            "HL7_ParticipationFunction",
            "http://terminology.hl7.org/CodeSystem/v3-ParticipationFunction",
        ),
        (
            "HL7_ParticipationType",
            "http://terminology.hl7.org/CodeSystem/v3-ParticipationType",
        ),
        (
            "HL7_RoleClass",
            "http://terminology.hl7.org/CodeSystem/v3-RoleClass",
        ),
        (
            "HL7_RoleCode",
            "http://terminology.hl7.org/CodeSystem/v3-RoleCode",
        ),
        (
            "HL7_SecurityRoleType",
            "http://terminology.hl7.org/CodeSystem/extra-security-role-type",
        ),
    ];
    URLS.iter()
        .find(|(id, _)| *id == terminology)
        .map(|(_, url)| *url)
}
