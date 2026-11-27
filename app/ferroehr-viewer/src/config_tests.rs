// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer configuration's load, defaults and refusals.

use std::collections::BTreeMap;

use ferrohealth_config::tls::TlsVersion;

use super::{CdrConfig, ConfigError, ViewerConfig, load_sources};

/// The base path defaults to the CDR's own default, and the product root
/// derives from it by the CDR's rule: drop the trailing `v1` segment, and
/// an `openehr` segment directly before it when the deployment spells one.
#[test]
fn the_rest_root_derives_from_the_configured_base_path() {
    assert_eq!(
        CdrConfig::default().base_path,
        "/ferroehr/rest/openehr/v1",
        "the viewer default must mirror the CDR default"
    );
    for (base_path, expected) in [
        ("/ferroehr/rest/openehr/v1", "/ferroehr/rest"),
        ("/ferroehr/v1", "/ferroehr"),
        ("/ferroehr/openehr/v1", "/ferroehr"),
        ("/ferroehr/cdr/v1", "/ferroehr/cdr"),
        ("/ferroehr/v1/", "/ferroehr"),
    ] {
        let cfg = CdrConfig {
            base_path: base_path.to_owned(),
            ..CdrConfig::default()
        };
        assert_eq!(cfg.rest_root(), expected, "base_path {base_path}");
        assert_eq!(cfg.rest_base_path(), base_path.trim_end_matches('/'));
    }
}

#[test]
fn management_base_defaults_to_the_cdr_origin() {
    let cfg = CdrConfig::default();
    assert_eq!(cfg.management_base(), "http://localhost:8080/management");
}

#[test]
fn a_configured_management_base_wins_and_may_move_host_port_and_path() {
    // The whole prefix is configurable: a separate internal listener
    // (`management.listen` CDR-side) and a renamed `management.base_path`
    // are the same one knob here.
    let cfg = CdrConfig {
        management_base_url: "http://cdr.internal:9464/ops/".to_owned(),
        ..CdrConfig::default()
    };
    assert_eq!(cfg.management_base(), "http://cdr.internal:9464/ops");
}

/// `[telemetry]` is the family's section, and `RUST_LOG` sets its filter
/// below the canonical override, as on the CDR.
#[test]
fn the_telemetry_section_parses_and_rust_log_is_the_filter_fallback() {
    let parsed = load_sources(
        Some("[telemetry]\nformat = \"json\"\nfilter = \"warn\"\nlogged_query_parameters = [\"limit\"]\n"),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        parsed.telemetry.format,
        ferrohealth_telemetry::section::Format::Json
    );
    assert_eq!(parsed.telemetry.filter.as_str(), "warn");
    assert_eq!(parsed.telemetry.logged_query_parameters, ["limit"]);
    assert_eq!(
        ViewerConfig::default().telemetry.filter,
        ferrohealth_telemetry::section::Filter::default()
    );

    let rust_log = BTreeMap::from([("RUST_LOG".to_owned(), "debug".to_owned())]);
    let parsed = load_sources(Some("[telemetry]\nfilter = \"warn\"\n"), &rust_log).unwrap();
    assert_eq!(parsed.telemetry.filter.as_str(), "debug");

    let both = BTreeMap::from([
        ("RUST_LOG".to_owned(), "debug".to_owned()),
        (
            "FERROEHR_VIEWER__TELEMETRY__FILTER".to_owned(),
            "error".to_owned(),
        ),
    ]);
    assert_eq!(
        load_sources(None, &both).unwrap().telemetry.filter.as_str(),
        "error"
    );
}

/// A key the viewer's sections do not read is unknown and refuses the
/// start, naming the key, whatever release once read it.
#[test]
fn an_unknown_key_of_each_section_refuses_the_start() {
    for (text, key) in [
        ("[server]\nbody_limit_bytes = 1\n", "body_limit_bytes"),
        ("[oidc]\nca_file = \"x\"\n", "ca_file"),
        ("[session]\nidle_timeout_s = 60\n", "idle_timeout_s"),
        ("[cdr]\nrequest_timeout_secs = 30\n", "request_timeout_secs"),
    ] {
        let refused = load_sources(Some(text), &BTreeMap::new())
            .expect_err(text)
            .to_string();
        assert!(
            refused.contains(&format!("unknown field `{key}`")),
            "{text}: {refused}"
        );
    }
}

#[test]
fn a_log_filter_that_does_not_parse_refuses_the_start() {
    let refused = BTreeMap::from([("RUST_LOG".to_owned(), "info,=,,".to_owned())]);
    assert!(load_sources(None, &refused).is_err());
}

#[test]
fn the_login_section_parses_and_defaults_to_empty() {
    let parsed = load_sources(
        Some(
            "[login]\nnotice = \"demo\"\n\n[[login.links]]\nlabel = \"API reference\"\nhref = \"https://example.org/swagger\"\n",
        ),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(parsed.login.notice, "demo");
    assert_eq!(parsed.login.links.len(), 1);
    assert_eq!(parsed.login.links[0].label, "API reference");
    assert_eq!(parsed.login.links[0].href, "https://example.org/swagger");

    let defaults = ViewerConfig::default();
    assert_eq!(defaults.login.notice, "");
    assert_eq!(defaults.login.links, Vec::<crate::auth::LoginLink>::new());
}

/// The session is `ferrohealth-session`'s `[session]`: its lifetimes
/// default to an hour idle and twelve hours absolute, a lifetime of
/// zero or an absolute one shorter than the idle one refuses the start, and
/// so do the removed sealed store and its key.
#[test]
fn the_session_section_is_checked_when_it_loads() {
    let defaults = load_sources(None, &BTreeMap::new()).unwrap();
    assert_eq!(
        ferrohealth_config::units::Duration::from_hours(1),
        defaults.session.idle_timeout
    );
    assert_eq!(
        ferrohealth_config::units::Duration::from_hours(12),
        defaults.session.absolute_timeout
    );
    for (refused, key) in [
        (
            "[session]\nabsolute_timeout = \"0s\"\n",
            "session.absolute_timeout",
        ),
        (
            "[session]\nidle_timeout = \"90s\"\nabsolute_timeout = \"60s\"\n",
            "session.absolute_timeout",
        ),
    ] {
        let err = load_sources(Some(refused), &BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains(key), "{refused}: {err}");
    }
    let sealed = load_sources(Some("[session]\nstore = \"sealed\"\n"), &BTreeMap::new());
    assert!(
        matches!(
            &sealed,
            Err(ConfigError::Session(refusal))
                if matches!(**refusal, ferrohealth_session::config::ConfigError::SealedRemoved)
        ),
        "{sealed:?}"
    );
    let keyed = load_sources(
        Some("[session]\nkey_file = \"/run/key\"\n"),
        &BTreeMap::new(),
    );
    assert!(
        matches!(
            &keyed,
            Err(ConfigError::Session(refusal))
                if matches!(
                    **refusal,
                    ferrohealth_session::config::ConfigError::SealingKeyRemoved {
                        key: "session.key_file"
                    }
                )
        ),
        "{keyed:?}"
    );
    let equal = load_sources(
        Some("[session]\nidle_timeout = \"60s\"\nabsolute_timeout = \"60s\"\n"),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        ferrohealth_config::units::Duration::from_secs(60),
        equal.session.absolute_timeout
    );
}

#[test]
fn a_login_link_without_an_href_is_refused() {
    assert!(
        load_sources(
            Some("[[login.links]]\nlabel = \"dangling\"\n"),
            &BTreeMap::new()
        )
        .is_err()
    );
}

/// The uniform variable grammar binds, the Compose parameters pass the
/// sweep, and any other variable under the prefix refuses the start.
#[test]
fn variables_bind_by_the_uniform_grammar_and_strays_are_refused() {
    let env = BTreeMap::from([
        (
            "FERROEHR_VIEWER__CDR__BASE_URL".to_owned(),
            "http://cdr:8080".to_owned(),
        ),
        ("FERROEHR_VIEWER_PORT".to_owned(), "8081".to_owned()),
    ]);
    let cfg = load_sources(None, &env).unwrap();
    assert_eq!(cfg.cdr.base_url, "http://cdr:8080");

    let stray = BTreeMap::from([("FERROEHR_VIEWER_CDR_BASE_URL".to_owned(), "x".to_owned())]);
    let err = load_sources(None, &stray).unwrap_err().to_string();
    assert!(err.contains("FERROEHR_VIEWER_CDR_BASE_URL"), "{err}");
}

/// `[server]` starts from the viewer's own defaults, and a partial table
/// keeps them for every key it does not name.
#[test]
fn the_server_section_layers_over_the_viewers_defaults() {
    let defaults = load_sources(None, &BTreeMap::new()).unwrap();
    assert_eq!("127.0.0.1:3000", defaults.server.listen.to_string());
    assert_eq!(16 * 1024 * 1024, defaults.server.body_limit_bytes());
    assert_eq!(
        ferrohealth_config::units::Duration::from_mins(1),
        defaults.server.request_timeout
    );

    let env = BTreeMap::from([(
        "FERROEHR_VIEWER__SERVER__LISTEN".to_owned(),
        "0.0.0.0:3000".to_owned(),
    )]);
    let partial = load_sources(Some("[server]\ndrain_delay = \"5s\"\n"), &env).unwrap();
    assert_eq!("0.0.0.0:3000", partial.server.listen.to_string());
    assert_eq!(
        ferrohealth_config::units::Duration::from_secs(5),
        partial.server.drain_delay
    );
    assert_eq!(16 * 1024 * 1024, partial.server.body_limit_bytes());
}

/// The keys of `[server]` the viewer cannot apply refuse the start.
#[test]
fn a_base_path_or_a_caller_rate_is_refused() {
    for (file, key) in [
        ("[server]\nbase_path = \"/viewer\"\n", "server.base_path"),
        (
            "[server.caller_rate]\nrequests_per_second = 5\nburst = 10\n",
            "server.caller_rate",
        ),
    ] {
        let refused = load_sources(Some(file), &BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(refused.contains(key), "{refused}");
    }
}

/// The CDR's outbound TLS table is the family's, and the provider's is
/// `ferrohealth-session`'s: its trust roots, its address overrides and
/// `[oidc.tls]` with the client certificate and the TLS floor.
#[test]
fn the_cdr_and_provider_tls_tables_parse() {
    let parsed = load_sources(
        Some(
            "[cdr.tls]\ntrust_roots_file = \"/run/cdr-ca.pem\"\nmin_version = \"1.2\"\n\n\
             [oidc]\nresolve = [\"keycloak=127.0.0.1:8443\"]\n\n\
             [oidc.tls]\ntrust_roots_file = \"/run/issuer-ca.pem\"\n\
             client_identity_file = \"/run/viewer.pem\"\nmin_version = \"1.3\"\n",
        ),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        Some(std::path::PathBuf::from("/run/cdr-ca.pem")),
        parsed.cdr.tls.trust_roots_file
    );
    assert_eq!(
        Some(std::path::PathBuf::from("/run/issuer-ca.pem")),
        parsed.oidc.tls.trust_roots_file
    );
    assert_eq!(parsed.oidc.resolve, ["keycloak=127.0.0.1:8443"]);
    assert_eq!(
        Some(std::path::PathBuf::from("/run/viewer.pem")),
        parsed.oidc.tls.client_identity_file
    );
    assert_eq!(TlsVersion::Tls13, parsed.oidc.tls.min_version);
    let floor = load_sources(
        Some("[oidc.tls]\nmin_version = \"1.2\"\n"),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        TlsVersion::Tls12,
        floor.oidc.tls.min_version,
        "the floor is lowered only when asked"
    );
    assert_eq!(
        TlsVersion::Tls13,
        load_sources(
            Some("[oidc]\nresolve = [\"keycloak=127.0.0.1:8443\"]\n"),
            &BTreeMap::new()
        )
        .unwrap()
        .oidc
        .tls
        .min_version,
        "a partial [oidc] keeps the viewer's TLS 1.3 floor"
    );
    assert!(
        load_sources(Some("[cdr.tls]\nca_file = \"x\"\n"), &BTreeMap::new()).is_err(),
        "an unknown key of the table is refused"
    );
    let env = BTreeMap::from([(
        "FERROEHR_VIEWER__OIDC__RESOLVE".to_owned(),
        "keycloak=127.0.0.1:8443".to_owned(),
    )]);
    assert_eq!(
        load_sources(None, &env).unwrap().oidc.resolve,
        ["keycloak=127.0.0.1:8443"],
        "a variable sets the list from its text"
    );
}

/// The redirect URI derives from the public URL when it is not set, and
/// an OpenID Connect sign-in is checked by `ferrohealth-session` when the
/// configuration loads.
#[test]
fn the_oidc_sign_in_is_checked_when_it_loads() {
    let env = BTreeMap::from([
        (
            "FERROEHR_VIEWER__AUTH__PUBLIC_BASE_URL".to_owned(),
            "http://localhost:3000".to_owned(),
        ),
        (
            "FERROEHR_VIEWER__AUTH__OIDC_ENABLED".to_owned(),
            "true".to_owned(),
        ),
        (
            "FERROEHR_VIEWER__OIDC__ISSUER".to_owned(),
            "https://keycloak:8443/auth/realms/ferroehr".to_owned(),
        ),
        (
            "FERROEHR_VIEWER__OIDC__CLIENT_ID".to_owned(),
            "ferroehr-viewer".to_owned(),
        ),
    ]);
    let loaded = load_sources(None, &env).unwrap();
    assert_eq!(
        loaded.oidc.redirect_uri.expose(),
        "http://localhost:3000/auth/callback"
    );
    assert_eq!(
        loaded
            .oidc
            .post_logout_redirect_uri
            .as_ref()
            .map(ferrohealth_secret::secret_url::SecretUrl::expose),
        Some("http://localhost:3000/sign-in?signed-out=1"),
        "RP-Initiated Logout 1.0 §3"
    );
    let mut unsafe_issuer = env;
    unsafe_issuer.insert(
        "FERROEHR_VIEWER__OIDC__ISSUER".to_owned(),
        "http://keycloak:8080/realms/ferroehr".to_owned(),
    );
    let refused = load_sources(None, &unsafe_issuer).unwrap_err().to_string();
    assert!(refused.contains("oidc.issuer"), "{refused}");
}

/// A public URL with no origin of its own cannot anchor the same-origin
/// check.
#[test]
fn a_public_url_without_an_origin_is_refused() {
    for url in ["not a url", "data:text/plain,viewer"] {
        let env = BTreeMap::from([(
            "FERROEHR_VIEWER__AUTH__PUBLIC_BASE_URL".to_owned(),
            url.to_owned(),
        )]);
        let refused = load_sources(None, &env).unwrap_err().to_string();
        assert!(refused.contains("auth.public_base_url"), "{refused}");
    }
}

#[test]
fn a_trailing_slash_on_the_cdr_base_url_does_not_double_up() {
    let cfg = CdrConfig {
        base_url: "http://cdr:8080/".to_owned(),
        ..CdrConfig::default()
    };
    assert_eq!(cfg.management_base(), "http://cdr:8080/management");
}
