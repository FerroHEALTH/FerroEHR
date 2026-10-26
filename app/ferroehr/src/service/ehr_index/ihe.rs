// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The IHE patient identity source: a PIX Manager asked as a PIXm Patient
//! Identifier Cross-reference Consumer (ITI-83) through `ihe-iti`.
//!
//! FerroEHR asks only what ITI-83 specifies: which identifier a domain holds
//! for the patient another domain's identifier names (`$ihe-pix`,
//! §2:3.83.4.1.2). It sends no product-specific parameter, matches no patient
//! and merges nothing; a patient the Manager does not know is an answer, and
//! every other failure is a typed error carrying what the Manager answered,
//! never an empty value (ITI-83 §2:3.83.4.2.2).
//!
//! PDQm (ITI-78) is not used: its search answers with the demographic
//! `Patient`, which FerroEHR would only discard (GDPR Art. 5(1)(c), data
//! minimisation), and its `$match` is patient matching.

use std::time::Duration;

use ihe_atna::user::OnBehalfOf;
use ihe_iti::pixm::error::{InvalidInput, PixmError};
use ihe_iti::pixm::identifier::{CrossReference, SourceIdentifier, TargetSystem};
use ihe_iti::pixm::{Invocation, PixmClient};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::config::trust::{TrustRootsError, trusting};
use crate::service::ehr_index::config::{EhrIndexConfig, PixmInvocation};

/// The media type a FHIR server answers the capabilities interaction in (FHIR
/// R4 http.html §Content Types and encodings).
const FHIR_JSON: &str = "application/fhir+json";

/// Why the identity source could not be built, reached or believed.
#[derive(Debug, thiserror::Error)]
pub enum IdentitySourceError {
    /// The configured `ca_file` cannot serve as trust roots.
    #[error("the identity source's trust roots are unusable")]
    TrustRoots(#[from] TrustRootsError),
    /// The HTTP client could not be built.
    #[error("the identity source's HTTP client could not be built")]
    Client(#[source] reqwest::Error),
    /// The PIXm base URL does not parse.
    #[error("ehr_index.pixm_url is not a URL")]
    BaseUrl(#[source] url::ParseError),
    /// The base URL, the assigning authority or a subject is no valid ITI-83
    /// input.
    #[error("the request to the identity source is not valid ITI-83 input")]
    Input(#[from] InvalidInput),
    /// The PIX Manager refused or failed the ITI-83 exchange.
    #[error("the identity source did not answer the cross-reference")]
    Pixm(#[from] PixmError),
    /// The PIX Manager answered more than one identifier in the clinical
    /// domain, which FerroEHR has no way to choose between.
    #[error("the identity source answered {count} identifiers in the clinical domain")]
    Ambiguous {
        /// How many identifiers it answered.
        count: usize,
    },
    /// The capabilities interaction could not be sent or read.
    #[error("the identity source could not be reached")]
    Unreachable(#[source] reqwest::Error),
    /// The capabilities interaction answered a status other than success.
    #[error("the identity source answered its capabilities read with {status}")]
    Refused {
        /// The status it answered.
        status: reqwest::StatusCode,
    },
    /// The capabilities interaction answered with no `CapabilityStatement`.
    #[error("the identity source's capabilities read holds no CapabilityStatement")]
    NotAFhirServer,
}

/// One identifier a domain holds for a patient: the assigning authority and
/// the value.
#[derive(Clone)]
pub struct DomainIdentifier {
    /// The assigning authority of the domain, an absolute URI.
    pub system: String,
    /// The identifier value, never rendered.
    pub value: SecretString,
}

impl std::fmt::Debug for DomainIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DomainIdentifier")
            .field("system", &self.system)
            .field("value", &crate::config::secret::REDACTED)
            .finish()
    }
}

/// The PIX Manager of the configured clinical domain.
#[derive(Debug, Clone)]
pub struct IheIdentitySource {
    /// The FHIR base URL, for the capabilities interaction.
    base: Url,
    /// The ITI-83 client.
    pixm: PixmClient,
    /// The HTTP client the capabilities interaction is sent through.
    http: reqwest::Client,
    /// The assigning authority of the clinical domain.
    domain: TargetSystem,
    /// How long one exchange may take.
    timeout: Duration,
}

impl IheIdentitySource {
    /// The identity source `config` names.
    ///
    /// The HTTP client follows no redirect, because the request carries a
    /// patient identifier (the `PixmClient::new` contract), and trusts
    /// `ehr_index.ca_file` beside the platform roots.
    ///
    /// # Errors
    /// [`IdentitySourceError`] when the trust roots, the client, the base URL
    /// or the assigning authority are unusable.
    pub fn new(config: &EhrIndexConfig) -> Result<Self, IdentitySourceError> {
        let timeout = Duration::from_millis(config.request_timeout_ms);
        let builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout);
        let http = trusting(builder, "ehr_index.ca_file", config.ca_file.as_deref())?
            .build()
            .map_err(IdentitySourceError::Client)?;
        let base =
            Url::parse(config.pixm_url.expose().trim()).map_err(IdentitySourceError::BaseUrl)?;
        let invocation = match config.invocation {
            PixmInvocation::Get => Invocation::Get,
            PixmInvocation::Post => Invocation::Post,
        };
        let pixm = PixmClient::new(base.clone(), http.clone())?.invoked_by(invocation);
        Ok(Self {
            base,
            pixm,
            http,
            domain: TargetSystem::new(config.assigning_authority.trim())?,
            timeout,
        })
    }

    /// The assigning authority of the clinical domain.
    #[must_use]
    pub fn domain(&self) -> &str {
        self.domain.as_str()
    }

    /// Reads the PIX Manager's `CapabilityStatement` (FHIR R4 http.html
    /// §capabilities), the boot check that the configured source is a FHIR
    /// server that answers.
    ///
    /// # Errors
    /// [`IdentitySourceError::Unreachable`], [`IdentitySourceError::Refused`]
    /// or [`IdentitySourceError::NotAFhirServer`].
    pub async fn probe(&self) -> Result<(), IdentitySourceError> {
        let mut url = self.base.clone();
        let path = format!("{}/metadata", url.path().trim_end_matches('/'));
        url.set_path(&path);
        let response = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, FHIR_JSON)
            .send()
            .await
            .map_err(IdentitySourceError::Unreachable)?;
        let status = response.status();
        if !status.is_success() {
            return Err(IdentitySourceError::Refused { status });
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(IdentitySourceError::Unreachable)?;
        if body.get("resourceType").and_then(serde_json::Value::as_str)
            == Some("CapabilityStatement")
        {
            Ok(())
        } else {
            Err(IdentitySourceError::NotAFhirServer)
        }
    }

    /// The identifier the clinical domain holds for the patient that `value`
    /// names in the domain `system`, or `None` when the source does not know
    /// the patient or holds no clinical identifier for them (ITI-83
    /// §2:3.83.4.2.2.1, §2:3.83.4.2.2.2).
    ///
    /// # Errors
    /// [`IdentitySourceError::Input`] for a `system` that is no absolute URI,
    /// [`IdentitySourceError::Pixm`] for every refusal or failure, and
    /// [`IdentitySourceError::Ambiguous`] for more than one clinical identifier.
    pub async fn clinical_identifier(
        &self,
        system: &str,
        value: &str,
    ) -> Result<Option<SecretString>, IdentitySourceError> {
        let source = SourceIdentifier::new(system, SecretString::from(value.to_owned()))?;
        let answer = self
            .pixm
            .cross_reference(
                &source,
                std::slice::from_ref(&self.domain),
                &OnBehalfOf::System,
                self.timeout,
            )
            .await?;
        let CrossReference::Matched(found) = answer else {
            return Ok(None);
        };
        let mut clinical: Vec<_> = found.in_domain(self.domain.as_str()).collect();
        match clinical.len() {
            0 => Ok(None),
            1 => Ok(clinical.pop().map(|id| id.value().clone())),
            count => Err(IdentitySourceError::Ambiguous { count }),
        }
    }

    /// Every identifier the other domains hold for the patient whose clinical
    /// identifier is `clinical`, in the PIX Manager's order (ITI-83
    /// §2:3.83.4.2.2.1); empty when the source does not know them.
    ///
    /// # Errors
    /// [`IdentitySourceError::Input`] for an empty `clinical` and
    /// [`IdentitySourceError::Pixm`] for every refusal or failure.
    pub async fn identifiers_of(
        &self,
        clinical: &str,
    ) -> Result<Vec<DomainIdentifier>, IdentitySourceError> {
        let source = SourceIdentifier::new(
            self.domain.as_str(),
            SecretString::from(clinical.to_owned()),
        )?;
        let answer = self
            .pixm
            .cross_reference(&source, &[], &OnBehalfOf::System, self.timeout)
            .await?;
        let CrossReference::Matched(found) = answer else {
            return Ok(Vec::new());
        };
        Ok(found
            .identifiers()
            .iter()
            .filter(|id| {
                id.system() != self.domain.as_str() || id.value().expose_secret() != clinical
            })
            .map(|id| DomainIdentifier {
                system: id.system().to_owned(),
                value: id.value().clone(),
            })
            .collect())
    }
}
