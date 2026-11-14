// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The rows of the mapping to the DICOM PS3.15 A.5 audit message.

use super::*;

/// The projections the DICOM rows read.
struct Dicom {
    read: Projected,
    refused: Projected,
    query: Projected,
    represented: Projected,
    urgent: Projected,
    exported: Projected,
}

/// The rows of the mapping to the DICOM PS3.15 A.5 audit message.
#[test]
fn the_dicom_message_carries_each_row_where_the_mapping_says() {
    let policy = policy();
    let mut plain = classified_read(&policy);
    plain.actor.professional_id = None;
    plain.actor.mode = None;
    let read = project(&policy, &plain, Some("subject-1"));
    let (refused, _) = outcomes(&policy);
    let query = project(&policy, &stored_query(&policy), Some("subject-1"));
    let represented = {
        let mut event = plain.clone();
        event.binding = Some(representative());
        project(&policy, &event, Some("subject-9"))
    };
    let urgent = emergency(&policy);
    let exported = export(&policy);
    let f = Dicom {
        read,
        refused,
        query,
        represented,
        urgent,
        exported,
    };
    let rows = [
        dicom_event_rows(&f),
        dicom_accessor_rows(&f),
        dicom_object_rows(&f),
    ]
    .concat();
    assert_eq!(failing(&rows), Vec::<&str>::new(), "{}", f.read.message);
}

/// Rows of the `the_dicom_message_carries_each_row_where_the_mapping_says` table.
fn dicom_event_rows(f: &Dicom) -> Vec<Row> {
    let read = &f.read;
    let refused = &f.refused;
    let exported = &f.exported;
    let xml = &read.message;
    vec![
        (
            "action: EventActionCode and the Patient Record EventID",
            xml.contains(r#"EventActionCode="R""#) && xml.contains(r#"csd-code="110110""#),
        ),
        (
            "action, loss: export folds into R, under the Export EventID",
            exported.message.contains(r#"EventActionCode="R""#)
                && exported.message.contains(r#"csd-code="110106""#),
        ),
        (
            "operation: EventTypeCode in openEHR-ITS-REST",
            xml.contains(r#"csd-code="composition_get" codeSystemName="openEHR-ITS-REST""#),
        ),
        (
            "time_recorded: EventDateTime",
            xml.contains(r#"EventDateTime="2026-11-13T10:15:30.123456Z""#),
        ),
        (
            "outcome: EventOutcomeIndicator and EventOutcomeDescription",
            xml.contains(r#"EventOutcomeIndicator="0""#)
                && refused.message.contains(r#"EventOutcomeIndicator="4""#)
                && xml.contains("<EventOutcomeDescription>"),
        ),
        (
            "accessor.credential: the requestor's UserID is the token subject",
            xml.contains(r#"UserID="alice" UserIsRequestor="true""#),
        ),
        (
            "accessor.credential, loss: the issuer and the token id are not written",
            !xml.contains("login.example.org") && !xml.contains("jti-7f3e"),
        ),
    ]
}

/// Rows of the `the_dicom_message_carries_each_row_where_the_mapping_says` table.
fn dicom_accessor_rows(f: &Dicom) -> Vec<Row> {
    let read = &f.read;
    let represented = &f.represented;
    let xml = &read.message;
    vec![
        (
            "accessor.roles: RoleIDCode in the deployment's role vocabulary",
            xml.contains(r#"csd-code="clinician" codeSystemName="urn:ferroehr:role""#),
        ),
        (
            "accessor.party and binding: details of the ehds-accessor object",
            xml.contains("<ParticipantObjectName>ehds-accessor</ParticipantObjectName>")
                && xml.contains(r#"ParticipantObjectDetail type="party""#),
        ),
        (
            "accessor.delegation: a detail of the ehds-accessor object",
            represented.message.contains("ehds-accessor")
                && represented.message.contains(r#"type="actor""#),
        ),
        (
            "system_id: AuditSourceID, the site from the deployment",
            xml.contains(r#"AuditEnterpriseSiteID="site-1" AuditSourceID="cdr.example.org""#),
        ),
        (
            "accessor.provider, loss: the organisation is not written",
            !xml.contains("Example Hospital"),
        ),
        (
            "channel.client_address: NetworkAccessPointID of type 2",
            xml.contains(r#"NetworkAccessPointID="10.216.24.150" NetworkAccessPointTypeCode="2""#),
        ),
    ]
}

/// Rows of the `the_dicom_message_carries_each_row_where_the_mapping_says` table.
fn dicom_object_rows(f: &Dicom) -> Vec<Row> {
    let read = &f.read;
    let query = &f.query;
    let urgent = &f.urgent;
    let xml = &read.message;
    vec![
        (
            "ehr_id: the patient participant names the subject of care",
            xml.contains(
                r#"ParticipantObjectID="subject-1" ParticipantObjectTypeCode="1" ParticipantObjectTypeCodeRole="1""#,
            ),
        ),
        (
            "ehr_id, loss: the ehr_id itself is not written",
            !xml.contains(EHR),
        ),
        (
            "items: a System Object of ID type URI naming the version",
            xml.contains(&format!(
                r#"ParticipantObjectID="{VERSION}" ParticipantObjectTypeCode="2""#
            )),
        ),
        (
            "items.categories, loss: no category is written",
            !xml.contains("Laboratory-Reports"),
        ),
        (
            "query: the query object of the Query EventID",
            query.message.contains(r#"csd-code="110112""#)
                && query.message.contains("org.example::vitals"),
        ),
        (
            "purposes, loss: no purpose of use is written",
            !xml.contains("ETREAT") && !xml.contains("TREAT\""),
        ),
        (
            "authorisations, loss: no permit is written",
            !xml.contains("permit-42"),
        ),
        (
            "is_emergency, loss: the mark is not written",
            !urgent.message.contains("emergency"),
        ),
        (
            "retention, security_labels, link and other_details, loss: none is written",
            !xml.contains("retention")
                && !xml.contains("ParticipantObjectSensitivity")
                && !xml.contains("ferroehr_access_details"),
        ),
    ]
}
