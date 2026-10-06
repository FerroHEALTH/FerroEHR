// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Commit-time resolution of archetype constraint bindings, the ac-code value
//! sets a template binds to an external terminology query.
//!
//! BASE `docs/architecture_overview/master12-terminology.adoc` §"Binding
//! Terminology Value-sets to Archetypes" defines an ac-code bound to queries
//! against one or more external terminologies, the query itself living in a
//! terminology query server rather than in the archetype; AOM 1.4
//! `AM/docs/AOM1.4/master04-constraint_model_package.adoc` §Reference Objects
//! says the same from the constraint-model side. AOM2
//! `AM/docs/AOM2/master08-validation.adoc` §Terminology places binding validity
//! in archetype validation; the data-side consequence, that an instance code
//! must be in the bound value set, is what this module enforces at ingestion.
//!
//! `openehr_sdt::flat::validation::collect_constraint_binding_checks` walks the
//! COMPOSITION against its `WebTemplate` and returns one
//! [`ConstraintBindingCheck`] per bound coded value present in the instance.
//! Each check is resolved against the terminology server the binding routes to
//! ([`super::router::TerminologyRouter`]) with the SM `value_set_validate`
//! call, i.e. FHIR `ValueSet/$validate-code` or `$expand` plus membership.
//!
//! # Outcomes
//!
//! - No external terminology configured (the default): no check is collected
//!   and behaviour is byte-identical to a deployment without this module.
//! - Resolved and the code is a member: accepted.
//! - Resolved and the code is not a member: a [`ValidationKind::Terminology`]
//!   violation, `422`. It is a constraint violation rather than a service
//!   failure, so `fail_on_error` does not apply.
//! - Unresolvable (server down, `5xx`, unknown value set, no provider routes to
//!   the binding's terminology): governed by `[terminology.external]
//!   fail_on_error` — fail-closed rejects the composition, fail-open (the
//!   default) accepts it and logs a warning.
//!
//! NOTE: no openEHR spec maps `CODE_PHRASE.terminology_id` values to FHIR system
//! URIs, so the value is forwarded verbatim as the FHIR `system` parameter and a
//! deployment aligns the two in its terminology-server configuration — our own
//! design/extension.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6): the walked value is the composition's \
              canonical openEHR JSON, dynamic by construction"
)]

use openehr_sdt::flat::validation::ConstraintBindingCheck;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use openehr_sdt::rm_instance::{ValidationKind, ValidationMessage};
use serde_json::Value;

use crate::service::FerroEhrService;
use crate::service::status::SmError;

impl FerroEhrService {
    /// Resolve every archetype constraint binding this COMPOSITION triggers
    /// against the routed terminology servers, returning the violations found.
    ///
    /// Empty (and free of any remote call) when external terminology is not
    /// configured or the template carries no constraint binding.
    pub(in crate::service) async fn constraint_binding_violations(
        &self,
        composition: &Value,
        wt: &WebTemplate,
    ) -> Vec<ValidationMessage> {
        let Some(router) = self.terminology.as_deref() else {
            return Vec::new();
        };
        let checks =
            openehr_sdt::flat::validation::collect_constraint_binding_checks(composition, wt);
        if checks.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for check in checks {
            match self.resolve_binding(&check).await {
                Ok(true) => {}
                Ok(false) => out.push(ValidationMessage {
                    path: check.path.clone(),
                    message: format!(
                        "code '{}' (terminology '{}') is not in the value set bound to \
                         constraint code '{}' ({}) — BASE master12 §Binding Terminology \
                         Value-sets to Archetypes",
                        check.instance_code,
                        check.instance_terminology,
                        check.ac_code,
                        check.query_uri,
                    ),
                    kind: ValidationKind::Terminology,
                }),
                Err(e) => {
                    if router.fail_on_error() {
                        out.push(ValidationMessage {
                            path: check.path.clone(),
                            message: format!(
                                "the value set bound to constraint code '{}' ({}) could not be \
                                 resolved and terminology.external.fail_on_error is set: {}",
                                check.ac_code, check.query_uri, e.message,
                            ),
                            kind: ValidationKind::Terminology,
                        });
                    } else {
                        tracing::warn!(
                            ac_code = %check.ac_code,
                            query_uri = %check.query_uri,
                            terminology = %check.binding_terminology,
                            error = %e.message,
                            "constraint-binding value set unresolved; accepting the composition \
                             (terminology.external.fail_on_error is not set)"
                        );
                    }
                }
            }
        }
        out
    }

    /// Ask the terminology server that serves this binding whether the
    /// instance code is a member of the bound value set.
    ///
    /// Routed by the binding's terminology first (the archetype's own
    /// statement of which terminology it binds to), then the instance code's
    /// terminology, then the query URI itself.
    async fn resolve_binding(&self, check: &ConstraintBindingCheck) -> Result<bool, SmError> {
        let provider = self
            .terminology_route(&check.binding_terminology)
            .or_else(|| self.terminology_route(&check.instance_terminology))
            .or_else(|| self.terminology_route(&check.query_uri))
            .or_else(|| self.terminology_default_provider())
            .ok_or_else(|| {
                // The 500-class body stays opaque about the deployment's
                // terminology routing (the #1809 adjudication, #1819); the
                // operator sees which terminology had no route on the trace.
                tracing::error!(
                    terminology = %check.binding_terminology,
                    "constraint binding: no terminology server is configured for this terminology"
                );
                SmError::exception(
                    "the archetype constraint binding could not be resolved; see the server log"
                        .to_owned(),
                )
            })?;
        provider
            .value_set_validate(
                &check.instance_terminology,
                &check.query_uri,
                &check.instance_code,
                None,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use openehr_sdt::flat::webtemplate::model::{
        WebTemplate, WebTemplateConstraintBinding, WebTemplateNode,
    };
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::service::terminology::config::{
        ExternalTerminologyConfig, FhirOperation, FhirProviderConfig, ProviderKind,
    };
    use crate::service::terminology::router::TerminologyRouter;

    const BOUND_VS: &str = "http://vs.example/blood-group";
    const SNOMED: &str = "http://snomed.info/sct";

    /// A one-node template whose `DV_CODED_TEXT` leaf carries the ac-code
    /// constraint binding, and the matching instance value. The walk-matching
    /// machinery itself is covered in `openehr-its`; here the node IS the root
    /// so the test isolates the resolution semantics.
    fn bound_template() -> WebTemplate {
        let mut node = WebTemplateNode::new(
            "DV_CODED_TEXT".to_owned(),
            "/content[at0001]/value".to_owned(),
        );
        node.id = "blood_group".to_owned();
        node.constraint_bindings = vec![WebTemplateConstraintBinding {
            attr: "defining_code".to_owned(),
            ac_code: "ac0001".to_owned(),
            terminology: "SNOMED-CT".to_owned(),
            query_uri: BOUND_VS.to_owned(),
        }];
        WebTemplate {
            template_id: "binding.test.v1".to_owned(),
            sem_ver: None,
            version: "2.3".to_owned(),
            default_language: "en".to_owned(),
            languages: vec!["en".to_owned()],
            tree: node,
            other_details: indexmap::IndexMap::new(),
        }
    }

    fn coded_instance(code: &str) -> Value {
        json!({
            "_type": "DV_CODED_TEXT",
            "value": "A positive",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": SNOMED },
                "code_string": code
            }
        })
    }

    /// A `$validate-code` server answering `result` for `member_code` and
    /// `false` for anything else.
    async fn validate_code_server(member_code: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ValueSet/$validate-code"))
            .and(query_param("url", BOUND_VS))
            .and(query_param("code", member_code))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resourceType": "Parameters",
                "parameter": [{"name": "result", "valueBoolean": true}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ValueSet/$validate-code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resourceType": "Parameters",
                "parameter": [{"name": "result", "valueBoolean": false}]
            })))
            .mount(&server)
            .await;
        server
    }

    fn provider_cfg(base: &str) -> FhirProviderConfig {
        FhirProviderConfig {
            kind: ProviderKind::Fhir,
            url: crate::config::secret::SecretUrl::new(base),
            operation: FhirOperation::ValidateCode,
            connect_timeout_ms: 500,
            request_timeout_ms: 800,
            oauth2_client: None,
            client_cert_path: None,
            client_key_path: None,
            ca_bundle_path: None,
            cache_ttl_secs: 0,
            cache_capacity: 0,
        }
    }

    /// A service routing `SNOMED-CT` at `base`, with the given fail-open /
    /// fail-closed posture.
    async fn service(base: &str, fail_on_error: bool) -> FerroEhrService {
        let cfg = ExternalTerminologyConfig {
            enabled: true,
            fail_on_error,
            providers: BTreeMap::from([("default".to_owned(), provider_cfg(base))]),
            routes: BTreeMap::from([("SNOMED-CT".to_owned(), "default".to_owned())]),
            ..ExternalTerminologyConfig::default()
        };
        let router = TerminologyRouter::build(&cfg)
            .expect("build router")
            .expect("router");
        let db = testkit::db().await.expect("testkit database");
        FerroEhrService::new(&crate::db::domain::DomainPools::from_shared(&db.pool()))
            .with_terminology_router(Arc::new(router))
    }

    /// A code the bound value set contains is accepted (BASE master12
    /// §"Binding Terminology Value-sets to Archetypes").
    #[tokio::test]
    async fn a_member_code_is_accepted() {
        let ts = validate_code_server("278149003").await;
        let service = service(&ts.uri(), false).await;
        let violations = service
            .constraint_binding_violations(&coded_instance("278149003"), &bound_template())
            .await;
        assert!(
            violations.is_empty(),
            "unexpected violations: {violations:?}"
        );
        assert_eq!(
            ts.received_requests().await.unwrap().len(),
            1,
            "the binding was resolved against the terminology server"
        );
    }

    /// A code the bound value set does NOT contain is a violation → 422. This
    /// is a resolved constraint violation, not a service failure, so
    /// `fail_on_error` must not change it.
    #[tokio::test]
    async fn a_non_member_code_is_refused_under_either_posture() {
        for fail_on_error in [false, true] {
            let ts = validate_code_server("278149003").await;
            let service = service(&ts.uri(), fail_on_error).await;
            let violations = service
                .constraint_binding_violations(&coded_instance("999999"), &bound_template())
                .await;
            assert_eq!(
                violations.len(),
                1,
                "a non-member code must be refused (fail_on_error = {fail_on_error})"
            );
            let v = &violations[0];
            assert_eq!(v.kind, ValidationKind::Terminology);
            assert_eq!(v.path, "/content[at0001]/value");
            assert!(v.message.contains("999999"), "got {}", v.message);
            assert!(v.message.contains("ac0001"), "got {}", v.message);
        }
    }

    /// Terminology server unreachable: fail-open (the default) accepts the
    /// composition, fail-closed rejects it. The observable difference is the
    /// whole point of `[terminology.external] fail_on_error`.
    #[tokio::test]
    async fn an_unresolvable_binding_follows_fail_on_error() {
        // TEST-NET-1 (RFC 5737 §3: 192.0.2.0/24 "for use in documentation",
        // never routed on the public internet), so the provider's connect fails
        // the way a TS outage does, within the 500 ms connect timeout. A
        // started-then-dropped mock server would leave a port the operating
        // system may hand to the next listener, which is not an unreachable
        // endpoint.
        let dead_uri = "http://192.0.2.1:1".to_owned();

        let open = service(&dead_uri, false).await;
        let violations = open
            .constraint_binding_violations(&coded_instance("278149003"), &bound_template())
            .await;
        assert!(
            violations.is_empty(),
            "fail-open must accept an unresolvable binding, got {violations:?}"
        );

        let closed = service(&dead_uri, true).await;
        let violations = closed
            .constraint_binding_violations(&coded_instance("278149003"), &bound_template())
            .await;
        assert_eq!(
            violations.len(),
            1,
            "fail-closed must reject an unresolvable binding"
        );
        assert!(
            violations[0].message.contains("fail_on_error"),
            "the refusal names the posture that caused it, got {}",
            violations[0].message
        );
    }

    /// With no external terminology configured — the shipped default — the
    /// binding pass does nothing at all: no check is collected, no request is
    /// made, and commit behaviour is byte-identical to a deployment without
    /// this module.
    #[tokio::test]
    async fn the_disabled_default_resolves_nothing() {
        let ts = validate_code_server("278149003").await;
        let db = testkit::db().await.expect("testkit database");
        let service =
            FerroEhrService::new(&crate::db::domain::DomainPools::from_shared(&db.pool()));
        let violations = service
            // A code that WOULD be refused if the binding were resolved.
            .constraint_binding_violations(&coded_instance("999999"), &bound_template())
            .await;
        assert!(
            violations.is_empty(),
            "no binding is resolved without [terminology.external]"
        );
        assert_eq!(
            ts.received_requests().await.unwrap().len(),
            0,
            "no terminology request is made"
        );
    }

    /// A template with no constraint binding raises no question even with
    /// terminology configured — the pass is free for the overwhelmingly common
    /// unbound template.
    #[tokio::test]
    async fn an_unbound_template_makes_no_request() {
        let ts = validate_code_server("278149003").await;
        let service = service(&ts.uri(), true).await;
        let mut wt = bound_template();
        wt.tree.constraint_bindings.clear();
        let violations = service
            .constraint_binding_violations(&coded_instance("999999"), &wt)
            .await;
        assert!(violations.is_empty());
        assert_eq!(ts.received_requests().await.unwrap().len(), 0);
    }
}
