// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The REST adapter's runtime configuration view.
//!
//! The configuration *data* lives in the platform tree
//! (`ferroehr::config` — one TOML, one schema); this module holds only the
//! adapter-side assembly the router consumes.

use ferroehr::config::auth::{AssuranceConfig, ProfessionalConfig};
use ferroehr::config::authz::RbacConfig;
use ferroehr::config::server::{AdminConfig, ServerConfig};
use ferroehr::config::smart::SmartConfig;
use ferrohealth_smart::config::AuthSection;

/// The REST adapter's runtime configuration view, assembled by the binary
/// from the root [`ferroehr::config::FerroEhrConfig`].
///
/// Not a serde root — it is built in code, so extension-group toggles are
/// plain `bool`s lifted from their owning sections
/// (`[terminology].api_enabled`, `[events].admin_api`).
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// `[server]` — listener + REST surface + System-Options identity.
    pub server: ServerConfig,
    /// `[auth]` — authentication: the `ferrohealth-smart` section (issuers,
    /// Basic users, product scope).
    pub auth: AuthSection,
    /// `[admin]` — the ADMIN API group toggle.
    pub admin: AdminConfig,
    /// `[smart]` — SMART App Launch resource-server posture.
    pub smart: SmartConfig,
    /// `[terminology].api_enabled` — mount the terminology extension API.
    pub terminology_api_enabled: bool,
    /// `[events].admin_api` — mount the `/admin/event_subscription` CRUD.
    pub events_admin_api: bool,
    /// `[authz.abac] organization_claim` — the access-token claim naming the
    /// organisation a caller acts for, lifted here because the access log
    /// records it whether or not the ABAC gate that shares the setting is
    /// switched on. `None` when the deployment configures no claim.
    pub audit_organization_claim: Option<String>,
    /// `[authz.rbac] role_claims` — the access-token claim paths Bearer role
    /// extraction reads, lifted here because the authenticator applies them
    /// whether or not any authorization gate is switched on.
    pub role_claims: Vec<String>,
    /// `[authz.rbac]` — the role rules a Basic user's roles are read from
    /// (`basic_roles`, `user_role`).
    pub rbac: RbacConfig,
    /// `[authz] basic_cache_ttl_seconds` — how long a verified Basic
    /// credential is reused before its hash is checked again.
    pub basic_cache_ttl_seconds: u64,
    /// `[authz.assurance]` — the assurance a patient-data request must carry.
    pub assurance: AssuranceConfig,
    /// `[authz.professional]` — how a token names the natural person behind it.
    pub professional: Option<ProfessionalConfig>,
    /// `spec_profile` — the openEHR specification generation set the server
    /// runs; the demographic ingress boundary reads it (the stable
    /// generation's released surface differs from the typed core's).
    pub spec_profile: ferroehr::config::profile::SpecProfile,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            auth: AuthSection::default(),
            admin: AdminConfig::default(),
            smart: SmartConfig::default(),
            terminology_api_enabled: false,
            events_admin_api: false,
            audit_organization_claim: None,
            role_claims: RbacConfig::default().role_claims,
            rbac: RbacConfig::default(),
            basic_cache_ttl_seconds: 60,
            assurance: AssuranceConfig::default(),
            professional: None,
            spec_profile: ferroehr::config::profile::SpecProfile::default(),
        }
    }
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
        assert!(!c.terminology_api_enabled);
        assert!(!c.events_admin_api);
        assert_eq!(c.role_claims, RbacConfig::default().role_claims);
    }
}
