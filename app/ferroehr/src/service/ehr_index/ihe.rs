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

use ferrohealth_server::tls_client::{TlsClient, TlsClientError};
use ihe_atna::user::OnBehalfOf;
use ihe_iti::authorizer::Authorizable as _;
use ihe_iti::pixm::capabilities::CapabilitiesError;
use ihe_iti::pixm::error::{InvalidInput, PixmError};
use ihe_iti::pixm::identifier::{CrossReference, SourceIdentifier, TargetSystem};
use ihe_iti::pixm::{Invocation, PixmClient};
use secrecy::SecretString;
use url::Url;

use crate::service::ehr_index::config::{PixmConfig, PixmInvocation};

/// Why the identity source could not be built, reached or believed.
#[derive(Debug, thiserror::Error)]
pub enum IdentitySourceError {
    /// The `[pixm.tls]` trust roots or client identity are unusable.
    #[error("the identity source's TLS material is unusable")]
    Tls(#[from] TlsClientError),
    /// The HTTP client could not be built.
    #[error("the identity source's HTTP client could not be built")]
    Client(#[source] reqwest::Error),
    /// The PIXm base URL does not parse.
    #[error("pixm.url is not a URL")]
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
    /// The `[pixm.oauth]` grant is incomplete or its token client could
    /// not be built.
    #[error("the identity source's client-credentials grant is unusable")]
    Grant(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The PIX Manager's capabilities interaction (FHIR R4 http.html
    /// §capabilities) failed: no answer, a refusal, or no FHIR R4
    /// `CapabilityStatement`.
    #[error("the identity source did not answer its capabilities read")]
    Capabilities(#[from] CapabilitiesError),
}

/// The PIX Manager of the configured clinical domain.
#[derive(Debug, Clone)]
pub struct IheIdentitySource {
    /// The ITI-83 client, which also reads the capabilities.
    pixm: PixmClient,
    /// The assigning authority of the clinical domain.
    domain: TargetSystem,
    /// How long one exchange may take.
    timeout: Duration,
}

impl IheIdentitySource {
    /// The identity source `config` names.
    ///
    /// The HTTP client follows no redirect, because the request carries a
    /// patient identifier (the `PixmClient::new` contract), and trusts the
    /// `[pixm.tls]` roots beside the platform's. With `[pixm.oauth]`
    /// every ITI-83 request carries that grant's access token (IUA ITI-72
    /// §3.72.4.2), renewed once after a `401`.
    ///
    /// # Errors
    /// [`IdentitySourceError`] when the trust roots, the client, the base URL
    /// or the assigning authority are unusable.
    pub fn new(config: &PixmConfig) -> Result<Self, IdentitySourceError> {
        let timeout = config.timeout.get();
        let builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout);
        let http = TlsClient::read("pixm.tls", &config.tls)?
            .apply(builder)?
            .build()
            .map_err(IdentitySourceError::Client)?;
        let base = Url::parse(config.url.expose().trim()).map_err(IdentitySourceError::BaseUrl)?;
        let invocation = match config.method {
            PixmInvocation::Get => Invocation::Get,
            PixmInvocation::Post => Invocation::Post,
        };
        let mut pixm = PixmClient::new(base, http)?.invoked_by(invocation);
        if let Some(oauth) = &config.oauth {
            let grant = oauth
                .resolve()
                .map_err(|e| IdentitySourceError::Grant(Box::new(e)))?;
            let source = ferrohealth_smart::client::cache::TokenSource::new(grant)
                .map_err(|e| IdentitySourceError::Grant(Box::new(e)))?;
            pixm = pixm.with_authorizer(std::sync::Arc::new(
                ihe_iti::authorizer::smart::TokenAuthorizer::new(source),
            ));
        }
        Ok(Self {
            pixm,
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
    /// §capabilities) through the ITI-83 client, with its transport and its
    /// authorizer: the boot check that the configured source is a FHIR server
    /// that answers.
    ///
    /// # Errors
    /// [`IdentitySourceError::Capabilities`] carrying the client's refusal.
    pub async fn probe(&self) -> Result<(), IdentitySourceError> {
        self.pixm.capabilities(self.timeout).await?;
        Ok(())
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
}
