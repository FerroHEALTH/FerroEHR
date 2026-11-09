// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reading an `[auth]` issuer at boot: the refusal of an issuer that could not
//! be read, and the `[smart] issuer_metadata_file` document.

use ferrohealth_smart::metadata::{ServerMetadata, parse_metadata};
use ferrohealth_smart::resource::ResourceError;

/// The boot refusal of an issuer whose metadata or key set could not be read,
/// with its whole cause chain: a key set or a document that does not parse is
/// named by the crate's refusal, its JSON failure class and position, which
/// quotes no part of it.
pub(super) fn unreadable_issuer(issuer: &str, error: &ResourceError) -> String {
    format!(
        "the authorization server {issuer} could not be read: {}",
        ferroehr::error::ErrorChain::new(error)
    )
}

/// Reads the RFC 8414 §2 authorization-server metadata at `path`
/// (`[smart] issuer_metadata_file`) through the shared parse, which refuses a
/// repeated member name (RFC 8259 §4); `ResourceServer::with_metadata` then
/// refuses another issuer (RFC 8414 §3.3).
///
/// A document that does not parse is named by the crate's refusal, its JSON
/// failure class and position, which quotes no part of the input.
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
        format!(
            "smart.issuer_metadata_file {path} is refused: {}",
            ferroehr::error::ErrorChain::new(&e)
        )
    })
}
