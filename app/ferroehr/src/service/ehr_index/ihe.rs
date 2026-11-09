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

use ihe_atna::user::OnBehalfOf;
use ihe_iti::authorizer::Authorizable as _;
use ihe_iti::pixm::capabilities::CapabilitiesError;
use ihe_iti::pixm::error::{InvalidInput, PixmError};
use ihe_iti::pixm::identifier::{CrossReference, SourceIdentifier, TargetSystem};
use ihe_iti::pixm::{Invocation, PixmClient};
use secrecy::SecretString;

use crate::service::ehr_index::config::{PixmConfig, PixmInvocation};

/// Why the identity source could not be built, reached or believed.
#[derive(Debug, thiserror::Error)]
pub enum IdentitySourceError {
    /// `[pixm.endpoint]` is refused: its URL, its TLS material, its
    /// credential or the HTTP client built from it.
    #[error("the identity source's endpoint is refused")]
    Endpoint(#[from] crate::outbound::EndpointError),
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
    /// The PIX Manager's capabilities interaction (FHIR R4 http.html
    /// §capabilities) failed: no answer, a refusal, or no FHIR R4
    /// `CapabilityStatement`.
    #[error("the identity source did not answer its capabilities read")]
    Capabilities(#[from] CapabilitiesError),
}

/// Whether an ITI-83 exchange that failed with `error` may be asked again:
/// no answer arrived, or the Manager answered a server error.
fn transient(error: &PixmError) -> bool {
    match error {
        PixmError::Timeout | PixmError::Transport(_) => true,
        PixmError::Rejected { status, .. } => status.is_server_error(),
        _ => false,
    }
}

/// The PIX Manager of the configured clinical domain.
#[derive(Debug, Clone)]
pub struct IheIdentitySource {
    /// The ITI-83 client, which also reads the capabilities.
    pixm: PixmClient,
    /// The assigning authority of the clinical domain.
    domain: TargetSystem,
    /// The endpoint: how long one exchange may take and how a failed one is
    /// tried again.
    endpoint: crate::outbound::Endpoint,
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
        let endpoint = crate::outbound::Endpoint::connect("pixm.endpoint", &config.endpoint)?;
        let invocation = match config.method {
            PixmInvocation::Get => Invocation::Get,
            PixmInvocation::Post => Invocation::Post,
        };
        let mut pixm =
            PixmClient::new(endpoint.url().clone(), endpoint.http())?.invoked_by(invocation);
        if let Some(token) = endpoint.token() {
            pixm = pixm.with_authorizer(std::sync::Arc::new(
                ihe_iti::authorizer::smart::TokenAuthorizer::new(token),
            ));
        }
        Ok(Self {
            pixm,
            domain: TargetSystem::new(config.assigning_authority.trim())?,
            endpoint,
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
        let timeout = self.endpoint.timeout();
        self.endpoint
            .retrying(
                |error| {
                    matches!(
                        error,
                        CapabilitiesError::Timeout | CapabilitiesError::Transport(_)
                    )
                },
                || self.pixm.capabilities(timeout),
            )
            .await?;
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
        let timeout = self.endpoint.timeout();
        let answer = self
            .endpoint
            .retrying(transient, || {
                self.pixm.cross_reference(
                    &source,
                    std::slice::from_ref(&self.domain),
                    &OnBehalfOf::System,
                    timeout,
                )
            })
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
