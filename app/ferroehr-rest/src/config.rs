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
    /// `[pix]` — the PMIR Patient Identity Registry whose ITI-93 messages the
    /// feed route receives; absent, the route is not mounted.
    pub pix:
        Option<std::sync::Arc<ferroehr::service::demographic::identity_feed::config::PixConfig>>,
    /// The clock an external caller's `caller_rate` is read against: the
    /// runtime's, unless the assembly holds another.
    pub rate_clock: RateClock,
}

/// The clock an agreement's caller rate reads each request's instant from.
///
/// The binary keeps the runtime's monotonic clock; a test that judges the
/// rate holds one still, so the refusal does not depend on how long the
/// admitted requests took. No specification governs it: our own design.
#[derive(Clone)]
pub struct RateClock(std::sync::Arc<dyn Fn() -> tokio::time::Instant + Send + Sync>);

impl RateClock {
    /// A clock reading `now` for each instant.
    #[must_use]
    pub fn new(now: impl Fn() -> tokio::time::Instant + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(now))
    }

    /// A clock that reads the instant it was made at, every time.
    #[must_use]
    pub fn still() -> Self {
        let at = tokio::time::Instant::now();
        Self::new(move || at)
    }

    /// The instant now, by this clock.
    #[must_use]
    pub fn now(&self) -> tokio::time::Instant {
        (self.0)()
    }
}

impl Default for RateClock {
    fn default() -> Self {
        Self::new(tokio::time::Instant::now)
    }
}

impl std::fmt::Debug for RateClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RateClock")
    }
}

impl AppConfig {
    /// Returns the base URLs `[auth]` is resolved for, the audience of an
    /// issuer that names none ([`ferroehr::config::auth::audience_bases`]).
    #[must_use]
    pub fn audience_bases(&self) -> Vec<String> {
        ferroehr::config::auth::audience_bases(self.public_root.as_ref(), &self.api.base_path)
    }
}

impl From<&ferroehr::config::FerroEhrConfig> for AppConfig {
    /// The REST adapter's runtime view of the platform configuration tree.
    fn from(config: &ferroehr::config::FerroEhrConfig) -> Self {
        Self {
            server: config.server.shared.clone(),
            public_root: config.server.public_root(),
            api: config.api.clone(),
            auth: config.auth.clone(),
            rbac: config.authz.rbac.clone(),
            assurance: config.authz.assurance.clone(),
            spec_profile: config.deployment.spec_profile,
            party_binding: config.authz.party_binding.clone(),
            pix: config.pix.clone().map(std::sync::Arc::new),
            rate_clock: RateClock::default(),
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

    #[tokio::test]
    async fn a_still_rate_clock_reads_one_instant_and_the_default_moves() {
        let still = RateClock::still();
        let first = still.now();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        assert_eq!(still.now(), first);
        let running = RateClock::default();
        let before = running.now();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        assert!(running.now() > before);
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
