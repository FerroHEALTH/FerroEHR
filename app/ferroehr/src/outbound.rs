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

use ferrohealth_server::tls_client::TlsClient;
use ferrohealth_smart::config::endpoint::EndpointSection;

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
}
