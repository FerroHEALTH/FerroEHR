// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CDR's **FHIR connector** surface the viewer consumes.
//!
//! The availability probe the whole screen and its nav entry are gated on, the
//! five mapping-store operations (list / create / read-by-id / update /
//! delete), the patient-scoped read facade, and the validate-only dry run.
//!
//! NOTE: no openEHR spec governs FHIR interop — our own extension; the external
//! standard is HL7 FHIR R4 (<https://hl7.org/fhir/R4/>), and the whole group
//! (paths, payloads, status codes) is the CDR's own design.
//!
//! **The viewer never commits a FHIR resource.** `POST /fhir/r4/{type}` is the
//! connector's real inbound door — it maps, validates and COMMITS — and it has
//! no viewer path at all. Verification here is read-only in effect: the read
//! facade shows what a stored mapping produces on read, and the dry run
//! (`POST /fhir/r4/{type}/$validate`) reports the verdict the ingest door would
//! reach while committing nothing.
//!
//! **Probe-and-hide.** The group is config-gated on the CDR (`[fhir]
//! api_enabled`, off by default) and answers `404` for every route as if
//! unmounted while it is off, so the viewer discovers it
//! ([`probe_fhir_connector`]) before offering any of it. Capability is not
//! authorization: a mounted-but-refused group (`401`/`403`) still counts as
//! present, and the refusal surfaces as copy on the screen that asked.
//!
//! **Two error vocabularies meet on this surface, and ONE reader speaks
//! both.** Every refusal the connector itself authors is a FHIR
//! `OperationOutcome` (`{resourceType, issue: [{severity, code,
//! diagnostics}]}`); authentication and authorization refusals come from the
//! layer ABOVE the handler and carry the openEHR `{error, message}` body. The
//! shared reader ([`crate::cdr`]'s `expect_success`) extracts the human
//! diagnostic from either shape; [`outcome_summary`] serves the VERDICT panel,
//! which classifies outcomes rather than reading refusals.
//!
//! Every `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first (a server fn is a
//! publicly reachable HTTP endpoint) and keeps the CDR credential server-side.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the FHIR carriers here are read on both targets, and the ssr-only ones \
              would leave an #[expect] unfulfilled on the hydrate target"
)]

use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;

/// The media type every FHIR-connector route speaks.
pub const FHIR_JSON: &str = "application/fhir+json";

/// One stored mapping record as `/admin/fhir_mapping` serves it.
///
/// Every field is a string or a `bool`: the record crosses the server-fn
/// boundary and is rendered or edited, never computed with (no `usize`, and no
/// parsing the viewer would have to keep in step with the CDR's own
/// formatting). `definition` carries the whole deep mapping document
/// pretty-printed, because that document IS what the editor edits.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FhirMappingRow {
    /// The mapping's UUID, as the CDR spelled it.
    pub id: String,
    /// The deployable name — unique in the store, and immutable once created.
    pub name: String,
    /// The FHIR resource type the mapping binds, projected from the definition.
    pub resource_type: String,
    /// The `meta.profile` URL the mapping matches on; empty = the type's
    /// profile-less default mapping.
    pub profile_url: String,
    /// The openEHR template the mapping builds a COMPOSITION under.
    pub template_id: String,
    /// Whether the connector resolves this mapping at all.
    pub enabled: bool,
    /// The store row's creation instant, verbatim from the wire.
    pub created_at: String,
    /// The whole mapping definition document, pretty-printed for the editor.
    pub definition: String,
}

/// One issue of a FHIR `OperationOutcome`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OutcomeIssue {
    /// `fatal` / `error` / `warning` / `information`.
    pub severity: String,
    /// The issue type code (`invalid`, `not-found`, `required`, …).
    pub code: String,
    /// The human diagnostic — the CDR's own words, never paraphrased.
    pub diagnostics: String,
}

impl OutcomeIssue {
    /// Whether this issue is a refusal rather than a remark.
    ///
    /// HL7 FHIR R4 orders `IssueSeverity` as fatal / error / warning /
    /// information (<https://hl7.org/fhir/R4/valueset-issue-severity.html>);
    /// the first two are the ones that make a verdict negative.
    #[must_use]
    pub fn is_failure(&self) -> bool {
        matches!(self.severity.as_str(), "fatal" | "error")
    }
}

/// One answer from a FHIR-connector route, as the viewer renders it.
///
/// The body travels VERBATIM (pretty-printed only) because both verification
/// panels show the CDR's own document — a Bundle, or an `OperationOutcome`
/// whose issues are the whole point.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FhirAnswer {
    /// The HTTP status the CDR answered with.
    pub status: u16,
    /// The response body, pretty-printed but otherwise untouched.
    pub body: String,
    /// The `OperationOutcome` issues, when the body is one.
    pub issues: Vec<OutcomeIssue>,
}

impl FhirAnswer {
    /// Whether the CDR completed the operation (any success status).
    #[must_use]
    pub fn completed(&self) -> bool {
        http::StatusCode::from_u16(self.status).is_ok_and(|s| s.is_success())
    }

    /// Whether the body is a FHIR `OperationOutcome` rather than a resource.
    #[must_use]
    pub fn is_outcome(&self) -> bool {
        !self.issues.is_empty()
    }
}

/// What a dry run concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DryRunVerdict {
    /// The validation ran and the resource maps to a committable COMPOSITION.
    Valid,
    /// The validation ran and the commit path would refuse the mapped
    /// COMPOSITION — the refusal rides the outcome's `error` issues.
    Invalid,
    /// No validation happened: the operation itself failed before a verdict
    /// (no mapping for the type, a type outside the connector's set, a
    /// malformed resource, the connector switched off).
    NotRun,
}

impl DryRunVerdict {
    /// The one-word label the panel renders.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Valid => "Valid",
            Self::Invalid => "Invalid",
            Self::NotRun => "Not validated",
        }
    }

    /// The `data-fhir-verdict` hook value — the stable machine-readable name.
    #[must_use]
    pub fn hook(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::NotRun => "not-run",
        }
    }
}

/// Classify a dry-run answer.
///
/// A COMPLETED validation is `200` whichever way the verdict fell — the CDR
/// carries the verdict in the outcome's issues, so a `200` with an
/// `error`/`fatal` issue is [`DryRunVerdict::Invalid`], not a transport
/// failure. Anything else means the operation never reached a verdict.
#[must_use]
pub fn verdict_of(answer: &FhirAnswer) -> DryRunVerdict {
    if !answer.completed() {
        return DryRunVerdict::NotRun;
    }
    if answer.issues.iter().any(OutcomeIssue::is_failure) {
        DryRunVerdict::Invalid
    } else {
        DryRunVerdict::Valid
    }
}

/// Whether the CDR serves its FHIR connector.
///
/// Carries only fixed-size, client-safe data — it crosses the server-fn
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FhirAvailability {
    /// The group answered: it is mounted, so the screen is offered. Whether
    /// THIS session may use it is a per-request answer.
    Available,
    /// The group is not mounted (`[fhir] api_enabled = false`) — the CDR
    /// answered `404` as if the routes did not exist.
    Disabled,
}

impl FhirAvailability {
    /// Whether the viewer may offer the FHIR connector screen.
    #[must_use]
    pub fn usable(self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Classify the probe's HTTP status: only a `404` means "not mounted".
///
/// A `401`/`403` is a mounted group refusing THIS session (the mapping store
/// sits under `/admin`, which the coarse RBAC gate classes as admin work) — the
/// surface exists, which is what the nav gate asks about.
#[must_use]
pub fn availability_of_status(status: http::StatusCode) -> FhirAvailability {
    if status == http::StatusCode::NOT_FOUND {
        FhirAvailability::Disabled
    } else {
        FhirAvailability::Available
    }
}

/// The single predicate every FHIR-gated affordance uses: render only for a
/// probe that succeeded and found the connector mounted.
///
/// A failed probe (CDR unreachable, expired session) hides it — never a nav
/// link to a screen that cannot work.
#[must_use]
pub fn renders_fhir_connector(probe: &Result<FhirAvailability, ViewerError>) -> bool {
    probe.as_ref().copied().is_ok_and(FhirAvailability::usable)
}

/// The connector gate: one probe [`Resource`].
///
/// Created in component setup — never inside a `Suspend` closure, which re-runs
/// and would re-create the resource.
#[must_use]
pub fn fhir_gate() -> Resource<Result<FhirAvailability, ViewerError>> {
    Resource::new(|| (), |()| async move { probe_fhir_connector().await })
}

/// Render `affordance` only when the gate found the connector mounted;
/// otherwise render nothing at all (probe-and-hide).
///
/// The probe is resolved INSIDE the `<Suspense>`: an SSR'd `ErrorBoundary`
/// fallback mismatches at hydration in leptos 0.8, and a render-time resource
/// read is itself a hydration mismatch. `affordance` creates no resources, so
/// re-runs are safe, and it is shared through an `Arc` because the `Suspend`
/// closure must not consume its environment.
#[must_use]
pub fn when_fhir_connector_usable(
    gate: Resource<Result<FhirAvailability, ViewerError>>,
    affordance: impl Fn() -> AnyView + Send + Sync + 'static,
) -> AnyView {
    let affordance: std::sync::Arc<dyn Fn() -> AnyView + Send + Sync> =
        std::sync::Arc::new(affordance);
    view! {
        <Suspense fallback=|| ()>
            {move || {
                let affordance = std::sync::Arc::clone(&affordance);
                Suspend::new(async move {
                    if renders_fhir_connector(&gate.await) { affordance() } else { ().into_any() }
                })
            }}
        </Suspense>
    }
    .into_any()
}

/// The client-side complaint about a mapping draft, or `Ok(())` when it can be
/// sent.
///
/// It is the same judgement the CDR makes on upload — a non-empty name matching
/// `[A-Za-z0-9_.-]`, and a `definition` that is a JSON object — checked here so
/// the submit button is inert until the form can actually succeed. The server fn
/// re-checks, because it is a public endpoint, and the CDR checks again: this is
/// convenience, never the authority.
///
/// # Errors
/// The sentence to render inline when the draft cannot be sent.
pub fn mapping_draft_complaint(name: &str, definition: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A mapping needs a name — it is the store's deployable identity.".to_owned());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err(format!(
            "The name `{name}` is not addressable: the CDR accepts letters, digits, `_`, `.` and \
             `-` only."
        ));
    }
    definition_complaint(definition)
}

/// The client-side complaint about a mapping definition alone (the edit card,
/// where the name is immutable and never sent).
///
/// # Errors
/// The sentence to render inline when the document cannot be sent.
pub fn definition_complaint(definition: &str) -> Result<(), String> {
    if definition.trim().is_empty() {
        return Err("The mapping definition is empty.".to_owned());
    }
    match serde_json::from_str::<serde_json::Value>(definition) {
        Ok(serde_json::Value::Object(_)) => Ok(()),
        Ok(_) => Err(
            "The mapping definition must be a JSON object (`{ \"resource_type\": … }`).".to_owned(),
        ),
        Err(e) => Err(format!("The mapping definition is not valid JSON: {e}")),
    }
}

/// Whether a read-facade request is complete enough to send: the facade serves
/// only the explicit patient scope, so both fields are required.
#[must_use]
pub fn read_request_is_complete(resource_type: &str, patient: &str) -> bool {
    !resource_type.trim().is_empty() && !patient.trim().is_empty()
}

/// Actionable copy for a refused mapping-store write.
///
/// Names the object, carries the CDR's own diagnostic verbatim, and names the
/// next action. The store's `409` is a duplicate deployable NAME (which is
/// immutable, so the remedy is a different name), and its `400` is most often
/// an unknown `template_id` — so this family gets its own wording rather than
/// the generic write copy.
#[must_use]
pub fn mapping_failure_copy(object: &str, error: &ViewerError) -> String {
    match error {
        ViewerError::Cdr { message, .. }
            if error.status_code() == Some(http::StatusCode::CONFLICT) =>
        {
            format!(
                "The mapping store already holds {object}: {message}. A mapping name is \
                 immutable — register the new mapping under a different name, or edit the \
                 existing one."
            )
        }
        ViewerError::Cdr { message, .. }
            if error.status_code() == Some(http::StatusCode::NOT_FOUND) =>
        {
            format!(
                "{object} is not in the mapping store any more ({message}) — another operator may \
                 have deleted it. Reload this screen."
            )
        }
        ViewerError::Forbidden(message) => format!(
            "This session may not administer the FHIR mapping store ({message}). Sign in with an \
             ADMIN-role account and retry."
        ),
        other => crate::feedback::write_failure_copy(object, other),
    }
}

/// The issues of a FHIR `OperationOutcome`, read defensively.
///
/// A body that is not an outcome (a Bundle, a mapping record) yields no
/// issues, which is exactly how [`FhirAnswer::is_outcome`] tells the two apart.
#[must_use]
pub fn outcome_issues(body: &serde_json::Value) -> Vec<OutcomeIssue> {
    if body.get("resourceType").and_then(serde_json::Value::as_str) != Some("OperationOutcome") {
        return Vec::new();
    }
    let text = |item: &serde_json::Value, key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    body.get("issue")
        .and_then(serde_json::Value::as_array)
        .map(|issues| {
            issues
                .iter()
                .map(|issue| OutcomeIssue {
                    severity: text(issue, "severity"),
                    code: text(issue, "code"),
                    diagnostics: text(issue, "diagnostics"),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The one-line diagnostic of a FHIR `OperationOutcome` body, for a toast.
///
/// `None` when the body is not an outcome — the caller then falls back to the
/// shared openEHR reader, which is what an authentication or authorization
/// refusal from the layer above the handler actually carries.
#[must_use]
pub fn outcome_summary(body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let issues = outcome_issues(&value);
    if issues.is_empty() {
        return None;
    }
    let summary = issues
        .iter()
        .map(|issue| {
            if issue.diagnostics.is_empty() {
                issue.code.clone()
            } else {
                issue.diagnostics.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    (!summary.is_empty()).then_some(summary)
}

/// Probe the CDR for its FHIR connector (`GET admin/fhir_mapping`).
///
/// The mapping-store listing is the availability signal: a `404` means the
/// group is not mounted for this deployment (`[fhir] api_enabled = false`), any
/// other answer means it is.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnreachable`] on transport failure.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn probe_fhir_connector() -> Result<FhirAvailability, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("admin/fhir_mapping");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    Ok(availability_of_status(response.status))
}

/// List the stored FHIR mappings (`GET admin/fhir_mapping`).
///
/// `Ok(None)` is the CDR answering `404` — the connector is disabled, which is
/// an absent surface rather than an error, and the screen renders it as its own
/// first-class state.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn list_fhir_mappings() -> Result<Option<Vec<FhirMappingRow>>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("admin/fhir_mapping");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let value = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| ViewerError::Internal(format!("FHIR mapping list JSON: {e}")))?;
    let rows = value
        .as_array()
        .map(|items| items.iter().map(mapping_row).collect())
        .unwrap_or_default();
    Ok(Some(rows))
}

/// Store a new FHIR mapping (`POST admin/fhir_mapping`, body
/// `{name, definition, enabled}`).
///
/// The CDR's `400` (malformed definition, unknown `template_id`), `409`
/// (duplicate name) and `415` diagnostics surface verbatim as
/// `OperationOutcome` text.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when the draft cannot be sent;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the stored record is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn create_fhir_mapping(
    /// The deployable mapping name (unique in the store, immutable afterwards).
    name: String,
    /// The mapping definition document, as JSON text.
    definition: String,
    /// Whether the connector should resolve the mapping.
    enabled: bool,
) -> Result<FhirMappingRow, ViewerError> {
    let session = crate::session::require_session().await?;
    mapping_draft_complaint(&name, &definition).map_err(ViewerError::Invalid)?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("admin/fhir_mapping");
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            mapping_body(Some(name.trim()), &definition, enabled)?,
        )
        .await?;
    stored_record(&crate::cdr::CdrClient::expect_success(response)?.body)
}

/// Replace a stored mapping's definition and enabled flag
/// (`PUT admin/fhir_mapping/{mapping_id}`).
///
/// The name is NOT sent: the CDR treats it as the immutable deployable
/// identity, so the viewer does not offer to change it.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for an empty id or an unsendable document;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the stored record is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn update_fhir_mapping(
    /// The mapping to update, by store id.
    mapping_id: String,
    /// The mapping definition document to store, as JSON text.
    definition: String,
    /// Whether the connector should resolve the mapping.
    enabled: bool,
) -> Result<FhirMappingRow, ViewerError> {
    let session = crate::session::require_session().await?;
    if mapping_id.trim().is_empty() {
        return Err(ViewerError::Invalid("no mapping id to update".to_owned()));
    }
    definition_complaint(&definition).map_err(ViewerError::Invalid)?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "admin/fhir_mapping/{}",
        urlencoding::encode(&mapping_id)
    ));
    let response = state
        .cdr
        .put(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            mapping_body(None, &definition, enabled)?,
        )
        .await?;
    stored_record(&crate::cdr::CdrClient::expect_success(response)?.body)
}

/// Delete one stored mapping (`DELETE admin/fhir_mapping/{mapping_id}`).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for an empty id;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn delete_fhir_mapping(
    /// The mapping to delete, by store id.
    mapping_id: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    if mapping_id.trim().is_empty() {
        return Err(ViewerError::Invalid("no mapping id to delete".to_owned()));
    }
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "admin/fhir_mapping/{}",
        urlencoding::encode(&mapping_id)
    ));
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    crate::cdr::CdrClient::expect_success(response)?;
    Ok(())
}

/// Read what a stored mapping produces for one patient
/// (`GET fhir/r4/{resource_type}?patient=…`).
///
/// A refusal the connector authors — a missing scope, an unsupported resource
/// type — is an `OperationOutcome`, and the viewer renders it VERBATIM, so it
/// comes back as an `Ok` [`FhirAnswer`] carrying the CDR's own document rather
/// than as an error whose text a screen would have to paraphrase. Only a
/// failure that produced no FHIR document at all (no session, a refusal from
/// the authentication layer, an unreachable CDR) is an `Err`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when either scope field is blank;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] on an
/// authentication or authorization refusal;
/// [`ViewerError::CdrUnreachable`] on transport failure.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn read_fhir_resources(
    /// The FHIR resource type to read.
    resource_type: String,
    /// The patient scope (an EHR subject id, or an EHR id).
    patient: String,
) -> Result<FhirAnswer, ViewerError> {
    let session = crate::session::require_session().await?;
    if !read_request_is_complete(&resource_type, &patient) {
        return Err(ViewerError::Invalid(
            "the read facade serves only an explicit resource type and patient scope".to_owned(),
        ));
    }
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "fhir/r4/{}?patient={}",
        urlencoding::encode(resource_type.trim()),
        urlencoding::encode(patient.trim())
    ));
    let response = state.cdr.get(&session.credential, &url, FHIR_JSON).await?;
    fhir_answer(&response)
}

/// Validate a FHIR resource against its stored mapping WITHOUT committing
/// (`POST fhir/r4/{resource_type}/$validate`).
///
/// The wire convention is HL7 FHIR R4's own validation operation
/// (<https://hl7.org/fhir/R4/resource-operation-validate.html>). A COMPLETED
/// validation is `200` whichever way the verdict fell, so both outcomes come
/// back as an `Ok` [`FhirAnswer`] and [`verdict_of`] classifies it; an
/// operation-level refusal (no mapping, an unsupported type, a malformed
/// resource) is also a FHIR document and is rendered verbatim the same way.
/// **Nothing is committed either way** — this is the ingest door's dry twin,
/// and the viewer offers no path to the ingest door itself.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for a blank resource type or body;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] on an
/// authentication or authorization refusal;
/// [`ViewerError::CdrUnreachable`] on transport failure.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn dry_run_fhir_resource(
    /// The FHIR resource type to validate against.
    resource_type: String,
    /// The FHIR resource, as JSON text.
    resource: String,
) -> Result<FhirAnswer, ViewerError> {
    let session = crate::session::require_session().await?;
    if resource_type.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "a dry run needs the FHIR resource type to validate against".to_owned(),
        ));
    }
    if resource.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "a dry run needs a FHIR resource to validate".to_owned(),
        ));
    }
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "fhir/r4/{}/$validate",
        urlencoding::encode(resource_type.trim())
    ));
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            FHIR_JSON,
            &[],
            resource,
        )
        .await?;
    fhir_answer(&response)
}

/// Turn a connector response into the answer the panels render.
///
/// A body the CDR authored travels through whatever its status: the panels show
/// the document, so a `400`/`404`/`501` `OperationOutcome` is content, not an
/// error. Only a refusal from the authentication layer above the handler — which
/// carries the openEHR error body, never an outcome — becomes an `Err`, and
/// which refusal it was (`401` or `403`) is the shared reader's call.
#[cfg(feature = "ssr")]
fn fhir_answer(response: &crate::cdr::CdrResponse) -> Result<FhirAnswer, ViewerError> {
    if (response.is(http::StatusCode::UNAUTHORIZED) || response.is(http::StatusCode::FORBIDDEN))
        // NOTE: the shared reader speaks both refusal vocabularies, so the
        // auth layer's openEHR body needs no FHIR-side shim.
        && let Err(e) = crate::cdr::CdrClient::expect_success(response.clone())
    {
        return Err(e);
    }
    let parsed = serde_json::from_str::<serde_json::Value>(&response.body).ok();
    let issues = parsed.as_ref().map(outcome_issues).unwrap_or_default();
    let body = parsed
        .as_ref()
        .and_then(|value| serde_json::to_string_pretty(value).ok())
        .unwrap_or_else(|| response.body.clone());
    Ok(FhirAnswer {
        status: response.status.as_u16(),
        body,
        issues,
    })
}

/// The `{name?, definition, enabled}` request body a create or update sends.
///
/// Built through `serde_json` so the definition document travels as JSON rather
/// than as an escaped string, and so a name carrying a quote is escaped by the
/// encoder. The update form omits the name, which the CDR treats as immutable.
#[cfg(feature = "ssr")]
fn mapping_body(
    name: Option<&str>,
    definition: &str,
    enabled: bool,
) -> Result<String, ViewerError> {
    let document = serde_json::from_str::<serde_json::Value>(definition).map_err(|e| {
        ViewerError::Invalid(format!("the mapping definition is not valid JSON: {e}"))
    })?;
    let mut body = serde_json::Map::new();
    if let Some(name) = name {
        drop(body.insert("name".to_owned(), serde_json::Value::from(name)));
    }
    drop(body.insert("definition".to_owned(), document));
    drop(body.insert("enabled".to_owned(), serde_json::Value::from(enabled)));
    Ok(serde_json::Value::Object(body).to_string())
}

/// Parse the stored record a create or update answered with.
#[cfg(feature = "ssr")]
fn stored_record(body: &str) -> Result<FhirMappingRow, ViewerError> {
    let value = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|e| ViewerError::Internal(format!("FHIR mapping record JSON: {e}")))?;
    Ok(mapping_row(&value))
}

/// Distil one store element into a [`FhirMappingRow`], reading each field
/// defensively so a missing or renamed field empties that cell rather than
/// dropping the whole row from the listing.
///
/// `profile_url` is nullable on the wire — a mapping without one is the
/// resource type's profile-less default — and reads as the empty string here,
/// which the table renders as an em dash.
#[must_use]
pub fn mapping_row(item: &serde_json::Value) -> FhirMappingRow {
    let text = |key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let definition = item
        .get("definition")
        .and_then(|value| serde_json::to_string_pretty(value).ok())
        .unwrap_or_default();
    FhirMappingRow {
        id: text("id"),
        name: text("name"),
        resource_type: text("resource_type"),
        profile_url: text("profile_url"),
        template_id: text("template_id"),
        enabled: item
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        created_at: text("created_at"),
        definition,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DryRunVerdict, FhirAnswer, FhirAvailability, OutcomeIssue, availability_of_status,
        definition_complaint, mapping_draft_complaint, mapping_failure_copy, mapping_row,
        outcome_issues, outcome_summary, read_request_is_complete, renders_fhir_connector,
        verdict_of,
    };
    use crate::error::ViewerError;

    /// The `$validate` pass verdict, exactly as the CDR served it.
    const VALID_OUTCOME: &str = r#"{"resourceType":"OperationOutcome","issue":[
        {"severity":"information","code":"informational","diagnostics":"valid: the resource maps to a COMPOSITION under template 'minimal_evaluation.en.v1' that passes commit validation; nothing was committed"},
        {"severity":"information","code":"informational","diagnostics":"would create a new EHR for subject 'p-42' (namespace 'fhir')"}]}"#;

    /// The `$validate` refusal verdict, exactly as the CDR served it.
    const INVALID_OUTCOME: &str = r#"{"resourceType":"OperationOutcome","issue":[
        {"severity":"error","code":"invalid","diagnostics":"/territory: code 'ZZ' is not a valid country (ISO 3166-1) (openEHR terminology)"},
        {"severity":"information","code":"informational","diagnostics":"would create a new EHR for subject 'p-42' (namespace 'fhir')"}]}"#;

    fn answer(status: u16, body: &str) -> FhirAnswer {
        let value = serde_json::from_str::<serde_json::Value>(body).expect("fixture JSON");
        FhirAnswer {
            status,
            body: body.to_owned(),
            issues: outcome_issues(&value),
        }
    }

    #[test]
    fn only_a_404_means_the_connector_is_not_mounted() {
        assert_eq!(
            availability_of_status(http::StatusCode::NOT_FOUND),
            FhirAvailability::Disabled
        );
        // Mounted-but-refused, and mounted-and-served, are both "it exists".
        for status in [
            http::StatusCode::OK,
            http::StatusCode::UNAUTHORIZED,
            http::StatusCode::FORBIDDEN,
            http::StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert_eq!(
                availability_of_status(status),
                FhirAvailability::Available,
                "{status}"
            );
        }
        assert!(FhirAvailability::Available.usable());
        assert!(!FhirAvailability::Disabled.usable());
    }

    #[test]
    fn only_an_available_connector_renders_its_affordances() {
        assert!(renders_fhir_connector(&Ok(FhirAvailability::Available)));
        assert!(!renders_fhir_connector(&Ok(FhirAvailability::Disabled)));
        // A probe that never got an answer hides the entry too.
        assert!(!renders_fhir_connector(&Err(ViewerError::Unauthenticated)));
        assert!(!renders_fhir_connector(&Err(ViewerError::CdrUnreachable(
            "connection refused".to_owned()
        ))));
    }

    #[test]
    fn a_completed_validation_is_200_whichever_way_the_verdict_fell() {
        assert_eq!(
            verdict_of(&answer(200, VALID_OUTCOME)),
            DryRunVerdict::Valid
        );
        assert_eq!(
            verdict_of(&answer(200, INVALID_OUTCOME)),
            DryRunVerdict::Invalid
        );
        // An operation-level refusal reached no verdict at all.
        for status in [400_u16, 404, 501] {
            assert_eq!(
                verdict_of(&answer(
                    status,
                    r#"{"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"not-found","diagnostics":"no enabled FHIR mapping for resource type 'Patient'"}]}"#
                )),
                DryRunVerdict::NotRun,
                "{status}"
            );
        }
    }

    #[test]
    fn a_fatal_issue_fails_the_verdict_like_an_error_does() {
        assert!(
            OutcomeIssue {
                severity: "fatal".to_owned(),
                ..OutcomeIssue::default()
            }
            .is_failure()
        );
        assert!(
            OutcomeIssue {
                severity: "error".to_owned(),
                ..OutcomeIssue::default()
            }
            .is_failure()
        );
        // A warning is a remark, not a refusal — the CDR's own severity order.
        for severity in ["warning", "information"] {
            assert!(
                !OutcomeIssue {
                    severity: severity.to_owned(),
                    ..OutcomeIssue::default()
                }
                .is_failure(),
                "{severity}"
            );
        }
    }

    #[test]
    fn the_verdict_labels_and_hooks_are_distinct() {
        let all = [
            DryRunVerdict::Valid,
            DryRunVerdict::Invalid,
            DryRunVerdict::NotRun,
        ];
        let mut hooks: Vec<&str> = all.iter().map(|v| v.hook()).collect();
        hooks.sort_unstable();
        hooks.dedup();
        assert_eq!(hooks.len(), all.len(), "every verdict has its own hook");
        assert_eq!(DryRunVerdict::Valid.label(), "Valid");
        assert_eq!(DryRunVerdict::Invalid.label(), "Invalid");
        assert_eq!(DryRunVerdict::NotRun.label(), "Not validated");
    }

    #[test]
    fn an_outcome_yields_its_issues_and_a_bundle_yields_none() {
        let outcome =
            serde_json::from_str::<serde_json::Value>(INVALID_OUTCOME).expect("fixture JSON");
        let issues = outcome_issues(&outcome);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].severity, "error");
        assert_eq!(issues[0].code, "invalid");
        assert!(issues[0].diagnostics.contains("ISO 3166-1"));

        // The read facade's success body is a Bundle, which carries no issues —
        // that is how the viewer tells a resource from a refusal.
        let bundle = serde_json::json!({
            "resourceType": "Bundle", "type": "searchset", "total": 0, "entry": [],
        });
        assert!(outcome_issues(&bundle).is_empty());
        assert!(!answer(200, &bundle.to_string()).is_outcome());
        assert!(answer(200, VALID_OUTCOME).is_outcome());
    }

    #[test]
    fn the_summary_joins_the_diagnostics_and_declines_a_non_outcome() {
        let summary = outcome_summary(
            r#"{"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"conflict","diagnostics":"a FHIR mapping with that name exists"}]}"#,
        );
        assert_eq!(
            summary.as_deref(),
            Some("a FHIR mapping with that name exists")
        );
        // Several issues read as one sentence.
        assert_eq!(
            outcome_summary(INVALID_OUTCOME)
                .as_deref()
                .map(|s| s.contains("; ")),
            Some(true)
        );
        // An issue with no diagnostics still names its code rather than nothing.
        assert_eq!(
            outcome_summary(
                r#"{"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"invalid"}]}"#
            )
            .as_deref(),
            Some("invalid")
        );
        // The openEHR error body an auth refusal carries is not an outcome, so
        // the caller falls back to the shared reader.
        assert_eq!(
            outcome_summary(
                r#"{"error":"Forbidden","message":"forbidden: operation requires the 'ADMIN' role"}"#
            ),
            None
        );
        assert_eq!(outcome_summary("not json at all"), None);
    }

    #[test]
    fn a_draft_needs_an_addressable_name_and_a_json_object() {
        assert!(mapping_draft_complaint("bp", r#"{"resource_type":"Observation"}"#).is_ok());
        assert!(mapping_draft_complaint("bp.v1-2_x", "{}").is_ok());
        // The CDR's own name rule: non-empty, [A-Za-z0-9_.-].
        assert!(mapping_draft_complaint("  ", "{}").is_err());
        let spaced = mapping_draft_complaint("blood pressure", "{}").expect_err("space refused");
        assert!(spaced.contains("addressable"), "{spaced}");
        // The definition must be an object, not a scalar or an array.
        let scalar = mapping_draft_complaint("bp", "42").expect_err("scalar refused");
        assert!(scalar.contains("JSON object"), "{scalar}");
        let broken = mapping_draft_complaint("bp", "{ oops").expect_err("bad JSON refused");
        assert!(broken.contains("not valid JSON"), "{broken}");
        // The edit card sends no name, so it judges the document alone.
        assert!(definition_complaint(r#"{"resource_type":"Observation"}"#).is_ok());
        assert!(definition_complaint("   ").is_err());
    }

    #[test]
    fn the_read_facade_needs_both_halves_of_its_scope() {
        assert!(read_request_is_complete("Observation", "p-42"));
        assert!(!read_request_is_complete("", "p-42"));
        assert!(!read_request_is_complete("Observation", ""));
        // Whitespace is not a scope: the CDR refuses it with a 400.
        assert!(!read_request_is_complete("  ", "  "));
    }

    #[test]
    fn a_refused_write_names_the_mapping_the_diagnostic_and_the_next_action() {
        let conflict = mapping_failure_copy(
            "mapping `bp`",
            &ViewerError::Cdr {
                status: 409,
                message: "a FHIR mapping with that name exists".to_owned(),
            },
        );
        assert!(conflict.contains("mapping `bp`"), "{conflict}");
        assert!(
            conflict.contains("a FHIR mapping with that name exists"),
            "{conflict}"
        );
        assert!(conflict.contains("different name"), "{conflict}");

        let gone = mapping_failure_copy(
            "mapping `bp`",
            &ViewerError::Cdr {
                status: 404,
                message: "FHIR mapping 01a0".to_owned(),
            },
        );
        assert!(gone.contains("Reload this screen"), "{gone}");

        let refused = mapping_failure_copy(
            "mapping `bp`",
            &ViewerError::Forbidden("operation requires the 'ADMIN' role".to_owned()),
        );
        assert!(
            refused.contains("ADMIN-role") && refused.contains("requires the 'ADMIN' role"),
            "{refused}"
        );

        // Everything else keeps the shared write copy rather than inventing a
        // second vocabulary — an unknown template_id is the store's own 400.
        let rejected = ViewerError::Cdr {
            status: 400,
            message: "FHIR mapping references an unknown template_id (ingest the OPT first)"
                .to_owned(),
        };
        assert_eq!(
            mapping_failure_copy("mapping `bp`", &rejected),
            crate::feedback::write_failure_copy("mapping `bp`", &rejected)
        );
    }

    /// The store record shape, pinned against the answer the CDR actually
    /// serves.
    #[test]
    fn a_store_element_distils_into_a_row_and_a_null_profile_empties_its_cell() {
        let item = serde_json::json!({
            "id": "01a02ade-8384-74fd-bbc6-2265ae68d58e",
            "name": "bp",
            "resource_type": "Observation",
            "profile_url": "http://example.org/StructureDefinition/bp",
            "template_id": "minimal_evaluation.en.v1",
            "definition": { "resource_type": "Observation", "template_id": "minimal_evaluation.en.v1" },
            "enabled": true,
            "created_at": "2026-08-22T19:07:00.867924Z",
        });
        let row = mapping_row(&item);
        assert_eq!(row.id, "01a02ade-8384-74fd-bbc6-2265ae68d58e");
        assert_eq!(row.name, "bp");
        assert_eq!(row.resource_type, "Observation");
        assert_eq!(row.template_id, "minimal_evaluation.en.v1");
        assert!(row.enabled);
        assert_eq!(row.created_at, "2026-08-22T19:07:00.867924Z");
        // The definition reaches the editor pretty-printed, never as one line.
        assert!(
            row.definition.contains("\n  \"resource_type\""),
            "{}",
            row.definition
        );

        // A profile-less default mapping serves `null`, which empties the cell
        // rather than rendering the word "null".
        let default_mapping = serde_json::json!({
            "id": "01a0", "name": "obs", "resource_type": "Observation",
            "profile_url": serde_json::Value::Null,
            "template_id": "t.v1", "definition": {}, "enabled": false,
            "created_at": "2026-08-22T19:07:00.867924Z",
        });
        let row = mapping_row(&default_mapping);
        assert!(row.profile_url.is_empty());
        assert!(!row.enabled);
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn the_request_body_carries_the_definition_as_json_and_omits_an_immutable_name() {
        let created = super::mapping_body(Some("bp"), r#"{"resource_type":"Observation"}"#, true)
            .expect("a valid draft");
        assert_eq!(
            created,
            r#"{"name":"bp","definition":{"resource_type":"Observation"},"enabled":true}"#
        );
        // The update form sends no name — the CDR treats it as immutable.
        let updated =
            super::mapping_body(None, r#"{"resource_type":"Observation"}"#, false).expect("valid");
        assert_eq!(
            updated,
            r#"{"definition":{"resource_type":"Observation"},"enabled":false}"#
        );
        // A quote in the name is escaped by the encoder, never by hand.
        let quoted = super::mapping_body(Some("a\"b"), "{}", true).expect("valid");
        assert!(quoted.starts_with(r#"{"name":"a\"b""#), "{quoted}");
        assert!(super::mapping_body(Some("bp"), "{ oops", true).is_err());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn a_refusal_reads_the_fhir_vocabulary_and_falls_back_to_the_openehr_one() {
        // #2581: ONE reader for both vocabularies — the store's own
        // OperationOutcome refusals and the auth layer's openEHR bodies both
        // go through the shared `expect_success`.
        let outcome = crate::cdr::CdrResponse {
            status: http::StatusCode::CONFLICT,
            content_type: Some("application/fhir+json".to_owned()),
            etag: None,
            location: None,
            last_modified: None,
            preference_applied: None,
            body: r#"{"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"conflict","diagnostics":"a FHIR mapping with that name exists"}]}"#.to_owned(),
        };
        assert_eq!(
            crate::cdr::CdrClient::expect_success(outcome).unwrap_err(),
            ViewerError::Cdr {
                status: 409,
                message: "a FHIR mapping with that name exists".to_owned(),
            }
        );
        let refused = crate::cdr::CdrResponse {
            status: http::StatusCode::FORBIDDEN,
            content_type: Some("application/json".to_owned()),
            etag: None,
            location: None,
            last_modified: None,
            preference_applied: None,
            body: r#"{"error":"Forbidden","message":"forbidden: operation requires the 'ADMIN' role"}"#.to_owned(),
        };
        assert_eq!(
            crate::cdr::CdrClient::expect_success(refused).unwrap_err(),
            ViewerError::Forbidden("forbidden: operation requires the 'ADMIN' role".to_owned())
        );
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn an_operation_outcome_is_content_and_only_an_auth_refusal_is_an_error() {
        // A 400 the connector authored is a document the panel renders.
        let missing_scope = crate::cdr::CdrResponse {
            status: http::StatusCode::BAD_REQUEST,
            content_type: Some("application/fhir+json".to_owned()),
            etag: None,
            location: None,
            last_modified: None,
            preference_applied: None,
            body: r#"{"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"required","diagnostics":"the `patient` query parameter is required"}]}"#.to_owned(),
        };
        let answered = super::fhir_answer(&missing_scope).expect("a FHIR document, not an error");
        assert_eq!(answered.status, 400);
        assert!(!answered.completed());
        assert!(answered.is_outcome());
        // The body reaches the pane pretty-printed.
        assert!(answered.body.contains("\n  \"issue\""), "{}", answered.body);

        let refused = crate::cdr::CdrResponse {
            status: http::StatusCode::UNAUTHORIZED,
            content_type: Some("application/json".to_owned()),
            etag: None,
            location: None,
            last_modified: None,
            preference_applied: None,
            body: r#"{"error":"Unauthorized","message":"unauthorized: authentication failed"}"#
                .to_owned(),
        };
        // A 401 is the "credential no longer accepted" refusal, not the
        // wrong-role one — the panel's copy differs, so the variant does too.
        assert!(matches!(
            super::fhir_answer(&refused),
            Err(ViewerError::CdrUnauthorized(_))
        ));
    }
}
