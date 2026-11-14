// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **access directive exchange** wire: the controller's import and
//! export of an EHR's access directives as HL7 FHIR `Consent` (R4 and R5),
//! IHE BPPC and IHE APPC, **our own extension**.
//!
//! An import commits the directives a document states as a new `EHR_ACCESS`
//! version, appended to the directives in force, over the version
//! `If-Match` names (RFC 9110 §13.1.1), and answers with the new version
//! uid; a document that holds what no directive can state is refused whole
//! with the element named. An export answers the current directives in the
//! format. The mappings, their citations of the vendored FHIR, IHE and XACML
//! texts and the choices those texts leave open are in
//! `ferroehr::access_exchange`. No openEHR ITS-REST operation governs these
//! routes.
//!
//! Gating: mounted under `/admin/`, so every route inherits the group's RBAC
//! Admin class (`401`/`403`) and the `AppConfig::api.admin` config gate
//! (`405` with an empty `Allow` when off). Each import writes its commit
//! audit and an access record, each export an access record.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use axum::response::{IntoResponse as _, Response};
use ferroehr::service::response::ResourceMeta;
use http::{HeaderMap, StatusCode, header};
use openehr_its::rest::runtime::ApiError;
use openehr_rm::draft::access_control::terminology::AccessDefault;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::body::{AdminRefusal, required_query};
use crate::api::{RequestParts, guarded_handler};
use crate::extensions::access::caller::access_default;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::version_id::{if_match, parse_ehr_id};
use crate::state::AppState;
use ferroehr::config::authz::EhrAccessDefault;

/// The access directive exchange routes — **no ITS-REST contract** (module
/// docs).
pub(crate) fn access_exchange_routes() -> OpenApiRouter<AppState> {
    let routes = OpenApiRouter::new()
        .routes(routes!(
            admin_ehr_access_bppc_import,
            admin_ehr_access_bppc_export
        ))
        .routes(routes!(
            admin_ehr_access_appc_import,
            admin_ehr_access_appc_export
        ));
    #[cfg(feature = "fhir")]
    let routes = routes.routes(routes!(
        fhir::admin_ehr_access_consent_import,
        fhir::admin_ehr_access_consent_export
    ));
    routes
}

/// The `default_access` of an EHR with no settings, `[authz.rbac]
/// ehr_access_default`.
fn server_default(state: &AppState) -> AccessDefault {
    access_default(state.authz().as_deref().map_or(
        EhrAccessDefault::Open,
        crate::extensions::access::authz::AuthzHandle::ehr_access_default,
    ))
}

/// The `EHR_ACCESS` version `If-Match` names; a missing one is a `400`.
fn preceding(headers: &HeaderMap) -> Result<String, RestError> {
    if_match(headers).ok_or_else(|| {
        RestError::from(ApiError::BadRequest(
            "If-Match is required: name the EHR_ACCESS version the import is made over".to_owned(),
        ))
    })
}

/// The body as UTF-8 text of one of `media`; another type is a `415`.
fn text_body<'a>(parts: &'a RequestParts, media: &[&str]) -> Result<&'a str, RestError> {
    let content_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::trim)
        .unwrap_or_default();
    if !media.iter().any(|m| m.eq_ignore_ascii_case(content_type)) {
        return Err(RestError::from(ApiError::UnsupportedMediaType(format!(
            "the body must be one of {}",
            media.join(", ")
        ))));
    }
    std::str::from_utf8(&parts.body).map_err(|error| {
        RestError::from(ApiError::BadRequest(format!(
            "the body is not UTF-8 text: {error}"
        )))
    })
}

/// The answer to an import: the committed version and how many directives
/// it added.
fn imported(parts: &RequestParts, meta: &ResourceMeta, count: usize) -> Response {
    negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &serde_json::json!({ "ehr_id": meta.ehr_id, "version_uid": meta.uid, "imported": count }),
    )
}

/// `body` as `media`.
fn document(media: &'static str, body: impl Into<axum::body::Body>) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, media)], body.into()).into_response()
}

const XML: [&str; 2] = ["application/xml", "text/xml"];

guarded_handler! {
    /// Import an IHE BPPC acknowledgement (`POST /admin/ehr-access/bppc`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/ehr-access/bppc", tag = "admin-marks",
        params(
            ("ehr_id" = String, Query, description = "The EHR whose directives the document joins."),
            ("evidence" = String, Query,
             description = "The `ehr:` URI of the composition that holds the document, \
                            the copied directives' `evidence`."),
            ("If-Match" = String, Header,
             description = "The `EHR_ACCESS` version uid the import is made over.")
        ),
        request_body(content = String, content_type = "application/xml",
                     description = "A Patient Privacy Acknowledgement Document (ITI TF-3 5.1). \
                                    Each service event names an `ACCESS_POLICY.catalogue` \
                                    entry by code and code system; the entry is copied with \
                                    the service event's effectiveTime as its time_validity."),
        responses(
            (status = 200, description = "`{ \"ehr_id\": …, \"version_uid\": …, \
                                          \"imported\": n }`: the committed `EHR_ACCESS` \
                                          version.", body = serde_json::Value),
            (status = 400, description = "No If-Match, or a document refused whole: no \
                                          acknowledgement document, a service event that \
                                          breaks ITI TF-3 5.1.2.2, or a policy the \
                                          catalogue does not hold. The problem names the \
                                          element.", body = serde_json::Value),
            (status = 404, description = "No such EHR.", body = serde_json::Value),
            (status = 412, description = "If-Match is not the current `EHR_ACCESS` version.",
             body = serde_json::Value),
            (status = 415, description = "The body is not XML.", body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_ehr_access_bppc_import => |state, parts| super::body::gated(state, parts, bppc_import)
}

guarded_handler! {
    /// Export the catalogue copies as IHE BPPC acknowledgements
    /// (`GET /admin/ehr-access/bppc`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        get, path = "/admin/ehr-access/bppc", tag = "admin-marks",
        params(("ehr_id" = String, Query, description = "The EHR whose directives to export.")),
        responses(
            (status = 200, description = "`{ \"documents\": [\"<ClinicalDocument …>\"], \
                                          \"refused\": [{ \"directive\": i, \"element\": …, \
                                          \"reason\": … }] }`: one acknowledgement per distinct \
                                          time_validity of the directives copied from the \
                                          catalogue (ITI TF-3 5.1.2.2.7), and each directive \
                                          an acknowledgement cannot state, by position.",
             body = serde_json::Value),
            (status = 404, description = "No such EHR.", body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_ehr_access_bppc_export => |state, parts| super::body::gated(state, parts, bppc_export)
}

guarded_handler! {
    /// Import an IHE APPC Privacy Consent Document
    /// (`POST /admin/ehr-access/appc`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        post, path = "/admin/ehr-access/appc", tag = "admin-marks",
        params(
            ("ehr_id" = String, Query, description = "The EHR whose directives the document joins."),
            ("If-Match" = String, Header,
             description = "The `EHR_ACCESS` version uid the import is made over.")
        ),
        request_body(content = String, content_type = "application/xml",
                     description = "An XACML 2.0 policy set (APPC TF-3 5.6): each policy one \
                                    directive, each reference a catalogue entry."),
        responses(
            (status = 200, description = "`{ \"ehr_id\": …, \"version_uid\": …, \
                                          \"imported\": n }`.", body = serde_json::Value),
            (status = 400, description = "No If-Match, or a document refused whole: a root \
                                          combining by other than deny-overrides, a policy \
                                          whose rule-combining algorithm lets no exception \
                                          decide, a condition over another attribute, an \
                                          attribute with no directive form, or a reference \
                                          the catalogue does not hold. The problem names the \
                                          element.", body = serde_json::Value),
            (status = 404, description = "No such EHR.", body = serde_json::Value),
            (status = 412, description = "If-Match is not the current `EHR_ACCESS` version.",
             body = serde_json::Value),
            (status = 415, description = "The body is not XML.", body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_ehr_access_appc_import => |state, parts| super::body::gated(state, parts, appc_import)
}

guarded_handler! {
    /// Export the directives as one IHE APPC Privacy Consent Document
    /// (`GET /admin/ehr-access/appc`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module docs).
    #[utoipa::path(
        get, path = "/admin/ehr-access/appc", tag = "admin-marks",
        params(("ehr_id" = String, Query, description = "The EHR whose directives to export.")),
        responses(
            (status = 200, description = "The XACML 2.0 policy set, `application/xml`: one \
                                          policy per directive under deny-overrides.",
             body = String, content_type = "application/xml"),
            (status = 400, description = "A directive holds a scope APPC has no attribute \
                                          for; the problem names the attribute.",
             body = serde_json::Value),
            (status = 404, description = "No such EHR.", body = serde_json::Value),
            AdminRefusal
        )
    )]
    admin_ehr_access_appc_export => |state, parts| super::body::gated(state, parts, appc_export)
}

async fn bppc_import(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let ehr_id = parse_ehr_id(&required_query(&parts, "ehr_id")?)?;
    let evidence = required_query(&parts, "evidence")?;
    let preceding = preceding(&parts.headers)?;
    let text = text_body(&parts, &XML)?;
    let (meta, count) = state
        .backend()
        .import_bppc(ehr_id, &preceding, server_default(&state), &evidence, text)
        .await?;
    Ok(imported(&parts, &meta, count))
}

async fn bppc_export(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let ehr_id = parse_ehr_id(&required_query(&parts, "ehr_id")?)?;
    let exported = state.backend().export_bppc(ehr_id).await?;
    let refused: Vec<serde_json::Value> = exported
        .refused
        .iter()
        .map(|(at, why)| {
            serde_json::json!({ "directive": at, "element": why.element, "reason": why.to_string() })
        })
        .collect();
    Ok(negotiate::body::respond(
        &parts.headers,
        StatusCode::OK,
        &serde_json::json!({ "documents": exported.documents, "refused": refused }),
    ))
}

async fn appc_import(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let ehr_id = parse_ehr_id(&required_query(&parts, "ehr_id")?)?;
    let preceding = preceding(&parts.headers)?;
    let text = text_body(&parts, &XML)?;
    let (meta, count) = state
        .backend()
        .import_appc(ehr_id, &preceding, server_default(&state), text)
        .await?;
    Ok(imported(&parts, &meta, count))
}

async fn appc_export(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let ehr_id = parse_ehr_id(&required_query(&parts, "ehr_id")?)?;
    let text = state.backend().export_appc(ehr_id).await?;
    Ok(document("application/xml", text))
}

#[cfg(feature = "fhir")]
mod fhir {
    use axum::response::Response;
    use ferroehr::access_exchange::fhir::Release;
    use openehr_its::rest::runtime::ApiError;

    use super::{document, imported, preceding, server_default, text_body};
    use crate::api::admin::body::{AdminRefusal, required_query};
    use crate::api::{RequestParts, guarded_handler};
    use crate::overview::error::RestError;
    use crate::overview::version_id::parse_ehr_id;
    use crate::state::AppState;

    /// The release `{release}` names: `r4` or `r5`.
    fn release(parts: &RequestParts) -> Result<Release, RestError> {
        match parts.path.get("release").map(String::as_str) {
            Some("r4") => Ok(Release::R4),
            Some("r5") => Ok(Release::R5),
            other => Err(RestError::from(ApiError::NotFound(format!(
                "no FHIR release {}: r4 and r5 are served",
                other.unwrap_or_default()
            )))),
        }
    }

    guarded_handler! {
        /// Import a FHIR `Consent`
        /// (`POST /admin/ehr-access/fhir/{release}/Consent`).
        ///
        /// **Our own extension — no ITS-REST operation governs this** (module docs).
        #[utoipa::path(
            post, path = "/admin/ehr-access/fhir/{release}/Consent", tag = "admin-marks",
            params(
                ("release" = String, Path, description = "`r4` (FHIR 4.0.1) or `r5` (FHIR 5.0.0)."),
                ("ehr_id" = String, Query,
                 description = "The EHR whose directives the `Consent` joins."),
                ("If-Match" = String, Header,
                 description = "The `EHR_ACCESS` version uid the import is made over.")
            ),
            request_body(content = serde_json::Value, content_type = "application/fhir+json",
                         description = "One `Consent` of the release, `scope` \
                                        `patient-privacy` in R4. The base policy and its \
                                        provisions become directives with one level of \
                                        exceptions; a foreign action coding is translated by \
                                        the terminology server."),
            responses(
                (status = 200, description = "`{ \"ehr_id\": …, \"version_uid\": …, \
                                              \"imported\": n }`.", body = serde_json::Value),
                (status = 400, description = "No If-Match, or a `Consent` refused whole: a \
                                              provision nested deeper than one exception \
                                              level, `provision.data.meaning` other than \
                                              `instance`, R5 `provision.expression`, a \
                                              policy the catalogue does not hold, a party the \
                                              demographic service does not hold, or another \
                                              element with no directive form. The problem \
                                              names the element.", body = serde_json::Value),
                (status = 404, description = "No such EHR or release.",
                 body = serde_json::Value),
                (status = 412, description = "If-Match is not the current `EHR_ACCESS` \
                                              version.", body = serde_json::Value),
                (status = 415, description = "The body is not FHIR JSON.",
                 body = serde_json::Value),
                AdminRefusal
            )
        )]
        admin_ehr_access_consent_import => |state, parts| {
            crate::api::admin::body::gated(state, parts, consent_import)
        }
    }

    guarded_handler! {
        /// Export the directives as FHIR `Consent`
        /// (`GET /admin/ehr-access/fhir/{release}/Consent`).
        ///
        /// **Our own extension — no ITS-REST operation governs this** (module docs).
        #[utoipa::path(
            get, path = "/admin/ehr-access/fhir/{release}/Consent", tag = "admin-marks",
            params(
                ("release" = String, Path, description = "`r4` (FHIR 4.0.1) or `r5` (FHIR 5.0.0)."),
                ("ehr_id" = String, Query, description = "The EHR whose directives to export.")
            ),
            responses(
                (status = 200, description = "A `Bundle` of type `collection`, one `Consent` \
                                              per directive; every attribute with no core \
                                              element in a FerroEHR extension.",
                 body = serde_json::Value, content_type = "application/fhir+json"),
                (status = 400, description = "A directive holds what a `Consent` of the \
                                              release cannot state; the problem names the \
                                              attribute.", body = serde_json::Value),
                (status = 404, description = "No such EHR or release.",
                 body = serde_json::Value),
                AdminRefusal
            )
        )]
        admin_ehr_access_consent_export => |state, parts| {
            crate::api::admin::body::gated(state, parts, consent_export)
        }
    }

    async fn consent_import(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
        let release = release(&parts)?;
        let ehr_id = parse_ehr_id(&required_query(&parts, "ehr_id")?)?;
        let preceding = preceding(&parts.headers)?;
        let text = text_body(&parts, &["application/fhir+json", "application/json"])?;
        let (meta, count) = state
            .backend()
            .import_fhir_consent(
                ehr_id,
                &preceding,
                server_default(&state),
                release,
                text.as_bytes(),
            )
            .await?;
        Ok(imported(&parts, &meta, count))
    }

    async fn consent_export(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
        let release = release(&parts)?;
        let ehr_id = parse_ehr_id(&required_query(&parts, "ehr_id")?)?;
        let body = state.backend().export_fhir_consent(ehr_id, release).await?;
        Ok(document("application/fhir+json", body))
    }
}
