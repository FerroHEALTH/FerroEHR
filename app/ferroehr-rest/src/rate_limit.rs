// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Request-rate limiting — the two tiers described by
//! [`ferroehr::config::server::RateLimitConfig`].
//!
//! Built on `tower_governor` over `governor`'s GCRA cell-rate limiter
//! (<https://docs.rs/tower_governor/>), never hand-rolled. Both tiers render
//! their refusal through [`crate::overview::error`], so a `429` carries the
//! openEHR `{ error, message }` body like every other refusal, alongside the
//! `Retry-After` and `x-ratelimit-*` headers the limiter computed.
//!
//! `429` is not in the ITS-REST status table; it is admitted there as an
//! additional, non-conflicting code
//! (`specifications/docs/overview/Requests_and_responses.md` §HTTP status
//! codes) and is the status RFC 6585 §4 defines for this refusal. No openEHR
//! spec governs request rates — our own design.
//!
//! NOTE: the crate's `tracing` feature is deliberately off, which is what
//! removes its `KeyExtractor::name`/`key_name` hooks — a bucket key here is a
//! subject identifier or a client address, and neither belongs in telemetry.

use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use ferroehr::config::server::RateLimitConfig;
use governor::middleware::StateInformationMiddleware;
use http::Request;
use http::Response;
use http::StatusCode;
use tower_governor::GovernorLayer;
use tower_governor::errors::GovernorError;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;

use crate::extensions::access::authn::Principal;
use crate::overview::error::status_error_response;

/// The layer type both tiers produce: state-reporting middleware (the
/// `x-ratelimit-*` headers) over an axum response body.
type Tier<K> = GovernorLayer<K, StateInformationMiddleware, Body>;

/// Keys the clinical tier on the authenticated subject.
///
/// Runs inside the authentication layer, so [`Principal`] is in the request
/// extensions. A request that reaches here without one means authentication is
/// disabled altogether; those share a single bucket, which is the honest
/// behaviour — with no identity there is nothing fairer to key on, and the outer
/// address tier is still limiting them individually.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrincipalKey;

impl KeyExtractor for PrincipalKey {
    type Key = String;

    fn extract<T>(&self, request: &Request<T>) -> Result<Self::Key, GovernorError> {
        Ok(request
            .extensions()
            .get::<Principal>()
            .map_or_else(|| "-".to_owned(), |principal| principal.subject.clone()))
    }
}

/// Keys the outer tier on the peer address.
///
/// `ConnectInfo<SocketAddr>` is populated by the listener
/// ([`crate::serve_full`] serves with connect info). A request carrying none
/// shares one bucket rather than escaping the limiter: failing open on the key
/// would make the tier optional in precisely the case an attacker controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressKey;

impl KeyExtractor for AddressKey {
    type Key = IpAddr;

    fn extract<T>(&self, request: &Request<T>) -> Result<Self::Key, GovernorError> {
        Ok(request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |info| info.0.ip()))
    }
}

/// Returns the principal-keyed layer for the clinical API subtree, or `None`
/// when rate limiting is off.
#[must_use]
pub fn principal_layer(cfg: &RateLimitConfig) -> Option<Tier<PrincipalKey>> {
    tier(
        cfg,
        PrincipalKey,
        cfg.principal_per_second,
        cfg.principal_burst,
    )
}

/// Returns the address-keyed layer for the whole tree, or `None` when rate
/// limiting is off.
#[must_use]
pub fn address_layer(cfg: &RateLimitConfig) -> Option<Tier<AddressKey>> {
    tier(cfg, AddressKey, cfg.address_per_second, cfg.address_burst)
}

/// Builds one tier, or `None` when rate limiting is off.
fn tier<K: KeyExtractor>(
    cfg: &RateLimitConfig,
    key: K,
    per_second: u64,
    burst: u32,
) -> Option<Tier<K>> {
    if !cfg.enabled {
        return None;
    }
    let (per_second, burst) = sanitize(per_second, burst);
    let mut base = GovernorConfigBuilder::default();
    base.period(period_for(per_second)).burst_size(burst);
    let mut keyed = base.key_extractor(key);
    let conf = keyed.use_headers().finish()?;
    Some(GovernorLayer::new(conf).error_handler(refusal))
}

/// Renders a limiter refusal as the openEHR error body.
///
/// The message states the retry delay and nothing else — no key, no bucket
/// state, no internal type name (the OWASP REST Security Cheat Sheet's
/// error-hygiene control). The limiter's own headers are preserved, because they
/// are what makes a `429` actionable for a client.
fn refusal(error: GovernorError) -> Response<Body> {
    match error {
        GovernorError::TooManyRequests { wait_time, headers } => {
            let mut response = status_error_response(
                StatusCode::TOO_MANY_REQUESTS,
                &format!("request rate exceeded; retry in {wait_time} second(s)"),
            );
            if let Some(from_limiter) = headers {
                response.headers_mut().extend(from_limiter);
            }
            response
        }
        GovernorError::UnableToExtractKey => status_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "rate limiter could not classify the request",
        ),
        GovernorError::Other { code, .. } => {
            status_error_response(code, "request refused by the rate limiter")
        }
    }
}

/// Raises a configured pair to the smallest values the limiter accepts.
///
/// The builder answers `None` to a zero period or burst, which would silently
/// disable the tier — the opposite of what a typo in a security control should
/// do. One per second is loud but reachable.
const fn sanitize(per_second: u64, burst: u32) -> (u64, u32) {
    (
        if per_second == 0 { 1 } else { per_second },
        if burst == 0 { 1 } else { burst },
    )
}

/// The cell replenishment interval for a requested rate.
///
/// GCRA replenishes one cell per period, so N per second is a period of 1/N
/// seconds (<https://docs.rs/governor/latest/governor/>). Nanoseconds keep
/// rates above 1000/s expressible.
#[expect(
    clippy::integer_division,
    reason = "a replenishment interval is a whole number of nanoseconds; the \
              sub-nanosecond remainder is below the limiter's own clock resolution"
)]
const fn period_for(per_second: u64) -> Duration {
    Duration::from_nanos(1_000_000_000_u64 / per_second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_builds_no_layer() {
        let cfg = RateLimitConfig {
            enabled: false,
            ..RateLimitConfig::default()
        };
        assert!(principal_layer(&cfg).is_none());
        assert!(address_layer(&cfg).is_none());
    }

    #[test]
    fn enabled_builds_both_layers() {
        let cfg = RateLimitConfig::default();
        assert!(principal_layer(&cfg).is_some());
        assert!(address_layer(&cfg).is_some());
    }

    /// A zero rate must not silently disable the tier.
    #[test]
    fn zero_rates_are_raised_not_accepted() {
        assert_eq!(sanitize(0, 0), (1, 1));
        let cfg = RateLimitConfig {
            enabled: true,
            principal_per_second: 0,
            principal_burst: 0,
            address_per_second: 0,
            address_burst: 0,
        };
        assert!(principal_layer(&cfg).is_some());
        assert!(address_layer(&cfg).is_some());
    }

    /// The defaults sit above this implementation's own measured ceiling, so the
    /// limiter cannot refuse a caller the server could still have served — that
    /// boundary belongs to the load shed
    /// (`docs/conformance/ferroehr/stress.json` records 512 requests/second
    /// maximum sustainable throughput on the reference SUT).
    #[test]
    fn defaults_sit_above_the_measured_server_ceiling() {
        const MEASURED_MAX_SUSTAINABLE_PER_SECOND: u64 = 512;
        let cfg = RateLimitConfig::default();
        assert!(cfg.principal_per_second >= MEASURED_MAX_SUSTAINABLE_PER_SECOND * 2);
        assert!(cfg.address_per_second >= cfg.principal_per_second);
        assert!(u64::from(cfg.principal_burst) >= cfg.principal_per_second);
    }

    #[test]
    fn a_rate_becomes_its_replenishment_period() {
        assert_eq!(period_for(1), Duration::from_secs(1));
        assert_eq!(period_for(1000), Duration::from_millis(1));
        assert_eq!(period_for(1024).as_nanos(), 976_562);
    }
}
