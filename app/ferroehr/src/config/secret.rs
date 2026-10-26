// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The URL mask every rendering of the configuration tree applies to a value
//! it did not type as a secret.
//!
//! The secret types themselves are `ferrohealth-secret`'s `Secret` and
//! `SecretUrl`, the family's one mask: a secret renders as `***`, and a URL
//! with its userinfo as `***@` and its query as `?***`. This module adds the
//! one rule the configuration tree needs beside them: a string leaf that
//! holds a URL is masked the same way, and any other string is left as
//! written. No openEHR spec governs configuration: our own design.

use ferrohealth_secret::secret_url::RedactedUrl;

/// Returns `text` masked as `ferrohealth-secret` masks a URL when it holds
/// one (a `://`), and unchanged otherwise.
///
/// `RedactedUrl` masks text with no `://` whole, the rule for a libpq
/// key/value string; a configuration rendering passes every string leaf
/// through here, so a plain value such as a log filter keeps its text.
#[must_use]
pub fn redact_userinfo(text: &str) -> String {
    if text.contains("://") {
        RedactedUrl(text).to_string()
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::redact_userinfo;

    #[test]
    fn a_url_is_masked_and_any_other_text_is_left_as_written() {
        assert_eq!(
            redact_userinfo("postgres://u:p@db:5432/ferroehr?sslmode=require"),
            "postgres://***@db:5432/ferroehr?***"
        );
        assert_eq!(
            redact_userinfo("https://host/path/@handle"),
            "https://host/path/@handle"
        );
        assert_eq!(redact_userinfo("info,ferroehr=info"), "info,ferroehr=info");
        assert_eq!(redact_userinfo("not a url"), "not a url");
    }

    /// GHSA-vphj-3f8c-5hpx: a URL no parser accepts still has its userinfo
    /// masked, so a credential never reaches a rendering whole.
    #[test]
    fn an_unparsable_url_is_masked_too() {
        let shown = redact_userinfo("postgres://u:p[x@db/ferroehr");
        assert!(!shown.contains("p[x"), "{shown}");
        assert!(shown.contains("***@db"), "{shown}");
    }
}
