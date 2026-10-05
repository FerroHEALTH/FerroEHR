// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`TerminologyRouter`] — every configured terminology server, materialised,
//! with the terminology → provider routing that picks one per call.
//!
//! `BASE/docs/architecture_overview/master12-terminology.adoc` §Overview names
//! the ecosystem archetypes bind to (LOINC, `ICDx`, ICPC, SNOMED CT "and the
//! many other terminologies and vocabularies used in healthcare") and
//! §"Binding Terminology Value-sets to Archetypes" binds an ac-code "to queries
//! to one or more external terminologies", so the CDR holds every configured
//! server open at once rather than picking one at boot.
//!
//! # The routing rule
//!
//! NOTE: no openEHR spec governs how a terminology is mapped to a server — our
//! own design/extension, kept mechanical so a deployment can predict which
//! server answers a given call:
//!
//! 1. the caller offers one or more **candidate keys** in priority order (a
//!    terminology id, a system URI, a value-set URL, the AQL `service_api`);
//! 2. the first candidate with an entry in
//!    [`ExternalTerminologyConfig::routes`] (matched case-insensitively, whole
//!    string — never a prefix) selects that provider;
//! 3. otherwise the **default** provider answers: the one named `default`, or
//!    the sole configured provider when exactly one exists;
//! 4. with no default and no matching route, the call has no remote provider
//!    and falls through to the caller's local behaviour.
//!
//! A route naming an unconfigured provider is a boot error
//! ([`crate::config::FerroEhrConfig::validate`]), so step 2 never silently
//! degrades to step 3.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::service::status::SmError;

use super::config::{DEFAULT_PROVIDER_NAME, ExternalTerminologyConfig};
use super::fhir::FhirTerminologyProvider;

/// Every materialised terminology-server provider plus the routing that
/// selects among them.
#[derive(Debug)]
pub struct TerminologyRouter {
    /// Materialised providers, keyed by their configured name.
    providers: BTreeMap<String, Arc<FhirTerminologyProvider>>,
    /// Routing keys (lower-cased terminology id / system URI) → provider name.
    routes: BTreeMap<String, String>,
    /// The provider answering an unrouted call, when one can be chosen.
    default: Option<Arc<FhirTerminologyProvider>>,
    /// Whether a terminology-server error rejects the composition
    /// (fail-closed) or is tolerated (fail-open) —
    /// [`ExternalTerminologyConfig::fail_on_error`].
    fail_on_error: bool,
}

impl TerminologyRouter {
    /// Materialise every configured provider and its routing.
    ///
    /// `Ok(None)` when external terminology is disabled — the byte-identical
    /// bundle-only default (enabled-with-no-provider is a boot error, so it
    /// never reaches here).
    ///
    /// # Errors
    ///
    /// [`SmError`] when this binary was built without the `fhir` cargo
    /// feature (it cannot decode a terminology server's FHIR responses, so a
    /// configured provider is refused loudly rather than silently ignored —
    /// the in-process bundle remains the terminology), or from the first
    /// provider whose configuration is invalid (an empty URL, an un-buildable
    /// HTTP client, a missing or invalid `oauth2_client`). A provider that
    /// cannot be built is a boot failure, never a silently-absent route.
    pub fn build(cfg: &ExternalTerminologyConfig) -> Result<Option<Self>, SmError> {
        if !cfg.enabled || cfg.providers.is_empty() {
            return Ok(None);
        }
        #[cfg(not(feature = "fhir"))]
        return Err(SmError::exception(
            "terminology.external is enabled with configured providers, but this binary was \
             built without the `fhir` cargo feature (external FHIR terminology servers are \
             unavailable; the in-process bundle remains)",
        ));
        #[cfg(feature = "fhir")]
        Self::build_providers(cfg).map(Some)
    }

    /// Materialise the configured providers and their routing.
    #[cfg(feature = "fhir")]
    fn build_providers(cfg: &ExternalTerminologyConfig) -> Result<Self, SmError> {
        let mut providers = BTreeMap::new();
        for (name, provider_cfg) in &cfg.providers {
            let provider = cfg.build_provider(name, provider_cfg)?;
            providers.insert(name.clone(), Arc::new(provider));
        }
        let default = providers
            .get(DEFAULT_PROVIDER_NAME)
            .or_else(|| {
                // Exactly one configured provider → it is unambiguously the
                // default; two or more without a `default` entry means every
                // call must be routed explicitly.
                let mut it = providers.values();
                match (it.next(), it.next()) {
                    (Some(only), None) => Some(only),
                    _ => None,
                }
            })
            .map(Arc::clone);
        let routes = cfg
            .routes
            .iter()
            .map(|(key, provider)| (key.trim().to_ascii_lowercase(), provider.clone()))
            .collect();
        Ok(Self {
            providers,
            routes,
            default,
            fail_on_error: cfg.fail_on_error,
        })
    }

    /// A router over one already-built provider, registered under the
    /// `default` name so every unrouted call reaches it.
    #[must_use]
    pub fn single(provider: Arc<FhirTerminologyProvider>) -> Self {
        Self {
            providers: BTreeMap::from([(DEFAULT_PROVIDER_NAME.to_owned(), Arc::clone(&provider))]),
            routes: BTreeMap::new(),
            default: Some(provider),
            fail_on_error: false,
        }
    }

    /// Whether a terminology-server error rejects the composition
    /// (`[terminology.external] fail_on_error`).
    #[must_use]
    pub fn fail_on_error(&self) -> bool {
        self.fail_on_error
    }

    /// The provider `key` is **explicitly routed** to, or `None` when the
    /// routing map does not name it (routing rule step 2 alone — no default
    /// fallback). A caller with several candidate keys chains this and ends
    /// with [`Self::default_provider`].
    ///
    /// Every handle is an owned [`Arc`] clone rather than a borrow: callers
    /// await a remote call on it, and a borrow of the router held across that
    /// await would make the resulting future non-`Send`. For the same reason
    /// the candidate keys are passed one at a time rather than as a slice — a
    /// `&[&str]` in an async body carries two extra lifetimes that defeat the
    /// compiler's auto-trait inference.
    #[must_use]
    pub fn route(&self, key: &str) -> Option<Arc<FhirTerminologyProvider>> {
        let key = key.trim();
        if key.is_empty() {
            return None;
        }
        self.routes
            .get(&key.to_ascii_lowercase())
            .and_then(|name| self.providers.get(name))
            .map(Arc::clone)
    }

    /// The provider answering an unrouted call (routing rule step 3): the one
    /// named `default`, or the sole configured provider.
    #[must_use]
    pub fn default_provider(&self) -> Option<Arc<FhirTerminologyProvider>> {
        self.default.clone()
    }

    /// The provider serving `terminology` — its explicit route, else the
    /// default (routing rule steps 2–3).
    #[must_use]
    pub fn provider_for(&self, terminology: &str) -> Option<Arc<FhirTerminologyProvider>> {
        self.route(terminology).or_else(|| self.default_provider())
    }

    /// The provider configured under `name`, if any.
    #[must_use]
    pub fn named(&self, name: &str) -> Option<Arc<FhirTerminologyProvider>> {
        self.providers.get(name).map(Arc::clone)
    }

    /// The configured provider names, in stable (sorted) order — boot logging
    /// and operational introspection.
    pub fn provider_names(&self) -> impl Iterator<Item = &str> {
        self.providers.keys().map(String::as_str)
    }

    /// How many providers are materialised.
    #[must_use]
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Whether no provider is materialised (never true for a built router —
    /// [`Self::build`] answers `None` instead).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::terminology::config::test_provider_config;

    fn config(providers: &[(&str, &str)], routes: &[(&str, &str)]) -> ExternalTerminologyConfig {
        ExternalTerminologyConfig {
            enabled: true,
            providers: providers
                .iter()
                .map(|(name, url)| ((*name).to_owned(), test_provider_config(url)))
                .collect(),
            routes: routes
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            ..ExternalTerminologyConfig::default()
        }
    }

    #[test]
    fn disabled_or_empty_builds_no_router() {
        let disabled = ExternalTerminologyConfig::default();
        assert!(
            TerminologyRouter::build(&disabled)
                .expect("build")
                .is_none()
        );
        let enabled_but_empty = ExternalTerminologyConfig {
            enabled: true,
            ..ExternalTerminologyConfig::default()
        };
        assert!(
            TerminologyRouter::build(&enabled_but_empty)
                .expect("build")
                .is_none()
        );
    }

    /// Every configured provider is materialised — not just `default`
    /// (BASE master12 §Overview: several terminologies at the same time).
    #[test]
    fn every_provider_is_materialised_and_routed() {
        let cfg = config(
            &[
                ("default", "http://default.example/fhir"),
                ("snomed", "http://snomed.example/fhir"),
                ("loinc", "http://loinc.example/fhir"),
            ],
            &[
                ("SNOMED-CT", "snomed"),
                ("http://snomed.info/sct", "snomed"),
                ("http://loinc.org", "loinc"),
            ],
        );
        let router = TerminologyRouter::build(&cfg)
            .expect("build")
            .expect("router");
        assert_eq!(router.len(), 3);
        assert_eq!(
            router.provider_names().collect::<Vec<_>>(),
            ["default", "loinc", "snomed"]
        );
        let named = |key: &str| router.provider_for(key).map(|p| p.name().to_owned());
        assert_eq!(named("SNOMED-CT").as_deref(), Some("snomed"));
        // Route keys match case-insensitively.
        assert_eq!(named("snomed-ct").as_deref(), Some("snomed"));
        assert_eq!(named("http://snomed.info/sct").as_deref(), Some("snomed"));
        assert_eq!(named("http://loinc.org").as_deref(), Some("loinc"));
        // Unrouted terminologies fall back to `default`.
        assert_eq!(named("ICD-10").as_deref(), Some("default"));
        assert_eq!(named("").as_deref(), Some("default"));
    }

    /// Candidate keys are tried in order; the first explicitly routed one
    /// wins, and a fully unrouted candidate list falls back to the default.
    #[test]
    fn candidate_keys_are_tried_in_order() {
        let cfg = config(
            &[
                ("default", "http://default.example/fhir"),
                ("snomed", "http://snomed.example/fhir"),
            ],
            &[("http://snomed.info/sct", "snomed")],
        );
        let router = TerminologyRouter::build(&cfg)
            .expect("build")
            .expect("router");
        let named = |keys: [&str; 2]| {
            keys.iter()
                .find_map(|k| router.route(k))
                .or_else(|| router.default_provider())
                .map(|p| p.name().to_owned())
        };
        assert_eq!(
            named(["hl7.org/fhir/4.0", "http://snomed.info/sct"]).as_deref(),
            Some("snomed")
        );
        assert_eq!(
            named(["hl7.org/fhir/4.0", "unknown"]).as_deref(),
            Some("default")
        );
    }

    /// With exactly one provider and no `default` entry, that provider IS the
    /// default; with two and no `default`, an unrouted call has no provider.
    #[test]
    fn default_selection() {
        let sole = config(&[("snomed", "http://snomed.example/fhir")], &[]);
        let router = TerminologyRouter::build(&sole)
            .expect("build")
            .expect("router");
        assert_eq!(
            router.provider_for("anything").map(|p| p.name().to_owned()),
            Some("snomed".to_owned())
        );

        let ambiguous = config(
            &[
                ("snomed", "http://snomed.example/fhir"),
                ("loinc", "http://loinc.example/fhir"),
            ],
            &[("http://loinc.org", "loinc")],
        );
        let router = TerminologyRouter::build(&ambiguous)
            .expect("build")
            .expect("router");
        assert!(router.provider_for("unrouted").is_none());
        assert_eq!(
            router
                .provider_for("http://loinc.org")
                .map(|p| p.name().to_owned()),
            Some("loinc".to_owned())
        );
    }

    /// A provider whose configuration is invalid fails the whole build — an
    /// unbuildable terminology server never becomes a silently absent route.
    #[test]
    fn an_invalid_provider_fails_the_build() {
        let cfg = config(
            &[
                ("default", "http://default.example/fhir"),
                ("broken", "   "),
            ],
            &[],
        );
        assert!(TerminologyRouter::build(&cfg).is_err());
    }

    #[test]
    fn fail_on_error_is_carried() {
        let mut cfg = config(&[("default", "http://default.example/fhir")], &[]);
        cfg.fail_on_error = true;
        let router = TerminologyRouter::build(&cfg)
            .expect("build")
            .expect("router");
        assert!(router.fail_on_error());
    }
}
