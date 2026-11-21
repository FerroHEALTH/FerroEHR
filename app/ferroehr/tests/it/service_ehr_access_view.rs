// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The settings of a marked record as each reader is served them, against a
//! real `PostgreSQL` 18 (#6165): the subject and the controller read the
//! `EHR_ACCESS` whole, a representative named a gate-keeper reads it without
//! the marking and with no signature on a version, in the object reads, the
//! versioned reads, the CONTRIBUTION, the EHR Extract and AQL (draft access
//! control, The Decision Procedure, Reading and Writing the Settings,
//! item 3).

use ferroehr::error::ServiceError;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr::object::EhrObject;
use ferroehr::service::query::request::AqlQueryRequest;
use serde_json::Value;

use crate::service_ehr_access::{
    Caller, administrator, as_caller, kept, marked, representative, setup, subject, write,
};

/// A record whose settings name the subject and the guardian gate-keepers
/// and whose marking the controller set to `R`, with the CONTRIBUTION that
/// set it.
async fn marked_record() -> (
    testkit::TestDb,
    FerroEhrService,
    EhrId,
    uuid::Uuid,
    uuid::Uuid,
) {
    let (db, svc, ehr_id, guardian) = setup().await;
    let contribution = write(
        &svc,
        ehr_id,
        &administrator(),
        1,
        &marked(kept(guardian), "R"),
    )
    .await
    .expect("the controller marks the record");
    let contribution = contribution.parse().expect("a uuid");
    (db, svc, ehr_id, guardian, contribution)
}

/// The readers: who they are, the caller, and whether they read the
/// settings whole.
fn readers(guardian: uuid::Uuid) -> [(&'static str, Caller, bool); 3] {
    [
        ("the controller", administrator(), true),
        ("the subject", subject(), true),
        ("the guardian", representative(guardian), false),
    ]
}

/// Whether `settings` carry the marking, asserting they still name their
/// gate-keepers.
fn carries_marking(settings: &Value, at: &str) -> bool {
    assert!(
        settings.get("gate_keepers").is_some(),
        "{at}: the gate-keepers are served: {settings}"
    );
    settings.get("marking").is_some()
}

/// The object reads and the versioned reads: whole to the subject and the
/// controller, and to the guardian without the marking, with no signature on
/// a version and the response marked redacted.
#[tokio::test]
async fn a_representative_reads_the_settings_without_the_marking() {
    let (_db, svc, ehr_id, guardian, _) = marked_record().await;
    for (who, caller, whole) in readers(guardian) {
        let current = as_caller(&caller, svc.ehr_object(ehr_id, EhrObject::Access, None))
            .await
            .expect("the EHR_ACCESS reads");
        assert_eq!(
            carries_marking(&current.body["settings"], who),
            whole,
            "{who}: the current object"
        );
        let meta = current.meta.expect("the version metadata");
        assert_eq!(
            meta.redacted, !whole,
            "{who}: the object is marked redacted"
        );
        let (vo, version) = meta.uid.split_once("::").expect("an OBJECT_VERSION_ID");
        let vo = VoId(vo.parse().expect("a uuid"));
        let tree = version.rsplit("::").next().expect("a version tree id");
        let at_version = as_caller(
            &caller,
            svc.ehr_object_at_version(ehr_id, EhrObject::Access, vo, tree),
        )
        .await
        .expect("the version reads");
        assert_eq!(
            carries_marking(&at_version.body["settings"], who),
            whole,
            "{who}: the object at its version"
        );
        for envelope in [
            as_caller(
                &caller,
                svc.ehr_object_version_at_time(ehr_id, EhrObject::Access, None),
            )
            .await
            .expect("the latest VERSION reads"),
            as_caller(
                &caller,
                svc.ehr_object_version_envelope(ehr_id, EhrObject::Access, vo, tree),
            )
            .await
            .expect("the VERSION reads"),
        ] {
            assert_eq!(
                carries_marking(&envelope.body["data"]["settings"], who),
                whole,
                "{who}: the VERSION"
            );
            assert_eq!(
                envelope.body.get("signature").is_some(),
                whole,
                "{who}: the VERSION's signature: {}",
                envelope.body
            );
            assert_eq!(
                envelope.meta.as_ref().is_some_and(|meta| meta.redacted),
                !whole,
                "{who}: the VERSION is marked redacted"
            );
        }
    }
}

/// The resolved CONTRIBUTION that set the marking and the EHR Extract serve
/// the guardian the version without its marking and signature.
#[tokio::test]
async fn a_representative_reads_the_contribution_and_the_extract_without_the_marking() {
    let (_db, svc, ehr_id, guardian, contribution) = marked_record().await;
    for (who, caller, whole) in readers(guardian) {
        let read = as_caller(&caller, svc.ehr_contribution(ehr_id, contribution, true))
            .await
            .expect("the CONTRIBUTION reads");
        let version = &read.body["versions"][0];
        assert_eq!(
            carries_marking(&version["data"]["settings"], who),
            whole,
            "{who}: the CONTRIBUTION"
        );
        assert_eq!(
            version.get("signature").is_some(),
            whole,
            "{who}: signature"
        );
        assert_eq!(read.redacted, !whole, "{who}: the CONTRIBUTION is redacted");

        let extracts = as_caller(&caller, svc.extract_ehrs(ehr_id))
            .await
            .expect("the export runs");
        let access = extracts[0]["chapters"][0]["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|item| item["item"]["_type"] == "X_VERSIONED_EHR_ACCESS")
            .expect("the extract carries the EHR_ACCESS")
            .clone();
        let version = &access["item"]["versions"][0];
        assert_eq!(
            carries_marking(&version["data"]["settings"], who),
            whole,
            "{who}: the extract"
        );
        assert_eq!(
            version.get("signature").is_some(),
            whole,
            "{who}: signature"
        );
        assert_eq!(
            as_caller(&caller, svc.extract_redacts(ehr_id))
                .await
                .expect("the view reads"),
            !whole,
            "{who}: the extract is redacted"
        );
    }
}

/// The outcome of `aql` for `caller`: its rows, or the refusal.
async fn rows(
    svc: &FerroEhrService,
    caller: &Caller,
    aql: &str,
) -> Result<Vec<Value>, ServiceError> {
    let outcome = as_caller(
        caller,
        svc.execute_aql(aql, None, &AqlQueryRequest::default()),
    )
    .await?;
    Ok(
        outcome.result_set.to_value().expect("the RESULT_SET")["rows"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
    )
}

/// In AQL, pinned to the EHR or across the store, the guardian is served
/// the whole object and its settings without the marking, `DISTINCT`
/// included, and the controller and the subject with it. No path below
/// `settings` resolves, for any reader alike, so no predicate, projection,
/// aggregate or order reads the marking itself.
#[tokio::test]
async fn a_representatives_query_reads_the_settings_without_the_marking() {
    let (_db, svc, ehr_id, guardian, _) = marked_record().await;
    let level = "a/settings/marking/confidentiality/defining_code/code_string";
    for from in [
        format!("FROM EHR e[ehr_id/value='{ehr_id}'] CONTAINS EHR_ACCESS a"),
        "FROM EHR e CONTAINS EHR_ACCESS a".to_owned(),
    ] {
        for (who, caller, whole) in readers(guardian) {
            let at = format!("{who}, {from}");
            for (select, settings) in [
                ("a", "/settings"),
                ("a/settings", ""),
                ("DISTINCT a/settings", ""),
            ] {
                let served = rows(&svc, &caller, &format!("SELECT {select} {from}"))
                    .await
                    .unwrap_or_else(|error| panic!("{at}: SELECT {select}: {error:?}"));
                assert_eq!(served.len(), 1, "{at}: SELECT {select}: one row");
                let settings = served[0][0].pointer(settings).cloned().unwrap_or_default();
                assert_eq!(
                    carries_marking(&settings, &at),
                    whole,
                    "{at}: SELECT {select}"
                );
            }
            for aql in [
                format!("SELECT {level} {from}"),
                format!("SELECT a/uid/value {from} WHERE {level} = 'R'"),
                format!("SELECT a/uid/value {from} WHERE EXISTS a/settings/marking"),
                format!("SELECT COUNT({level}) {from}"),
                format!("SELECT a/uid/value {from} ORDER BY {level}"),
            ] {
                assert!(
                    matches!(
                        rows(&svc, &caller, &aql).await,
                        Err(ServiceError::BadRequest(_))
                    ),
                    "{at}: no path below settings resolves: {aql}"
                );
            }
        }
    }
}
