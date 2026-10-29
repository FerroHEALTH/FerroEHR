// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[ehr_index]` section: which provider holds the subject-to-EHR
//! cross-reference.
//!
//! An IHE patient identity source asked over PIXm (ITI-83) is the default, and
//! it is named by its endpoint and the assigning authority of the clinical
//! domain, never by a product. The in-instance map, `provider = "local"`, is
//! the other supported posture, the subject-id and subject-namespace model.
//! With neither provider configured, the server refuses to start and names
//! both choices. `provider` alone switches between them: the keys of the
//! provider not chosen are kept and not used.
//!
//! NOTE: no openEHR spec governs where the cross-reference lives (RM common
//! master04 §`PARTY_SELF`: "may be done elsewhere for security reasons"): our own design.

use serde::{Deserialize, Serialize};

use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_server::tls_client::TlsClientSection;

/// The provider that holds the subject-to-EHR cross-reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// An IHE patient identity source asked over PIXm (ITI-83), the default.
    #[default]
    Ihe,
    /// The in-instance `linkage` map: the subject-id and subject-namespace
    /// model, which `EHR_STATUS.subject` reads directly.
    Local,
}

/// How the PIX Manager is asked (FHIR R4 operations.html §3.2.0.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PixmInvocation {
    /// `POST [base]/Patient/$ihe-pix` with a `Parameters` body, the default: the
    /// patient identifier stays out of every URL and every access log that
    /// records one.
    #[default]
    Post,
    /// `GET [base]/Patient/$ihe-pix?sourceIdentifier=…`, the form ITI-83 prescribes
    /// (§2:3.83.4.1.2), for a PIX Manager that accepts no `POST`.
    Get,
}

/// The `[ehr_index]` section.
///
/// The defaults name the IHE provider with no endpoint, which the boot check
/// refuses until the endpoint is configured or `provider = "local"` is chosen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EhrIndexConfig {
    /// Which provider holds the cross-reference.
    pub provider: ProviderKind,
    /// The FHIR base URL of the PIX Manager (`[base]` of `[base]/Patient/$ihe-pix`,
    /// PIXm §2:3.83.4.1.2). Any `userinfo` in it is masked in every rendering.
    pub pixm_url: SecretUrl,
    /// The assigning authority of the clinical domain, an absolute URI such as
    /// `urn:oid:2.999.1`: the `targetSystem` of every ITI-83 query and the
    /// `EHR_STATUS.subject.external_ref.namespace` the answered identifier is
    /// stored under. It must be one of `privacy.subject_namespaces`.
    pub assigning_authority: String,
    /// How the PIX Manager is asked: `post` (the default) or `get`.
    pub invocation: PixmInvocation,
    /// How long one ITI-83 exchange may take, connection included (ms).
    pub request_timeout_ms: u64,
    /// `[ehr_index.tls]`: the trust roots beside the platform's
    /// (`trust_roots_file`), for a PIX Manager behind a private PKI, and the
    /// client identity presented for mutual TLS.
    pub tls: TlsClientSection,
    /// The OAuth 2.0 client credentials grant whose access token every request
    /// carries (IUA ITI-72 §3.72.4.2, `[ehr_index.oauth]`). Unset sends none.
    pub oauth: Option<ferrohealth_smart::config::OauthSection>,
}

impl Default for EhrIndexConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::Ihe,
            pixm_url: SecretUrl::default(),
            assigning_authority: String::new(),
            invocation: PixmInvocation::Post,
            request_timeout_ms: 10_000,
            tls: TlsClientSection::default(),
            oauth: None,
        }
    }
}

impl EhrIndexConfig {
    /// The configuration of a FerroEHR that opted in to the in-instance map.
    #[must_use]
    pub fn local() -> Self {
        Self {
            provider: ProviderKind::Local,
            ..Self::default()
        }
    }

    /// Every rule the section breaks, as boot-refusal messages.
    ///
    /// `subject_namespaces` is `privacy.subject_namespaces`: the clinical
    /// domain's identifier is the pseudonym on `EHR_STATUS`, so its assigning
    /// authority must be a declared pseudonym namespace. Only the chosen
    /// provider's keys are checked; the other provider's are kept and not used
    /// ([`Self::unused_keys`]), so `provider` alone switches the index, from
    /// the file or as `FERROEHR__EHR_INDEX__PROVIDER`.
    #[must_use]
    pub fn problems(&self, subject_namespaces: &[String]) -> Vec<String> {
        match self.provider {
            ProviderKind::Local => Vec::new(),
            ProviderKind::Ihe => {
                if self.pixm_url.expose().trim().is_empty()
                    && self.assigning_authority.trim().is_empty()
                {
                    return vec![
                        "no patient identity source is configured: set ehr_index.pixm_url \
                         (the PIXm base URL of an IHE patient identity source) and \
                         ehr_index.assigning_authority (the clinical domain), or choose the \
                         in-instance map with ehr_index.provider = \"local\" \
                         (FERROEHR__EHR_INDEX__PROVIDER=local), where EHR_STATUS.subject names \
                         a patient by its own namespace and identifier"
                            .to_owned(),
                    ];
                }
                self.ihe_problems(subject_namespaces)
            }
        }
    }

    /// The keys set for the IHE provider while `provider = "local"`, which the
    /// server keeps and does not use; boot names them.
    #[must_use]
    pub fn unused_keys(&self) -> Vec<&'static str> {
        if self.provider != ProviderKind::Local {
            return Vec::new();
        }
        let tls = &self.tls;
        [
            ("ehr_index.pixm_url", !self.pixm_url.is_empty()),
            (
                "ehr_index.assigning_authority",
                !self.assigning_authority.trim().is_empty(),
            ),
            ("ehr_index.oauth", self.oauth.is_some()),
            (
                "ehr_index.tls",
                tls.client_identity.is_some()
                    || tls.client_identity_file.is_some()
                    || tls.trust_roots_file.is_some(),
            ),
        ]
        .into_iter()
        .filter_map(|(key, set)| set.then_some(key))
        .collect()
    }

    /// The rules of a configured IHE provider.
    fn ihe_problems(&self, subject_namespaces: &[String]) -> Vec<String> {
        let mut problems = Vec::new();
        match url::Url::parse(self.pixm_url.expose().trim()) {
            Ok(url) if matches!(url.scheme(), "http" | "https") => {}
            _ => problems.push(format!(
                "ehr_index.pixm_url ({}) must be an http or https URL",
                self.pixm_url
            )),
        }
        let authority = self.assigning_authority.trim();
        if url::Url::parse(authority).is_err() {
            problems.push(
                "ehr_index.assigning_authority must be an absolute URI, such as urn:oid:2.999.1 \
                 (the assigning authority of the clinical domain, PIXm §2:3.83.4.1.2.2)"
                    .to_owned(),
            );
        } else if !subject_namespaces.iter().any(|n| n == authority) {
            problems.push(format!(
                "ehr_index.assigning_authority ({authority}) must be one of \
                 privacy.subject_namespaces: the identifier the identity source answers for \
                 the clinical domain is the pseudonym EHR_STATUS carries"
            ));
        }
        if self.request_timeout_ms == 0 {
            problems.push("ehr_index.request_timeout_ms must be greater than 0".to_owned());
        }
        if let Some(oauth) = &self.oauth {
            problems.extend(oauth.problems("ehr_index.oauth"));
            let userinfo = url::Url::parse(self.pixm_url.expose())
                .is_ok_and(|u| !u.username().is_empty() || u.password().is_some());
            if userinfo {
                problems.push(
                    "ehr_index.pixm_url carries credentials in its URL and an [ehr_index.oauth] \
                     grant; authenticate one way: remove the userinfo or the grant"
                        .to_owned(),
                );
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOMAIN: &str = "urn:oid:2.999.1";

    fn ihe() -> EhrIndexConfig {
        EhrIndexConfig {
            pixm_url: SecretUrl::new("https://pix.example/fhir"),
            assigning_authority: DOMAIN.to_owned(),
            ..EhrIndexConfig::default()
        }
    }

    /// The binary's default is the IHE provider with no endpoint, which is
    /// refused with both choices named, the environment switch included.
    #[test]
    fn the_default_names_both_choices() {
        let problems = EhrIndexConfig::default().problems(&[]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        let message = &problems[0];
        assert!(message.contains("ehr_index.pixm_url"), "{message}");
        assert!(message.contains("provider = \"local\""), "{message}");
        assert!(
            message.contains("FERROEHR__EHR_INDEX__PROVIDER=local"),
            "{message}"
        );
    }

    /// `provider = "local"` beside the IHE keys runs the local index: the keys
    /// are kept and named as unused, so the provider alone switches (#4379).
    #[test]
    fn the_provider_alone_switches_the_index() {
        assert_eq!(EhrIndexConfig::local().problems(&[]), Vec::<String>::new());
        assert_eq!(EhrIndexConfig::local().unused_keys(), Vec::<&str>::new());
        let switched = EhrIndexConfig {
            provider: ProviderKind::Local,
            oauth: Some(ferrohealth_smart::config::OauthSection::default()),
            ..ihe()
        };
        assert_eq!(switched.problems(&[]), Vec::<String>::new());
        assert_eq!(
            switched.unused_keys(),
            [
                "ehr_index.pixm_url",
                "ehr_index.assigning_authority",
                "ehr_index.oauth"
            ]
        );
        assert_eq!(ihe().unused_keys(), Vec::<&str>::new());
    }

    #[test]
    fn a_configured_source_needs_a_declared_pseudonym_domain() {
        assert_eq!(ihe().problems(&[DOMAIN.to_owned()]), Vec::<String>::new());
        let problems = ihe().problems(&["urn:oid:2.999.9".to_owned()]);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("privacy.subject_namespaces")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_half_configured_source_is_refused() {
        let no_url = EhrIndexConfig {
            pixm_url: SecretUrl::default(),
            ..ihe()
        };
        assert!(
            no_url
                .problems(&[DOMAIN.to_owned()])
                .iter()
                .any(|p| p.contains("ehr_index.pixm_url"))
        );
        let not_a_uri = EhrIndexConfig {
            assigning_authority: "nl-bsn".to_owned(),
            ..ihe()
        };
        assert!(
            not_a_uri
                .problems(&["nl-bsn".to_owned()])
                .iter()
                .any(|p| p.contains("absolute URI"))
        );
    }

    #[test]
    fn the_provider_reads_in_lowercase() {
        let local: EhrIndexConfig = toml::from_str("provider = \"local\"").expect("local");
        assert_eq!(local.provider, ProviderKind::Local);
        assert!(toml::from_str::<EhrIndexConfig>("provider = \"ferropix\"").is_err());
    }
}
