// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The boot rules of one outbound integration's `[<x>.endpoint]` table, the
//! family's `EndpointSection`.
//!
//! Each integration builds its client with `ferrohealth-smart`'s
//! `client::outbound::OutboundClient`, which refuses a table at its first
//! broken rule; [`problems`] lists every rule a table breaks, so a refused
//! configuration names them all at once. No openEHR spec governs an outbound
//! client: our own design.

use ferrohealth_config::tls::TlsClientSection;
use ferrohealth_server::tls_client::{TlsClient, TlsClientError};
use ferrohealth_smart::config::endpoint::EndpointSection;

use crate::config::FerroEhrConfig;

/// Every rule the table at the dotted `key` breaks, as boot refusals.
///
/// The rules: the section's own, an `http` or `https` URL, TLS material that
/// reads, no onward exchange, one credential at most, a grant the client
/// credentials rules admit, and no credential in the URL beside one in
/// `credentials`.
#[must_use]
pub fn problems(key: &str, section: &EndpointSection) -> Vec<String> {
    let mut problems = Vec::new();
    match section.validate() {
        Ok(url) if matches!(url.scheme(), "http" | "https") => {
            let userinfo = !url.username().is_empty() || url.password().is_some();
            let credential =
                !section.credentials.user.is_empty() || section.credentials.oauth.is_some();
            if userinfo && credential {
                problems.push(format!(
                    "{key}.url carries credentials and so does [{key}.credentials]; \
                     authenticate one way: remove the userinfo or the credential"
                ));
            }
        }
        Ok(_) => problems.push(format!("{key}.url must be an http or https URL")),
        Err(error) => problems.push(format!("{key}: {error}")),
    }
    if let Err(error) = TlsClient::read(&format!("{key}.tls"), &section.tls) {
        problems.push(error.to_string());
    }
    let credentials = &section.credentials;
    if credentials.onward.is_some() {
        problems.push(format!(
            "{key}.credentials.onward is not read by this integration: it exchanges no caller \
             token"
        ));
    }
    if !credentials.user.is_empty() && credentials.oauth.is_some() {
        problems.push(format!(
            "{key}.credentials sets user and oauth; authenticate one way"
        ));
    }
    if let Some(oauth) = &credentials.oauth {
        problems.extend(oauth.problems(&format!("{key}.credentials.oauth")));
    }
    problems
}

/// Every outbound TLS table of `config`, by its dotted key: the collector,
/// the two audit sinks, the terminology server, the patient identity sources,
/// their token endpoints, and the issuers of `[auth]`.
fn tls_tables(config: &FerroEhrConfig) -> Vec<(String, &TlsClientSection)> {
    let mut tables = vec![("auth.tls".to_owned(), &config.auth.shared.tls)];
    if let Some(otlp) = &config.telemetry.otlp {
        tables.push(("telemetry.otlp.tls".to_owned(), &otlp.tls));
    }
    if let Some(syslog) = &config.audit.syslog {
        tables.push(("audit.syslog.tls".to_owned(), &syslog.tls));
    }
    let endpoints = [
        (
            "audit.fhir_feed.endpoint",
            config.audit.fhir_feed.as_ref().map(|f| &f.endpoint),
        ),
        (
            "terminology.endpoint",
            config.terminology.as_ref().map(|t| &t.endpoint),
        ),
        ("pixm.endpoint", config.pixm.as_ref().map(|p| &p.endpoint)),
        ("pix.endpoint", config.pix.as_ref().map(|p| &p.endpoint)),
    ];
    for (key, endpoint) in endpoints {
        let Some(endpoint) = endpoint else { continue };
        tables.push((format!("{key}.tls"), &endpoint.tls));
        if let Some(oauth) = &endpoint.credentials.oauth {
            tables.push((format!("{key}.credentials.oauth.tls"), &oauth.tls));
        }
    }
    tables
}

/// The roots every outbound TLS table of `config` trusts with no revocation
/// check (`trust_roots_without_revocation_file`), one line per table naming
/// its key and the roots' subjects, for the start to name.
///
/// # Errors
/// The [`TlsClientError`] of a table naming such roots whose material cannot
/// be read or is refused.
pub fn unchecked_roots(config: &FerroEhrConfig) -> Result<Vec<String>, TlsClientError> {
    let mut lines = Vec::new();
    for (key, section) in tls_tables(config) {
        if section.trust_roots_without_revocation_file.is_some() {
            let platform = TlsClient::read(&key, section)?.platform_config()?;
            lines.extend(platform.unchecked_roots());
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use ferrohealth_smart::client::outbound::OutboundClient;
    use ferrohealth_smart::config::ServerTls;

    use super::*;

    /// A table with no URL is refused naming the table.
    #[test]
    fn a_table_without_url_or_with_onward_is_refused() {
        let err = OutboundClient::build(&EndpointSection::default(), "pixm.endpoint", &ServerTls)
            .expect_err("no url");
        assert!(err.to_string().contains("pixm.endpoint"), "{err}");
        assert!(
            problems("pixm.endpoint", &EndpointSection::default())
                .iter()
                .any(|problem| problem.contains("pixm.endpoint")),
        );
    }

    /// A table naming `trust_roots_without_revocation_file` is named by its
    /// key and its roots' subject; one naming only `trust_roots_file` is not.
    #[tokio::test]
    async fn the_roots_trusted_with_no_revocation_check_are_named_by_table() {
        let listener = testkit::syslog_tls::SyslogTlsListener::bind(0)
            .await
            .expect("the listener mints its CA");
        let mut config = FerroEhrConfig::default();
        config.audit.syslog = Some(crate::system_log::config::SyslogConfig {
            tls: TlsClientSection {
                trust_roots_without_revocation_file: Some(listener.ca_file.clone()),
                ..TlsClientSection::default()
            },
            ..crate::system_log::config::SyslogConfig::default()
        });
        config.auth.shared.tls.trust_roots_file = Some(listener.ca_file.clone());
        let lines = unchecked_roots(&config).expect("the roots read");
        assert_eq!(1, lines.len(), "{lines:?}");
        assert!(
            lines[0].contains("audit.syslog.tls") && lines[0].contains("ferroehr-test-ca"),
            "{lines:?}"
        );
    }
}
