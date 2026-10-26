// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[ehr_index]` section: which provider holds the subject-to-EHR
//! cross-reference.
//!
//! An IHE patient identity source asked over PIXm (ITI-83) is the default, and
//! it is named by its endpoint and the assigning authority of the clinical
//! domain, never by a product. A FerroEHR that runs on its own opts in to the
//! in-instance map with `provider = "local"`. With neither, the server refuses
//! to start and names both choices.
//!
//! NOTE: no openEHR spec governs where the cross-reference lives (RM common
//! master04 §`PARTY_SELF`: "may be done elsewhere for security reasons"): our own design.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use ferrohealth_secret::secret_url::SecretUrl;

/// The provider that holds the subject-to-EHR cross-reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// An IHE patient identity source asked over PIXm (ITI-83), the default.
    #[default]
    Ihe,
    /// The in-instance `linkage` map, the opt-in for a FerroEHR run on its own.
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
    /// A PEM bundle of certificate authorities trusted beside the platform
    /// roots, for a PIX Manager behind a private PKI.
    pub ca_file: Option<PathBuf>,
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
            ca_file: None,
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
    /// authority must be a declared pseudonym namespace.
    #[must_use]
    pub fn problems(&self, subject_namespaces: &[String]) -> Vec<String> {
        let mut problems = Vec::new();
        match self.provider {
            ProviderKind::Local => {
                if !self.pixm_url.is_empty() || !self.assigning_authority.is_empty() {
                    problems.push(
                        "ehr_index.provider = \"local\" takes no ehr_index.pixm_url or \
                         ehr_index.assigning_authority; remove them, or choose provider = \"ihe\""
                            .to_owned(),
                    );
                }
            }
            ProviderKind::Ihe => {
                if self.pixm_url.expose().trim().is_empty()
                    && self.assigning_authority.trim().is_empty()
                {
                    problems.push(
                        "no patient identity source is configured: set ehr_index.pixm_url \
                         (the PIXm base URL of an IHE patient identity source) and \
                         ehr_index.assigning_authority (the clinical domain), or opt in to \
                         the in-instance map with ehr_index.provider = \"local\" for a \
                         FerroEHR run on its own"
                            .to_owned(),
                    );
                    return problems;
                }
                problems.extend(self.ihe_problems(subject_namespaces));
            }
        }
        problems
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
    /// refused with both choices named.
    #[test]
    fn the_default_names_both_choices() {
        let problems = EhrIndexConfig::default().problems(&[]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        let message = &problems[0];
        assert!(message.contains("ehr_index.pixm_url"), "{message}");
        assert!(message.contains("provider = \"local\""), "{message}");
    }

    #[test]
    fn local_is_an_explicit_opt_in() {
        assert_eq!(EhrIndexConfig::local().problems(&[]), Vec::<String>::new());
        let mixed = EhrIndexConfig {
            provider: ProviderKind::Local,
            ..ihe()
        };
        assert_ne!(mixed.problems(&[DOMAIN.to_owned()]), Vec::<String>::new());
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
