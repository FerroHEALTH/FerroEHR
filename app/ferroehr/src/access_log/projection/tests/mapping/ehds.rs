// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The rows of the mapping to the EHDS access record.

use super::*;

/// The projections the EHDS rows read.
struct Ehds {
    read: Projected,
    refused: Projected,
    failed: Projected,
    query: Projected,
    exported: Projected,
    urgent: Projected,
    represented: Projected,
    relayed: Projected,
}

/// The rows of the mapping to the EHDS access record (`ehds-logging`
/// `AccessRecord`).
#[test]
fn the_ehds_access_record_carries_each_row_where_the_mapping_says() {
    let policy = policy();
    let read = project(&policy, &classified_read(&policy), Some("subject-1"));
    let (refused, failed) = outcomes(&policy);
    let query = project(&policy, &stored_query(&policy), Some("subject-1"));
    let exported = export(&policy);
    let urgent = emergency(&policy);
    let represented = {
        let mut event = classified_read(&policy);
        event.binding = Some(representative());
        project(&policy, &event, Some("subject-9"))
    };
    let relayed = {
        let mut event = classified_read(&policy);
        event.binding = Some(organisation());
        project(&policy, &event, Some("subject-1"))
    };
    let f = Ehds {
        read,
        refused,
        failed,
        query,
        exported,
        urgent,
        represented,
        relayed,
    };
    let rows = [
        ehds_event_rows(&f),
        ehds_accessor_rows(&f),
        ehds_object_rows(&f),
    ]
    .concat();
    assert_eq!(failing(&rows), Vec::<&str>::new(), "{:?}", f.read.record);
}

/// Rows of the `the_ehds_access_record_carries_each_row_where_the_mapping_says` table.
fn ehds_event_rows(f: &Ehds) -> Vec<Row> {
    let read = &f.read;
    let refused = &f.refused;
    let failed = &f.failed;
    let query = &f.query;
    let exported = &f.exported;
    let record = read
        .record
        .as_ref()
        .expect("a classified access is a record");
    vec![
        ("action: Read", record.action == Action::Read),
        (
            "action: Query",
            query.record.as_ref().map(|r| r.action) == Some(Action::Query),
        ),
        (
            "action, loss: export folds into Read",
            exported.record.as_ref().map(|r| r.action) == Some(Action::Read),
        ),
        (
            "time_recorded: recorded",
            record.recorded.to_string() == "2026-11-13T10:15:30.123456Z",
        ),
        (
            "outcome: granted is Success",
            record.outcome == Outcome::Success,
        ),
        (
            "outcome, loss: refused shares MinorFailure with an admitted failure",
            refused.record.as_ref().map(|r| r.outcome) == Some(Outcome::MinorFailure),
        ),
        (
            "outcome: failed is the failure's own grade",
            failed.record.as_ref().map(|r| r.outcome) == Some(Outcome::SeriousFailure),
        ),
    ]
}

/// Rows of the `the_ehds_access_record_carries_each_row_where_the_mapping_says` table.
fn ehds_accessor_rows(f: &Ehds) -> Vec<Row> {
    let read = &f.read;
    let represented = &f.represented;
    let relayed = &f.relayed;
    let record = read
        .record
        .as_ref()
        .expect("a classified access is a record");
    vec![
        (
            "accessor.credential: issuer, subject, client and token id",
            record.accessor.issuer.as_deref() == Some("https://login.example.org")
                && record.accessor.subject == "alice"
                && record.accessor.client_id.as_deref() == Some("ward-app")
                && record.accessor.token_id.as_deref() == Some("jti-7f3e"),
        ),
        (
            "accessor.kind: acting",
            record.accessor.acting == Some(Acting::Client),
        ),
        (
            "accessor.roles: roles",
            record.accessor.roles.iter().any(|r| r.code == "clinician"),
        ),
        (
            "accessor.credential.assurance: assurance",
            record.accessor.assurance.is_some(),
        ),
        (
            "accessor.party, groups, binding: the binding",
            record
                .accessor
                .party()
                .is_some_and(|party| party.id == "6f1c2d3e-4b5a-4c6d-8e7f-90a1b2c3d4e5"),
        ),
        (
            "accessor.provider: provider",
            record.accessor.provider.as_deref() == Some("Example Hospital"),
        ),
        (
            "accessor.delegation: actors",
            represented
                .record
                .as_ref()
                .is_some_and(|r| !r.accessor.actors.is_empty()),
        ),
        (
            "authorisations, representation: represents",
            represented
                .record
                .as_ref()
                .is_some_and(|r| r.accessor.represents.is_some()),
        ),
        (
            "authorisations, agreement: relayed",
            relayed
                .record
                .as_ref()
                .is_some_and(|r| r.accessor.relayed.is_some()),
        ),
        (
            "purposes and permit: purposes and policies",
            record.accessor.purposes.iter().any(|p| p.code == "ETREAT")
                && record.accessor.policies == ["permit-42"],
        ),
    ]
}

/// Rows of the `the_ehds_access_record_carries_each_row_where_the_mapping_says` table.
fn ehds_object_rows(f: &Ehds) -> Vec<Row> {
    let read = &f.read;
    let query = &f.query;
    let urgent = &f.urgent;
    let record = read
        .record
        .as_ref()
        .expect("a classified access is a record");
    vec![
        (
            "ehr_id: one EhrAt, the subject of care found",
            record.subject.ehrs.len() == 1
                && record.subject.ehrs[0].ehr_id == EHR
                && matches!(record.subject.ehrs[0].patient, PatientLookup::Found(_)),
        ),
        (
            "items.categories: categories",
            record
                .categories
                .categories()
                .keys()
                .any(|c| c.code() == "Laboratory-Reports"),
        ),
        (
            "outcome.delivered and items.origins: delivered, origins, origin_count",
            record.delivered == Some(1)
                && record.origins.len() == 2
                && record.origin_count == Some(3),
        ),
        (
            "channel and operation: request id, operation, resource, client address",
            record.request.id.as_deref() == Some("01JC2Q7N3X")
                && record.request.operation == "composition_get"
                && record.request.resource.as_deref() == Some(VERSION)
                && record
                    .request
                    .client_address
                    .map(|a| a.to_string())
                    .as_deref()
                    == Some("10.216.24.150"),
        ),
        (
            "query, loss: the text is not carried, the name stands in",
            query.record.as_ref().is_some_and(|r| {
                r.request
                    .query
                    .as_ref()
                    .is_none_or(|text| !text.expose_secret().contains("SELECT"))
            }),
        ),
        (
            "retention: the period the entry states",
            record.retention.period().code() == "indefinite",
        ),
        (
            "is_emergency: emergency",
            urgent
                .record
                .as_ref()
                .is_some_and(|r| r.emergency.is_some()),
        ),
        (
            "link, loss: the record carries no chain",
            !format!("{record:?}").contains("content_digest"),
        ),
    ]
}
