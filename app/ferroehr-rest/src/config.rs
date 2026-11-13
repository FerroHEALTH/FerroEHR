// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The REST adapter's runtime configuration view.
//!
//! The configuration *data* lives in the platform tree
//! (`ferroehr::config` — one TOML, one schema); this module holds only the
//! adapter-side assembly the router consumes.

use ferroehr::config::auth_section::AuthConfig;
use ferroehr::config::authz::RbacConfig;
use ferroehr::config::rest::ApiConfig;
use ferroehr::config::server::PublicRoot;
use ferrohealth_server::section::ServerSection;
use ferrohealth_smart::assurance::AssuranceSection;

/// The REST adapter's runtime configuration view, assembled by the binary
/// from the root [`ferroehr::config::FerroEhrConfig`].
///
/// Not a serde root — it is built in code, holding the tables the router
/// reads under their names in the tree.
#[derive(Debug, Clone, Default)]
pub struct AppConfig {
    /// `[server]` — the listener and the stack around the routes: the
    /// family's section.
    pub server: ServerSection,
    /// The URL every route is reached at, `[server] public_base_url` with
    /// `server.base_path`: the SMART discovery document's `baseUrl`, the SCIM
    /// base URL and the base of a `DPoP` proof's `htu` hang off it. `None`
    /// when the deployment names no public URL.
    pub public_root: Option<PublicRoot>,
    /// `[api]` — the REST API, the System-Options identity and the ADMIN and
    /// terminology groups.
    pub api: ApiConfig,
    /// `[auth]` — authentication: the `ferrohealth-smart` section (issuers,
    /// Basic users, product scope), the vendor adapter tables, the SMART App
    /// Launch posture and the SCIM service provider.
    pub auth: AuthConfig,
    /// `[authz.abac] organization_claim` — the access-token claim naming the
    /// organisation a caller acts for, lifted here because the access log
    /// records it whether or not the ABAC gate that shares the setting is
    /// switched on. `None` when the deployment configures no claim.
    pub audit_organization_claim: Option<String>,
    /// `[authz.rbac]` — the role rules.
    pub rbac: RbacConfig,
    /// `[authz.assurance]` — the assurance a patient-data request must carry.
    pub assurance: AssuranceSection,
    /// `deployment.spec_profile` — the openEHR specification generation set
    /// the server runs; the demographic ingress boundary reads it (the stable
    /// generation's released surface differs from the typed core's).
    pub spec_profile: ferroehr::config::profile::SpecProfile,
    /// `[authz.party_binding]` — the namespace a bound party is named under
    /// and the key each Basic user is bound by.
    pub party_binding: ferroehr::config::binding::PartyBindingConfig,
    /// `[[authz.system_client]]` — the system clients that commit.
    pub system_clients: Vec<ferroehr::config::clients::SystemClient>,
    /// `[[authz.federation.agreement]]` — what each `federation` issuer's
    /// external callers may do.
    pub agreements: Vec<ferroehr::config::agreement::Agreement>,
}

impl From<&ferroehr::config::FerroEhrConfig> for AppConfig {
    /// The REST adapter's runtime view of the platform configuration tree.
    fn from(config: &ferroehr::config::FerroEhrConfig) -> Self {
        Self {
            server: config.server.shared.clone(),
            public_root: config.server.public_root(),
            api: config.api.clone(),
            auth: config.auth.clone(),
            audit_organization_claim: config.authz.abac.organization_claim().map(str::to_owned),
            rbac: config.authz.rbac.clone(),
            assurance: config.authz.assurance.clone(),
            spec_profile: config.deployment.spec_profile,
            party_binding: config.authz.party_binding.clone(),
            system_clients: config.authz.system_client.clone(),
            agreements: config.authz.federation.agreement.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_defaults_are_sane() {
        let c = ApiConfig::default();
        assert_eq!(c.base_path, "/ferroehr/rest/openehr/v1");
        assert_eq!(c.body_limit, None);
        assert_eq!(
            c.openapi_ui,
            ferroehr::config::management::AccessLevel::Private
        );
        assert!(c.cors.is_none());
        assert_eq!(c.swagger_ui_path(), "/ferroehr/rest/swagger-ui");
        assert_eq!(
            c.openapi_json_path(),
            "/ferroehr/rest/api-docs/openapi.json"
        );
        // The System-Options identity carries the released ITS-REST contract
        // version (shared provenance).
        assert_eq!(
            ferroehr::config::rest::SystemOptionsConfig::restapi_specs_version(),
            ferroehr::telemetry::provenance::ITS_REST
        );
    }

    #[test]
    fn app_config_defaults_off() {
        let c = AppConfig::default();
        assert!(c.auth.shared.enabled);
        assert!(!c.api.admin);
        assert!(!c.auth.smart.discovery);
        assert!(c.auth.scim.is_none());
        assert!(!c.api.terminology);
    }
}
