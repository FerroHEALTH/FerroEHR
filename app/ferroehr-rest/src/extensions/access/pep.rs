// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy-enforcement point (PEP) for the ITS-REST surface.
//!
//! Composes the ABAC gate with the SMART resource-scope + launch-context gate,
//! driven from the generic dispatch mount (`crate::api`).
//!
//! ABAC and RBAC are our own enterprise design — no openEHR spec governs them
//! (the SM places authorisation out of band, SM
//! `openehr_platform/master02-overview.adoc` §General Assumptions). SMART is
//! spec-grounded: the scope grammar and launch-context binding come from
//! `ITS-REST/docs/smart_app_launch/master08-scopes.adoc` §Resource Scopes and
//! `master07-*.adoc` §Context Selection, and the 401/403 discipline from
//! `.../overview/Requests_and_responses.md` §Authentication and authorization.
//!
//! The gates AND-compose as `EHR_ACCESS` → RBAC → ABAC → SMART, each an
//! additive restriction, so the SMART gate runs after the RBAC/Cedar decision
//! has permitted, never in place of it. Pre-checks run before the backend call
//! (op id, path params, query, body); post-checks run after it on a successful
//! response, from the
//! [`AuditObject`](crate::system_log::middleware::AuditObject) the dispatch set.
//! Every deny is a `403` and every engine failure a `500` (fail-closed), each
//! carrying the [`Principal`] on the response extensions for the ATNA layer.
//! ABAC is inert without a wired `crate::extensions::access::authz::AbacGate`.
//! The SMART gate here covers the COMPOSITION operations, whose template only
//! the body or the stored version names (`crate::smart::routes`); every other
//! route's SMART requirement is checked by the authentication middleware. It
//! runs whenever authentication is on.
#![expect(
    clippy::result_large_err,
    reason = "the Err variant of every decision point in this module is a \
              ready-to-return axum `Response`, which is large by nature"
)]

use axum::response::{IntoResponse, Response};

use crate::extensions::access::authz::classify::{access_of, kind_of};
use crate::extensions::access::authz::request::{
    AccessMode, Attr, AuthzRequest, Decision, ResourceKind,
};
use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::Composition;

use crate::api::RequestParts;
use crate::extensions::access::authn::{Principal, current_principal};
use crate::extensions::access::authz::AbacGate;
use crate::overview::error::RestError;

use crate::smart::routes;
use crate::state::AppState;
use crate::{negotiate, params};
use ferroehr::aql::sql::subject::SubjectScope;
use ferroehr::aql::subject::{EhrSubject, TouchedSubjects};
use ferroehr::service::status::{CallStatusType, SmError};
use ferroehr::versioning::object_version_id::parse_uid_based_id;
use ferrohealth_smart::guard::Requirement;
use smart_grammar::smart_scopes::ResourceFamily;

/// Whether an operation is ABAC-checked, and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Not ABAC-checked (item tags, definition/demographic/admin — RBAC only —,
    /// and query, whose scope + check live in the query path).
    Skip,
    /// Checked before the backend call.
    Pre,
    /// Checked after the backend call, on a successful response.
    Post,
}

/// The enforcement matrix, keyed by operation id.
#[expect(
    clippy::match_same_arms,
    reason = "the arms are grouped by resource family, so naming every operation \
              explicitly is the point — merging equal arms would hide which \
              family an operation is gated as"
)]
fn mode_of(op: &str) -> Mode {
    match op {
        "ehr_create" | "ehr_create_with_id" | "ehr_get_by_id" | "ehr_get_by_subject" => Mode::Pre,
        "composition_create" | "composition_update" | "composition_delete" => Mode::Pre,
        "contribution_create" => Mode::Pre,
        // The contribution-list extension is an EHR-scoped read whose `ehr_id`
        // is a path parameter, so the subject gate runs before any backend work.
        "contribution_list" => Mode::Pre,
        "composition_get" => Mode::Post,
        "contribution_get" => Mode::Post,
        _ if op.starts_with("versioned_composition_") => Mode::Post,
        _ if op.starts_with("ehr_status_") || op.starts_with("versioned_ehr_status_") => Mode::Pre,
        _ if op.starts_with("directory_") => Mode::Pre,
        _ => Mode::Skip,
    }
}

/// Prepares a query execution: the patient subject scope to thread into the
/// SQL, and whether the executor collects the touched EHR/template sets for the
/// post-check.
///
/// `Ok((None, false))` when ABAC is off; `Err(response)` is a ready 403 for a
/// missing configured patient claim.
pub(crate) fn query_pre(
    state: &AppState,
    op: &'static str,
) -> Result<(Option<SubjectScope>, bool), Response> {
    let Some(handle) = state.authz() else {
        return Ok((None, false));
    };
    let Some(abac) = handle.abac() else {
        return Ok((None, false));
    };
    if kind_of(op) != Some(ResourceKind::Query) {
        return Ok((None, false));
    }
    let principal = current_principal().ok_or_else(|| {
        forbidden_unauthenticated("authentication required for attribute-based access control")
    })?;
    let patient = resolve_patient_claim(abac, &principal)?;
    // Collect attributes whenever ABAC is on so the post-check has the touched
    // subject and template sets; the subject scope pre-filters rows to the
    // caller's patient, read in the configured namespace.
    let scope = patient.map(|id| SubjectScope {
        id,
        namespace: abac.patient_namespace.clone(),
    });
    Ok((scope, true))
}

/// Post-checks a query execution: the PDP decides over the touched patients
/// and templates.
///
/// The executor collects every EHR the query touches, from its EHR sources as
/// well as its versioned-object roots, and reads their subjects. The patient
/// attribute is the set of those subject ids (a set for query, as
/// [`AuthzRequest::patient`] states), so a patient-keyed rule applies whether
/// or not a patient claim is configured. Subject ids are read in the configured
/// patient namespace ([`TouchedSubjects::fold`]); a touched EHR whose subject no
/// claim can name is asked once more with no patient. Without a configured
/// namespace, a touched subject whose id another namespace also holds is
/// refused: the bare id the PDP sees would name both. A query touching nothing
/// asks nothing of the PDP.
///
/// # Errors
/// A ready `403` when the PDP denies, a touched subject id is ambiguous or the
/// patient claim is missing, or a `500` when the PDP fails.
pub(crate) async fn query_post(
    state: &AppState,
    op: &'static str,
    outcome: &ferroehr::service::query::request::QueryOutcome,
) -> Result<(), Response> {
    let Some(handle) = state.authz() else {
        return Ok(());
    };
    let Some(abac) = handle.abac() else {
        return Ok(());
    };
    if kind_of(op) != Some(ResourceKind::Query) {
        return Ok(());
    }
    // Empty result set → nothing touched → permit.
    if outcome.ehr_ids.is_empty() && outcome.template_ids.is_empty() {
        return Ok(());
    }
    let principal = current_principal().ok_or_else(|| {
        forbidden_unauthenticated("authentication required for attribute-based access control")
    })?;
    // The claim gate stays: a configured claim the token lacks is a 403 here too.
    resolve_patient_claim(abac, &principal)?;
    let subjects = TouchedSubjects::fold(&outcome.subjects, abac.patient_namespace.as_deref());
    if !subjects.shared.is_empty() {
        return Err(forbidden(
            &principal,
            "patient scope (a touched subject id is held under several namespaces)",
        ));
    }
    let template =
        (!outcome.template_ids.is_empty()).then(|| Attr::Set(outcome.template_ids.clone()));
    let mut patients = Vec::with_capacity(2);
    if !subjects.ids.is_empty() {
        patients.push(Some(Attr::Set(subjects.ids.clone())));
    }
    if subjects.unnamed || subjects.ids.is_empty() {
        patients.push(None);
    }
    for patient in patients {
        let req = AuthzRequest {
            operation_id: op,
            subject: &principal.subject,
            roles: &principal.roles,
            scopes: &principal.scopes,
            kind: ResourceKind::Query,
            access: AccessMode::Execute,
            organization: organization(abac, &principal),
            patient,
            template: template.clone(),
        };
        decide(abac, &principal, &req).await?;
    }
    Ok(())
}

/// Pre-check the request: the ABAC decision, then the SMART gate of a
/// COMPOSITION write. `Err(response)` short-circuits the dispatch with a
/// ready 403/500; `Ok(())` lets it proceed.
pub(crate) async fn pre_check(
    state: &AppState,
    op: &'static str,
    parts: &RequestParts,
) -> Result<(), Response> {
    abac_pre(state, op, parts).await?;
    smart_pre(state, op, parts).await
}

/// The ABAC pre-check of the request.
async fn abac_pre(
    state: &AppState,
    op: &'static str,
    parts: &RequestParts,
) -> Result<(), Response> {
    let Some(handle) = state.authz() else {
        return Ok(());
    };
    let Some(abac) = handle.abac() else {
        return Ok(());
    };
    if mode_of(op) != Mode::Pre {
        return Ok(());
    }
    let Some(kind) = kind_of(op) else {
        return Ok(());
    };
    // DIRECTORY is gated only when `abac.check_directory` says so — engine-
    // independent, so a Cedar deployment can enable it too.
    if kind == ResourceKind::Directory && !abac.directory_checked {
        return Ok(());
    }

    let principal = current_principal().ok_or_else(|| {
        forbidden_unauthenticated("authentication required for attribute-based access control")
    })?;

    let patient = resolve_patient_claim(abac, &principal)?;
    let ehr_id = parts.path.get("ehr_id").cloned();

    // Before any engine call.
    patient_gate(
        abac,
        &principal,
        patient.as_deref(),
        op,
        ehr_id.as_deref(),
        parts,
    )
    .await?;

    let template = pre_template(abac, &principal, op, kind, parts).await?;

    let req = AuthzRequest {
        operation_id: op,
        subject: &principal.subject,
        roles: &principal.roles,
        scopes: &principal.scopes,
        kind,
        access: access_of(op).unwrap_or(AccessMode::Read),
        organization: organization(abac, &principal),
        patient: patient.map(Attr::One),
        template,
    };
    decide(abac, &principal, &req).await
}

/// Post-check a successful response: the ABAC decision over the resource ids
/// the dispatch recorded, then the SMART gate of a COMPOSITION read.
pub(crate) async fn post_check(state: &AppState, op: &'static str, resp: Response) -> Response {
    let resp = abac_post(state, op, resp).await;
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

/// The ABAC post-check of a successful response.
async fn abac_post(state: &AppState, op: &'static str, resp: Response) -> Response {
    if !resp.status().is_success() {
        return resp;
    }
    let Some(handle) = state.authz() else {
        return resp;
    };
    let Some(abac) = handle.abac() else {
        return resp;
    };
    if mode_of(op) != Mode::Post {
        return resp;
    }
    let Some(kind) = kind_of(op) else {
        return resp;
    };
    let Some(principal) = current_principal() else {
        return resp;
    };

    let object = resp
        .extensions()
        .get::<crate::system_log::middleware::AuditObject>()
        .cloned();
    let ehr_id = object.as_ref().and_then(|o| o.ehr_id.clone());
    let uid = object.as_ref().and_then(|o| o.uid.clone());

    let patient = match resolve_patient_claim(abac, &principal) {
        Ok(p) => p,
        Err(deny) => return deny,
    };
    if let Err(deny) = subject_gate(abac, &principal, patient.as_deref(), ehr_id.as_deref()).await {
        return deny;
    }

    // Template attribute: for a composition read, from the returned version uid.
    let template = match kind {
        ResourceKind::Composition => match post_template(abac, &principal, uid.as_deref()).await {
            Ok(t) => t.map(Attr::One),
            Err(deny) => return deny,
        },
        _ => None,
    };

    let req = AuthzRequest {
        operation_id: op,
        subject: &principal.subject,
        roles: &principal.roles,
        scopes: &principal.scopes,
        kind,
        access: access_of(op).unwrap_or(AccessMode::Read),
        organization: organization(abac, &principal),
        patient: patient.map(Attr::One),
        template,
    };
    match decide(abac, &principal, &req).await {
        Ok(()) => resp,
        Err(deny) => deny,
    }
}

/// Resolves the patient claim, if the gate is configured.
///
/// A configured-but-missing claim is a 403, not a 500: the server is working
/// and the caller's token is what is insufficient.
fn resolve_patient_claim(
    abac: &AbacGate,
    principal: &Principal,
) -> Result<Option<String>, Response> {
    let Some(claim) = &abac.patient_claim else {
        return Ok(None);
    };
    match crate::extensions::access::authz::roles::claim_string(&principal.claims, claim) {
        Some(v) => Ok(Some(v)),
        None => Err(forbidden(
            principal,
            &format!("ABAC requires a '{claim}' claim on the access token"),
        )),
    }
}

/// Returns the organization attribute, if the claim is configured and present.
fn organization(abac: &AbacGate, principal: &Principal) -> Option<String> {
    abac.organization_claim
        .as_ref()
        .and_then(|c| crate::extensions::access::authz::roles::claim_string(&principal.claims, c))
}

/// The pre-check patient gate: the subject match for target-EHR ops, plus the
/// by-subject case, where the request parameter IS the subject.
///
/// For `ehr_get_by_subject` an absent `subject_id` deliberately passes this gate
/// so the route answers `400`, not `403`: the released specification declares
/// the parameter required (`specifications/parameters/query/subject_id.yaml`),
/// so the request is invalid for every caller and no authorization decision
/// could make it succeed (RFC 9110 §15.5.1).
async fn patient_gate(
    abac: &AbacGate,
    principal: &Principal,
    patient: Option<&str>,
    op: &str,
    ehr_id: Option<&str>,
    parts: &RequestParts,
) -> Result<(), Response> {
    if op == "ehr_get_by_subject" {
        if let Some(patient) = patient
            && let Some(subject) = params::query_param(parts.query.as_deref(), "subject_id")
            && subject != patient
        {
            return Err(forbidden(principal, "patient scope (subject mismatch)"));
        }
        return Ok(());
    }
    subject_gate(abac, principal, patient, ehr_id).await
}

/// Gates a target EHR on the patient claim: the claim, read in the configured
/// patient namespace, must name the EHR's subject
/// ([`EhrSubject::is_named_by`]), the rule the query patient scope applies.
///
/// A subject-less EHR is refused; without a configured namespace, so is one
/// whose subject id another namespace also holds. An id no EHR has passes, so
/// the route answers it. No openEHR spec governs the patient scope — our own
/// extension.
///
/// # Errors
/// A ready `403` on a refusal, a `503` when the subject lookup cannot obtain a
/// database connection, or a `500` when it fails otherwise.
async fn subject_gate(
    abac: &AbacGate,
    principal: &Principal,
    patient: Option<&str>,
    ehr_id: Option<&str>,
) -> Result<(), Response> {
    let (Some(patient), Some(ehr_id)) = (patient, ehr_id) else {
        return Ok(());
    };
    let subject = (abac.resolvers.subject)(ehr_id.to_owned())
        .await
        .map_err(|e| engine_error(principal, "resolve the EHR subject attribute", &e))?;
    match subject {
        None => Ok(()),
        Some(subject) if subject.is_named_by(patient, abac.patient_namespace.as_deref()) => Ok(()),
        Some(EhrSubject::Absent) => Err(forbidden(
            principal,
            "patient scope (the EHR records no subject)",
        )),
        Some(EhrSubject::Shared { .. }) if abac.patient_namespace.is_none() => Err(forbidden(
            principal,
            "patient scope (the subject id is held under several namespaces)",
        )),
        Some(EhrSubject::Unique { .. } | EhrSubject::Shared { .. }) => {
            Err(forbidden(principal, "patient scope (subject mismatch)"))
        }
    }
}

/// Returns the template attribute for a pre-checked op.
async fn pre_template(
    abac: &AbacGate,
    principal: &Principal,
    op: &str,
    kind: ResourceKind,
    parts: &RequestParts,
) -> Result<Option<Attr>, Response> {
    match op {
        "composition_create" | "composition_update" => {
            Ok(composition_template(parts).map(Attr::One))
        }
        "composition_delete" => {
            // The template of the preceding (current) version being deleted.
            let Some(uid) = parts.path.get("uid_based_id") else {
                return Ok(None);
            };
            let (vo_id, _) = resolve_target(principal, uid)?;
            let template = (abac.resolvers.template_of_version)(vo_id, None)
                .await
                .map_err(|e| engine_error(principal, "resolve the template attribute", &e))?;
            Ok(template.map(Attr::One))
        }
        "contribution_create" => {
            let templates = contribution_templates(parts, principal)?;
            Ok((!templates.is_empty()).then_some(Attr::Set(templates)))
        }
        _ => {
            // EHR / EHR_STATUS / DIRECTORY carry no template.
            let _ = kind;
            Ok(None)
        }
    }
}

/// Returns the template of a returned composition version, via the version uid
/// the dispatch recorded.
async fn post_template(
    abac: &AbacGate,
    principal: &Principal,
    uid: Option<&str>,
) -> Result<Option<String>, Response> {
    let Some(uid) = uid else {
        return Ok(None);
    };
    let (vo_id, version) = resolve_target(principal, uid)?;
    (abac.resolvers.template_of_version)(vo_id, version)
        .await
        .map_err(|e| engine_error(principal, "resolve the template attribute", &e))
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
    let composition = negotiate::rm_value::<Composition>(h, &parts.body).ok()?;
    composition
        .archetype_details
        .and_then(|a| a.template_id)
        .map(|id| id.value)
}

/// Returns the COMPOSITION template ids in a contribution payload.
///
/// A COMPOSITION version without a template is unresolvable, so it is
/// fail-closed (403).
fn contribution_templates(
    parts: &RequestParts,
    principal: &Principal,
) -> Result<Vec<String>, Response> {
    let Ok(value) = negotiate::json_value(&parts.headers, &parts.body) else {
        // A malformed body: let the dispatch return the 400; nothing to check.
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    if let Some(versions) = value.get("versions").and_then(|v| v.as_array()) {
        for version in versions {
            let Some(data) = version.get("data") else {
                continue;
            };
            if data.get("_type").and_then(|t| t.as_str()) != Some("COMPOSITION") {
                continue;
            }
            // A CONTRIBUTION is a wrapper with no canonical RM type, so the
            // enclosing value stays JSON; each `versions[i].data` is a canonical
            // COMPOSITION, decoded so the template id comes off the typed value.
            let Ok(composition) = openehr_its::json::from_canonical_value::<Composition>(data)
            else {
                // Undecodable: the dispatch answers 400 for this body, so there
                // is nothing here to authorize.
                return Ok(Vec::new());
            };
            match composition
                .archetype_details
                .and_then(|a| a.template_id)
                .map(|id| id.value)
            {
                Some(t) => out.push(t),
                None => {
                    return Err(forbidden(
                        principal,
                        "contribution COMPOSITION version has no template id",
                    ));
                }
            }
        }
    }
    Ok(out)
}

/// Runs the engine and maps the verdict: permit to `Ok`, deny to 403, error to
/// 500.
async fn decide(
    abac: &AbacGate,
    principal: &Principal,
    req: &AuthzRequest<'_>,
) -> Result<(), Response> {
    match abac.engine.decide(req).await {
        Ok(Decision::Permit) => Ok(()),
        Ok(Decision::Deny) => Err(forbidden(principal, "policy denied")),
        Err(e) => Err(engine_error(
            principal,
            "reach an authorization decision",
            &e,
        )),
    }
}

/// Decomposes a `UID_BASED_ID` — a bare `HIER_OBJECT_ID` or a full
/// `OBJECT_VERSION_ID` — into the `(vo_id, version_tree_id)` pair the attribute
/// resolvers take.
///
/// The string goes through this adapter's one strict decoder
/// ([`parse_uid_based_id`]; BASE `base_types`
/// `master05-identification_package.adoc` §Syntaxes), so the PEP can never read
/// an id differently from the route it guards. A value that decoder rejects is
/// denied, not degraded: the attributes a policy binds on cannot be computed
/// for an unaddressable id, and permitting on an absent one would walk a
/// malformed path parameter past a template- or patient-scoped policy.
fn resolve_target(principal: &Principal, uid: &str) -> Result<(String, Option<String>), Response> {
    match parse_uid_based_id(uid) {
        Ok(decoded) => Ok((
            decoded.vo_id.to_string(),
            decoded
                .version
                .map(|ovid| ovid.version_tree_id().value().to_owned()),
        )),
        Err(e) => Err(forbidden(
            principal,
            &format!("unaddressable resource id: {e}"),
        )),
    }
}

/// A 403 carrying the principal (audited by the ATNA layer).
fn forbidden(principal: &Principal, detail: &str) -> Response {
    let mut resp =
        RestError(ApiError::Forbidden(format!("access denied: {detail}"))).into_response();
    resp.extensions_mut().insert(principal.clone());
    resp
}

/// A 403 for a request that reached a gate with no authenticated principal.
///
/// Nothing is attached to the response extensions because there is no principal
/// to attribute: attributing a denial to an identity that never authenticated
/// would write a false subject into the audit trail, so the ATNA layer records
/// the denial from the request itself.
fn forbidden_unauthenticated(detail: &str) -> Response {
    RestError(ApiError::Forbidden(format!("access denied: {detail}"))).into_response()
}

/// A fail-closed refusal carrying the principal: `503` plus `Retry-After` when
/// the failure is a call status of `service_overloaded` (no database
/// connection could be obtained, so the request can be repeated), else `500`.
///
/// `step` names what could not be done and `error` the failure that broke it;
/// both go to the trace record, and the body carries the curated opaque message.
fn engine_error(
    principal: &Principal,
    step: &'static str,
    error: &(dyn std::error::Error + 'static),
) -> Response {
    let overloaded = std::iter::successors(Some(error), |e| e.source())
        .filter_map(|e| e.downcast_ref::<SmError>())
        .find(|sm| sm.status == CallStatusType::ServiceOverloaded);
    let mut resp = match overloaded {
        Some(sm) => RestError::from(sm.clone()).into_response(),
        None => {
            RestError(crate::overview::error::internal_fault_caused(step, error)).into_response()
        }
    };
    resp.extensions_mut().insert(principal.clone());
    resp
}

/// Whether `op` is a COMPOSITION operation whose SMART requirement the
/// policy-enforcement point checks (`crate::smart::routes`, `Need::Composition`).
fn smart_composition_op(op: &str) -> bool {
    !op.contains("_tags_")
        && (op.starts_with("composition_") || op.starts_with("versioned_composition_"))
}

/// The SMART gate of a COMPOSITION write, on the template its body names or,
/// for a delete, the template of the version it deletes (master08 §Resource
/// Scopes, `composition-<templateId>`). Inert without an authenticated
/// principal, which only an unauthenticated surface lacks.
async fn smart_pre(
    state: &AppState,
    op: &'static str,
    parts: &RequestParts,
) -> Result<(), Response> {
    if !matches!(
        op,
        "composition_create" | "composition_update" | "composition_delete"
    ) {
        return Ok(());
    }
    let Some(principal) = current_principal() else {
        return Ok(());
    };
    let template = match op {
        "composition_delete" => {
            composition_template_of(
                state,
                &principal,
                parts.path.get("uid_based_id").map(String::as_str),
            )
            .await?
        }
        _ => composition_template(parts),
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
    if !smart_composition_op(op) {
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
        permission: routes::permission_of_op(op),
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
        .map_err(|e| engine_error(principal, "resolve the composition template", &e))
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_types,
        reason = "a test builds the Principal's claim map directly; the claim set \
                  is RFC 7519-open, so the fixtures name serde_json's types"
    )]

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::extensions::access::authz::engine::{AuthzError, PolicyEngine};
    use async_trait::async_trait;
    use http::StatusCode;

    use ferrohealth_smart::resource::{Access, Caller, Honoured};

    use super::*;
    use crate::extensions::access::authz::{AuthzResolvers, ResolveError};

    /// A bearer principal for `subject` carrying `claims`.
    fn bearer_principal(
        subject: &str,
        claims: serde_json::Map<String, serde_json::Value>,
        roles: Vec<String>,
    ) -> Principal {
        let caller = Caller::new(subject.to_owned(), None, "", None, false).with_claims(claims);
        Principal::new(Access::Caller(Arc::new(caller)), roles, Honoured::default())
    }

    /// A counting engine: records how often `decide` is called (the patient gate
    /// must deny *without* reaching it), always permits.
    #[derive(Debug, Default)]
    struct CountingEngine(Arc<AtomicUsize>);

    #[async_trait]
    impl PolicyEngine for CountingEngine {
        async fn decide(&self, _req: &AuthzRequest<'_>) -> Result<Decision, AuthzError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Decision::Permit)
        }
    }

    /// A gate whose resolver answers `subject` for every EHR id (`None`: no EHR
    /// has the id).
    fn gate_with_subject(subject: Option<EhrSubject>, calls: Arc<AtomicUsize>) -> AbacGate {
        AbacGate {
            engine: Arc::new(CountingEngine(calls)),
            resolvers: AuthzResolvers {
                subject: Arc::new(move |_ehr_id: String| {
                    let subject = subject.clone();
                    Box::pin(async move { Ok::<_, ResolveError>(subject) })
                }),
                template_of_version: Arc::new(|_vo: String, _v: Option<String>| {
                    Box::pin(async move { Ok::<_, ResolveError>(None) })
                }),
            },
            organization_claim: None,
            patient_claim: Some("patient_id".to_owned()),
            patient_namespace: None,
            directory_checked: false,
        }
    }

    fn principal_with_patient(patient: &str) -> Principal {
        let mut claims = serde_json::Map::new();
        claims.insert("patient_id".to_owned(), serde_json::json!(patient));
        bearer_principal("alice", claims, vec!["USER".to_owned()])
    }

    #[tokio::test]
    async fn subject_mismatch_denies_without_engine_call() {
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = gate_with_subject(Some(unique("P2")), Arc::clone(&calls));
        let principal = principal_with_patient("P1");
        let err = subject_gate(&gate, &principal, Some("P1"), Some("ehr-1"))
            .await
            .expect_err("mismatch must deny");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "no engine call on gate deny"
        );
    }

    #[tokio::test]
    async fn subject_match_passes_gate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = gate_with_subject(Some(unique("P1")), calls);
        let principal = principal_with_patient("P1");
        assert!(
            subject_gate(&gate, &principal, Some("P1"), Some("ehr-1"))
                .await
                .is_ok()
        );
    }

    /// A subject the claim id names under one namespace.
    fn unique(id: &str) -> EhrSubject {
        EhrSubject::Unique {
            id: id.to_owned(),
            namespace: Some("test.patients".to_owned()),
        }
    }

    #[tokio::test]
    async fn caller_without_patient_scope_passes_gate() {
        // A caller with no patient scope is unaffected by the subject gate, so
        // an anonymous EHR stays fully operable for it (RM ehr
        // `master04-ehr_package.adoc` §EHR Status).
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = gate_with_subject(Some(EhrSubject::Absent), calls);
        let principal = principal_with_patient("P1");
        assert!(
            subject_gate(&gate, &principal, None, Some("ehr-1"))
                .await
                .is_ok()
        );
    }

    /// A subject-less EHR is outside every patient scope, as the query scope
    /// excludes it.
    #[tokio::test]
    async fn subject_less_ehr_is_refused_without_engine_call() {
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = gate_with_subject(Some(EhrSubject::Absent), Arc::clone(&calls));
        let principal = principal_with_patient("P1");
        let err = subject_gate(&gate, &principal, Some("P1"), Some("ehr-1"))
            .await
            .expect_err("a subject-less EHR must deny");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// The claim carries no namespace, so an id another namespace also holds
    /// names neither subject.
    #[tokio::test]
    async fn subject_id_held_under_two_namespaces_is_refused() {
        let calls = Arc::new(AtomicUsize::new(0));
        let shared = EhrSubject::Shared {
            id: "P1".to_owned(),
            namespace: Some("test.patients".to_owned()),
        };
        let gate = gate_with_subject(Some(shared), Arc::clone(&calls));
        let principal = principal_with_patient("P1");
        let err = subject_gate(&gate, &principal, Some("P1"), Some("ehr-1"))
            .await
            .expect_err("an ambiguous subject id must deny");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// A subject lookup that cannot obtain a database connection answers `503`
    /// with `Retry-After`: no statement ran, so the request can be repeated.
    #[tokio::test]
    async fn a_resolver_acquire_failure_answers_503() {
        let options = sqlx::postgres::PgConnectOptions::new()
            .host("ferroehr-acquire-test.invalid")
            .username("nobody")
            .database("nothing");
        let pool = sqlx::postgres::PgPoolOptions::new().connect_lazy_with(options);
        pool.close().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let mut gate = gate_with_subject(None, Arc::clone(&calls));
        gate.resolvers.subject = Arc::new(move |_ehr_id: String| {
            let pool = pool.clone();
            Box::pin(async move {
                ferroehr::aql::subject::ehr_subject(&pool, ferroehr::ids::EhrId(uuid::Uuid::nil()))
                    .await
                    .map_err(|e| ResolveError::new("ehr subject lookup", SmError::from(e)))
            })
        });
        let principal = principal_with_patient("P1");
        let err = subject_gate(&gate, &principal, Some("P1"), Some("ehr-1"))
            .await
            .expect_err("an unreachable database must not pass the gate");
        assert_eq!(err.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            err.headers().contains_key(http::header::RETRY_AFTER),
            "a 503 carries Retry-After"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// A resolver failure that is not a lost connection stays a `500`.
    #[tokio::test]
    async fn a_resolver_fault_answers_500() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut gate = gate_with_subject(None, calls);
        gate.resolvers.subject = Arc::new(|_ehr_id: String| {
            Box::pin(async move {
                Err(ResolveError::new(
                    "ehr subject lookup",
                    SmError::exception("lookup failed".to_owned()),
                ))
            })
        });
        let principal = principal_with_patient("P1");
        let err = subject_gate(&gate, &principal, Some("P1"), Some("ehr-1"))
            .await
            .expect_err("a resolver fault must not pass the gate");
        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// With a configured patient namespace the claim names the (id, namespace)
    /// pair, so an id another namespace also holds is no longer ambiguous.
    #[tokio::test]
    async fn a_configured_namespace_names_the_pair() {
        let shared = |namespace: &str| EhrSubject::Shared {
            id: "P1".to_owned(),
            namespace: Some(namespace.to_owned()),
        };
        let principal = principal_with_patient("P1");
        for (subject, passes) in [
            (shared("test.patients"), true),
            (shared("other.patients"), false),
            (unique("P1"), true),
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let mut gate = gate_with_subject(Some(subject.clone()), calls);
            gate.patient_namespace = Some("test.patients".to_owned());
            let decided = subject_gate(&gate, &principal, Some("P1"), Some("ehr-1")).await;
            assert_eq!(decided.is_ok(), passes, "{subject:?}");
        }
    }

    /// An id no EHR has passes, so the route answers it.
    #[tokio::test]
    async fn unknown_ehr_passes_gate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = gate_with_subject(None, calls);
        let principal = principal_with_patient("P1");
        assert!(
            subject_gate(&gate, &principal, Some("P1"), Some("ehr-1"))
                .await
                .is_ok()
        );
    }

    #[test]
    fn missing_patient_claim_is_forbidden() {
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = gate_with_subject(Some(unique("P1")), calls);
        // A principal with no patient claim configured-but-absent → 403.
        let principal = bearer_principal("alice", serde_json::Map::new(), vec!["USER".to_owned()]);
        let err = resolve_patient_claim(&gate, &principal).expect_err("missing claim");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
    }

    /// Every ITS-REST operation the route tables name is either gated
    /// (`Pre`/`Post`) or belongs to a family the matrix deliberately leaves to
    /// RBAC: the item tags, and the definition, demographic, admin, query and
    /// system groups. An EHR-area operation reaching `Skip` by falling through
    /// fails here.
    #[test]
    fn every_route_is_gated_or_rbac_only_by_family() {
        use openehr_its::rest::generated::{admin, definition, demographic, ehr, query, system};
        type Group = (
            &'static str,
            &'static [(&'static str, &'static str, &'static str)],
        );
        let groups: [Group; 6] = [
            ("admin", admin::ROUTES),
            ("definition", definition::ROUTES),
            ("demographic", demographic::ROUTES),
            ("ehr", ehr::ROUTES),
            ("query", query::ROUTES),
            ("system", system::ROUTES),
        ];
        for (group, routes) in groups {
            for (_, _, op) in routes {
                if mode_of(op) == Mode::Skip {
                    assert!(
                        group != "ehr" || op.contains("_tags_"),
                        "{op} ({group}) falls through to Skip without a decision"
                    );
                }
            }
        }
    }

    #[test]
    fn mode_matrix() {
        assert_eq!(mode_of("ehr_create"), Mode::Pre);
        assert_eq!(mode_of("ehr_get_by_subject"), Mode::Pre);
        assert_eq!(mode_of("composition_create"), Mode::Pre);
        assert_eq!(mode_of("composition_delete"), Mode::Pre);
        assert_eq!(mode_of("composition_get"), Mode::Post);
        assert_eq!(mode_of("versioned_composition_get"), Mode::Post);
        assert_eq!(mode_of("contribution_create"), Mode::Pre);
        assert_eq!(mode_of("contribution_get"), Mode::Post);
        // The contribution-list extension is pre-checked (EHR-scoped read).
        assert_eq!(mode_of("contribution_list"), Mode::Pre);
        assert_eq!(
            kind_of("contribution_list"),
            Some(ResourceKind::Contribution)
        );
        assert_eq!(mode_of("ehr_status_update"), Mode::Pre);
        assert_eq!(mode_of("versioned_ehr_status_get"), Mode::Pre);
        assert_eq!(mode_of("directory_create"), Mode::Pre);
        // Query is handled in the query path, not the generic PEP — the
        // cohort extension included.
        assert_eq!(mode_of("query_execute_adhoc_query"), Mode::Skip);
        assert_eq!(mode_of("query_execute_cohort"), Mode::Skip);
        assert_eq!(mode_of("ehr_tags_get"), Mode::Skip);
        assert_eq!(mode_of("definition_template_adl1.4_upload"), Mode::Skip);
    }

    /// A minimal authenticated principal for the pure-function tests below.
    ///
    /// Deliberately built HERE rather than in production code: the gates now
    /// refuse when no principal is present, so production has no reason to
    /// construct one, and a shared fabricated principal is exactly what #1963
    /// removed (attributing a denial to an identity that never authenticated
    /// writes a false subject into the audit trail).
    fn test_principal() -> Principal {
        bearer_principal("test-subject", serde_json::Map::new(), Vec::new())
    }

    /// The PEP reads a resource id through this adapter's one strict
    /// decoder (BASE `base_types` `master05-identification_package.adoc`
    /// §Syntaxes), so a well-formed id decomposes and a malformed one is
    /// DENIED rather than degraded into a whole-string `vo_id`.
    #[test]
    fn object_version_id_parsing() {
        let principal = test_principal();
        let ok = |uid: &str| resolve_target(&principal, uid).map_err(|_resp| "denied");

        let uid = "0197f1c2-3aa0-7000-8000-000000000001::ferroehr.local::2";
        assert_eq!(
            ok(uid),
            Ok((
                "0197f1c2-3aa0-7000-8000-000000000001".to_owned(),
                Some("2".to_owned())
            ))
        );
        // A branch VERSION_TREE_ID is carried whole (RM common master06
        // §The 'Virtual Version Tree').
        assert_eq!(
            ok("0197f1c2-3aa0-7000-8000-000000000001::sys::2.1.3"),
            Ok((
                "0197f1c2-3aa0-7000-8000-000000000001".to_owned(),
                Some("2.1.3".to_owned())
            ))
        );
        // A bare HIER_OBJECT_ID (a UUID) → the versioned object, no version.
        assert_eq!(
            ok("0197f1c2-3aa0-7000-8000-000000000001"),
            Ok(("0197f1c2-3aa0-7000-8000-000000000001".to_owned(), None))
        );
    }

    /// The invalid twins: none of these is a `UID_BASED_ID` this CDR can
    /// address, so each is a fail-closed deny (403) instead of the former
    /// whole-string `vo_id` degradation.
    #[test]
    fn malformed_resource_ids_are_denied() {
        let principal = test_principal();
        for bad in [
            // not a UUID at all
            "abc",
            // two parts, not three (BASE master05 §Syntaxes)
            "0197f1c2-3aa0-7000-8000-000000000001::sys",
            // an empty creating_system_id component
            "0197f1c2-3aa0-7000-8000-000000000001::::2",
            // a VERSION_TREE_ID that is not `N` or `N.B.V`
            "0197f1c2-3aa0-7000-8000-000000000001::sys::v2",
            // a three-part id whose object_id is not this CDR's UUID key
            "not-a-uuid::sys::1",
            "",
        ] {
            let denied = resolve_target(&principal, bad).expect_err("must deny");
            assert_eq!(
                denied.status(),
                StatusCode::FORBIDDEN,
                "id {bad:?} must be a fail-closed deny"
            );
        }
    }

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
        let caller = Caller::new("alice".to_owned(), None, scope, None, false)
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
        assert!(smart_composition_op("composition_get"));
        assert!(smart_composition_op(
            "versioned_composition_revision_history"
        ));
        assert!(!smart_composition_op("composition_tags_get"));
        assert!(!smart_composition_op("ehr_create"));
    }
}
