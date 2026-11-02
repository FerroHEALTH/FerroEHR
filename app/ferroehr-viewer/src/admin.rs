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
//! RBAC (`401`/`403`), and a refusal surfaces on the page through
//! [`refused`](crate::pages::common::refused).
//!
//! NOTE: no openEHR spec governs the viewer, and none governs the template or
//! stored-query deletes it drives either — our own design. The EHR delete is the
//! spec-grounded one: ITS-REST `operations/admin_ehr_delete.yaml`, whose unknown
//! EHR is the `404`.
//!
//! Every read guards with
//! [`require_session`](crate::session::require_session) first and keeps the
//! CDR credential server-side.

use crate::error::ViewerError;
use crate::locale::server::ServerMsg;
use crate::locale::tr_served;
use crate::system_api::ConformanceManifest;

/// The API group name the conformance manifest advertises for the Admin API
/// (`Options.yaml` `example`: `/ehr`, `/demographic`, `/definition`, `/query`,
/// `/admin`).
const ADMIN_ENDPOINT: &str = "/admin";

/// Whether the CDR advertises its Admin API as mounted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Cdr`] /
/// [`ViewerError::Internal`] when the manifest cannot be read.
pub async fn probe_admin_api() -> Result<AdminAvailability, ViewerError> {
    let manifest = crate::system_api::fetch_conformance_manifest().await?;
    Ok(availability_of(&manifest))
}

/// Physically delete an operational template
/// (`DELETE admin/template/{template_id}`).
///
/// The id is a CDR-supplied string, so the path segment is percent-encoded with
/// the `urlencoding` crate — a template id containing a `/`, `#`, `?` or `%`
/// would otherwise address a different resource. A template still referenced by
/// a committed version is refused `409`, the CDR's diagnostic naming why.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] for an empty id; [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
pub async fn admin_delete_template(
    // The `template_id` of the OPT to delete.
    template_id: String,
) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    if template_id.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            ServerMsg::NoTemplateIdDelete,
        )));
    }
    let url = cdr.extensions_v1(&format!(
        "admin/template/{}",
        urlencoding::encode(&template_id)
    ));
    cdr.delete(&url).ok().await?;
    Ok(())
}

/// Delete ONE version of a stored query from the CDR
/// (`DELETE admin/query/{name}/{version}`) — the CDR's own store, so every
/// client loses that version.
///
/// Both segments are percent-encoded (the qualified name carries `::`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] for an empty name or version; [`ViewerError::Cdr`]
/// / [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
pub async fn admin_delete_stored_query(
    // The qualified stored-query name (`[{namespace}::]{query-name}`).
    name: String,
    // The concrete `major.minor.patch` version to delete.
    version: String,
) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    if name.trim().is_empty() || version.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            ServerMsg::StoredQueryDeleteNeedsBothQualified,
        )));
    }
    let url = cdr.extensions_v1(&format!(
        "admin/query/{}/{}",
        urlencoding::encode(&name),
        urlencoding::encode(&version)
    ));
    cdr.delete(&url).ok().await?;
    Ok(())
}

/// Physically delete an EHR and everything under it.
///
/// `DELETE admin/ehr/{ehr_id}` (ITS-REST `operations/admin_ehr_delete.yaml`):
/// an unknown EHR is the CDR's `404`.
///
/// This is NOT the openEHR logical delete — the versions, contributions and
/// audit trail are gone.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] for an empty id; [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
pub async fn admin_delete_ehr(
    // The EHR to delete, with everything committed under it.
    ehr_id: String,
) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    if ehr_id.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(ServerMsg::NoEhrIdDelete)));
    }
    let url = cdr.rest_v1(&format!("admin/ehr/{}", urlencoding::encode(&ehr_id)));
    cdr.delete(&url).ok().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AdminAvailability, availability_of, renders_admin_ops};
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
}
