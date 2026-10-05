// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CDR **Admin API** surface the viewer consumes.
//!
//! The availability probe every admin affordance is gated on, plus the three
//! destructive operations (template delete, stored-query delete, physical EHR
//! delete).
//!
//! The CDR's admin group is **off by default**, and while it is disabled every
//! admin route answers `405 Method Not Allowed` with an empty `Allow` (the
//! route exists but supports no method — ITS-REST overview
//! `Requests_and_responses.md` §HTTP Methods: "If a method is recognized but not
//! allowed for the target resource, the response SHOULD be 405 Method Not
//! Allowed"). The viewer therefore discovers the group before offering any of
//! it, and renders no destructive affordance at all when it is not mounted.
//!
//! NOTE: admin availability is discovered via the System API conformance
//! manifest (`OPTIONS {base_path}` → `endpoints[]`,
//! `docs/specs/openehr/ITS-REST/specifications/system.openapi.yaml`) — the
//! spec's own capability-discovery operation, so the viewer never pokes a
//! destructive group to learn whether it exists.
//!
//! **Capability is not authorization.** The manifest says the admin group is
//! MOUNTED; whether THIS session may use it is answered per request by the CDR's
//! RBAC (`401`/`403`), and a refusal on click surfaces through
//! [`delete_failure_copy`].
//!
//! NOTE: no openEHR spec governs the viewer, and none governs the template or
//! stored-query deletes it drives either — our own design. The EHR delete is the
//! spec-grounded one: SM `I_ADMIN_SERVICE.physical_ehr_delete`
//! (`docs/specs/openehr/SM/docs/UML/classes/i_admin_service.adoc`
//! §`physical_ehr_delete`), whose `ehr_id_does_not_exist` error is the `404`.
//!
//! Every `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first (a server fn is
//! a public HTTP endpoint) and keeps the CDR credential server-side.

use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;
use crate::system_api::ConformanceManifest;

/// The API group name the conformance manifest advertises for the Admin API
/// (`Options.yaml` `example`: `/ehr`, `/demographic`, `/definition`, `/query`,
/// `/admin`).
const ADMIN_ENDPOINT: &str = "/admin";

/// Whether the CDR advertises its Admin API as mounted.
///
/// Carries only fixed-size, client-safe data — it crosses the server-fn
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdminAvailability {
    /// The conformance manifest lists `/admin`: the group is mounted, so the
    /// destructive affordances render (authorization is still per request).
    Available,
    /// The manifest does not list `/admin` — the CDR runs with the admin group
    /// disabled and every admin route answers `405` with an empty `Allow`
    /// (overview `Requests_and_responses.md` §HTTP Methods).
    Disabled,
}

impl AdminAvailability {
    /// Whether destructive admin affordances may be rendered.
    #[must_use]
    pub fn usable(self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Classify a conformance manifest: is the Admin API mounted?
#[must_use]
pub fn availability_of(manifest: &ConformanceManifest) -> AdminAvailability {
    if manifest.advertises(ADMIN_ENDPOINT) {
        AdminAvailability::Available
    } else {
        AdminAvailability::Disabled
    }
}

/// The single predicate every admin-gated view uses: destructive affordances
/// render ONLY for a probe that succeeded and found the group mounted.
///
/// A failed probe (CDR unreachable, unreadable manifest, expired session)
/// hides them — never render a button that cannot work.
#[must_use]
pub fn renders_admin_ops(probe: &Result<AdminAvailability, ViewerError>) -> bool {
    probe.as_ref().copied().is_ok_and(AdminAvailability::usable)
}

/// Probe the CDR for its Admin API by reading the System API conformance
/// manifest (see this module's NOTE for the spec citation).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnreachable`] on transport failure;
/// [`ViewerError::Cdr`] / [`ViewerError::Internal`] when the manifest cannot
/// be read.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn probe_admin_api() -> Result<AdminAvailability, ViewerError> {
    let manifest = crate::system_api::fetch_conformance_manifest().await?;
    Ok(availability_of(&manifest))
}

/// The per-screen admin gate: one probe [`Resource`].
///
/// Created in component setup — never inside a `Suspend` closure, which
/// re-runs and would re-create the resource.
#[must_use]
pub fn admin_gate() -> Resource<Result<AdminAvailability, ViewerError>> {
    Resource::new(|| (), |()| async move { probe_admin_api().await })
}

/// Render `affordance` only when the gate found the Admin API mounted;
/// otherwise render nothing at all (discover-and-hide).
///
/// The probe is resolved INSIDE the `<Suspense>`: an SSR'd `ErrorBoundary`
/// fallback mismatches at hydration in leptos 0.8, and a render-time resource
/// read is itself a hydration mismatch. `affordance` creates no resources, so
/// re-runs are safe, and it is shared through an `Arc` because the `Suspend`
/// closure must not consume its environment.
#[must_use]
pub fn when_admin_usable(
    gate: Resource<Result<AdminAvailability, ViewerError>>,
    affordance: impl Fn() -> AnyView + Send + Sync + 'static,
) -> AnyView {
    let affordance: std::sync::Arc<dyn Fn() -> AnyView + Send + Sync> =
        std::sync::Arc::new(affordance);
    view! {
        <Suspense fallback=|| ()>
            {move || {
                let affordance = std::sync::Arc::clone(&affordance);
                Suspend::new(async move {
                    if renders_admin_ops(&gate.await) { affordance() } else { ().into_any() }
                })
            }}
        </Suspense>
    }
    .into_any()
}

/// Actionable copy for a refused admin delete.
///
/// It names the exact object, the reason, and the next action, and carries the
/// CDR's own diagnostic verbatim (the CDR's `409` names the
/// referencing-version count).
#[must_use]
pub fn delete_failure_copy(object: &str, error: &ViewerError) -> String {
    match error {
        ViewerError::Cdr { message, .. } => match error.status_code() {
            Some(http::StatusCode::CONFLICT) => format!(
                "{object} is still referenced by committed data, so the CDR refused the delete \
                 ({message}). Delete the versions that reference it first, then retry."
            ),
            Some(http::StatusCode::NOT_FOUND) => format!(
                "{object} is not in the CDR any more — it was already deleted. Reload this screen."
            ),
            Some(http::StatusCode::BAD_REQUEST | http::StatusCode::UNPROCESSABLE_ENTITY) => {
                format!("The CDR rejected the delete of {object}: {message}.")
            }
            _ => format!("Deleting {object} failed: {error}"),
        },
        ViewerError::Forbidden(message) => format!(
            "This session may not delete {object} ({message}). Sign in with an ADMIN-role \
             account and retry."
        ),
        ViewerError::CdrUnauthorized(message) => format!(
            "The CDR no longer accepts this session, so {object} was not deleted ({message}). \
             Sign in again and retry."
        ),
        ViewerError::Unauthenticated => format!(
            "The viewer session expired before {object} was deleted — sign in again and retry."
        ),
        other => format!("Deleting {object} failed: {other}"),
    }
}

/// Physically delete an operational template
/// (`DELETE admin/template/{template_id}`).
///
/// The id is a CDR-supplied string, so the path segment is percent-encoded with
/// the `urlencoding` crate — a template id containing a `/`, `#`, `?` or `%`
/// would otherwise address a different resource. A template still referenced by
/// a committed version is refused `409`; the diagnostic surfaces through
/// [`delete_failure_copy`].
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for an empty id;
/// [`ViewerError::Cdr`] / [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn admin_delete_template(
    /// The `template_id` of the OPT to delete.
    template_id: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    if template_id.trim().is_empty() {
        return Err(ViewerError::Invalid("no template id to delete".to_owned()));
    }
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "admin/template/{}",
        urlencoding::encode(&template_id)
    ));
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    crate::cdr::CdrClient::expect_success(response)?;
    Ok(())
}

/// Delete ONE version of a stored query from the CDR
/// (`DELETE admin/query/{name}/{version}`) — the CDR's own store, so every
/// client loses that version.
///
/// Both segments are percent-encoded (the qualified name carries `::`).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for an empty name or version;
/// [`ViewerError::Cdr`] / [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn admin_delete_stored_query(
    /// The qualified stored-query name (`[{namespace}::]{query-name}`).
    name: String,
    /// The concrete `major.minor.patch` version to delete.
    version: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    if name.trim().is_empty() || version.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "a stored-query delete needs both a qualified name and a version".to_owned(),
        ));
    }
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "admin/query/{}/{}",
        urlencoding::encode(&name),
        urlencoding::encode(&version)
    ));
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    crate::cdr::CdrClient::expect_success(response)?;
    Ok(())
}

/// Physically delete an EHR and everything under it
/// (`DELETE admin/ehr/{ehr_id}`) — SM `I_ADMIN_SERVICE.physical_ehr_delete`
/// (`docs/specs/openehr/SM/docs/UML/classes/i_admin_service.adoc`): the
/// precondition is `has_ehr`, and `ehr_id_does_not_exist` is the CDR's `404`.
/// This is NOT the openEHR logical delete — the versions, contributions and
/// audit trail are gone.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for an empty id;
/// [`ViewerError::Cdr`] / [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn admin_delete_ehr(
    /// The EHR to delete, with everything committed under it.
    ehr_id: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    if ehr_id.trim().is_empty() {
        return Err(ViewerError::Invalid("no EHR id to delete".to_owned()));
    }
    let state: crate::state::AppState = expect_context();
    let url = state
        .cdr
        .rest_v1(&format!("admin/ehr/{}", urlencoding::encode(&ehr_id)));
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    crate::cdr::CdrClient::expect_success(response)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AdminAvailability, availability_of, delete_failure_copy, renders_admin_ops};
    use crate::error::ViewerError;
    use crate::system_api::ConformanceManifest;

    /// A manifest advertising exactly `endpoints`.
    fn manifest(endpoints: &[&str]) -> ConformanceManifest {
        ConformanceManifest {
            endpoints: endpoints.iter().map(|e| (*e).to_owned()).collect(),
            ..ConformanceManifest::default()
        }
    }

    #[test]
    fn availability_follows_the_manifests_mounted_group_set() {
        // `/admin` advertised = the group is mounted.
        assert_eq!(
            availability_of(&manifest(&["/ehr", "/query", "/admin"])),
            AdminAvailability::Available
        );
        // The CDR omits it while `admin.enabled` is false.
        assert_eq!(
            availability_of(&manifest(&[
                "/ehr",
                "/definition",
                "/query",
                "/demographic"
            ])),
            AdminAvailability::Disabled
        );
        assert_eq!(availability_of(&manifest(&[])), AdminAvailability::Disabled);
    }

    #[test]
    fn only_an_available_admin_api_renders_destructive_affordances() {
        // The hidden-when-off contract, asserted on the one predicate every
        // gated view calls: with the admin group absent from the manifest (the
        // CDR runs `admin.enabled = false`) no delete button is rendered at all
        // — nor when the probe itself failed.
        assert!(renders_admin_ops(&Ok(AdminAvailability::Available)));
        assert!(!renders_admin_ops(&Ok(AdminAvailability::Disabled)));
        assert!(!renders_admin_ops(&Err(ViewerError::Unauthenticated)));
        assert!(!renders_admin_ops(&Err(ViewerError::CdrUnreachable(
            "connection refused".to_owned()
        ))));
        assert!(!renders_admin_ops(&Err(ViewerError::Internal(
            "conformance manifest JSON: expected value".to_owned()
        ))));
        assert!(AdminAvailability::Available.usable());
        assert!(!AdminAvailability::Disabled.usable());
    }

    #[test]
    fn in_use_refusal_names_the_object_the_reason_and_the_next_action() {
        let copy = delete_failure_copy(
            "template `vital_signs.v2`",
            &ViewerError::Cdr {
                status: 409,
                message: "3 versions reference this template".to_owned(),
            },
        );
        assert!(copy.contains("template `vital_signs.v2`"), "{copy}");
        assert!(
            copy.contains("3 versions reference this template"),
            "{copy}"
        );
        assert!(copy.contains("then retry"), "{copy}");
    }

    #[test]
    fn other_failures_name_the_object_and_their_own_next_action() {
        let gone = delete_failure_copy(
            "EHR 7d44",
            &ViewerError::Cdr {
                status: 404,
                message: "HTTP 404".to_owned(),
            },
        );
        assert!(
            gone.contains("EHR 7d44") && gone.contains("Reload this screen"),
            "{gone}"
        );

        let refused = delete_failure_copy(
            "query `org.example::recent`",
            &ViewerError::Forbidden("insufficient role".to_owned()),
        );
        assert!(
            refused.contains("query `org.example::recent`")
                && refused.contains("ADMIN-role")
                && refused.contains("insufficient role"),
            "{refused}"
        );

        let expired = delete_failure_copy("EHR 7d44", &ViewerError::Unauthenticated);
        assert!(expired.contains("sign in again"), "{expired}");

        let other = delete_failure_copy(
            "template `x`",
            &ViewerError::Internal("serialization".to_owned()),
        );
        assert!(
            other.contains("template `x`") && other.contains("serialization"),
            "{other}"
        );
    }
}
