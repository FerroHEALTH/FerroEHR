// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The REST adapter's runtime configuration view.
//!
//! The configuration *data* lives in the platform tree
//! (`ferroehr::config` — one TOML, one schema); this module holds only the
//! adapter-side assembly the router consumes.

use ferroehr::config::auth::AuthConfig;
use ferroehr::config::server::{AdminConfig, ServerConfig};
use ferroehr::config::smart::SmartConfig;

/// The REST adapter's runtime configuration view, assembled by the binary
/// from the root [`ferroehr::config::FerroEhrConfig`].
///
/// Not a serde root — it is built in code, so extension-group toggles are
/// plain `bool`s lifted from their owning sections (`[fhir].api_enabled`,
/// `[terminology].api_enabled`, `[events].admin_api`).
#[derive(Debug, Clone, Default)]
pub struct AppConfig {
    /// `[server]` — listener + REST surface + System-Options identity.
    pub server: ServerConfig,
    /// `[auth]` — authentication (Basic + OAuth2/OIDC).
    pub auth: AuthConfig,
    /// `[admin]` — the ADMIN API group toggle.
    pub admin: AdminConfig,
    /// `[smart]` — SMART App Launch resource-server posture.
    pub smart: SmartConfig,
    /// `[fhir].api_enabled` — mount the FHIR R4 inbound façade + admin mapping.
    pub fhir_api_enabled: bool,
    /// `[terminology].api_enabled` — mount the terminology extension API.
    pub terminology_api_enabled: bool,
    /// `[events].admin_api` — mount the `/admin/event_subscription` CRUD.
    pub events_admin_api: bool,
    /// `[authz.abac] organization_claim` — the access-token claim naming the
    /// organisation a caller acts for, lifted here because the access log
    /// records it whether or not the ABAC gate that shares the setting is
    /// switched on. `None` when the deployment configures no claim.
    pub audit_organization_claim: Option<String>,
    /// `spec_profile` — the openEHR specification generation set the server
    /// runs; the demographic ingress boundary reads it (the stable
    /// generation's released surface differs from the typed core's).
    pub spec_profile: ferroehr::config::profile::SpecProfile,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_defaults_are_sane() {
        let c = ServerConfig::default();
        assert_eq!(c.base_path, "/ferroehr/rest/openehr/v1");
        assert_eq!(c.max_in_flight, 256);
        assert_eq!(
            c.swagger_ui,
            ferroehr::config::management::AccessLevel::Private
        );
        assert!(!c.cors_permissive);
        assert_eq!(c.swagger_ui_path(), "/ferroehr/rest/swagger-ui");
        assert_eq!(
            c.openapi_json_path(),
            "/ferroehr/rest/api-docs/openapi.json"
        );
        // The System-Options identity carries the released ITS-REST contract
        // version (shared provenance).
        assert_eq!(
            c.identity.restapi_specs_version,
            ferroehr::telemetry::provenance::ITS_REST
        );
    }

    #[test]
    fn app_config_defaults_off() {
        let c = AppConfig::default();
        assert!(c.auth.enabled);
        assert!(!c.admin.enabled);
        assert!(!c.smart.enabled);
        assert!(!c.fhir_api_enabled);
        assert!(!c.terminology_api_enabled);
        assert!(!c.events_admin_api);
    }
}
