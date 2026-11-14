// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(
    clippy::disallowed_types,
    reason = "a test builds a token's claim map directly; the claim set is RFC 7519-open"
)]

use std::sync::Arc;

use ferrohealth_smart::resource::{Access, Caller, Honoured};

use super::*;

const BASE: &str = "/ferroehr/rest/openehr/v1";

/// The permission letter `op` exercises, `r` for an operation without a row.
fn permission_of_op(op: &str) -> Permission {
    operation(op).map_or(Permission::Read, Operation::permission)
}

fn routes() -> SmartRoutes {
    SmartRoutes::new(
        &crate::api::Namespaces::of(&ferroehr::config::rest::ApiConfig::default()),
        "__ferroehr.platform",
    )
}

fn principal(scope: &str, client: Option<&str>, honoured: Honoured) -> Principal {
    let mut claims = serde_json::Map::new();
    claims.insert("ehrId".to_owned(), serde_json::json!("ehr-1"));
    let caller = Caller::new(
        "alice".to_owned(),
        client.map(str::to_owned),
        scope,
        None,
        false,
        ferrohealth_smart::kind::CallerKind::Person,
    )
    .with_issuer("https://as.example".to_owned())
    .with_claims(claims);
    Principal::new(Access::Caller(Arc::new(caller)), Vec::new(), honoured)
}

/// Every generated ITS-REST operation carries a requirement: the family
/// master08 §Resource Scopes names for it, the `composition-` family for
/// the EHR's other data, the client list for the DEMOGRAPHIC group and the
/// product scope for the ADMIN group. None is refused.
#[test]
fn every_generated_route_has_a_requirement() {
    use openehr_its::rest::generated::{admin, definition, demographic, ehr, query, system};
    let table = routes();
    for routes in [
        admin::ROUTES,
        definition::ROUTES,
        demographic::ROUTES,
        ehr::ROUTES,
        query::ROUTES,
        system::ROUTES,
    ] {
        for (method, path, op) in routes {
            let template = format!("{BASE}{}", crate::api::normalize_path(path));
            let method = method.parse::<Method>().expect("a method");
            let need = table.need(&method, Some(&template), &template);
            assert_ne!(need, Need::Now(Requirement::Refused), "{op}");
        }
    }
}

/// Every route the API router mounts, the extensions included, carries a
/// requirement, so no route answers to a token that carries no grant for
/// it merely because the table forgot it.
#[test]
fn every_mounted_route_has_a_requirement() {
    let table = routes();
    let document = crate::api::api_doc(&crate::api::Namespaces::of(
        &ferroehr::config::rest::ApiConfig::default(),
    ));
    for (path, item) in &document.paths.paths {
        let template = path.clone();
        for (method, operation) in [
            (Method::GET, item.get.as_ref()),
            (Method::PUT, item.put.as_ref()),
            (Method::POST, item.post.as_ref()),
            (Method::DELETE, item.delete.as_ref()),
            (Method::PATCH, item.patch.as_ref()),
        ] {
            if operation.is_none() {
                continue;
            }
            let need = table.need(&method, Some(&template), &template);
            assert_ne!(
                need,
                Need::Now(Requirement::Refused),
                "{method} {path} has no SMART on openEHR requirement"
            );
        }
    }
}

#[test]
fn an_unknown_route_is_refused() {
    let table = routes();
    assert_eq!(
        table.need(&Method::GET, Some("/elsewhere"), "/elsewhere"),
        Need::Now(Requirement::Refused)
    );
    assert_eq!(
        table.need(&Method::GET, None, "/x"),
        Need::Now(Requirement::Refused)
    );
}

/// The access log has its own access control, which its handler applies,
/// so its routes need an authenticated caller and no clinical grant.
#[test]
fn the_access_log_needs_a_caller_and_no_clinical_grant() {
    let table = routes();
    let ext = "/ferroehr/rest/extensions/v1";
    for template in [
        format!("{ext}/ehr/{{ehr_id}}/access_log"),
        format!("{ext}/ehr/{{ehr_id}}/access_log/{{uid}}"),
    ] {
        assert_eq!(
            table.need(&Method::GET, Some(&template), &template),
            Need::Now(Requirement::Caller),
            "{template}"
        );
    }
}

#[test]
fn the_families_follow_the_operation() {
    let table = routes();
    let vitals_read = Need::Now(Requirement::OpenEhr {
        family: ResourceFamily::Template,
        permission: Permission::Read,
        resource: Some("Vitals.v1".to_owned()),
    });
    let template = format!("{BASE}/definition/template/adl1.4/{{template_id}}");
    let path = format!("{BASE}/definition/template/adl1.4/Vitals.v1");
    assert_eq!(
        table.need(&Method::GET, Some(&template), &path),
        vitals_read
    );
    // The ADL 2 conversion of a stored template, an extension route, is
    // held to the requirement of the released template read.
    let ext = "/ferroehr/rest/extensions/v1";
    let template = format!("{ext}/definition/template/adl1.4/{{template_id}}/adl2");
    let path = format!("{ext}/definition/template/adl1.4/Vitals.v1/adl2");
    assert_eq!(
        table.need(&Method::GET, Some(&template), &path),
        vitals_read
    );
    let template = format!("{BASE}/ehr/{{ehr_id}}/composition/{{uid_based_id}}");
    assert_eq!(
        table.need(&Method::GET, Some(&template), &template),
        Need::Composition(Permission::Read)
    );
    let template = format!("{BASE}/ehr/{{ehr_id}}/ehr_status");
    assert_eq!(
        table.need(&Method::PUT, Some(&template), &template),
        Need::Now(ehr_data(Permission::Update))
    );
}

#[test]
fn permission_mapping() {
    assert_eq!(permission_of_op("composition_create"), Permission::Create);
    assert_eq!(permission_of_op("composition_get"), Permission::Read);
    assert_eq!(permission_of_op("composition_update"), Permission::Update);
    assert_eq!(permission_of_op("composition_delete"), Permission::Delete);
    assert_eq!(
        permission_of_op("query_execute_stored_query"),
        Permission::Search
    );
    assert_eq!(
        permission_of_op("definition_template_adl1.4_upload"),
        Permission::Create
    );
    assert_eq!(
        permission_of_op("definition_query_store.yaml"),
        Permission::Create
    );
}

#[test]
fn a_user_grant_meets_its_family_and_not_another() {
    let alice = principal("user/composition-*.r", None, Honoured::default());
    assert!(check(&ehr_data(Permission::Read), &alice, Some("ehr-9")).is_ok());
    assert!(check(&ehr_data(Permission::Create), &alice, Some("ehr-9")).is_err());
    let query = Requirement::OpenEhr {
        family: ResourceFamily::Aql,
        permission: Permission::Search,
        resource: None,
    };
    assert!(check(&query, &alice, None).is_err());
}

/// master07 §Context Selection: a `patient/` grant counts only for a bound
/// issuer, and meets a requirement by its family and permission on any EHR;
/// the access evaluator holds it to the EHR the token's `ehrId` names (#5606).
#[test]
fn a_patient_grant_is_checked_for_its_family_and_permission_alone() {
    let bound = Honoured {
        backend: false,
        patient: true,
    };
    let patient = principal("patient/composition-*.r", None, bound);
    for target in [Some("ehr-1"), Some("ehr-2"), None] {
        assert!(check(&ehr_data(Permission::Read), &patient, target).is_ok());
        assert!(check(&ehr_data(Permission::Create), &patient, target).is_err());
        assert!(
            check(&routes().product(), &patient, target).is_err(),
            "a patient grant meets no product-scope surface"
        );
    }
    let unbound = principal("patient/composition-*.r", None, Honoured::default());
    assert!(check(&ehr_data(Permission::Read), &unbound, Some("ehr-1")).is_err());
    // A user/ grant beside it widens the permissions (the union of the grants).
    let both = principal("patient/composition-*.r user/composition-*.c", None, bound);
    assert!(check(&ehr_data(Permission::Create), &both, Some("ehr-2")).is_ok());
}

/// A subject granted `access_without_scope` whose token carries no
/// resource scope meets every SMART on openEHR requirement; one without
/// it, or one whose token carries resource scopes, gets only its scopes.
#[test]
fn a_scopeless_subject_meets_an_openehr_requirement_only_without_scopes() {
    let caller = |scope: &str, scopeless: bool| {
        let caller = Caller::new(
            "svc".to_owned(),
            None,
            scope,
            None,
            scopeless,
            ferrohealth_smart::kind::CallerKind::System,
        );
        Principal::new(
            Access::Caller(Arc::new(caller)),
            Vec::new(),
            Honoured::default(),
        )
    };
    let read = ehr_data(Permission::Read);
    assert!(check(&read, &caller("openid", true), Some("ehr-1")).is_ok());
    assert!(check(&read, &caller("openid", false), Some("ehr-1")).is_err());
    assert!(
        check(&read, &caller("user/template-*.r", true), Some("ehr-1")).is_err(),
        "a token carrying scopes gets only its scopes"
    );
    assert!(
        check(&routes().product(), &caller("openid", true), None).is_err(),
        "the allowlist reaches no product-scope surface"
    );
}

/// The DEMOGRAPHIC API has no family: a client granted
/// `access_demographic` or the product scope reaches it, and no SMART
/// grant does. The grant arm reads the caller's grants
/// (`authn::gates::smart_refusal`); the requirement held is the product
/// scope's, which no client name meets.
#[test]
fn the_demographic_api_needs_a_granted_client_or_the_product_scope() {
    let table = routes();
    let ext = "/ferroehr/rest/extensions/v1";
    let none = Honoured::default();
    for template in [
        format!("{BASE}/demographic/person/{{uid_based_id}}"),
        format!("{ext}/demographic/party/{{party_id}}/relationships"),
    ] {
        let Need::Demographic(requirement) = table.need(&Method::GET, Some(&template), &template)
        else {
            panic!("a demographic route reads the caller's grants: {template}");
        };
        assert_eq!(requirement, table.product());
        assert!(
            check(
                &requirement,
                &principal("__ferroehr.platform", None, none),
                None
            )
            .is_ok()
        );
        for client in ["mpi", "app"] {
            assert!(
                check(
                    &requirement,
                    &principal("system/composition-*.cruds", Some(client), none),
                    None
                )
                .is_err(),
                "no client name meets it: {client}"
            );
        }
    }
}

/// The patient writes a standalone relationship; reading one is the DEMOGRAPHIC API's.
#[test]
fn a_patient_writes_a_standalone_relationship_and_reads_none() {
    let create = "/ferroehr/rest/extensions/v1/demographic/party_relationship".to_owned();
    let (table, one) = (routes(), format!("{create}/{{uid_based_id}}"));
    for (method, path) in [
        (Method::POST, &create),
        (Method::PUT, &one),
        (Method::DELETE, &one),
    ] {
        let need = table.need(&method, Some(path), path);
        assert_eq!(need, Need::Representation(table.product()), "{method}");
    }
    let read = table.need(&Method::GET, Some(&one), &one);
    assert!(matches!(read, Need::Demographic(_)));
}

/// The outbox feed needs its own scope: the product scope does not reach it,
/// and the outbox scope reaches no other admin route.
#[test]
fn the_outbox_feed_needs_the_outbox_scope_alone() {
    let table = routes();
    let ext = "/ferroehr/rest/extensions/v1";
    let none = Honoured::default();
    let reader = principal(OUTBOX_SCOPE, Some("ferrobridge"), none);
    let platform = principal("__ferroehr.platform", None, none);
    for (method, template) in [
        (Method::GET, format!("{ext}/admin/outbox/{{reader}}")),
        (Method::POST, format!("{ext}/admin/outbox/{{reader}}/ack")),
    ] {
        let Need::Now(requirement) = table.need(&method, Some(&template), &template) else {
            panic!("the feed is checked at once");
        };
        assert_eq!(requirement, Requirement::Scope(OUTBOX_SCOPE.to_owned()));
        assert!(check(&requirement, &reader, None).is_ok());
        assert!(check(&requirement, &platform, None).is_err());
    }
    let report = format!("{ext}/admin/report/statistics");
    let Need::Now(requirement) = table.need(&Method::GET, Some(&report), &report) else {
        panic!("an admin route is checked at once");
    };
    assert!(
        check(&requirement, &reader, None).is_err(),
        "the outbox scope reaches no other admin route"
    );
}

/// master08 §Resource Scopes: `system/aql-*` "would grant access to all
/// registered and ad-hoc AQL queries system-wide", so it counts only for a
/// listed backend client.
#[test]
fn a_system_wide_aql_grant_counts_for_a_backend_client_alone() {
    let query = Requirement::OpenEhr {
        family: ResourceFamily::Aql,
        permission: Permission::Search,
        resource: None,
    };
    let backend = Honoured {
        backend: true,
        patient: false,
    };
    assert!(
        check(
            &query,
            &principal("system/aql-*.s", Some("etl"), backend),
            None
        )
        .is_ok()
    );
    assert!(
        check(
            &query,
            &principal("system/aql-*.s", Some("app"), Honoured::default()),
            None
        )
        .is_err()
    );
}

#[test]
fn the_query_target_is_the_single_ehr_scope() {
    let query = Requirement::OpenEhr {
        family: ResourceFamily::Aql,
        permission: Permission::Search,
        resource: None,
    };
    let uri: Uri = "/x/query/aql?q=SELECT&ehr_id=ehr-7".parse().expect("uri");
    assert_eq!(
        target_ehr(&query, None, &uri, &HeaderMap::new()).as_deref(),
        Some("ehr-7")
    );
    let mut headers = HeaderMap::new();
    headers.insert("openehr-ehr-id", http::HeaderValue::from_static("ehr-8"));
    let bare: Uri = "/x/query/aql".parse().expect("uri");
    assert_eq!(
        target_ehr(&query, None, &bare, &headers).as_deref(),
        Some("ehr-8")
    );
}
