// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A migration as a system client (#5432), over a real PostgreSQL 18: an
//! `AGENT` whose `ROLE` grants `import_ehr`, `import_ehr_extract` or
//! `export_ehr_extract` for a subject namespace imports and exports the EHRs
//! of that namespace with no person's token, under its own `ACTOR`, and a
//! system client without such a grant imports and exports nothing.
//!
//! An `IMPORTED_VERSION` has "its own audit trail and Contribution"
//! (`org.openehr.rm.common.imported_version.adoc`). Who may import or export
//! is governed by no openEHR spec: our own design.

use serde_json::{Value, json};

use crate::fixtures::{composition, create_ehr_subject};
use crate::system_grants::{System, capability, named, run, system, system_client, text};
use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr_index::types::SubjectRef;
use ferroehr::service::message::import_policy::ImportPolicy;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::change::Committed;
use openehr_rm::v1_2::ehr_extract::common::extract::Extract;
use openehr_rm::v1_2::ehr_extract::common::extract_spec::ExtractSpec;

/// The subject namespace of the legacy system's patients.
const LEGACY: &str = "nl.legacy-his.patients";

/// A subject of [`LEGACY`].
fn subject(id: &str) -> SubjectRef {
    SubjectRef {
        id: id.to_owned(),
        namespace: LEGACY.to_owned(),
        r#type: "PERSON".to_owned(),
    }
}

/// A capability granting `operation` over the EHRs of `namespace`.
fn grant(operation: &str, namespace: &str) -> Value {
    capability(operation, &[named("subject_namespace", &text(namespace))])
}

/// The system caller `client_id` of `svc`, acting for a provider and granted
/// `capabilities`.
async fn granted(svc: &FerroEhrService, client_id: &str, capabilities: &[Value]) -> System {
    system_client(svc, client_id, true, capabilities).await;
    system(svc, client_id).await
}

/// Commits a COMPOSITION into `ehr` as the server.
async fn commit(svc: &FerroEhrService, ehr: EhrId, name: &str) -> Committed {
    Box::pin(svc.create_composition(
        ehr,
        WriteRequest::new(crate::typed_body::typed(&composition(name))),
    ))
    .await
    .expect("the source commits")
}

/// The extract of the one COMPOSITION `committed` of `ehr` on `source`.
async fn composition_extract(
    source: &FerroEhrService,
    ehr: EhrId,
    committed: &Committed,
) -> Extract {
    let spec: ExtractSpec = openehr_its::json::from_canonical_value(&json!({
        "_type": "EXTRACT_SPEC",
        "manifest": { "_type": "EXTRACT_MANIFEST", "entities": [{
            "_type": "EXTRACT_ENTITY_MANIFEST", "extract_id_key": ehr.to_string(),
            "ehr_id": ehr.to_string(), "other_ids": [],
            "item_list": [{ "_type": "OBJECT_REF", "namespace": "local",
                "type": "VERSIONED_COMPOSITION",
                "id": { "_type": "HIER_OBJECT_ID", "value": committed.vo_id.to_string() } }]
        }] },
        "extract_type": { "_type": "DV_CODED_TEXT", "value": "openehr-ehr",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "openehr-ehr" } },
        "include_multimedia": true, "priority": 0, "link_depth": 0, "criteria": []
    }))
    .expect("an EXTRACT_SPEC");
    let mut extracts = source
        .export_ehr_extracts(spec)
        .await
        .expect("the composition export");
    openehr_its::json::from_canonical_value(&extracts.remove(0)).expect("an EXTRACT")
}

/// `extract` as the legacy system sends it: FerroEHR's own export rewrites
/// every reference's namespace to `local` (RM `ehr_extract`
/// `master09-semantics.adoc` §Creation Semantics), and the legacy system's
/// `EHR_STATUS` names its subject under [`LEGACY`].
fn from_legacy(extract: &Extract) -> Extract {
    let mut value = openehr_its::json::to_canonical_value(extract);
    for chapter in value["chapters"].as_array_mut().into_iter().flatten() {
        for item in chapter["items"].as_array_mut().into_iter().flatten() {
            if item.pointer("/item/_type").and_then(Value::as_str) != Some("X_VERSIONED_EHR_STATUS")
            {
                continue;
            }
            for version in item["item"]["versions"]
                .as_array_mut()
                .into_iter()
                .flatten()
            {
                version["data"]["subject"]["external_ref"]["namespace"] = json!(LEGACY);
            }
        }
    }
    openehr_its::json::from_canonical_value(&value).expect("an EXTRACT")
}

/// Asserts a `403`.
fn forbidden<T: std::fmt::Debug>(result: Result<T, ServiceError>) {
    match result.map_err(ServiceError::flattened) {
        Err(ServiceError::Forbidden(_)) => {}
        other => panic!("expected 403, got {other:?}"),
    }
}

#[tokio::test]
async fn a_migration_agent_imports_and_exports_only_its_granted_namespaces() {
    let (_source_db, source) = crate::fixtures::service().await;
    let (_target_db, target) = crate::fixtures::service().await;
    let target = target
        .with_import_policy(ImportPolicy::new(
            Some("IMPORTER".to_owned()),
            Vec::new(),
            Vec::new(),
        ))
        .with_grant_ceiling(GrantCeiling::new(Operation::ALL));

    let legacy = create_ehr_subject(&source, &subject("pt-1")).await;
    let first = commit(&source, legacy, "before the cut").await;
    let whole: Extract = openehr_its::json::from_canonical_value(
        &source.extract_ehrs(legacy).await.expect("the export")[0],
    )
    .expect("an EXTRACT");
    let whole = from_legacy(&whole);
    let later = commit(&source, legacy, "after the cut").await;
    let appended = composition_extract(&source, legacy, &later).await;

    // A client with no grant, or a grant of another namespace or another
    // operation, imports nothing.
    let nobody = system(&target, "nobody").await;
    let elsewhere = granted(
        &target,
        "elsewhere",
        &[grant("import_ehr", "other.patients")],
    )
    .await;
    let exporter = granted(&target, "exporter", &[grant("export_ehr_extract", LEGACY)]).await;
    for caller in [&nobody, &elsewhere, &exporter] {
        forbidden(run(Some(caller), target.import_ehr(None, whole.clone())).await);
    }

    // The migration imports the EHR, as its own ACTOR.
    let migration = granted(
        &target,
        "migration",
        &[
            grant("import_ehr", LEGACY),
            grant("import_ehr_extract", LEGACY),
        ],
    )
    .await;
    let agent = migration
        .identity
        .agent
        .expect("the client names its AGENT");
    let imported = run(Some(&migration), target.import_ehr(None, whole))
        .await
        .expect("the granted migration imports the EHR");
    let ovid = format!(
        "{}::{}::{}",
        first.vo_id, first.creating_system_id, first.tree
    );
    let envelope = target
        .composition_version_envelope(imported, ovid.parse().expect("an OBJECT_VERSION_ID"))
        .await
        .expect("the imported version");
    assert_eq!(
        envelope.body["commit_audit"]["committer"]["external_ref"],
        json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": "ACTOR",
                "id": { "_type": "HIER_OBJECT_ID", "value": agent.to_string() } }),
        "the import is committed by the migration's ACTOR: {}",
        envelope.body["commit_audit"]
    );

    // An extract appended into an EHR of the namespace needs its own grant.
    let into = create_ehr_subject(&target, &subject("pt-2")).await;
    let first_only = granted(&target, "importer", &[grant("import_ehr", LEGACY)]).await;
    forbidden(
        run(
            Some(&first_only),
            target.import_ehr_extract(into, appended.clone()),
        )
        .await,
    );
    run(Some(&migration), target.import_ehr_extract(into, appended))
        .await
        .expect("the granted migration appends the extract");

    // Only an export grant of the namespace exports, for a system caller.
    forbidden(run(Some(&migration), target.extract_ehrs(into)).await);
    forbidden(run(Some(&elsewhere), target.extract_ehrs(into)).await);
    run(Some(&exporter), target.extract_ehrs(into))
        .await
        .expect("the granted exporter exports");
    run(None, target.extract_ehrs(into))
        .await
        .expect("an export by no system caller is not a grant's to decide");
}

#[tokio::test]
async fn the_import_role_imports_without_a_grant() {
    let (_source_db, source) = crate::fixtures::service().await;
    let (_target_db, target) = crate::fixtures::service().await;
    let target = target.with_import_policy(ImportPolicy::new(
        Some("IMPORTER".to_owned()),
        Vec::new(),
        Vec::new(),
    ));
    let legacy = create_ehr_subject(&source, &subject("pt-1")).await;
    commit(&source, legacy, "a note").await;
    let whole: Extract = openehr_its::json::from_canonical_value(
        &source.extract_ehrs(legacy).await.expect("the export")[0],
    )
    .expect("an EXTRACT");
    let person = system(&target, "nobody").await.identity;
    let person = ferroehr::versioning::committer::CommitterIdentity {
        system: false,
        agent: None,
        ..person
    };
    ferroehr::system_log::access_context::with_roles(
        vec!["IMPORTER".to_owned()],
        ferroehr::versioning::committer::with_committer(
            Some(person),
            Box::pin(target.import_ehr(None, whole)),
        ),
    )
    .await
    .expect("the import role imports as before");
}
