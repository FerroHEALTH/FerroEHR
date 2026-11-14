// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy-enforcement point (PEP) for the ITS-REST surface: the SMART
//! resource-scope and launch-context gate of a COMPOSITION operation, driven
//! from the generic dispatch mount (`crate::api`).
//!
//! SMART takes its scope grammar and launch-context binding from
//! `ITS-REST/docs/smart_app_launch/master08-scopes.adoc` §Resource Scopes and
//! `master07-*.adoc` §Context Selection, and the 401/403 discipline from
//! `.../overview/Requests_and_responses.md` §Authentication and authorization.
//!
//! The gate runs after the access evaluator and RBAC. Pre-checks run before
//! the backend call (op id, path params, query, body); post-checks run on a
//! successful response, from the dispatch's
//! [`AuditObject`](crate::system_log::middleware::AuditObject). A deny is a
//! `403` and a lookup failure a `503` or `500`, each carrying the
//! [`Principal`] for the ATNA layer. The gate, run whenever authentication is
//! on, covers the COMPOSITION operations, whose template only the body or the
//! stored version names (`crate::smart::routes`).
#![expect(
    clippy::result_large_err,
    reason = "the Err variant of every decision point in this module is a \
              ready-to-return axum `Response`, which is large by nature"
)]

use axum::response::{IntoResponse, Response};

use crate::api::operation::{CompositionGate, Operation, Verb, operation};
use openehr_its::rest::runtime::ApiError;

use ferroehr::error::ServiceError;
use openehr_rm::prelude::Composition;

use crate::api::RequestParts;
use crate::extensions::access::authn::{Principal, current_principal};
use crate::overview::error::RestError;

use crate::negotiate;
use crate::smart::routes;
use crate::state::AppState;
use ferroehr::versioning::object_version_id::parse_uid_based_id;
use ferrohealth_smart::guard::Requirement;
use ferrohealth_smart::openehr::smart_scopes::{Permission, ResourceFamily};

/// Post-check a successful response: the SMART gate of a COMPOSITION read.
pub(crate) async fn post_check(state: &AppState, op: &'static str, resp: Response) -> Response {
    if !resp.status().is_success() {
        return resp;
    }
    let object = resp
        .extensions()
        .get::<crate::system_log::middleware::AuditObject>()
        .cloned();
    match smart_post(state, op, object).await {
        Ok(()) => resp,
        Err(deny) => deny,
    }
}

/// Extracts the composition template id from the request body, or from the
/// `openehr-template-id` request header for a FLAT/STRUCTURED body
/// (`Requests_and_responses` §openehr-template-id).
fn composition_template(parts: &RequestParts) -> Option<String> {
    let h = &parts.headers;
    if crate::formats::dispatch::is_simplified_body(h) {
        return crate::formats::dispatch::header_template_id(h);
    }
    // The same typed decode the commit seam performs: the template id comes off
    // `ARCHETYPED.template_id`, never a JSON pointer a shape change could
    // silently stop matching.
    let composition = negotiate::decode::rm_value::<Composition>(h, &parts.body).ok()?;
    composition
        .archetype_details
        .and_then(|a| a.template_id)
        .map(|id| id.value)
}

/// A 403 carrying the principal (audited by the ATNA layer).
fn forbidden(principal: &Principal, detail: &str) -> Response {
    let mut resp =
        RestError::from(ApiError::Forbidden(format!("access denied: {detail}"))).into_response();
    resp.extensions_mut().insert(principal.clone());
    resp
}

/// A fail-closed refusal carrying the principal: `503` plus `Retry-After` when
/// the failure is a call status of `service_overloaded` (no database
/// connection could be obtained, so the request can be repeated), else `500`.
///
/// `step` names what could not be done and `error` the failure that broke it;
/// both go to the trace record, and the body carries the curated opaque message.
fn lookup_error(
    principal: &Principal,
    step: &'static str,
    error: &(dyn std::error::Error + 'static),
) -> Response {
    let overloaded = std::iter::successors(Some(error), |e| e.source())
        .filter_map(<dyn std::error::Error>::downcast_ref::<ServiceError>)
        .find_map(|err| match err {
            ServiceError::Overloaded(curated) => Some(curated.message),
            _ => None,
        });
    let mut resp = match overloaded {
        Some(message) => {
            RestError::from(ApiError::ServiceUnavailable(message.to_owned())).into_response()
        }
        None => RestError::from(crate::overview::error::internal_fault_caused(step, error))
            .into_response(),
    };
    resp.extensions_mut().insert(principal.clone());
    resp
}

/// The SMART gate of a COMPOSITION write, on the template its body names or,
/// for a delete, the template of the version it deletes (master08 §Resource
/// Scopes, `composition-<templateId>`), checked before the backend call:
/// `Err(response)` short-circuits the dispatch with a ready 403/500. Inert
/// without an authenticated principal, which only an unauthenticated surface
/// lacks.
pub(crate) async fn pre_check(
    state: &AppState,
    op: &'static str,
    parts: &RequestParts,
) -> Result<(), Response> {
    let Some(row) = operation(op).filter(|row| row.composition == CompositionGate::Write) else {
        return Ok(());
    };
    let Some(principal) = current_principal() else {
        return Ok(());
    };
    let template = if row.verb == Verb::Delete {
        composition_template_of(
            state,
            &principal,
            parts.path.get("uid_based_id").map(String::as_str),
        )
        .await?
    } else {
        composition_template(parts)
    };
    smart_composition(
        &principal,
        op,
        template.as_deref(),
        parts.path.get("ehr_id").map(String::as_str),
    )
}

/// The SMART gate of a COMPOSITION read, on the template of the version the
/// dispatch returned (`object`, the ids it recorded).
async fn smart_post(
    state: &AppState,
    op: &'static str,
    object: Option<crate::system_log::middleware::AuditObject>,
) -> Result<(), Response> {
    if operation(op).is_none_or(|row| row.composition == CompositionGate::None) {
        return Ok(());
    }
    let Some(principal) = current_principal() else {
        return Ok(());
    };
    let ehr_id = object.as_ref().and_then(|o| o.ehr_id.clone());
    let uid = object.as_ref().and_then(|o| o.uid.clone());
    let template = composition_template_of(state, &principal, uid.as_deref()).await?;
    smart_composition(&principal, op, template.as_deref(), ehr_id.as_deref())
}

/// Checks the `composition-` requirement of `op` on `template` for
/// `principal`, a `patient/` grant confined to the launch context's EHR.
///
/// An unresolved template is covered only by a grant on every template, `*`
/// or `**` (master08 §Resource Scopes).
fn smart_composition(
    principal: &Principal,
    op: &str,
    template: Option<&str>,
    ehr_id: Option<&str>,
) -> Result<(), Response> {
    let requirement = Requirement::OpenEhr {
        family: ResourceFamily::Composition,
        permission: operation(op).map_or(Permission::Read, Operation::permission),
        resource: template.map(str::to_owned),
    };
    routes::check(&requirement, principal, ehr_id).map_err(|reason| {
        tracing::debug!(reason, "a SMART on openEHR requirement was not met");
        forbidden(principal, "the credential does not permit this request")
    })
}

/// Returns the template of the COMPOSITION version `uid` names, read from the
/// store, or `None` when the request names no version.
async fn composition_template_of(
    state: &AppState,
    principal: &Principal,
    uid: Option<&str>,
) -> Result<Option<String>, Response> {
    let Some(uid) = uid else {
        return Ok(None);
    };
    let decoded = parse_uid_based_id(uid)
        .map_err(|e| forbidden(principal, &format!("unaddressable resource id: {e}")))?;
    let version = decoded
        .version
        .map(|ovid| ovid.version_tree_id().value().to_owned());
    state
        .backend()
        .template_of_version(decoded.vo_id, version.as_deref())
        .await
        .map_err(|e| lookup_error(principal, "resolve the composition template", &e))
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_types,
        reason = "a test builds the Principal's claim map directly; the claim set \
                  is RFC 7519-open, so the fixtures name serde_json's types"
    )]

    use std::sync::Arc;

    use http::StatusCode;

    use ferrohealth_smart::resource::{Access, Caller, Honoured};

    use super::*;

    // ── SMART scope + launch-context gate on COMPOSITION operations ────────
    //    (master08 §Resource Scopes; master07 §Context Selection)

    fn principal_with_scopes(scope: &str, claims: &serde_json::Value) -> Principal {
        principal_honoured(scope, claims, Honoured::default())
    }

    fn principal_honoured(
        scope: &str,
        claims: &serde_json::Value,
        honoured: Honoured,
    ) -> Principal {
        let caller = crate::test_support::person("alice", scope)
            .with_issuer("https://as.example".to_owned())
            .with_claims(claims.as_object().cloned().unwrap_or_default());
        Principal::new(
            Access::Caller(Arc::new(caller)),
            vec!["USER".to_owned()],
            honoured,
        )
    }

    #[test]
    fn smart_user_scope_permits_its_template() {
        let p = principal_with_scopes("user/composition-Vitals.v1.r", &serde_json::json!({}));
        assert!(smart_composition(&p, "composition_get", Some("Vitals.v1"), Some("ehr-1")).is_ok());
        let other = smart_composition(&p, "composition_get", Some("Labs.v1"), Some("ehr-1"))
            .expect_err("another template is not granted");
        assert_eq!(other.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn smart_permission_mismatch_denies() {
        // A read-only scope cannot authorise a create.
        let p = principal_with_scopes("user/composition-*.r", &serde_json::json!({}));
        assert!(
            smart_composition(&p, "composition_create", Some("Vitals.v1"), Some("ehr-1")).is_err()
        );
    }

    /// SMART on openEHR scopes are enforced on every route: a token carrying no
    /// resource scope is refused on a COMPOSITION operation (master08 §Scopes:
    /// the platform validates requested scopes against its access policy).
    #[test]
    fn smart_a_token_without_a_resource_scope_is_refused() {
        let p = principal_with_scopes("openid profile", &serde_json::json!({}));
        assert!(
            smart_composition(&p, "composition_create", Some("Vitals.v1"), Some("ehr-1")).is_err()
        );
    }

    /// A Basic user holds the scopes `[[auth.basic.users]]` grants it, and
    /// none beyond them.
    #[test]
    fn smart_a_basic_caller_holds_its_configured_scopes() {
        let granted = Principal::new(
            Access::Caller(Arc::new(Caller::basic(
                "alice".to_owned(),
                "user/composition-*.cruds",
            ))),
            vec!["USER".to_owned()],
            Honoured::default(),
        );
        assert!(
            smart_composition(
                &granted,
                "composition_create",
                Some("Vitals.v1"),
                Some("ehr-1")
            )
            .is_ok()
        );
        let bare = Principal::new(
            Access::Caller(Arc::new(Caller::basic("bob".to_owned(), ""))),
            vec!["USER".to_owned()],
            Honoured::default(),
        );
        assert!(
            smart_composition(
                &bare,
                "composition_create",
                Some("Vitals.v1"),
                Some("ehr-1")
            )
            .is_err()
        );
    }

    /// An unresolved template is covered only by a grant on every template.
    #[test]
    fn smart_an_unresolved_template_needs_a_wildcard() {
        let specific =
            principal_with_scopes("user/composition-Vitals.v1.r", &serde_json::json!({}));
        assert!(smart_composition(&specific, "composition_get", None, Some("ehr-1")).is_err());
        let every = principal_with_scopes("user/composition-*.r", &serde_json::json!({}));
        assert!(smart_composition(&every, "composition_get", None, Some("ehr-1")).is_ok());
    }

    /// master07 §Context Selection: a `patient/` grant from a bound issuer
    /// reaches the EHR the token's `ehrId` names, and no other.
    #[test]
    fn smart_patient_scope_binds_the_launch_context_ehr() {
        let bound = Honoured {
            backend: false,
            patient: true,
        };
        let p = principal_honoured(
            "patient/composition-*.r",
            &serde_json::json!({ "ehrId": "ehr-1" }),
            bound,
        );
        assert!(smart_composition(&p, "composition_get", Some("Vitals.v1"), Some("ehr-1")).is_ok());
        assert!(
            smart_composition(&p, "composition_get", Some("Vitals.v1"), Some("ehr-2")).is_err()
        );
    }

    /// A `patient/` grant whose token names no `ehrId` reaches nothing.
    #[test]
    fn smart_patient_scope_requires_a_launch_context_claim() {
        let bound = Honoured {
            backend: false,
            patient: true,
        };
        let p = principal_honoured("patient/composition-*.r", &serde_json::json!({}), bound);
        assert!(
            smart_composition(&p, "composition_get", Some("Vitals.v1"), Some("ehr-1")).is_err()
        );
    }

    /// The SMART `patient` claim is no openEHR launch context: master07
    /// §Context Selection names `ehrId` for the EHR, so a token carrying only
    /// `patient` binds no EHR.
    #[test]
    fn smart_the_patient_claim_is_no_ehr_launch_context() {
        let bound = Honoured {
            backend: false,
            patient: true,
        };
        let p = principal_honoured(
            "patient/composition-*.r",
            &serde_json::json!({ "patient": "ehr-1" }),
            bound,
        );
        assert!(
            smart_composition(&p, "composition_get", Some("Vitals.v1"), Some("ehr-1")).is_err()
        );
    }

    /// A `patient/` grant from an issuer FerroEHR has not bound grants nothing.
    #[test]
    fn smart_patient_scope_of_an_unbound_issuer_grants_nothing() {
        let p = principal_with_scopes(
            "patient/composition-*.r",
            &serde_json::json!({ "ehrId": "ehr-1" }),
        );
        assert!(
            smart_composition(&p, "composition_get", Some("Vitals.v1"), Some("ehr-1")).is_err()
        );
    }

    #[test]
    fn smart_user_scope_needs_no_patient_binding() {
        let p = principal_with_scopes("user/composition-*.r", &serde_json::json!({}));
        assert!(smart_composition(&p, "composition_get", Some("Vitals.v1"), Some("ehr-2")).is_ok());
    }

    /// Only the COMPOSITION operations are gated here; the tags are held to
    /// every template by the middleware.
    #[test]
    fn smart_composition_ops_are_the_composition_family() {
        let smart_composition_op =
            |op| operation(op).is_some_and(|row| row.composition != CompositionGate::None);
        assert!(smart_composition_op("composition_get"));
        assert!(smart_composition_op(
            "versioned_composition_revision_history"
        ));
        assert!(!smart_composition_op("composition_tags_get"));
        assert!(!smart_composition_op("ehr_create"));
    }
}
