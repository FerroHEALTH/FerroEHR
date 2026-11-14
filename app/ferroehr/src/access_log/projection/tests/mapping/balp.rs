// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The rows of the mapping to the FHIR R4 `AuditEvent` under IHE BALP 1.1.4.

use super::*;

/// The projections the BALP rows read.
struct Balp {
    read: Projected,
    refused: Projected,
    failed: Projected,
    query: Projected,
    represented: Projected,
    relayed: Projected,
    urgent: Projected,
    exported: Projected,
}

/// The rows of the mapping to the FHIR R4 `AuditEvent` under IHE BALP 1.1.4.
#[test]
fn the_balp_audit_event_carries_each_row_where_the_mapping_says() {
    let policy = policy();
    let read = project(&policy, &classified_read(&policy), Some("subject-1"));
    let (refused, failed) = outcomes(&policy);
    let query = project(&policy, &stored_query(&policy), Some("subject-1"));
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
    let urgent = emergency(&policy);
    let exported = export(&policy);
    let f = Balp {
        read,
        refused,
        failed,
        query,
        represented,
        relayed,
        urgent,
        exported,
    };
    let rows = [
        balp_event_rows(&f),
        balp_accessor_rows(&f),
        balp_object_rows(&f),
        balp_authorisation_rows(&f),
    ]
    .concat();
    assert_eq!(failing(&rows), Vec::<&str>::new(), "{}", f.read.document);
}

/// Rows of the `the_balp_audit_event_carries_each_row_where_the_mapping_says` table.
fn balp_event_rows(f: &Balp) -> Vec<Row> {
    let read = &f.read;
    let refused = &f.refused;
    let failed = &f.failed;
    let exported = &f.exported;
    let doc = &read.document;
    vec![
        (
            "action: read is action R, type rest, subtype read",
            read.entry["action"]["value"] == "read"
                && doc["action"] == "R"
                && doc["type"]["code"] == "rest"
                && doc["subtype"][0]["code"] == "read",
        ),
        (
            "action, loss: the openEHR action is written in no code system of its own",
            doc["subtype"]
                .as_array()
                .map(|subtypes| {
                    subtypes
                        .iter()
                        .filter_map(|s| s["system"].as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
                == [
                    "http://hl7.org/fhir/restful-interaction",
                    "urn:openehr:its-rest:operation",
                ],
        ),
        (
            "action, loss: export folds into R",
            exported.entry["action"]["value"] == "export" && exported.document["action"] == "R",
        ),
        (
            "operation: a subtype in the ITS-REST operation system",
            doc["subtype"][1]["system"] == "urn:openehr:its-rest:operation"
                && doc["subtype"][1]["code"] == "composition_get",
        ),
        (
            "time_recorded: recorded",
            doc["recorded"] == read.entry["time_recorded"]["value"],
        ),
        ("occurred, loss: no period", doc.get("period").is_none()),
        (
            "outcome granted: 0",
            doc["outcome"] == "0" && read.entry["outcome"]["result"]["value"] == "granted",
        ),
        (
            "outcome refused: 4",
            refused.document["outcome"] == "4"
                && refused.entry["outcome"]["result"]["value"] == "refused",
        ),
        (
            "outcome failed: 8 for a serious failure",
            failed.document["outcome"] == "8"
                && failed.entry["outcome"]["result"]["value"] == "failed",
        ),
        (
            "outcome, loss: the description is the outcome's text, never a reason code",
            doc["outcomeDesc"] == "Operation performed successfully"
                && read.entry["outcome"].get("reason").is_none(),
        ),
        (
            "outcome.delivered: the delivered detail of ehds-categories",
            detail(doc, "ehds-categories", "delivered", "1"),
        ),
    ]
}

/// Rows of the `the_balp_audit_event_carries_each_row_where_the_mapping_says` table.
fn balp_accessor_rows(f: &Balp) -> Vec<Row> {
    let read = &f.read;
    let represented = &f.represented;
    let doc = &read.document;
    vec![
        (
            "accessor.party: the PRN otherId of agent:user",
            other_id(user(doc), "PRN", "6f1c2d3e-4b5a-4c6d-8e7f-90a1b2c3d4e5"),
        ),
        (
            "accessor.credential: issuer and subject are agent:user who.identifier",
            user(doc)["who"]["identifier"]["system"] == "https://login.example.org"
                && user(doc)["who"]["identifier"]["value"] == "alice",
        ),
        (
            "accessor.credential: the token id is the first policy of agent:user",
            user(doc)["policy"][0] == "jti-7f3e",
        ),
        (
            "accessor.credential: the client is an agent of its own",
            doc["agent"].as_array().is_some_and(|agents| {
                agents
                    .iter()
                    .any(|a| a["who"]["identifier"]["value"] == "ward-app")
            }),
        ),
        (
            "accessor.credential: the assurance is the ihe-assuranceLevel of agent:user",
            user(doc)["extension"].as_array().is_some_and(|x| {
                x.iter()
                    .any(|e| e["valueCodeableConcept"]["coding"][0]["code"] == "high")
            }),
        ),
        (
            "accessor.provider: an agent naming the organisation",
            doc["agent"].as_array().is_some_and(|agents| {
                agents.iter().any(|a| {
                    a["who"]["type"] == "Organization"
                        && a["who"]["identifier"]["value"] == "Example Hospital"
                })
            }),
        ),
        (
            "accessor.roles and kind: agent:user role, the acting mode first",
            user(doc)["role"][0]["coding"][0]["code"] == "client"
                && user(doc)["role"][1]["coding"][0]["code"] == "clinician",
        ),
        (
            "accessor.groups and binding: details of ehds-accessor",
            detail(doc, "ehds-accessor", "binding", "bound")
                && detail(
                    doc,
                    "ehds-accessor",
                    "group",
                    "demographic|GROUP|c1a9e0e4-0d8f-4d9c-9b51-3f6a2f1b7c11",
                ),
        ),
        (
            "accessor.delegation: each actor a detail of ehds-accessor, in order",
            detail(
                &represented.document,
                "ehds-accessor",
                "actor",
                "https://idp.example.org|app-1",
            ),
        ),
        (
            "authorisations, representation: a detail of ehds-accessor",
            named(&represented.document, "ehds-accessor")
                .is_some_and(|entity| entity.to_string().contains("subject-9")),
        ),
    ]
}

/// Rows of the `the_balp_audit_event_carries_each_row_where_the_mapping_says` table.
fn balp_object_rows(f: &Balp) -> Vec<Row> {
    let read = &f.read;
    let query = &f.query;
    let doc = &read.document;
    vec![
        (
            "ehr_id: the ehr entity, the subject of care the patient entity",
            named(doc, "ehr").is_some_and(|e| e["what"]["identifier"]["value"] == EHR)
                && entities(doc).iter().any(|e| {
                    e["role"]["code"] == "1" && e["what"]["identifier"]["value"] == "subject-1"
                }),
        ),
        (
            "items: entity:data names the version in urn:openehr:object-id, type 2, role 4",
            entities(doc).iter().any(|e| {
                e["what"]["identifier"]["system"] == "urn:openehr:object-id"
                    && e["what"]["identifier"]["value"] == VERSION
                    && e["type"]["code"] == "2"
                    && e["role"]["code"] == "4"
            }),
        ),
        (
            "items.categories and template: details of ehds-categories",
            detail(doc, "ehds-categories", "template-id", TEMPLATE)
                && named(doc, "ehds-categories")
                    .is_some_and(|e| e.to_string().contains("Laboratory-Reports")),
        ),
        (
            "items.origins: one origin entity each",
            entities(doc)
                .iter()
                .filter(|e| e["name"] == "origin")
                .count()
                == 2,
        ),
        (
            "query: entity:query, the stored query's name a detail",
            entities(&query.document)
                .iter()
                .any(|e| e["role"]["code"] == "24")
                && detail(&query.document, "ehds-categories", "delivered", "4"),
        ),
        (
            "query, loss: the text digest is not written",
            !query
                .document
                .to_string()
                .contains(query.entry["query"]["text_digest"].as_str().unwrap_or("?")),
        ),
    ]
}

/// Rows of the `the_balp_audit_event_carries_each_row_where_the_mapping_says` table.
fn balp_authorisation_rows(f: &Balp) -> Vec<Row> {
    let read = &f.read;
    let relayed = &f.relayed;
    let urgent = &f.urgent;
    let doc = &read.document;
    vec![
        (
            "purposes: purposeOfEvent and agent:user purposeOfUse",
            doc["purposeOfEvent"][0]["coding"][0]["code"] == "ETREAT"
                && user(doc)["purposeOfUse"][0]["coding"][0]["code"] == "ETREAT",
        ),
        (
            "authorisations, permit: a policy of agent:user after the token id",
            user(doc)["policy"][1] == "permit-42",
        ),
        (
            "authorisations, agreement: details of ehds-relayed",
            detail(
                &relayed.document,
                "ehds-relayed",
                "agreement-name",
                "Regional exchange",
            ),
        ),
        (
            "is_emergency: the ehds-emergency-access entity naming its purpose",
            named(&urgent.document, "ehds-emergency-access")
                .is_some_and(|e| e.to_string().contains("ETREAT")),
        ),
        (
            "channel.request_id: entity of type X-Request-Id",
            entities(doc).iter().any(|e| {
                e["type"]["code"] == "XrequestId"
                    && e["what"]["identifier"]["value"] == "01JC2Q7N3X"
            }),
        ),
        (
            "channel.client_address: the network address of the client's agent",
            doc["agent"][0]["network"]["address"] == "10.216.24.150",
        ),
        (
            "system_id: source.observer.identifier, the site from the deployment",
            doc["source"]["observer"]["identifier"]["value"] == "cdr.example.org"
                && doc["source"]["site"] == "site-1",
        ),
        (
            "security_labels: on the entity they label, never meta.security",
            doc["meta"].get("security").is_none(),
        ),
        (
            "retention: details of ehds-categories",
            named(doc, "ehds-categories").is_some_and(|e| e.to_string().contains("retention")),
        ),
        (
            "link, loss: the chain does not travel",
            !doc.to_string().contains("content_digest"),
        ),
        (
            "other_details, loss: FerroEHR's access details are written as their values only",
            !doc.to_string().contains("ferroehr_access_details"),
        ),
    ]
}
