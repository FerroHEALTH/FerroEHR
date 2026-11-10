// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `record.subject_index` and the `[pixm]` table: which provider holds the
//! subject-to-EHR cross-reference.
//!
//! An IHE patient identity source asked over PIXm (ITI-83, `[pixm]`) is named
//! by its endpoint and the assigning authority of the clinical domain, never by
//! a product. The in-instance map, `subject_index = "local"`, is the other
//! supported posture, the subject-id and subject-namespace model. The key has
//! no default: unset, the server refuses to start and names both choices.
//! `subject_index` alone switches between them: a `[pixm]` table kept beside
//! `"local"` is not used.
//!
//! NOTE: no openEHR spec governs where the cross-reference lives (RM common
//! master04 §`PARTY_SELF`: "may be done elsewhere for security reasons"): our own design.

use ferrohealth_config::units::Duration;
use serde::{Deserialize, Serialize};

use ferrohealth_config::tls::TlsClientSection;
use ferrohealth_secret::secret_url::SecretUrl;

/// The provider that holds the subject-to-EHR cross-reference
/// (`record.subject_index`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubjectIndex {
    /// An IHE patient identity source asked over PIXm (ITI-83), configured in
    /// `[pixm]`.
    Pixm,
    /// The subject-id and subject-namespace model in this instance, which
    /// `EHR_STATUS.subject` reads directly.
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

/// The `[pixm]` table: the IHE PIXm Patient Identifier Cross-reference
/// Manager the subject index asks (ITI-83).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PixmConfig {
    /// The FHIR base URL of the PIX Manager (`[base]` of `[base]/Patient/$ihe-pix`,
    /// PIXm §2:3.83.4.1.2). Any `userinfo` in it is masked in every rendering.
    pub url: SecretUrl,
    /// The assigning authority of the clinical domain, an absolute URI such as
    /// `urn:oid:2.999.1`: the `targetSystem` of every ITI-83 query and the
    /// `EHR_STATUS.subject.external_ref.namespace` the answered identifier is
    /// stored under. It must be one of `privacy.subject_namespaces`.
    pub assigning_authority: String,
    /// How the PIX Manager is asked: `post` (the default) or `get`.
    pub method: PixmInvocation,
    /// How long one ITI-83 exchange may take, connection included.
    pub timeout: Duration,
    /// `[pixm.tls]`: the trust roots beside the platform's
    /// (`trust_roots_file`), for a PIX Manager behind a private PKI, and the
    /// client identity presented for mutual TLS.
    pub tls: TlsClientSection,
    /// The OAuth 2.0 client credentials grant whose access token every request
    /// carries (IUA ITI-72 §3.72.4.2, `[pixm.oauth]`). Unset sends none.
    pub oauth: Option<ferrohealth_smart::config::credentials::OauthSection>,
}

impl Default for PixmConfig {
    fn default() -> Self {
        Self {
            url: SecretUrl::default(),
            assigning_authority: String::new(),
            method: PixmInvocation::Post,
            timeout: Duration::from_secs(10),
            tls: TlsClientSection::default(),
            oauth: None,
        }
    }
}

/// Every rule the subject index breaks, as boot-refusal messages.
///
/// `subject_namespaces` is `privacy.subject_namespaces`: the clinical
/// domain's identifier is the pseudonym on `EHR_STATUS`, so its assigning
/// authority must be a declared pseudonym namespace. Only the chosen
/// provider's table is checked; a `[pixm]` table beside `"local"` is kept and
/// not used ([`unused_keys`]), so `subject_index` alone switches the index,
/// from the file or as `FERROEHR__RECORD__SUBJECT_INDEX`.
#[must_use]
pub fn problems(
    index: Option<SubjectIndex>,
    pixm: Option<&PixmConfig>,
    subject_namespaces: &[String],
) -> Vec<String> {
    match (index, pixm) {
        (Some(SubjectIndex::Local), _) => Vec::new(),
        (Some(SubjectIndex::Pixm), Some(pixm)) => pixm.problems(subject_namespaces),
        (Some(SubjectIndex::Pixm), None) => vec![
            "record.subject_index = \"pixm\" names no patient identity source: add a [pixm] \
             table with url (the PIXm base URL of an IHE patient identity source) and \
             assigning_authority (the clinical domain)"
                .to_owned(),
        ],
        (None, _) => vec![
            "record.subject_index is not set: choose an IHE patient identity source with \
             record.subject_index = \"pixm\" and a [pixm] table naming its url and \
             assigning_authority, or the in-instance map with record.subject_index = \"local\" \
             (FERROEHR__RECORD__SUBJECT_INDEX=local), where EHR_STATUS.subject names a patient \
             by its own namespace and identifier"
                .to_owned(),
        ],
    }
}

/// The `[pixm]` keys set while `subject_index = "local"`, which the server
/// keeps and does not use; boot names them.
#[must_use]
pub fn unused_keys(index: Option<SubjectIndex>, pixm: Option<&PixmConfig>) -> Vec<&'static str> {
    match (index, pixm) {
        (Some(SubjectIndex::Local), Some(pixm)) => pixm.set_keys(),
        _ => Vec::new(),
    }
}

impl PixmConfig {
    /// The keys of this table that carry a value.
    fn set_keys(&self) -> Vec<&'static str> {
        let tls = &self.tls;
        [
            ("pixm.url", !self.url.is_empty()),
            (
                "pixm.assigning_authority",
                !self.assigning_authority.trim().is_empty(),
            ),
            ("pixm.oauth", self.oauth.is_some()),
            (
                "pixm.tls",
                tls.client_identity.is_some()
                    || tls.client_identity_file.is_some()
                    || tls.trust_roots_file.is_some(),
            ),
        ]
        .into_iter()
        .filter_map(|(key, set)| set.then_some(key))
        .collect()
    }

    /// The rules of a configured identity source.
    fn problems(&self, subject_namespaces: &[String]) -> Vec<String> {
        let mut problems = Vec::new();
        match url::Url::parse(self.url.expose().trim()) {
            Ok(url) if matches!(url.scheme(), "http" | "https") => {}
            _ => problems.push(format!(
                "pixm.url ({}) must be an http or https URL",
                self.url
            )),
        }
        let authority = self.assigning_authority.trim();
        if url::Url::parse(authority).is_err() {
            problems.push(
                "pixm.assigning_authority must be an absolute URI, such as urn:oid:2.999.1 \
                 (the assigning authority of the clinical domain, PIXm §2:3.83.4.1.2.2)"
                    .to_owned(),
            );
        } else if !subject_namespaces.iter().any(|n| n == authority) {
            problems.push(format!(
                "pixm.assigning_authority ({authority}) must be one of \
                 privacy.subject_namespaces: the identifier the identity source answers for \
                 the clinical domain is the pseudonym EHR_STATUS carries"
            ));
        }
        if self.timeout.is_zero() {
            problems.push("pixm.timeout must be longer than \"0s\"".to_owned());
        }
        if let Some(oauth) = &self.oauth {
            problems.extend(oauth.problems("pixm.oauth"));
            let userinfo = url::Url::parse(self.url.expose())
                .is_ok_and(|u| !u.username().is_empty() || u.password().is_some());
            if userinfo {
                problems.push(
                    "pixm.url carries credentials in its URL and a [pixm.oauth] grant; \
                     authenticate one way: remove the userinfo or the grant"
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

    fn pixm() -> PixmConfig {
        PixmConfig {
            url: SecretUrl::new("https://pix.example/fhir"),
            assigning_authority: DOMAIN.to_owned(),
            ..PixmConfig::default()
        }
    }

    /// An unset subject index is refused with both choices named, the
    /// environment switch included.
    #[test]
    fn an_unset_index_names_both_choices() {
        let problems = problems(None, None, &[]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        let message = &problems[0];
        assert!(message.contains("subject_index = \"pixm\""), "{message}");
        assert!(message.contains("subject_index = \"local\""), "{message}");
        assert!(
            message.contains("FERROEHR__RECORD__SUBJECT_INDEX=local"),
            "{message}"
        );
        let missing = problems_of(Some(SubjectIndex::Pixm), None);
        assert!(missing[0].contains("[pixm]"), "{missing:?}");
    }

    fn problems_of(index: Option<SubjectIndex>, pixm: Option<&PixmConfig>) -> Vec<String> {
        problems(index, pixm, &[])
    }

    /// `subject_index = "local"` beside a `[pixm]` table runs the local index:
    /// the keys are kept and named as unused, so the index alone switches
    /// (#4379).
    #[test]
    fn the_index_alone_switches() {
        let local = Some(SubjectIndex::Local);
        assert_eq!(problems_of(local, None), Vec::<String>::new());
        assert_eq!(unused_keys(local, None), Vec::<&str>::new());
        let kept = PixmConfig {
            oauth: Some(ferrohealth_smart::config::credentials::OauthSection::default()),
            ..pixm()
        };
        assert_eq!(problems_of(local, Some(&kept)), Vec::<String>::new());
        assert_eq!(
            unused_keys(local, Some(&kept)),
            ["pixm.url", "pixm.assigning_authority", "pixm.oauth"]
        );
        assert_eq!(
            unused_keys(Some(SubjectIndex::Pixm), Some(&pixm())),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn a_configured_source_needs_a_declared_pseudonym_domain() {
        let index = Some(SubjectIndex::Pixm);
        assert_eq!(
            problems(index, Some(&pixm()), &[DOMAIN.to_owned()]),
            Vec::<String>::new()
        );
        let problems = problems(index, Some(&pixm()), &["urn:oid:2.999.9".to_owned()]);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("privacy.subject_namespaces")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_half_configured_source_is_refused() {
        let index = Some(SubjectIndex::Pixm);
        let no_url = PixmConfig {
            url: SecretUrl::default(),
            ..pixm()
        };
        assert!(
            problems(index, Some(&no_url), &[DOMAIN.to_owned()])
                .iter()
                .any(|p| p.contains("pixm.url"))
        );
        let not_a_uri = PixmConfig {
            assigning_authority: "nl-bsn".to_owned(),
            ..pixm()
        };
        assert!(
            problems(index, Some(&not_a_uri), &["nl-bsn".to_owned()])
                .iter()
                .any(|p| p.contains("absolute URI"))
        );
    }

    #[test]
    fn the_index_reads_in_lowercase() {
        #[derive(Deserialize)]
        struct Holder {
            index: SubjectIndex,
        }
        let local: Holder = toml::from_str("index = \"local\"").expect("local");
        assert_eq!(local.index, SubjectIndex::Local);
        assert!(toml::from_str::<Holder>("index = \"ihe\"").is_err());
        assert!(toml::from_str::<Holder>("index = \"ferropix\"").is_err());
    }
}
