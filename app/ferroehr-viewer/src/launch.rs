// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SMART EHR launch of a configured app from an EHR's page, through
//! FerroSMART's EHR launch API.
//!
//! SMART App Launch 2.2.0 fixes only the last step of an EHR launch: the
//! EHR opens the app's launch URL with `iss` and `launch`, "an opaque handle
//! to the EHR context" (Launch App: EHR Launch, Request). It defines no way
//! for the EHR to hand the authorization server that context, so FerroSMART
//! takes it at its own API, the `launch_registration_endpoint` its discovery
//! document names (`{issuer}/.well-known/smart-configuration`): a form in which the EHR
//! authenticates with its `private_key_jwt` client assertion as at the token
//! endpoint and names the app, the base and the context, answered `201`
//! with `{"launch", "expires_in"}`. The viewer is that EHR client: it holds
//! the key server-side, registers each click's context there and sends the
//! browser to `{launch_url}?iss={resource}&launch={value}`. The launch JWT
//! FerroSMART also takes is not used, so every value is single use at
//! FerroSMART's side. A SMART launch Task read at the FHIR facade
//! ([`crate::launch_tasks`]) launches the same way, its app named by its
//! launch URL (`app_launch_uri`) and its base the facade's. No openEHR spec
//! governs this: our own design.

use std::path::PathBuf;

use ferrohealth_config::tls::TlsClientSection;
use ferrohealth_config::units::Duration;
use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_smart::client::assertion::{ASSERTION_TYPE, AssertionError, AssertionKey};
use ferrohealth_smart::client::outbound::{OutboundClient, OutboundError};
use ferrohealth_smart::config::ServerTls;
use ferrohealth_smart::config::endpoint::EndpointSection;
use jsonwebtoken::Algorithm;
use serde::{Deserialize, Serialize};

/// The `typ` of the client assertion the viewer sends: RFC 10070 §4's
/// explicit type, which FerroSMART takes beside SMART's `JWT`.
const ASSERTION_TYP: &str = "client-authentication+jwt";

/// `[launch]`: the viewer as a FerroSMART EHR client and the SMART apps it
/// launches. An empty `issuer` offers no launch.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LaunchSection {
    /// FerroSMART's issuer identifier: the launch API is the
    /// `launch_registration_endpoint` of its discovery document, and the
    /// client assertion's sole `aud` is the issuer.
    pub issuer: String,
    /// The viewer's `client_id` at FerroSMART: a `confidential_asymmetric`
    /// client whose scopes list `system/launch.c`.
    pub client_id: String,
    /// The PEM private key that signs the client assertion, RSA (RS384) or
    /// P-384 (ES384), the two algorithms SMART has a server support.
    pub private_key_file: Option<PathBuf>,
    /// The `kid` FerroSMART knows the key by; empty: its RFC 7638 thumbprint.
    pub kid: String,
    /// The base an app gets as `iss`: the CDR's public openEHR base, one
    /// `[[ferrosmart.resource]]` of FerroSMART.
    pub resource: String,
    /// The EHR subject namespaces whose subject id is a FHIR `Patient` id,
    /// sent as `patient` beside `ehrId`.
    pub patient_namespaces: Vec<String>,
    /// The FHIR R4 base of the facade that serves the SMART launch Tasks,
    /// one `[[ferrosmart.resource]]` paired with `resource`; empty, the
    /// viewer lists no Task.
    pub fhir_base: String,
    /// How long one request to FerroSMART may take.
    pub timeout: Duration,
    /// The trust roots beside the platform's and the client identity the
    /// connection to FerroSMART presents.
    pub tls: TlsClientSection,
    /// The apps an EHR's page offers to launch, in this order.
    pub app: Vec<LaunchApp>,
}

impl Default for LaunchSection {
    fn default() -> Self {
        Self {
            issuer: String::new(),
            client_id: String::new(),
            private_key_file: None,
            kid: String::new(),
            resource: String::new(),
            patient_namespaces: Vec::new(),
            fhir_base: String::new(),
            // NOTE: no specification governs this: our own design; one form post to a server
            // beside the viewer answers in well under ten seconds.
            timeout: Duration::from_secs(10),
            tls: TlsClientSection::default(),
            app: Vec::new(),
        }
    }
}

/// `[[launch.app]]`: one SMART app, a FerroSMART client of the
/// `authorization_code` grant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LaunchApp {
    /// The name the EHR's More menu draws.
    pub name: String,
    /// The app's `client_id` at FerroSMART, sent as `app_client_id`.
    pub client_id: String,
    /// The app's registered launch URL, which the browser opens with `iss`
    /// and `launch`.
    pub launch_url: String,
}

/// A `[launch]` section the viewer cannot start with.
#[derive(Debug, thiserror::Error)]
pub enum LaunchConfigError {
    /// A key the launch needs is empty.
    #[error("launch.{0} is required when launch.issuer is set")]
    Missing(&'static str),
    /// The issuer is no RFC 8414 §2 issuer identifier.
    #[error(
        "launch.issuer `{0}` is no issuer identifier (https, or http on a loopback host; RFC 8414 §2)"
    )]
    Issuer(String),
    /// A URL of the section is no absolute `http` or `https` URL.
    #[error("launch.{key} `{value}` is no absolute http or https URL")]
    Url {
        /// The key.
        key: &'static str,
        /// The value as configured.
        value: String,
    },
    /// Two apps name the same `client_id`.
    #[error("launch.app names the client `{0}` twice")]
    DuplicateApp(String),
    /// The key file could not be read.
    #[error("launch.private_key_file `{path}` could not be read")]
    KeyFile {
        /// The path.
        path: PathBuf,
        /// What reading it reported.
        #[source]
        source: std::io::Error,
    },
    /// The key is no key the assertion can be signed with.
    #[error("launch.private_key_file holds no key a client assertion is signed with")]
    Key(#[source] AssertionError),
    /// The key signs with neither RS384 nor ES384.
    #[error("launch.private_key_file signs with {0:?}; SMART asks RS384 or ES384")]
    Algorithm(Algorithm),
    /// The connection to FerroSMART could not be built.
    #[error("the connection to FerroSMART could not be built")]
    Client(#[source] OutboundError),
}

/// A launch that did not get its `launch` value.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// The post named an app `[launch]` does not list.
    #[error("no configured app has the client id `{0}`")]
    UnknownApp(String),
    /// The client assertion could not be signed.
    #[error("the client assertion could not be signed")]
    Assertion(#[source] AssertionError),
    /// FerroSMART could not be reached.
    #[error("FerroSMART could not be reached")]
    Unreachable(#[source] reqwest::Error),
    /// FerroSMART refused the launch with an RFC 6749 §5.2 error.
    #[error("FerroSMART answered {status} {error}: {description}")]
    Refused {
        /// The status it answered.
        status: http::StatusCode,
        /// The `error` code, empty when its body named none.
        error: String,
        /// The `error_description`, empty when its body carried none.
        description: String,
    },
    /// FerroSMART answered `201` with a body that is no launch.
    #[error("FerroSMART's answer carries no launch value")]
    Answer(#[source] serde_json::Error),
    /// FerroSMART's discovery document answered no
    /// `launch_registration_endpoint` the viewer may post to.
    #[error("FerroSMART's discovery document names no usable launch_registration_endpoint")]
    NoEndpoint,
}

/// The form of `POST {issuer}/launch`, its parameters in this order: the
/// client assertion as at the token endpoint, the app, the base and the
/// context (FerroSMART `docs/architecture.md` §13.6 item 3).
#[derive(Serialize)]
struct LaunchForm<'a> {
    /// The assertion type, RFC 7523 §2.2.
    client_assertion_type: &'a str,
    /// The viewer's signed assertion.
    client_assertion: &'a str,
    /// The app launched.
    app_client_id: &'a str,
    /// The base the app gets as `iss`.
    resource: &'a str,
    /// The EHR on screen.
    #[serde(rename = "ehrId")]
    ehr_id: &'a str,
    /// The FHIR `Patient` id of its subject, for a configured namespace.
    #[serde(skip_serializing_if = "Option::is_none")]
    patient: Option<&'a str>,
    /// Always `false`: the viewer shows whose record it is.
    need_patient_banner: bool,
}

impl std::fmt::Debug for LaunchForm<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchForm")
            .field("app_client_id", &self.app_client_id)
            .finish_non_exhaustive()
    }
}

/// The form that registers the launch of a SMART launch Task's app: the
/// app by its launch URL, the facade's base, and the Task's context
/// (FerroSMART `docs/architecture.md` §13.6 item 3; SMART App Launch 2.2.0,
/// Task profile for requesting a SMART launch).
#[derive(Serialize)]
struct TaskLaunchForm<'a> {
    /// The assertion type, RFC 7523 §2.2.
    client_assertion_type: &'a str,
    /// The viewer's signed assertion.
    client_assertion: &'a str,
    /// The app's configured launch URL, which the Task's URL matched.
    app_launch_uri: &'a str,
    /// The facade's base, the app's `iss`.
    resource: &'a str,
    /// The Task's `for`, a FHIR `Patient` id.
    #[serde(skip_serializing_if = "Option::is_none")]
    patient: Option<&'a str>,
    /// The Task's `encounter`, a FHIR `Encounter` id.
    #[serde(skip_serializing_if = "Option::is_none")]
    encounter: Option<&'a str>,
    /// The Task's `smartonfhir-appcontext` input, unchanged.
    #[serde(rename = "appContext", skip_serializing_if = "Option::is_none")]
    app_context: Option<&'a str>,
    /// Always `false`: the viewer shows whose record it is.
    need_patient_banner: bool,
}

impl std::fmt::Debug for TaskLaunchForm<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskLaunchForm")
            .field("app_launch_uri", &self.app_launch_uri)
            .finish_non_exhaustive()
    }
}

/// The context a SMART launch Task carries into its app's launch.
#[derive(Debug, Clone, Copy)]
pub struct TaskContext<'a> {
    /// The Task's patient, a FHIR `Patient` id.
    pub patient: Option<&'a str>,
    /// The Task's encounter, a FHIR `Encounter` id.
    pub encounter: Option<&'a str>,
    /// The Task's application context.
    pub app_context: Option<&'a str>,
}

/// The member of FerroSMART's discovery document that names its launch
/// API (FerroSMART `docs/architecture.md` §13.6 item 3).
#[derive(Deserialize)]
struct Discovered {
    /// The endpoint.
    launch_registration_endpoint: String,
}

/// The answer of `POST {issuer}/launch`.
#[derive(Deserialize)]
struct Registered {
    /// The `launch` value.
    launch: String,
}

impl std::fmt::Debug for Registered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registered").finish_non_exhaustive()
    }
}

/// The RFC 6749 §5.2 body of a refusal.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Refusal {
    /// The error code.
    error: String,
    /// The description.
    error_description: String,
}

/// The EHR whose context a launch carries.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// The EHR's id, sent as `ehrId`.
    pub ehr_id: &'a str,
    /// The `EHR_STATUS` subject's external reference id.
    pub subject: &'a str,
    /// Its namespace.
    pub subject_namespace: &'a str,
}

/// The viewer's EHR client at FerroSMART, built once from `[launch]`.
#[derive(Clone)]
pub struct Launcher {
    /// The connection to FerroSMART.
    http: reqwest::Client,
    /// The key that signs the client assertion.
    key: AssertionKey,
    /// The issuer identifier, the assertion's `aud`.
    issuer: String,
    /// `{issuer}/.well-known/smart-configuration`.
    discovery: url::Url,
    /// The `launch_registration_endpoint` it names, read once.
    endpoint: tokio::sync::OnceCell<url::Url>,
    /// The facade's FHIR base, when the viewer lists SMART launch Tasks.
    fhir_base: Option<String>,
    /// The viewer's `client_id`.
    client_id: String,
    /// The base the apps get as `iss`.
    resource: String,
    /// The namespaces whose subject is sent as `patient`.
    patient_namespaces: Vec<String>,
    /// The apps, each with its parsed launch URL.
    apps: Vec<(LaunchApp, url::Url)>,
}

impl std::fmt::Debug for Launcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Launcher")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("resource", &self.resource)
            .field("apps", &self.apps.len())
            .finish_non_exhaustive()
    }
}

/// Parses `value`, the `key` of the section, as an absolute `http` or
/// `https` URL.
fn web_url(key: &'static str, value: &str) -> Result<url::Url, LaunchConfigError> {
    let refused = || LaunchConfigError::Url {
        key,
        value: value.to_owned(),
    };
    let url = url::Url::parse(value).map_err(|_unparsed| refused())?;
    if matches!(url.scheme(), "http" | "https") && url.has_host() {
        Ok(url)
    } else {
        Err(refused())
    }
}

impl Launcher {
    /// The launcher `section` describes, or `None` when its `issuer` is
    /// empty and the viewer offers no launch.
    ///
    /// # Errors
    ///
    /// [`LaunchConfigError`] for an issuer that is no RFC 8414 issuer, an
    /// empty `client_id`, key file or `resource`, no app, an app with no
    /// name or no absolute launch URL, two apps naming one client, a key
    /// that does not read or signs with neither RS384 nor ES384, or a
    /// connection that cannot be built.
    pub fn from_section(section: &LaunchSection) -> Result<Option<Self>, LaunchConfigError> {
        if section.issuer.is_empty() {
            return Ok(None);
        }
        let issuer = ferrohealth_smart::metadata::issuer_url(&section.issuer)
            .map_err(|_insecure| LaunchConfigError::Issuer(section.issuer.clone()))?;
        // NOTE: SMART App Launch 2.2.0, Conformance, Metadata: the document lives at
        // `{issuer}/.well-known/smart-configuration`.
        let discovery = web_url(
            "issuer",
            &format!(
                "{}/.well-known/smart-configuration",
                issuer.as_str().trim_end_matches('/')
            ),
        )?;
        if section.client_id.is_empty() {
            return Err(LaunchConfigError::Missing("client_id"));
        }
        if section.resource.is_empty() {
            return Err(LaunchConfigError::Missing("resource"));
        }
        web_url("resource", &section.resource)?;
        if !section.fhir_base.is_empty() {
            web_url("fhir_base", &section.fhir_base)?;
        }
        if section.app.is_empty() {
            return Err(LaunchConfigError::Missing("app"));
        }
        let mut apps: Vec<(LaunchApp, url::Url)> = Vec::with_capacity(section.app.len());
        for app in &section.app {
            if app.name.is_empty() {
                return Err(LaunchConfigError::Missing("app.name"));
            }
            if app.client_id.is_empty() {
                return Err(LaunchConfigError::Missing("app.client_id"));
            }
            if apps.iter().any(|(seen, _)| seen.client_id == app.client_id) {
                return Err(LaunchConfigError::DuplicateApp(app.client_id.clone()));
            }
            let url = web_url("app.launch_url", &app.launch_url)?;
            apps.push((app.clone(), url));
        }
        let path = section
            .private_key_file
            .clone()
            .ok_or(LaunchConfigError::Missing("private_key_file"))?;
        let pem = std::fs::read(&path).map_err(|source| LaunchConfigError::KeyFile {
            path: path.clone(),
            source,
        })?;
        let kid = (!section.kid.is_empty()).then(|| section.kid.clone());
        let key = AssertionKey::from_pem(&pem, None, kid).map_err(LaunchConfigError::Key)?;
        // NOTE: SMART App Launch 2.2.0, Client Authentication: Asymmetric: a server
        // SHALL support RS384 and ES384, so the viewer signs with no other.
        if !matches!(key.algorithm(), Algorithm::RS384 | Algorithm::ES384) {
            return Err(LaunchConfigError::Algorithm(key.algorithm()));
        }
        let connection = EndpointSection {
            url: SecretUrl::new(discovery.as_str()),
            timeout: section.timeout,
            tls: section.tls.clone(),
            ..EndpointSection::default()
        };
        let http = OutboundClient::build(&connection, "launch", &ServerTls)
            .map_err(LaunchConfigError::Client)?
            .http()
            .clone();
        Ok(Some(Self {
            http,
            key,
            issuer: section.issuer.clone(),
            discovery,
            endpoint: tokio::sync::OnceCell::new(),
            fhir_base: (!section.fhir_base.is_empty()).then(|| section.fhir_base.clone()),
            client_id: section.client_id.clone(),
            resource: section.resource.clone(),
            patient_namespaces: section.patient_namespaces.clone(),
            apps,
        }))
    }

    /// The facade's FHIR base, when the viewer lists SMART launch Tasks.
    #[must_use]
    pub fn fhir_base(&self) -> Option<&str> {
        self.fhir_base.as_deref()
    }

    /// Whether an EHR whose subject is in `namespace` names a FHIR
    /// `Patient` by its subject id.
    #[must_use]
    pub fn names_patients(&self, namespace: &str) -> bool {
        self.patient_namespaces
            .iter()
            .any(|configured| configured == namespace)
    }

    /// The connection to FerroSMART and the facade.
    #[must_use]
    pub const fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// The configured app whose launch URL is `launch_url`, compared after
    /// the RFC 3986 §6.2.2 normalization FerroSMART compares a registered
    /// launch URL by, or `None` for an app the viewer does not launch.
    #[must_use]
    pub fn app_launched_at(&self, launch_url: &str) -> Option<&LaunchApp> {
        let wanted = ferrohealth_smart::uri::normalized(launch_url)?;
        self.apps
            .iter()
            .find(|(app, _)| {
                ferrohealth_smart::uri::normalized(&app.launch_url).as_deref()
                    == Some(wanted.as_str())
            })
            .map(|(app, _)| app)
    }

    /// The apps an EHR's page offers, in configured order.
    pub fn apps(&self) -> impl Iterator<Item = &LaunchApp> {
        self.apps.iter().map(|(app, _)| app)
    }

    /// The origins of the apps' launch URLs, each once, which the pages'
    /// `form-action` names beside `'self'`: a launch post is answered with a
    /// `303` to the app, and the policy covers a redirect after a form
    /// submission (CSP Level 3 §6.4.1, `form-action`).
    #[must_use]
    pub fn form_action_origins(&self) -> Vec<String> {
        let mut origins: Vec<String> = self
            .apps
            .iter()
            .map(|(_, url)| url.origin().ascii_serialization())
            .collect();
        origins.sort();
        origins.dedup();
        origins
    }

    /// The form of one launch request.
    fn form<'a>(
        &'a self,
        app: &'a LaunchApp,
        assertion: &'a str,
        context: Context<'a>,
    ) -> LaunchForm<'a> {
        let patient = (!context.subject.is_empty()
            && self
                .patient_namespaces
                .iter()
                .any(|namespace| namespace == context.subject_namespace))
        .then_some(context.subject);
        LaunchForm {
            client_assertion_type: ASSERTION_TYPE,
            client_assertion: assertion,
            app_client_id: &app.client_id,
            resource: &self.resource,
            ehr_id: context.ehr_id,
            patient,
            need_patient_banner: false,
        }
    }

    /// Registers the context of `context` at FerroSMART for the app
    /// `app_client_id` and returns the app's launch URL with `iss` and the
    /// fresh `launch` value.
    ///
    /// The context is `ehrId` always, `patient` when the subject's namespace
    /// is one of `patient_namespaces`, and `need_patient_banner=false`, since
    /// the viewer already shows whose record it is (SMART App Launch 2.2.0,
    /// Launch context arrives with your `access_token`). The value is never
    /// logged.
    ///
    /// # Errors
    ///
    /// [`LaunchError::UnknownApp`] for an app `[launch]` does not list,
    /// [`LaunchError::Assertion`] when the assertion cannot be signed,
    /// [`LaunchError::Unreachable`] when FerroSMART cannot be reached,
    /// [`LaunchError::Refused`] for any answer but `201`,
    /// [`LaunchError::Answer`] for a `201` that carries no launch value, and
    /// [`LaunchError::NoEndpoint`] when FerroSMART's discovery document
    /// names no launch API.
    pub async fn launch(
        &self,
        app_client_id: &str,
        context: Context<'_>,
    ) -> Result<url::Url, LaunchError> {
        let (app, launch_url) = self
            .apps
            .iter()
            .find(|(app, _)| app.client_id == app_client_id)
            .ok_or_else(|| LaunchError::UnknownApp(app_client_id.to_owned()))?;
        let assertion = self.assertion()?;
        let launch = self.register(&self.form(app, &assertion, context)).await?;
        let mut url = launch_url.clone();
        url.query_pairs_mut()
            .append_pair("iss", &self.resource)
            .append_pair("launch", &launch);
        Ok(url)
    }

    /// Registers the launch of the configured `app` a SMART launch Task
    /// named, with the Task's `context`, at the facade's base, and returns
    /// the app's configured launch URL with `iss` and the fresh `launch`
    /// value; the URL is never one the Task carries (SMART App Launch
    /// 2.2.0, Task profile for requesting a SMART launch).
    ///
    /// # Errors
    ///
    /// As [`Launcher::launch`], and [`LaunchError::NoEndpoint`] when the
    /// viewer lists no Task.
    pub async fn launch_task(
        &self,
        app: &LaunchApp,
        context: TaskContext<'_>,
    ) -> Result<url::Url, LaunchError> {
        let base = self.fhir_base.as_deref().ok_or(LaunchError::NoEndpoint)?;
        let launch_url = self
            .apps
            .iter()
            .find(|(configured, _)| configured == app)
            .map(|(_, url)| url)
            .ok_or_else(|| LaunchError::UnknownApp(app.client_id.clone()))?;
        let assertion = self.assertion()?;
        let form = TaskLaunchForm {
            client_assertion_type: ASSERTION_TYPE,
            client_assertion: &assertion,
            app_launch_uri: &app.launch_url,
            resource: base,
            patient: context.patient,
            encounter: context.encounter,
            app_context: context.app_context,
            need_patient_banner: false,
        };
        let launch = self.register(&form).await?;
        let mut url = launch_url.clone();
        url.query_pairs_mut()
            .append_pair("iss", base)
            .append_pair("launch", &launch);
        Ok(url)
    }

    /// The viewer's client assertion to FerroSMART.
    fn assertion(&self) -> Result<String, LaunchError> {
        self.key
            .assertion(&self.client_id, &self.issuer, ASSERTION_TYP)
            .map_err(LaunchError::Assertion)
    }

    /// The `launch_registration_endpoint` of FerroSMART's discovery
    /// document, read once and then kept.
    async fn endpoint(&self) -> Result<&url::Url, LaunchError> {
        self.endpoint
            .get_or_try_init(|| async {
                let response = self
                    .http
                    .get(self.discovery.clone())
                    .header(http::header::ACCEPT, "application/json")
                    .send()
                    .await
                    .map_err(LaunchError::Unreachable)?;
                if response.status() != http::StatusCode::OK {
                    return Err(LaunchError::NoEndpoint);
                }
                let body = response.bytes().await.map_err(LaunchError::Unreachable)?;
                let discovered: Discovered =
                    serde_json::from_slice(&body).map_err(|_unread| LaunchError::NoEndpoint)?;
                // NOTE: RFC 8414 §2: an endpoint of the issuer is https, or http on a
                // loopback host, as the issuer itself.
                ferrohealth_smart::metadata::issuer_url(&discovered.launch_registration_endpoint)
                    .map_err(|_insecure| LaunchError::NoEndpoint)
            })
            .await
    }

    /// Posts `form` to FerroSMART's launch API and returns the `launch`
    /// value it answers.
    async fn register<F: Serialize>(&self, form: &F) -> Result<String, LaunchError> {
        let endpoint = self.endpoint().await?;
        let response = self
            .http
            .post(endpoint.clone())
            .header(http::header::ACCEPT, "application/json")
            .form(form)
            .send()
            .await
            .map_err(LaunchError::Unreachable)?;
        let status = response.status();
        let body = response.bytes().await.map_err(LaunchError::Unreachable)?;
        if status != http::StatusCode::CREATED {
            // NOTE: RFC 9126 §2.3: a refusal is the RFC 6749 §5.2 body; a body that is none
            // still refuses, with the status alone.
            let refusal: Refusal = serde_json::from_slice(&body).unwrap_or_default();
            return Err(LaunchError::Refused {
                status,
                error: refusal.error,
                description: refusal.error_description,
            });
        }
        let registered: Registered = serde_json::from_slice(&body).map_err(LaunchError::Answer)?;
        Ok(registered.launch)
    }
}

#[cfg(test)]
mod tests {
    use super::{LaunchApp, LaunchConfigError, LaunchSection, Launcher};

    fn section(issuer: &str) -> LaunchSection {
        LaunchSection {
            issuer: issuer.to_owned(),
            client_id: "ferroehr-viewer".to_owned(),
            resource: "https://ehr.example.org/ferroehr/rest/openehr/v1".to_owned(),
            app: vec![LaunchApp {
                name: "Growth chart".to_owned(),
                client_id: "growth-chart".to_owned(),
                launch_url: "https://apps.example.org/growth/launch".to_owned(),
            }],
            ..LaunchSection::default()
        }
    }

    #[test]
    fn an_empty_issuer_offers_no_launch() {
        assert!(matches!(
            Launcher::from_section(&LaunchSection::default()),
            Ok(None)
        ));
    }

    #[test]
    fn an_issuer_needs_its_client_its_key_its_resource_and_an_app() {
        let insecure = Launcher::from_section(&section("http://smart.example.org"));
        assert!(matches!(insecure, Err(LaunchConfigError::Issuer(_))));
        let no_key = Launcher::from_section(&section("https://smart.example.org"));
        assert!(matches!(
            no_key,
            Err(LaunchConfigError::Missing("private_key_file"))
        ));
        let mut no_app = section("https://smart.example.org");
        no_app.app.clear();
        assert!(matches!(
            Launcher::from_section(&no_app),
            Err(LaunchConfigError::Missing("app"))
        ));
        let mut twice = section("https://smart.example.org");
        twice
            .app
            .push(twice.app.first().cloned().unwrap_or_default());
        assert!(matches!(
            Launcher::from_section(&twice),
            Err(LaunchConfigError::DuplicateApp(client)) if client == "growth-chart"
        ));
        let mut relative = section("https://smart.example.org");
        if let Some(app) = relative.app.first_mut() {
            app.launch_url = "/launch".to_owned();
        }
        assert!(matches!(
            Launcher::from_section(&relative),
            Err(LaunchConfigError::Url {
                key: "app.launch_url",
                ..
            })
        ));
    }
}
