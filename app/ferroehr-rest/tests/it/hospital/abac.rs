// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ABAC rules of the hospital scenario, one test each: the care
//! relationship and its grace, the clearance, the restriction against the
//! ex-partner and its invisibility (EHDS Art. 8), the mental-health
//! restriction with its exception, the break-glass (Art. 11(5)), the subject
//! (Art. 3) and her representative, the regional consent, and the opt-out
//! from the access services (Art. 10). Each access's entry in the access log
//! is checked, and the chains verify at the end. The decision procedure is
//! our own design (#5487).

use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{BASE, EXT};
use crate::hospital::build::{directive, eva_directives, settle, world};
use crate::hospital::ledger::{Expect, Ledger};
use crate::hospital::world::{Answer, Ask, Doc, World, admit, ask, call, day};

/// The read of `doc` in Eva's EHR by `caller` declaring `purpose`.
pub(crate) async fn read(world: &World, caller: &str, doc: &Doc, purpose: Option<&str>) -> Answer {
    call(
        &world.app,
        caller,
        "GET",
        &format!("{BASE}/ehr/{}/composition/{}", world.ehr, doc.vo),
        purpose,
        None,
    )
    .await
}

/// The read of Eva's EHR itself by `caller` declaring `purpose`.
async fn read_ehr(world: &World, caller: &str, ehr: &str, purpose: Option<&str>) -> Answer {
    call(
        &world.app,
        caller,
        "GET",
        &format!("{BASE}/ehr/{ehr}"),
        purpose,
        None,
    )
    .await
}

/// The composition ids of Eva's EHR an AQL query scoped to it returns to
/// `caller` declaring `purpose`, and the answer.
pub(crate) async fn queried(
    world: &World,
    caller: &str,
    purpose: Option<&str>,
) -> (Vec<String>, Answer) {
    let body = json!({ "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c" });
    let answer = ask(
        &world.app,
        caller,
        ("POST", &format!("{BASE}/query/aql")),
        Ask {
            purpose,
            body: Some(&body),
            scope: Some(&world.ehr),
        },
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{caller}'s query: {}",
        answer.body
    );
    let mut ids: Vec<String> = answer.json()["rows"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row[0].as_str())
        .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
        .collect();
    ids.sort();
    (ids, answer)
}

/// The versioned objects of `docs`, sorted.
pub(crate) fn ids(docs: &[&Doc]) -> Vec<String> {
    let mut ids: Vec<String> = docs.iter().map(|doc| doc.vo.clone()).collect();
    ids.sort();
    ids
}

/// Asserts `answer` is `status` with the refusal `code` in its problem body.
#[track_caller]
pub(crate) fn answered(answer: &Answer, status: StatusCode, code: Option<&str>) {
    assert_eq!(answer.status, status, "{}", answer.body);
    let named = code.map_or(Value::Null, |code| json!(code));
    assert_eq!(answer.refusal(), named, "{}", answer.body);
}

/// The answer of a restriction says nothing: no restriction, no refusal
/// code, the body an unknown EHR's.
#[track_caller]
fn says_nothing(answer: &Answer) {
    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.body);
    let lowered = answer.body.to_lowercase();
    assert!(
        !lowered.contains("restrict")
            && !lowered.contains("refused")
            && !lowered.contains("directive"),
        "the answer names the restriction: {}",
        answer.body
    );
    assert_eq!(answer.json()["type"], "about:blank", "{}", answer.body);
}

const TREATMENT: Expect = Expect::granted(&["treatment relationship"]);

/// Dr. Jansen, rostered on Cardiology, reads the cardiology encounter of a
/// patient under Cardiology's care, on the grounds of that care relationship.
#[tokio::test]
async fn dr_jansen_reads_the_cardiology_data() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let answer = read(&world, "jansen", &world.encounter, Some("TREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(&world, "the encounter", &answer, TREATMENT);
    ledger.verify(&world).await;
}

/// Discharged from Cardiology, Eva stays readable to Dr. Jansen within the
/// 14-day grace the policy gives, and past it she is refused
/// `refused-no-care-relationship` at step 5.
#[tokio::test]
async fn past_the_grace_dr_jansen_is_refused_without_a_care_relationship() {
    let mut world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let (ward, recent, old_from, old_until) = (day(-2), day(-3), day(-40), day(-20));
    world.eva_version = admit(
        &world,
        &world.eva_version,
        &[
            ("Cardiology", &old_from, Some(&recent)),
            ("Ward 3", &ward, None),
            ("Psychiatry", &ward, None),
        ],
    )
    .await;
    let answer = read(&world, "jansen", &world.encounter, Some("TREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(&world, "within the grace", &answer, TREATMENT);

    world.eva_version = admit(
        &world,
        &world.eva_version,
        &[
            ("Cardiology", &old_from, Some(&old_until)),
            ("Ward 3", &ward, None),
            ("Psychiatry", &ward, None),
        ],
    )
    .await;
    let answer = read(&world, "jansen", &world.encounter, Some("TREAT")).await;
    answered(
        &answer,
        StatusCode::FORBIDDEN,
        Some("refused-no-care-relationship"),
    );
    ledger.note(
        &world,
        "past the grace",
        &answer,
        Expect::refused("refused-no-care-relationship", 5),
    );
    let (rows, _) = queried(&world, "jansen", Some("TREAT")).await;
    assert!(rows.is_empty(), "the query leaves the record out: {rows:?}");
    ledger.verify(&world).await;
}

/// Nurse de Vries reads the vital signs and is refused the HIV result,
/// which the policy classifies restricted and her privileges do not clear.
#[tokio::test]
async fn nurse_de_vries_reads_vitals_but_not_the_hiv_result() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let answer = read(&world, "devries", &world.vitals, Some("TREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(&world, "the vitals", &answer, TREATMENT);
    let answer = read(&world, "devries", &world.hiv, Some("TREAT")).await;
    answered(&answer, StatusCode::FORBIDDEN, Some("refused-sensitivity"));
    ledger.note(
        &world,
        "the HIV result",
        &answer,
        Expect::refused("refused-sensitivity", 4),
    );
    let (rows, _) = queried(&world, "devries", Some("TREAT")).await;
    assert!(
        !rows.contains(&world.hiv.vo),
        "the query leaves it out: {rows:?}"
    );
    ledger.verify(&world).await;
}

/// Eva's restriction refuses Gina everything as though the record did not
/// exist: each composition, the EHR, its settings and a query, each answer
/// the one an unknown EHR gets, each logged `refused-restricted` at step 2.
#[tokio::test]
async fn gina_is_refused_everything_and_never_sees_the_restriction() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    for doc in [&world.encounter, &world.vitals, &world.mental, &world.hiv] {
        let answer = read(&world, "gina", doc, Some("TREAT")).await;
        says_nothing(&answer);
        ledger.note(
            &world,
            format!("composition {}", doc.vo),
            &answer,
            Expect::refused("refused-restricted", 2),
        );
    }
    let answer = read_ehr(&world, "gina", &world.ehr, Some("TREAT")).await;
    says_nothing(&answer);
    ledger.note(
        &world,
        "the EHR",
        &answer,
        Expect::refused("refused-restricted", 2),
    );
    let unknown = read_ehr(
        &world,
        "gina",
        "00000000-0000-4000-8000-000000000000",
        Some("TREAT"),
    )
    .await;
    assert_eq!(
        unknown.status, answer.status,
        "an unknown EHR: {}",
        unknown.body
    );
    assert_eq!(unknown.json()["title"], answer.json()["title"]);
    let settings = call(
        &world.app,
        "gina",
        "GET",
        &format!("{EXT}/ehr/{}/ehr_access", world.ehr),
        Some("TREAT"),
        None,
    )
    .await;
    says_nothing(&settings);
    let (rows, _) = queried(&world, "gina", Some("TREAT")).await;
    assert!(rows.is_empty(), "the query returns nothing: {rows:?}");
    ledger.verify(&world).await;
}

/// The mental-health restriction hides the note from Dr. Jansen as absent,
/// logged `refused-restricted`, and its exception lets Dr. Smit read it.
#[tokio::test]
async fn the_mental_health_note_is_refused_to_dr_jansen_and_read_by_dr_smit() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let answer = read(&world, "jansen", &world.mental, Some("TREAT")).await;
    says_nothing(&answer);
    ledger.note(
        &world,
        "Dr. Jansen",
        &answer,
        Expect::refused("refused-restricted", 2),
    );
    let answer = read(&world, "smit", &world.mental, Some("TREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(&world, "Dr. Smit", &answer, TREATMENT);
    let (jansen, _) = queried(&world, "jansen", Some("TREAT")).await;
    let (smit, _) = queried(&world, "smit", Some("TREAT")).await;
    assert!(!jansen.contains(&world.mental.vo), "{jansen:?}");
    assert!(smit.contains(&world.mental.vo), "{smit:?}");
    ledger.verify(&world).await;
}

/// Dr. Bakker, in no care relationship, is refused; with the emergency role
/// and the emergency purpose he reads the restricted HIV result and the
/// restricted mental-health note, each access marked an emergency with the
/// ground it rests on.
#[tokio::test]
async fn dr_bakker_breaks_the_glass_and_the_log_marks_the_emergency() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let answer = read(&world, "bakker", &world.hiv, Some("TREAT")).await;
    answered(
        &answer,
        StatusCode::FORBIDDEN,
        Some("refused-no-care-relationship"),
    );
    ledger.note(
        &world,
        "an ordinary purpose",
        &answer,
        Expect::refused("refused-no-care-relationship", 5),
    );
    let answer = read(&world, "bakker", &world.hiv, Some("ETREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(
        &world,
        "the HIV result",
        &answer,
        Expect::granted(&["emergency"]).emergency(),
    );
    let answer = read(&world, "bakker", &world.mental, Some("ETREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(
        &world,
        "the mental-health note",
        &answer,
        Expect::granted(&["emergency"]).emergency(),
    );
    ledger.verify(&world).await;
}

/// Eva reads every composition of her own record, and her portal token
/// reaches no other EHR: `refused-not-subject`.
#[tokio::test]
async fn eva_reads_her_whole_record_and_nobody_elses() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let eva = format!("eva:{}", world.ehr);
    for doc in [&world.encounter, &world.vitals, &world.mental, &world.hiv] {
        let answer = read(&world, &eva, doc, None).await;
        answered(&answer, StatusCode::OK, None);
        ledger.note(
            &world,
            format!("her {}", doc.vo),
            &answer,
            Expect::granted(&["subject of care"]),
        );
    }
    let answer = read_ehr(&world, &eva, &world.other_ehr, None).await;
    answered(&answer, StatusCode::FORBIDDEN, Some("refused-not-subject"));
    ledger.note(
        &world,
        "another EHR",
        &answer,
        Expect::refused("refused-not-subject", 0),
    );
    ledger.verify(&world).await;
}

/// Frank, Eva's representative, reads her record under the delegation she
/// gave him, and reaches neither Gina's record nor any other.
#[tokio::test]
async fn frank_reads_evas_record_and_no_other() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let frank = format!("frank:{}", world.ehr);
    for doc in [&world.encounter, &world.vitals, &world.hiv] {
        let answer = read(&world, &frank, doc, None).await;
        answered(&answer, StatusCode::OK, None);
        ledger.note(
            &world,
            format!("her {}", doc.vo),
            &answer,
            Expect::granted(&[]),
        );
    }
    // NOTE: SMART App Launch 2.2.0 §Launch context, the authorization server chooses the EHR a token names, so his token names Eva's.
    let answer = read_ehr(&world, &frank, &world.other_ehr, None).await;
    answered(&answer, StatusCode::FORBIDDEN, Some("refused-not-subject"));
    ledger.note(
        &world,
        "Gina's record",
        &answer,
        Expect::refused("refused-not-subject", 0),
    );
    ledger.verify(&world).await;
}

/// The regional exchange's client of a member provider reads under Eva's
/// consent; a client of a provider outside the region is refused
/// `refused-consent-required`; once the consent's year has passed the
/// member's client is refused too.
#[tokio::test]
async fn the_regional_exchange_reads_under_the_consent_until_it_lapses() {
    let mut world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let answer = read(&world, "client:hie-member", &world.vitals, Some("TREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(&world, "a member", &answer, Expect::granted(&["consent"]));
    let answer = read(&world, "client:hie-outside", &world.vitals, Some("TREAT")).await;
    answered(
        &answer,
        StatusCode::FORBIDDEN,
        Some("refused-consent-required"),
    );
    ledger.note(
        &world,
        "an outsider",
        &answer,
        Expect::refused("refused-consent-required", 7),
    );
    let lapsed = eva_directives(&world, Some((-400, -35)), &[]);
    settle(&mut world, &lapsed).await;
    let answer = read(&world, "client:hie-member", &world.vitals, Some("TREAT")).await;
    answered(
        &answer,
        StatusCode::FORBIDDEN,
        Some("refused-consent-required"),
    );
    ledger.note(
        &world,
        "a lapsed consent",
        &answer,
        Expect::refused("refused-consent-required", 7),
    );
    ledger.verify(&world).await;
}

/// Eva's opt-out from the access services hides her record from the access
/// service as absent, logged `refused-opt-out`, while her treating clinician
/// reads it; withdrawing it restores the access service's access.
#[tokio::test]
async fn the_opt_out_hides_the_record_from_the_access_services_until_withdrawn() {
    let mut world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let opt_out = directive("opt_out", "deny", &json!({ "emergency_override": true }));
    let opted = eva_directives(&world, None, &[opt_out]);
    settle(&mut world, &opted).await;
    let answer = read(
        &world,
        "client:access-service",
        &world.vitals,
        Some("TREAT"),
    )
    .await;
    says_nothing(&answer);
    ledger.note(
        &world,
        "the access service",
        &answer,
        Expect::refused("refused-opt-out", 1),
    );
    let answer = read(&world, "jansen", &world.vitals, Some("TREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(&world, "her clinician", &answer, TREATMENT);
    let withdrawn = eva_directives(&world, None, &[]);
    settle(&mut world, &withdrawn).await;
    let answer = read(
        &world,
        "client:access-service",
        &world.vitals,
        Some("TREAT"),
    )
    .await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(
        &world,
        "after the withdrawal",
        &answer,
        Expect::granted(&["consent"]),
    );
    ledger.verify(&world).await;
}
