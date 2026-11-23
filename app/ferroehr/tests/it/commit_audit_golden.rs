// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every reader of a commit audit renders it as committed goldens hold it
//! (#6395): a professional's server-built audit, one with the client's own
//! description, a person's `PARTY_SELF` commit, a CONTRIBUTION's stated
//! audit and a demographic commit, each read as its version, its revision
//! history, its CONTRIBUTION and its export, with every served digest
//! signature recomputing. Identifiers and instants are numbered by first
//! appearance; `FERROEHR_BLESS_GOLDEN=1` writes the goldens again.

use std::collections::BTreeMap;
use std::path::PathBuf;

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::version_update::WriteRequest;
use ferroehr::service::{DEFAULT_SYSTEM_ID, FerroEhrService};
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};
use ferroehr::versioning::signature::signer::Signer;
use ferroehr::versioning::signature::verify::Verdict;
use openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json;
use serde_json::{Value, json};

use crate::fixtures::{change_type, composition, lifecycle};

/// The professional's login and its token issuer.
const LOGIN: &str = "dr.jansen";
const ISSUER: &str = "https://idp.example";

/// The authenticated professional.
fn professional() -> CommitterIdentity {
    CommitterIdentity {
        subject: LOGIN.to_owned(),
        id_type: "oauth2",
        issuer: Some(ISSUER.to_owned()),
        local_issuer: DEFAULT_SYSTEM_ID.into(),
        client: None,
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: false,
        agent: None,
        external: None,
    }
}

/// The record's person, writing through a patient-context credential.
fn person() -> CommitterIdentity {
    CommitterIdentity {
        subject: "jane".to_owned(),
        person: true,
        ..professional()
    }
}

/// The professional's committer as a client states it.
fn matching() -> Value {
    json!({
        "_type": "PARTY_IDENTIFIED",
        "name": LOGIN,
        "identifiers": [{ "issuer": ISSUER, "id": LOGIN, "type": "oauth2" }]
    })
}

/// An `UPDATE_VERSION` of `data` by `committer`, with the client's
/// `description` when given, superseding `preceding` when given.
fn version<T: serde::de::DeserializeOwned, R: crate::fixtures::Envelope<T>>(
    data: &Value,
    code: &str,
    preceding: Option<&str>,
    committer: &Value,
    description: Option<&str>,
) -> R {
    let mut audit = json!({ "change_type": change_type(code, "change"), "committer": committer });
    if let Some(text) = description {
        audit["description"] = json!({ "_type": "DV_TEXT", "value": text });
    }
    let mut envelope = json!({
        "lifecycle_state": lifecycle("532"),
        "data": data,
        "commit_audit": audit
    });
    if let Some(preceding) = preceding {
        envelope["preceding_version_uid"] = json!({ "value": preceding });
    }
    R::from_version(openehr_its::json::from_canonical_value(&envelope).expect("an UPDATE_VERSION"))
}

/// A minimal PERSON named `name`.
fn person_party(name: &str) -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": name }
                }]
            }
        }]
    })
}

/// The `OBJECT_VERSION_ID` of `committed`.
fn ovid(committed: &ferroehr::versioning::change::Committed) -> String {
    format!(
        "{}::{}::{}",
        committed.vo_id, committed.creating_system_id, committed.tree
    )
}

/// Asserts that a served `ORIGINAL_VERSION`'s digest signature recomputes
/// from its canonical form, the bytes it was signed over.
fn assert_signed(document: &Value) {
    if document["_type"] != "ORIGINAL_VERSION" {
        return;
    }
    let signature = document["signature"].as_str().expect("a signature");
    let canonical = canonical_form_of_json(document).expect("a canonical form");
    assert_eq!(
        Signer::digest_default().verify(&canonical, signature),
        Verdict::DigestMatch,
        "the signature recomputes over the served version"
    );
}

/// `value` with every UUID, instant and signature replaced by a placeholder
/// numbered by first appearance.
fn normalised(value: &Value, seen: &mut BTreeMap<String, String>) -> Value {
    let uuid = regex::Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
        .expect("a pattern");
    let instant =
        regex::Regex::new(r"\d{4}-\d{2}-\d{2}T[0-9:.]+(Z|[+-]\d{2}:?\d{2})?").expect("a pattern");
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let v = if k == "signature" && v.is_string() {
                        json!("<signature>")
                    } else {
                        normalised(v, seen)
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(|v| normalised(v, seen)).collect()),
        Value::String(text) => {
            let mut out = text.clone();
            for (pattern, tag) in [(&uuid, "uuid"), (&instant, "instant")] {
                let found: Vec<String> = pattern
                    .find_iter(&out)
                    .map(|m| m.as_str().to_owned())
                    .collect();
                for raw in found {
                    let next = seen.len();
                    let placeholder = seen
                        .entry(raw.clone())
                        .or_insert_with(|| format!("<{tag}#{next}>"))
                        .clone();
                    out = out.replace(&raw, &placeholder);
                }
            }
            Value::String(out)
        }
        other => other.clone(),
    }
}

/// Holds `documents` to the golden `name`, or writes it when blessing.
fn golden(name: &str, documents: &[(&str, Value)]) {
    let mut seen = BTreeMap::new();
    let mut text = String::new();
    for (label, document) in documents {
        assert_signed(document);
        let shown = serde_json::to_string_pretty(&normalised(document, &mut seen))
            .expect("a document renders");
        text.push_str(&format!("== {label}\n{shown}\n"));
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/it/golden/commit_audit")
        .join(format!("{name}.txt"));
    if std::env::var_os("FERROEHR_BLESS_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("the directory");
        std::fs::write(&path, &text).expect("the golden writes");
        return;
    }
    let expected = std::fs::read_to_string(&path).expect("the golden is committed");
    assert_eq!(text, expected, "{name} renders as its golden");
}

/// The version `committed`, its revision history and its CONTRIBUTION as the
/// readers serve them.
async fn read_back(
    svc: &FerroEhrService,
    ehr: EhrId,
    committed: &ferroehr::versioning::change::Committed,
) -> Vec<(&'static str, Value)> {
    let version = svc
        .composition_version_envelope(ehr, ovid(committed).parse().expect("an ovid"))
        .await
        .expect("the version")
        .body;
    let history = svc
        .composition_revision_history(ehr, committed.vo_id)
        .await
        .expect("the history")
        .body;
    let contribution: uuid::Uuid = version["contribution"]["id"]["value"]
        .as_str()
        .expect("the contribution id")
        .parse()
        .expect("a uuid");
    let listed = svc
        .ehr_contribution(ehr, contribution, false)
        .await
        .expect("the contribution")
        .body;
    let resolved = svc
        .ehr_contribution(ehr, contribution, true)
        .await
        .expect("the resolved contribution")
        .body;
    vec![
        ("version", version),
        ("revision history", history),
        ("contribution", listed),
        ("resolved contribution", resolved),
    ]
}

/// An EHR of the professional's, created by them.
async fn ehr(svc: &FerroEhrService) -> EhrId {
    with_committer(
        Some(professional()),
        Box::pin(svc.create_ehr_meta(WriteRequest::new(None))),
    )
    .await
    .expect("the EHR")
    .0
}

#[tokio::test]
async fn a_professionals_server_built_audit_reads_as_its_golden() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = ehr(&svc).await;
    let write = svc.create_composition(
        ehr,
        version(&composition("a"), "249", None, &matching(), None),
    );
    let committed = with_committer(Some(professional()), Box::pin(write))
        .await
        .expect("the commit");
    golden(
        "professional_default",
        &read_back(&svc, ehr, &committed).await,
    );
}

#[tokio::test]
async fn a_professionals_own_description_reads_as_its_golden() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = ehr(&svc).await;
    let write = svc.create_composition(
        ehr,
        version(&composition("a"), "249", None, &matching(), None),
    );
    let first = with_committer(Some(professional()), Box::pin(write))
        .await
        .expect("the commit");
    let write = svc.update_composition(
        ehr,
        first.vo_id,
        version(
            &composition("b"),
            "251",
            Some(&ovid(&first)),
            &matching(),
            Some("Dose corrected after review"),
        ),
    );
    let second = with_committer(Some(professional()), Box::pin(write))
        .await
        .expect("the update");
    golden(
        "professional_described",
        &read_back(&svc, ehr, &second).await,
    );
}

#[tokio::test]
async fn a_persons_audit_reads_as_its_golden() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = ehr(&svc).await;
    let mut body = composition("mine");
    body["composer"] = json!({ "_type": "PARTY_SELF" });
    let write = svc.create_composition(
        ehr,
        version(&body, "249", None, &json!({ "_type": "PARTY_SELF" }), None),
    );
    let committed = with_committer(Some(person()), Box::pin(write))
        .await
        .expect("the person's commit");
    golden("person", &read_back(&svc, ehr, &committed).await);
}

#[tokio::test]
async fn a_contributions_stated_audit_reads_as_its_golden() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = ehr(&svc).await;
    let member = json!({
        "data": composition("c"),
        "lifecycle_state": lifecycle("532"),
        "commit_audit": {
            "change_type": change_type("249", "creation"),
            "committer": matching(),
            "description": { "_type": "DV_TEXT", "value": "Imported from the ward system" }
        }
    });
    let audit = json!({
        "change_type": change_type("249", "creation"),
        "committer": matching(),
        "description": {
            "_type": "DV_CODED_TEXT", "value": "batch",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": "at0001"
            }
        }
    });
    let write =
        svc.ehr_contribution_commit(ehr, json!({ "versions": [member], "audit": audit }), true);
    let answer = with_committer(Some(professional()), Box::pin(write))
        .await
        .expect("the CONTRIBUTION")
        .body;
    let reference = answer["versions"][0]["id"]["value"]
        .as_str()
        .expect("the member's version id")
        .to_owned();
    let vo: VoId = reference
        .split("::")
        .next()
        .expect("a vo id")
        .parse()
        .expect("a vo uuid");
    let version = svc
        .composition_version_envelope(ehr, reference.parse().expect("an ovid"))
        .await
        .expect("the member")
        .body;
    let history = svc
        .composition_revision_history(ehr, vo)
        .await
        .expect("the history")
        .body;
    golden(
        "contribution",
        &[
            ("answer", answer),
            ("version", version),
            ("revision history", history),
        ],
    );
}

#[tokio::test]
async fn a_demographic_audit_reads_as_its_golden() {
    let (_db, svc) = crate::fixtures::service().await;
    let contribution = json!({
        "versions": [{
            "data": person_party("Jane"),
            "lifecycle_state": lifecycle("532"),
            "commit_audit": { "change_type": change_type("249", "creation"), "committer": matching() }
        }],
        "audit": { "change_type": change_type("249", "creation"), "committer": matching() }
    });
    let written = with_committer(
        Some(professional()),
        Box::pin(svc.demographic_contribution_create(contribution)),
    )
    .await
    .expect("the demographic CONTRIBUTION");
    let uid: uuid::Uuid = written
        .meta
        .expect("the CONTRIBUTION's meta")
        .uid
        .parse()
        .expect("a uuid");
    let read = svc
        .demographic_contribution_get(uid)
        .await
        .expect("the CONTRIBUTION")
        .body;
    let reference = read["versions"][0]["id"]["value"]
        .as_str()
        .expect("the member's version id")
        .to_owned();
    let vo: VoId = reference
        .split("::")
        .next()
        .expect("a vo")
        .parse()
        .expect("a vo uuid");
    let tree = crate::fixtures::tree(&reference);
    let version = svc
        .versioned_party_version_get_by_id(vo, tree)
        .await
        .expect("the party version")
        .body;
    golden(
        "demographic",
        &[("contribution", read), ("version", version)],
    );
}
