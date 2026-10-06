// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The boot-time outcome of the `[licence]` section: which licence is in
//! force, and where it came from.
//!
//! Two licence classes exist ([`crate::licence::document::Use`]): the
//! `non-commercial` grant every build embeds, and a `commercial` grant a
//! licensee installs through `[licence] file`. A configured token that
//! verifies is in force; one that does not is recorded as refused and the
//! embedded grant stays in force, so an operator sees at once that their
//! token is not doing anything. The only way to run with no licence at all is
//! a build that embeds no usable token, which a unit test makes unshippable.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use jiff::civil::Date;
use pgp::packet::PublicKey;
use serde::Serialize;

use crate::licence::config::LicenceConfig;
use crate::licence::stamp::StampKey;
use crate::licence::token::{Token, TokenError};
use crate::licence::verify::{Verified, VerifyError, verify};

/// Why a token was not accepted.
///
/// Each variant keeps its cause: the boot log renders the whole chain, and a
/// caller can walk [`std::error::Error::source`] to the `io`, token or
/// verification error underneath. The causes sit behind `Arc` because a
/// [`LicenceState`] is cloned into every service that stamps identifiers.
#[derive(Debug, Clone)]
pub enum Reason {
    /// The configured file could not be read.
    Unreadable(Arc<std::io::Error>),
    /// The text is not a token file.
    Malformed(Arc<TokenError>),
    /// The token parsed but failed verification.
    Refused(Arc<VerifyError>),
    /// The build embeds no token at all.
    NoneEmbedded,
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(_) => f.write_str("file unreadable"),
            Self::Malformed(_) => f.write_str("not a licence token"),
            Self::Refused(_) => f.write_str("refused"),
            Self::NoneEmbedded => f.write_str("the build embeds no token"),
        }
    }
}

impl std::error::Error for Reason {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable(cause) => Some(cause.as_ref()),
            Self::Malformed(cause) => Some(cause.as_ref()),
            Self::Refused(cause) => Some(cause.as_ref()),
            Self::NoneEmbedded => None,
        }
    }
}

/// Write `err` and every cause under it, colon-separated, on one line.
fn write_chain(f: &mut fmt::Formatter<'_>, err: &dyn std::error::Error) -> fmt::Result {
    write!(f, "{err}")?;
    let mut cause = err.source();
    while let Some(err) = cause {
        write!(f, ": {err}")?;
        cause = err.source();
    }
    Ok(())
}

/// Where the licence in force came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The configured `[licence] file`.
    Configured,
    /// The token every build embeds.
    Embedded,
}

/// What the boot verification concluded.
#[derive(Debug, Clone)]
pub enum LicenceState {
    /// A licence is in force; the server mints with its stamp.
    Licensed {
        /// The verified token.
        verified: Verified,
        /// Where it came from.
        source: Source,
        /// Why the configured token, if one was set, is not the one in force.
        configured_failure: Option<Reason>,
    },
    /// No licence at all: the build embeds no usable token and no configured
    /// one verified. The server mints with the fail-safe stamp.
    NoLicence(Reason),
}

/// What the configured `[licence] file` contributed, as the status reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfiguredToken {
    /// No file configured.
    Absent,
    /// The configured token is the licence in force.
    InForce,
    /// The configured token was refused; the embedded grant is in force.
    Refused,
}

/// The summary served on `/management/status`. Never a refusal reason,
/// which can name a file system path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LicenceStatus {
    /// `licensed` or `none`.
    pub state: &'static str,
    /// `commercial` or `non-commercial`, when licensed.
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub permitted_use: Option<&'static str>,
    /// The licensee, when licensed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub licensee: Option<String>,
    /// Last valid day, when licensed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_after: Option<Date>,
    /// What the configured file contributed.
    pub configured_token: ConfiguredToken,
}

impl LicenceState {
    /// Verify the configured token, then the embedded one, as of `today`.
    ///
    /// Never fails: a configured token that does not verify is recorded and
    /// the embedded grant stays in force; only a build with no usable
    /// embedded token ends up with no licence.
    #[must_use]
    pub fn load(
        config: &LicenceConfig,
        embedded: &str,
        anchors: &[PublicKey],
        today: Date,
    ) -> Self {
        let configured = config
            .file
            .as_deref()
            .map(|path| Self::from_file(path, anchors, today));
        let configured_failure = match configured {
            Some(Ok(verified)) => {
                return Self::Licensed {
                    verified,
                    source: Source::Configured,
                    configured_failure: None,
                };
            }
            Some(Err(reason)) => Some(reason),
            None => None,
        };
        if embedded.trim().is_empty() {
            return Self::NoLicence(configured_failure.unwrap_or(Reason::NoneEmbedded));
        }
        match Self::from_text(embedded, anchors, today) {
            Ok(verified) => Self::Licensed {
                verified,
                source: Source::Embedded,
                configured_failure,
            },
            Err(reason) => Self::NoLicence(reason),
        }
    }

    /// [`Self::load`] as of today in UTC, the calendar every licence window
    /// is read in.
    #[must_use]
    pub fn load_now(config: &LicenceConfig, embedded: &str, anchors: &[PublicKey]) -> Self {
        let today = jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .date();
        Self::load(config, embedded, anchors, today)
    }

    fn from_file(path: &Path, anchors: &[PublicKey], today: Date) -> Result<Verified, Reason> {
        let text = std::fs::read_to_string(path).map_err(|e| Reason::Unreadable(Arc::new(e)))?;
        Self::from_text(&text, anchors, today)
    }

    fn from_text(text: &str, anchors: &[PublicKey], today: Date) -> Result<Verified, Reason> {
        let token = Token::parse(text).map_err(|e| Reason::Malformed(Arc::new(e)))?;
        verify(&token, anchors, today).map_err(|e| Reason::Refused(Arc::new(e)))
    }

    /// The stamp key this state mints identifiers with.
    #[must_use]
    pub fn stamp_key(&self) -> StampKey {
        match self {
            Self::Licensed { verified, .. } => StampKey::for_licence(verified.licence.id),
            Self::NoLicence(_) => StampKey::fail_safe(),
        }
    }

    /// The public summary.
    #[must_use]
    pub fn status(&self) -> LicenceStatus {
        match self {
            Self::Licensed {
                verified,
                source,
                configured_failure,
            } => LicenceStatus {
                state: "licensed",
                permitted_use: Some(verified.licence.permitted_use.as_str()),
                licensee: Some(verified.licence.licensee.clone()),
                not_after: Some(verified.licence.not_after),
                configured_token: match (source, configured_failure) {
                    (Source::Configured, _) => ConfiguredToken::InForce,
                    (Source::Embedded, Some(_)) => ConfiguredToken::Refused,
                    (Source::Embedded, None) => ConfiguredToken::Absent,
                },
            },
            Self::NoLicence(reason) => LicenceStatus {
                state: "none",
                permitted_use: None,
                licensee: None,
                not_after: None,
                configured_token: match reason {
                    Reason::NoneEmbedded => ConfiguredToken::Absent,
                    _ => ConfiguredToken::Refused,
                },
            },
        }
    }

    /// Whether a licence is in force.
    #[must_use]
    pub fn is_licensed(&self) -> bool {
        matches!(self, Self::Licensed { .. })
    }
}

impl fmt::Display for LicenceState {
    /// The one boot log line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Licensed {
                verified,
                source,
                configured_failure,
            } => {
                let origin = match source {
                    Source::Configured => "configured",
                    Source::Embedded => "embedded",
                };
                write!(
                    f,
                    "{} ({}, {origin}) until {}",
                    verified.licence.licensee,
                    verified.licence.permitted_use.as_str(),
                    verified.licence.not_after
                )?;
                if let Some(reason) = configured_failure {
                    f.write_str("; configured token not in force: ")?;
                    write_chain(f, reason)?;
                }
                Ok(())
            }
            Self::NoLicence(reason) => {
                f.write_str("none (")?;
                write_chain(f, reason)?;
                f.write_str(")")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use uuid::Uuid;

    use super::*;
    use crate::licence::document::{Licence, Use};
    use crate::licence::verify::fixtures::Issuer;

    fn licence(licensee: &str, permitted_use: Use) -> Licence {
        Licence {
            id: Uuid::now_v7(),
            licensee: licensee.to_owned(),
            issued: date(2026, 9, 11),
            not_before: date(2026, 9, 11),
            not_after: date(2027, 9, 10),
            permitted_use,
        }
    }

    fn temp_file(name: &str, text: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("ferroehr-licence-{}-{name}", Uuid::now_v7()));
        std::fs::write(&path, text).unwrap();
        path
    }

    fn status_json(state: &LicenceState) -> String {
        serde_json::to_string(&state.status()).unwrap()
    }

    #[test]
    fn a_build_without_an_embedded_token_has_no_licence_and_the_fail_safe_stamp() {
        let state = LicenceState::load(&LicenceConfig::default(), "", &[], date(2026, 12, 1));
        assert!(matches!(
            state,
            LicenceState::NoLicence(Reason::NoneEmbedded)
        ));
        assert!(!state.is_licensed());
        assert_eq!(
            status_json(&state),
            r#"{"state":"none","configured_token":"absent"}"#
        );
        assert!(state.stamp_key().carries(StampKey::fail_safe().mint()));
        assert_eq!(state.to_string(), "none (the build embeds no token)");
    }

    #[test]
    fn the_embedded_non_commercial_grant_is_the_default_licence() {
        let issuer = Issuer::generate();
        let free = licence("Everyone, under BUSL-1.1", Use::NonCommercial);
        let embedded = issuer.token_text(&free);
        let state = LicenceState::load(
            &LicenceConfig::default(),
            &embedded,
            &[issuer.primary()],
            date(2026, 12, 1),
        );
        assert!(matches!(
            state,
            LicenceState::Licensed {
                source: Source::Embedded,
                configured_failure: None,
                ..
            }
        ));
        assert_eq!(
            status_json(&state),
            r#"{"state":"licensed","use":"non-commercial","licensee":"Everyone, under BUSL-1.1","not_after":"2027-09-10","configured_token":"absent"}"#
        );
        assert!(StampKey::for_licence(free.id).carries(state.stamp_key().mint()));
        assert!(!StampKey::fail_safe().carries(state.stamp_key().mint()));
        assert_eq!(
            state.to_string(),
            "Everyone, under BUSL-1.1 (non-commercial, embedded) until 2027-09-10"
        );
    }

    #[test]
    fn a_configured_commercial_token_takes_precedence() {
        let issuer = Issuer::generate();
        let free = licence("Everyone, under BUSL-1.1", Use::NonCommercial);
        let paid = licence("Example Hospital NV", Use::Commercial);
        let embedded = issuer.token_text(&free);
        let config = LicenceConfig {
            file: Some(temp_file("paid.asc", &issuer.token_text(&paid))),
        };
        let state = LicenceState::load(&config, &embedded, &[issuer.primary()], date(2026, 12, 1));
        assert!(matches!(
            state,
            LicenceState::Licensed {
                source: Source::Configured,
                configured_failure: None,
                ..
            }
        ));
        let status = state.status();
        assert_eq!(status.permitted_use, Some("commercial"));
        assert_eq!(status.licensee.as_deref(), Some("Example Hospital NV"));
        assert_eq!(status.configured_token, ConfiguredToken::InForce);
        assert!(StampKey::for_licence(paid.id).carries(state.stamp_key().mint()));
        assert!(!StampKey::for_licence(free.id).carries(state.stamp_key().mint()));
        assert_eq!(
            state.to_string(),
            "Example Hospital NV (commercial, configured) until 2027-09-10"
        );
    }

    #[test]
    fn a_refused_configured_token_leaves_the_embedded_grant_in_force_and_says_so() {
        let issuer = Issuer::generate();
        // The embedded grant outlives the configured token, so the third case
        // below exercises an expired configured token against a live fallback.
        let mut free = licence("Everyone, under BUSL-1.1", Use::NonCommercial);
        free.not_after = date(2099, 12, 31);
        let embedded = issuer.token_text(&free);
        let anchors = [issuer.primary()];

        let cases = [
            (
                LicenceConfig {
                    file: Some("/nonexistent/licence.asc".into()),
                },
                date(2026, 12, 1),
                "file unreadable",
            ),
            (
                LicenceConfig {
                    file: Some(temp_file("junk", "hello\n")),
                },
                date(2026, 12, 1),
                "not a licence token",
            ),
            (
                LicenceConfig {
                    file: Some(temp_file(
                        "expired.asc",
                        &issuer.token_text(&licence("Example Hospital NV", Use::Commercial)),
                    )),
                },
                date(2027, 12, 1),
                "refused: licence expired",
            ),
        ];
        for (config, today, expected) in cases {
            let state = LicenceState::load(&config, &embedded, &anchors, today);
            assert!(
                matches!(
                    state,
                    LicenceState::Licensed {
                        source: Source::Embedded,
                        configured_failure: Some(_),
                        ..
                    }
                ),
                "{state}"
            );
            let status = state.status();
            assert_eq!(status.state, "licensed");
            assert_eq!(status.permitted_use, Some("non-commercial"));
            assert_eq!(status.configured_token, ConfiguredToken::Refused);
            assert!(
                state.to_string().contains(expected),
                "{state} should mention {expected}"
            );
            assert!(StampKey::for_licence(free.id).carries(state.stamp_key().mint()));
        }
    }

    #[test]
    fn a_stranger_cannot_supply_the_embedded_token() {
        let issuer = Issuer::generate();
        let stranger = Issuer::generate();
        let embedded = stranger.token_text(&licence("Everyone", Use::NonCommercial));
        let state = LicenceState::load(
            &LicenceConfig::default(),
            &embedded,
            &[issuer.primary()],
            date(2026, 12, 1),
        );
        assert!(matches!(state, LicenceState::NoLicence(Reason::Refused(_))));
        assert_eq!(
            status_json(&state),
            r#"{"state":"none","configured_token":"refused"}"#
        );
    }

    #[test]
    fn a_refusal_reason_keeps_its_typed_cause() {
        let issuer = Issuer::generate();
        let stranger = Issuer::generate();
        let embedded = stranger.token_text(&licence("Everyone", Use::NonCommercial));
        let state = LicenceState::load(
            &LicenceConfig::default(),
            &embedded,
            &[issuer.primary()],
            date(2026, 12, 1),
        );
        let LicenceState::NoLicence(reason) = state else {
            panic!("a stranger's token yields no licence, got {state}");
        };
        let cause = std::error::Error::source(&reason).expect("a refusal carries its cause");
        assert!(
            matches!(
                cause.downcast_ref::<VerifyError>(),
                Some(VerifyError::UntrustedPrimary(_))
            ),
            "the cause is the verification error itself, not a wrapper: {cause}"
        );

        let malformed = LicenceState::load(
            &LicenceConfig {
                file: Some(temp_file("junk-typed", "hello\n")),
            },
            &embedded,
            &[stranger.primary()],
            date(2026, 12, 1),
        );
        let LicenceState::Licensed {
            configured_failure: Some(reason),
            ..
        } = malformed
        else {
            panic!("the embedded grant stays in force, got {malformed}");
        };
        let cause =
            std::error::Error::source(&reason).expect("a malformed token carries its cause");
        assert!(
            cause.downcast_ref::<TokenError>().is_some(),
            "the cause is the token error itself: {cause}"
        );
    }
}
