// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A patient's EHRs through the party domain, under both EHR indexes and on
//! two replicas (#5648, #5649).
//!
//! A contact point's TRC patient, an identifier of the patient's domain, is
//! found through the patient `PERSON` that carries it at the
//! `[authz.party_binding.patient]` identifier path, with no ITI-83 call, and
//! a subject the PIX Manager resolves reaches every EHR its `PERSON`'s record
//! subjects name, so after an identity merge, committed as demographic
//! content, the survivor's lookups find the merged patient's EHR. eHDSI XCA
//! Profile 8.0.1 §4.2.1 names the TRC patient and PMIR 1.6.0 §2:3.93.4.1.3
//! leaves a merge to the consumer; the resolution is our own design.

use std::sync::Arc;

use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

use crate::ehr_index_ihe::{CLINICAL, OPERATION, RED, answer, red_to_blue};
use crate::fixtures::{create_ehr_subject, if_match, object};
use crate::policy_fixtures::{as_admin, create, delete, granting};
use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::ehr_index::provider::EhrIndexProvider;
use ferroehr::service::ehr_index::types::SubjectRef;
use ferroehr::service::version_update::WriteRequest;
use openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_ACCESS;

/// The patient template the default mapping names.
pub(crate) const PATIENT: &str = "openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0";
/// The patient's identifier domain, as a TRC's `system` names it.
pub(crate) const NATIONAL: &str = "urn:oid:2.999.4.1";
/// A synthetic identifier of a protected scheme, in no national number's form.
const SEALED: &str = "SEALED-PATIENT-1";

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

pub(crate) fn identifier(issuer: &str, id: &str, kind: &str) -> Value {
    json!({ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id, "type": kind })
}

/// A patient `PERSON` of the shipped template carrying `identifiers` of the
/// patient's domain and declaring `subjects` as its record subjects.
pub(crate) fn patient(name: &str, identifiers: &[Value], subjects: &[Value]) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-PERSON.patient.v1";
    let mut items: Vec<Value> = identifiers
        .iter()
        .map(|value| {
            json!({ "_type": "ELEMENT", "archetype_node_id": "id3",
                             "name": text("Patient identifier"), "value": value })
        })
        .collect();
    items.extend(subjects.iter().map(|value| {
        json!({ "_type": "ELEMENT", "archetype_node_id": "id36",
            "name": { "_type": "DV_CODED_TEXT", "value": "record_subject",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY_ID_ACCESS },
                    "code_string": "record_subject" } },
            "value": value })
    }));
    json!({
        "_type": "PERSON", "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "template_id": { "_type": "TEMPLATE_ID", "value": PATIENT } },
        "name": text(name),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "id2", "name": text("Details"),
                     "items": items },
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "id6", "name": text("Legal name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "id7", "name": text("Name structure"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "id8", "name": text("Full name"),
                            "value": text(name) }] } }]
    })
}

/// A replica over `db` with `provider` as its EHR index, the clinical domain
/// declared as the pseudonym namespace.
pub(crate) async fn replica(db: &testkit::TestDb, provider: EhrIndexProvider) -> FerroEhrService {
    let policy =
        ferroehr::privacy::PrivacyPolicy::compile(&ferroehr::privacy::config::PrivacyConfig {
            subject_namespaces: vec![CLINICAL.to_owned()],
            ..Default::default()
        })
        .expect("the deployment policy compiles");
    granting(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
            .with_privacy(Arc::new(policy))
            .with_ehr_index(provider),
    )
    .await
}

/// A replica whose index is a PIX Manager at `pix`.
async fn pixm_replica(db: &testkit::TestDb, pix: &MockServer) -> FerroEhrService {
    let ihe = match crate::ehr_index_ihe::service(db, &pix.uri()).ehr_index_provider() {
        EhrIndexProvider::Ihe(source) => EhrIndexProvider::Ihe(Arc::clone(source)),
        other => panic!("an IHE provider, not {}", other.name()),
    };
    replica(db, ihe).await
}

/// Commits `body` as the next version of the `PERSON` `vo` after `uid`, as an
/// administrator.
async fn revise(svc: &FerroEhrService, vo: VoId, uid: &str, body: &Value) {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = Some(uid.parse().expect("an OBJECT_VERSION_ID"));
    as_admin(async {
        Box::pin(svc.party_update(
            PartyKind::Person,
            object(vo.to_string()),
            if_match(uid),
            write,
        ))
        .await
    })
    .await
    .expect("the PERSON is revised");
}

/// `ehrs`, sorted.
pub(crate) fn sorted(mut ehrs: Vec<EhrId>) -> Vec<EhrId> {
    ehrs.sort_unstable();
    ehrs
}

/// Two patients A and B, each with an EHR on its own pseudonym; the merge
/// of A into B; and what the survivor's lookups answer before and after it.
struct Merge {
    a_uid: String,
    b: (VoId, String),
    pseudonyms: [String; 2],
    ehrs: [EhrId; 2],
}

impl Merge {
    /// Patients A and B on `svc`, each carrying its identifier of the
    /// patient's domain and declaring its pseudonym, with an EHR on it.
    async fn before(svc: &FerroEhrService) -> Self {
        let pseudonyms = [Uuid::now_v7().to_string(), Uuid::now_v7().to_string()];
        let (_, a_uid) = create(svc, PartyKind::Person, &Self::a(&pseudonyms[0]))
            .await
            .expect("patient A commits");
        let b = create(svc, PartyKind::Person, &Self::b(&pseudonyms[1], None))
            .await
            .expect("patient B commits");
        let ehrs = [
            create_ehr_subject(svc, &SubjectRef::person(&pseudonyms[0], CLINICAL)).await,
            create_ehr_subject(svc, &SubjectRef::person(&pseudonyms[1], CLINICAL)).await,
        ];
        Self {
            a_uid,
            b,
            pseudonyms,
            ehrs,
        }
    }

    fn a(pseudonym: &str) -> Value {
        patient(
            "A",
            &[identifier(NATIONAL, "NAT-A", "NI")],
            &[identifier(CLINICAL, pseudonym, "PI")],
        )
    }

    /// Patient B, after the merge carrying A's identifier and pseudonym too.
    fn b(pseudonym: &str, merged: Option<&str>) -> Value {
        let mut identifiers = vec![identifier(NATIONAL, "NAT-B", "NI")];
        let mut subjects = vec![identifier(CLINICAL, pseudonym, "PI")];
        if let Some(merged) = merged {
            identifiers.push(identifier(NATIONAL, "NAT-A", "NI"));
            subjects.push(identifier(CLINICAL, merged, "PI"));
        }
        patient("B", &identifiers, &subjects)
    }

    /// The merge as the registering system commits it on `svc`: A is
    /// deleted, and B carries A's identifier and declares A's pseudonym.
    async fn merge(&self, svc: &FerroEhrService) {
        delete(svc, PartyKind::Person, &self.a_uid).await;
        let (b, uid) = &self.b;
        revise(
            svc,
            *b,
            uid,
            &Self::b(&self.pseudonyms[1], Some(&self.pseudonyms[0])),
        )
        .await;
    }

    fn both(&self) -> Vec<EhrId> {
        sorted(self.ehrs.to_vec())
    }
}

/// The TRC patient `id` of the patient's domain.
fn trc(id: &str) -> SubjectRef {
    SubjectRef::person(id, NATIONAL)
}

/// The ITI-83 requests `pix` has received.
async fn pix_calls(pix: &MockServer) -> usize {
    pix.received_requests().await.expect("recorded").len()
}

#[tokio::test]
async fn under_pixm_a_merge_reaches_both_ehrs_on_another_replica_with_no_trc_pix_call() {
    let pix = MockServer::start().await;
    let db = testkit::db().await.expect("testkit database");
    let (one, two) = (pixm_replica(&db, &pix).await, pixm_replica(&db, &pix).await);
    let merge = Merge::before(&one).await;
    // NOTE: FerroPIX answers the survivor's pseudonym for a merged identifier (PIXm §2:3.83.4.2.2.5).
    Mock::given(method("POST"))
        .and(path(OPERATION))
        .respond_with(answer(200, &red_to_blue(&merge.pseudonyms[1])))
        .mount(&pix)
        .await;
    let red = SubjectRef::person("IHERED-994", RED);

    two.grants().refresh().await.expect("the copy reads");
    assert_eq!(
        two.subject_ehrs(&red).await.expect("the index answers"),
        vec![merge.ehrs[1]],
        "before the merge the survivor's pseudonym names its own EHR"
    );
    assert_eq!(
        two.patient_ehrs(&trc("NAT-A"))
            .await
            .expect("the TRC resolves"),
        vec![merge.ehrs[0]]
    );

    merge.merge(&one).await;
    two.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    assert_eq!(
        sorted(two.subject_ehrs(&red).await.expect("the index answers")),
        merge.both(),
        "after the merge the survivor's lookup finds the merged patient's EHR"
    );

    let asked = pix_calls(&pix).await;
    for _ in 0..3 {
        assert_eq!(
            sorted(
                two.patient_ehrs(&trc("NAT-A"))
                    .await
                    .expect("the TRC resolves")
            ),
            merge.both()
        );
        assert_eq!(
            sorted(
                two.patient_ehrs(&trc("NAT-B"))
                    .await
                    .expect("the TRC resolves")
            ),
            merge.both()
        );
    }
    assert_eq!(
        pix_calls(&pix).await,
        asked,
        "a contact point's TRC patient is resolved with no ITI-83 call"
    );
    assert_eq!(
        two.patient_ehrs(&trc("NAT-X"))
            .await
            .expect("the TRC resolves"),
        Vec::new(),
        "an identifier no PERSON carries finds no EHR"
    );
}

#[tokio::test]
async fn under_local_a_merge_reaches_both_ehrs_on_another_replica() {
    let db = testkit::db().await.expect("testkit database");
    let (one, two) = (
        replica(&db, EhrIndexProvider::Local).await,
        replica(&db, EhrIndexProvider::Local).await,
    );
    let merge = Merge::before(&one).await;
    let survivor = SubjectRef::person(&merge.pseudonyms[1], CLINICAL);
    two.grants().refresh().await.expect("the copy reads");
    assert_eq!(
        two.subject_ehrs(&survivor)
            .await
            .expect("the index answers"),
        vec![merge.ehrs[1]]
    );

    merge.merge(&one).await;
    two.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    assert_eq!(
        sorted(
            two.subject_ehrs(&survivor)
                .await
                .expect("the index answers")
        ),
        merge.both()
    );
    assert_eq!(
        sorted(
            two.patient_ehrs(&trc("NAT-A"))
                .await
                .expect("the TRC resolves")
        ),
        merge.both()
    );

    // NOTE: a deployment that declares no pseudonym namespace stores the identifier verbatim.
    let verbatim =
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
            .with_ehr_index(EhrIndexProvider::Local);
    let named = create_ehr_subject(&verbatim, &trc("NAT-L")).await;
    assert_eq!(
        two.patient_ehrs(&trc("NAT-L"))
            .await
            .expect("the TRC resolves"),
        vec![named],
        "under local an identifier the EHR_STATUS carries names that EHR itself"
    );
}

#[tokio::test]
async fn an_identifier_two_persons_carry_finds_no_ehr() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db, EhrIndexProvider::Local).await;
    for name in ["C", "D"] {
        let pseudonym = Uuid::now_v7().to_string();
        create(
            &svc,
            PartyKind::Person,
            &patient(
                name,
                &[identifier(NATIONAL, "NAT-SHARED", "NI")],
                &[identifier(CLINICAL, &pseudonym, "PI")],
            ),
        )
        .await
        .expect("the patient commits");
        create_ehr_subject(&svc, &SubjectRef::person(&pseudonym, CLINICAL)).await;
    }
    assert_eq!(
        svc.patient_ehrs(&trc("NAT-SHARED"))
            .await
            .expect("the TRC resolves"),
        Vec::new(),
        "an ambiguous identifier fails closed"
    );
}

#[tokio::test]
async fn a_protected_identifier_is_found_through_its_lookup_digest() {
    let pix = MockServer::start().await;
    let db = testkit::db().await.expect("testkit database");
    let engine = Arc::new(crate::pseudonymisation_boundary::protection_engine(
        db.pool(),
    ));
    let one = pixm_replica(&db, &pix)
        .await
        .with_identifier_protection(Arc::clone(&engine));
    let two = pixm_replica(&db, &pix)
        .await
        .with_identifier_protection(engine);
    // NOTE: the scheme is the one the protection fixture seals; the issuer is a test domain.
    let issuer = "urn:oid:2.999.4.2";
    let pseudonym = Uuid::now_v7().to_string();
    let (person, _) = create(
        &one,
        PartyKind::Person,
        &patient(
            "E",
            &[identifier(issuer, SEALED, "nl-bsn")],
            &[identifier(CLINICAL, &pseudonym, "PI")],
        ),
    )
    .await
    .expect("the patient commits");
    let ehr = create_ehr_subject(&one, &SubjectRef::person(&pseudonym, CLINICAL)).await;

    let stored: Option<String> = sqlx::query_scalar(
        "SELECT value FROM party.binding_patient_identifier WHERE holder_id = $1",
    )
    .bind(person.0)
    .fetch_optional(&db.pool())
    .await
    .expect("the projection reads");
    assert!(
        stored.is_some_and(|value| !value.contains(SEALED)),
        "the projection holds the sealed row's reference, never the value"
    );
    assert_eq!(
        two.patient_ehrs(&SubjectRef::person(SEALED, issuer))
            .await
            .expect("the TRC resolves"),
        vec![ehr]
    );
    assert_eq!(
        two.patient_ehrs(&SubjectRef::person(SEALED, NATIONAL))
            .await
            .expect("the TRC resolves"),
        Vec::new(),
        "the issuer is compared with the value"
    );
    assert_eq!(pix_calls(&pix).await, 0);
}
