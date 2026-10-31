// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reading an `[auth]` issuer at boot: the refusal of an issuer that could not
//! be read, and the `[smart] issuer_metadata_file` document.

use ferrohealth_smart::metadata::{DiscoveryError, ServerMetadata, parse_metadata};
use ferrohealth_smart::resource::ResourceError;

/// The boot refusal of an issuer whose metadata or key set could not be read.
///
/// A key set that does not parse is named by its JSON failure class and
/// position, never by the parser's message, which quotes the offending value;
/// every other failure carries its whole cause chain.
pub(super) fn unreadable_issuer(issuer: &str, error: &ResourceError) -> String {
    match error {
        ResourceError::KeySet { error: json, .. } => format!(
            "the authorization server {issuer} could not be read: {error}: {:?} error at \
             line {}, column {}",
            json.classify(),
            json.line(),
            json.column()
        ),
        _ => format!(
            "the authorization server {issuer} could not be read: {}",
            ferroehr::service::error::ErrorChain::new(error)
        ),
    }
}

/// Reads the RFC 8414 §2 authorization-server metadata at `path`
/// (`[smart] issuer_metadata_file`) through the shared parse, which refuses a
/// repeated member name (RFC 8259 §4); `ResourceServer::with_metadata` then
/// refuses another issuer (RFC 8414 §3.3).
///
/// A document that does not parse is named by its JSON failure class and
/// position, never by the parser's message, which quotes the input.
///
/// # Errors
/// A message naming the file that could not be read or parsed.
pub(super) async fn issuer_metadata(path: &str) -> Result<ServerMetadata, String> {
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("smart.issuer_metadata_file {path} could not be read: {e}"))?;
    let absolute = std::path::absolute(path)
        .map_err(|e| format!("smart.issuer_metadata_file {path} could not be resolved: {e}"))?;
    let location = url::Url::from_file_path(&absolute)
        .map_err(|()| format!("smart.issuer_metadata_file {path} is not a file path"))?;
    parse_metadata(&location, &text).map_err(|e| {
        let position = match &e {
            DiscoveryError::Metadata { source, .. }
            | DiscoveryError::RepeatedName { source, .. } => {
                format!(
                    ": {:?} error at line {}, column {}",
                    source.classify(),
                    source.line(),
                    source.column()
                )
            }
            _ => String::new(),
        };
        format!("smart.issuer_metadata_file {path} is refused: {e}{position}")
    })
}
