// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The deployment posture: each gap, its refusal under `production` and its
//! acceptance by name.

use super::*;
use ferrohealth_secret::secret_url::SecretUrl;

fn separated() -> FerroEhrConfig {
    {
        let mut c = FerroEhrConfig {
            privacy: crate::privacy::config::PrivacyConfig {
                subject_namespaces: vec!["urn:example:pseudonym".to_owned()],
                ..crate::privacy::config::PrivacyConfig::default()
            },
            audit: crate::system_log::config::AuditConfig {
                fail_mode: crate::system_log::config::FailMode::Closed,
                ..crate::system_log::config::AuditConfig::default()
            },
            authz: crate::config::authz::AuthzConfig {
                rbac: crate::config::authz::RbacConfig {
                    ehr_access_default: crate::config::authz::EhrAccessDefault::Restricted,
                    ..crate::config::authz::RbacConfig::default()
                },
                ..crate::config::authz::AuthzConfig::default()
            },
            server: crate::config::server::ServerConfig {
                shared: ferrohealth_server::section::ServerSection {
                    listen: "0.0.0.0:8443".parse().expect("an address"),
                    tls: Some(ferrohealth_server::section::TlsSection::new(
                        "server.pem".into(),
                        "server.key".into(),
                    )),
                    ..ferrohealth_server::section::ServerSection::default()
                },
                public_base_url: None,
            },
            ..FerroEhrConfig::default()
        };
        c.database = crate::db::DbConfig {
            migrate_url: Some(SecretUrl::new("postgres://m@h/x")),
            clinical: crate::db::domain::DomainDsn {
                url: Some(SecretUrl::new("postgres://c@h/x")),
                ..Default::default()
            },
            party: crate::db::domain::DomainDsn {
                url: Some(SecretUrl::new("postgres://d@h/x")),
                ..Default::default()
            },
            ..crate::db::DbConfig::default()
        };
        c
    }
}

fn production(mut config: FerroEhrConfig) -> FerroEhrConfig {
    config.deployment.profile = DeploymentProfile::Production;
    config
}

fn accepting(mut config: FerroEhrConfig, gaps: Vec<DeploymentGap>) -> FerroEhrConfig {
    config.deployment.accepts = gaps;
    config
}

fn distinct_clusters() -> DatabaseFacts {
    DatabaseFacts {
        clusters: ClusterIdentities {
            clinical: Some("1".to_owned()),
            party: Some("2".to_owned()),
            audit: Some("4".to_owned()),
        },
        prepared_on_runtime_credential: Vec::new(),
    }
}

/// The clusters alone, with nothing measured about preparation.
fn on(clusters: ClusterIdentities) -> DatabaseFacts {
    DatabaseFacts {
        clusters,
        prepared_on_runtime_credential: Vec::new(),
    }
}

/// The shipped default is `sandbox`, and a stock configuration on one
/// cluster, published on every interface as the built-in listener is, is
/// exactly the deployment that must not hold real data: every gap but the
/// audit-off one is open, and boot is permitted. A loopback listener is
/// no gap.
#[test]
fn the_default_is_sandbox_and_a_stock_deployment_names_its_gaps() {
    let config = FerroEhrConfig::default();
    assert_eq!(config.server.shared.listen.to_string(), "0.0.0.0:8080");
    assert_eq!(config.deployment.profile, DeploymentProfile::Sandbox);
    let clusters = ClusterIdentities {
        clinical: Some("1".to_owned()),
        party: Some("1".to_owned()),
        audit: Some("1".to_owned()),
    };
    let posture = DeploymentPosture::evaluate(&config, &on(clusters));
    assert_eq!(
        posture.gaps,
        [
            DeploymentGap::SharedCredential,
            DeploymentGap::SharedCluster,
            DeploymentGap::OpenSubjectNamespace,
            DeploymentGap::AuditFailsOpen,
            DeploymentGap::OpenEhrAccessDefault,
            DeploymentGap::MigrateOnRuntimeCredential,
            DeploymentGap::PlaintextListener,
        ],
        "auditing and authentication are on by default, so neither is a gap"
    );
    assert!(posture.permits_boot());
    let mut on_loopback = FerroEhrConfig::default();
    on_loopback.server.shared.listen = "127.0.0.1:8080".parse().expect("an address");
    let loopback = DeploymentPosture::evaluate(&on_loopback, &on(clusters_of("1")));
    assert!(
        !loopback.gaps.contains(&DeploymentGap::PlaintextListener),
        "a loopback listener is no gap: {loopback:?}"
    );
}

/// Every storage domain on the one cluster `id`.
fn clusters_of(id: &str) -> ClusterIdentities {
    ClusterIdentities {
        clinical: Some(id.to_owned()),
        party: Some(id.to_owned()),
        audit: Some(id.to_owned()),
    }
}

/// `production` refuses every open gap by name and lists the remedy.
#[test]
fn production_refuses_each_open_gap_and_names_the_remedy() {
    let config = production(FerroEhrConfig {
        audit: crate::system_log::config::AuditConfig {
            enabled: false,
            ..crate::system_log::config::AuditConfig::default()
        },
        ..FerroEhrConfig::default()
    });
    let posture = DeploymentPosture::evaluate(&config, &DatabaseFacts::default());
    assert!(!posture.permits_boot());
    let message = posture.refusal_message();
    for gap in [
        DeploymentGap::SharedCredential,
        DeploymentGap::OpenSubjectNamespace,
        DeploymentGap::AuditOff,
        DeploymentGap::OpenEhrAccessDefault,
        DeploymentGap::MigrateOnRuntimeCredential,
    ] {
        assert!(message.contains(gap.as_str()), "{message}");
        assert!(message.contains(gap.describe()), "{message}");
    }
    assert!(
        !posture.gaps.contains(&DeploymentGap::SharedCluster),
        "an unknown cluster identity is never read as a gap"
    );
    assert!(
        !posture.gaps.contains(&DeploymentGap::AuditFailsOpen),
        "a disabled trail makes the fail mode moot, so it is named once"
    );
}

/// A fully separated deployment on three clusters boots as `production`
/// with nothing open.
#[test]
fn a_separated_deployment_boots_as_production() {
    let config = production(separated());
    let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
    assert!(posture.gaps.is_empty(), "{posture:?}");
    assert!(posture.permits_boot());
}

/// Two DSNs reaching one cluster are refused by the cluster identity, not
/// by the host string; accepting the gap by name permits boot and keeps
/// it listed.
#[test]
fn a_shared_cluster_is_refused_by_identity_and_accepted_only_by_name() {
    let config = production(separated());
    let same_cluster = on(ClusterIdentities {
        clinical: Some("7".to_owned()),
        party: Some("7".to_owned()),
        audit: Some("9".to_owned()),
    });
    let posture = DeploymentPosture::evaluate(&config, &same_cluster);
    assert_eq!(posture.gaps, [DeploymentGap::SharedCluster]);
    assert!(!posture.permits_boot());

    let config = accepting(config, vec![DeploymentGap::SharedCluster]);
    let accepted = DeploymentPosture::evaluate(&config, &same_cluster);
    assert!(accepted.permits_boot());
    assert_eq!(accepted.accepted, [DeploymentGap::SharedCluster]);
    assert_eq!(
        accepted.gaps,
        [DeploymentGap::SharedCluster],
        "accepting hides nothing"
    );
}

/// `Display`/`FromStr` round-trip the tokens, and serde spells them the
/// same way.
#[test]
fn tokens_round_trip() {
    for profile in [DeploymentProfile::Sandbox, DeploymentProfile::Production] {
        let token = profile.to_string();
        assert_eq!(token.parse::<DeploymentProfile>().ok(), Some(profile));
        assert_eq!(
            serde_json::to_string(&profile).ok(),
            Some(format!("\"{token}\""))
        );
    }
    assert!("research".parse::<DeploymentProfile>().is_err());
    for gap in [
        DeploymentGap::SharedCredential,
        DeploymentGap::SharedCluster,
        DeploymentGap::OpenSubjectNamespace,
        DeploymentGap::AuditOff,
        DeploymentGap::AuditFailsOpen,
        DeploymentGap::NoCategoryMap,
        DeploymentGap::OpenEhrAccessDefault,
        DeploymentGap::MigrateOnRuntimeCredential,
        DeploymentGap::AuthOff,
        DeploymentGap::RbacOff,
        DeploymentGap::PlaintextListener,
        DeploymentGap::PlaintextAuditFeed,
        DeploymentGap::PlaintextObjectStore,
    ] {
        assert_eq!(
            serde_json::to_string(&gap).ok(),
            Some(format!("\"{}\"", gap.as_str()))
        );
    }
}

/// Every gap token is in the shared vocabulary the viewer names, in
/// declaration order, and the vocabulary names no other token.
#[test]
fn every_gap_token_is_the_shared_vocabulary() {
    use crate::served_vocabulary::DEPLOYMENT_GAPS;
    let index = |gap: DeploymentGap| match gap {
        DeploymentGap::SharedCredential => 0,
        DeploymentGap::SharedCluster => 1,
        DeploymentGap::OpenSubjectNamespace => 2,
        DeploymentGap::AuditOff => 3,
        DeploymentGap::AuditFailsOpen => 4,
        DeploymentGap::NoCategoryMap => 5,
        DeploymentGap::OpenEhrAccessDefault => 6,
        DeploymentGap::MigrateOnRuntimeCredential => 7,
        DeploymentGap::AuthOff => 8,
        DeploymentGap::RbacOff => 9,
        DeploymentGap::PlaintextListener => 10,
        DeploymentGap::PlaintextAuditFeed => 11,
        DeploymentGap::PlaintextObjectStore => 12,
    };
    for (i, token) in DEPLOYMENT_GAPS.iter().enumerate() {
        let gap: DeploymentGap =
            serde_json::from_str(&format!("\"{token}\"")).expect("a known gap token");
        assert_eq!(gap.as_str(), *token);
        assert_eq!(index(gap), i, "{token} is out of order");
    }
}

/// The two compatibility defaults are open gaps under `production`, each
/// named with its key, its value and its remedy, and each accepted only by
/// name.
///
/// The shipped defaults do not change: they serve the conformance
/// instrument and the development stacks, where a closed `EHR_ACCESS` default
/// would refuse every read of a fresh EHR. What changes is that a
/// production deployment has to say it wants them.
/// `docs/law/eu/gdpr/text.html Art. 25(2)` is the ground for the access
/// default: measures "shall ensure that by default personal data are not
/// made accessible without the individual's intervention to an indefinite
/// number of natural persons".
#[test]
fn production_refuses_the_open_access_default_and_the_open_audit_fail_mode() {
    let mut open = separated();
    open.audit.fail_mode = crate::system_log::config::FailMode::Open;
    open.authz.rbac.ehr_access_default = crate::config::authz::EhrAccessDefault::Open;
    let config = production(open);
    let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
    assert_eq!(
        posture.gaps,
        [
            DeploymentGap::AuditFailsOpen,
            DeploymentGap::OpenEhrAccessDefault
        ]
    );
    assert!(!posture.permits_boot());
    let message = posture.refusal_message();
    for gap in [
        DeploymentGap::AuditFailsOpen,
        DeploymentGap::OpenEhrAccessDefault,
    ] {
        assert!(message.contains(gap.as_str()), "{message}");
        assert!(message.contains(gap.describe()), "{message}");
    }
    assert!(message.contains("ehr_access_default"), "{message}");
    assert!(message.contains("fail_mode"), "{message}");
    assert!(message.contains("restricted"), "{message}");
    assert!(message.contains("closed"), "{message}");

    // Accepted by name: the deployment boots and both stay listed.
    let accepted = DeploymentPosture::evaluate(
        &accepting(
            config,
            vec![
                DeploymentGap::AuditFailsOpen,
                DeploymentGap::OpenEhrAccessDefault,
            ],
        ),
        &distinct_clusters(),
    );
    assert!(accepted.permits_boot());
    assert_eq!(
        accepted.accepted,
        [
            DeploymentGap::AuditFailsOpen,
            DeploymentGap::OpenEhrAccessDefault
        ]
    );
}

/// A `sandbox` deployment runs on the shipped defaults and says so; the two
/// gaps refuse nothing there.
#[test]
fn sandbox_runs_the_shipped_defaults_and_only_names_them() {
    let config = FerroEhrConfig::default();
    assert_eq!(
        config.authz.rbac.ehr_access_default,
        crate::config::authz::EhrAccessDefault::Open,
        "the shipped access default is unchanged"
    );
    assert_eq!(
        config.audit.fail_mode,
        crate::system_log::config::FailMode::Open,
        "the shipped audit fail mode is unchanged"
    );
    let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
    assert!(posture.gaps.contains(&DeploymentGap::AuditFailsOpen));
    assert!(posture.gaps.contains(&DeploymentGap::OpenEhrAccessDefault));
    assert!(posture.permits_boot(), "sandbox refuses nothing");
}

/// An audit database sharing a cluster with a pseudonymisation domain is a
/// `SharedCluster` gap.
///
/// `docs/law/ch/dpo/text-de.html Art. 4 Abs. 5`: "Die Protokolle müssen
/// während mindestens einem Jahr getrennt vom System, in welchem die
/// Personendaten bearbeitet werden, aufbewahrt werden." The posture reads
/// the audit pool's own cluster identity, so an audit database co-located
/// with the clinical one is visible rather than silently uncounted.
#[test]
fn the_audit_cluster_counts_towards_the_shared_cluster_gap() {
    let config = production(separated());
    let audit_with_clinical = on(ClusterIdentities {
        clinical: Some("1".to_owned()),
        party: Some("2".to_owned()),
        audit: Some("1".to_owned()),
    });
    let posture = DeploymentPosture::evaluate(&config, &audit_with_clinical);
    assert_eq!(posture.gaps, [DeploymentGap::SharedCluster]);
    assert!(!posture.permits_boot());
    let message = posture.refusal_message();
    assert!(
        message.contains("[database.audit]"),
        "the remedy names the audit domain's own DSN: {message}"
    );
}

/// A domain whose database schema preparation reaches on its own runtime
/// DSN opens `MigrateOnRuntimeCredential`, even with `[database] migrate_url`
/// set.
#[test]
fn a_relocated_domain_prepared_on_its_runtime_credential_is_a_gap() {
    let config = production(separated());
    let separated_migrator = DeploymentPosture::evaluate(&config, &distinct_clusters());
    assert!(
        separated_migrator.gaps.is_empty(),
        "[database] migrate_url alone closes it while nothing is relocated: {separated_migrator:?}"
    );

    let relocated = DatabaseFacts {
        prepared_on_runtime_credential: vec![crate::db::domain::Domain::Party],
        ..distinct_clusters()
    };
    let posture = DeploymentPosture::evaluate(&config, &relocated);
    assert_eq!(posture.gaps, [DeploymentGap::MigrateOnRuntimeCredential]);
    assert!(!posture.permits_boot());
}

/// `production` refuses unauthenticated serving, and accepting `auth_off`
/// by name is the only way to boot with it (CRA Annex I Part I(2)(d)).
#[test]
fn production_refuses_auth_off_unless_accepted_by_name() {
    let mut config = production(separated());
    config.auth.shared.enabled = false;
    let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
    assert_eq!(posture.gaps, [DeploymentGap::AuthOff]);
    assert!(!posture.permits_boot());
    let message = posture.refusal_message();
    assert!(message.contains("auth_off"), "{message}");
    assert!(
        message.contains(DeploymentGap::AuthOff.describe()),
        "{message}"
    );

    config.deployment.accepts = vec![DeploymentGap::AuthOff];
    let accepted = DeploymentPosture::evaluate(&config, &distinct_clusters());
    assert!(accepted.permits_boot());
    assert_eq!(accepted.accepted, [DeploymentGap::AuthOff]);
    assert_eq!(
        accepted.gaps,
        [DeploymentGap::AuthOff],
        "accepting hides nothing"
    );

    config.deployment.accepts = vec![DeploymentGap::PlaintextListener];
    let other_name = DeploymentPosture::evaluate(&config, &distinct_clusters());
    assert!(
        !other_name.permits_boot(),
        "accepting a different gap does not accept this one"
    );
}

/// `production` refuses an authenticated deployment without the RBAC gate
/// unless `rbac_off` is accepted by name (CRA Annex I Part I(2)(d)); a
/// `restricted` EHR default stays closed with RBAC off, because the per-EHR
/// gate applies it either way; and `auth_off` subsumes the gap.
#[test]
fn production_refuses_rbac_off_and_a_restricted_default_stays_closed() {
    let mut config = production(separated());
    config.authz.rbac.enabled = false;
    let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
    assert_eq!(
        posture.gaps,
        [DeploymentGap::RbacOff],
        "the restricted default is in force with RBAC off, so only rbac_off is open"
    );
    assert!(!posture.permits_boot());
    let message = posture.refusal_message();
    assert!(message.contains("rbac_off"), "{message}");
    assert!(
        message.contains(DeploymentGap::RbacOff.describe()),
        "{message}"
    );

    config.deployment.accepts = vec![DeploymentGap::RbacOff];
    assert!(DeploymentPosture::evaluate(&config, &distinct_clusters()).permits_boot());

    config.auth.shared.enabled = false;
    config.deployment.accepts = Vec::new();
    assert_eq!(
        DeploymentPosture::evaluate(&config, &distinct_clusters()).gaps,
        [DeploymentGap::AuthOff],
        "with authentication off the wider gap is named once"
    );
}

/// `production` refuses a plaintext listener on a routable address,
/// exempts a loopback one, and boots once `plaintext_listener` is accepted
/// by name (CRA Annex I Part I(2)(e)).
#[test]
fn production_refuses_a_routable_plaintext_listener_and_exempts_loopback() {
    let mut config = production(separated());
    config.server.shared.tls = None;
    for bind in ["0.0.0.0:8080", "10.0.0.4:8080", "[::]:8080"] {
        config.server.shared.listen = bind.parse().expect("an address");
        let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
        assert_eq!(posture.gaps, [DeploymentGap::PlaintextListener], "{bind}");
        assert!(!posture.permits_boot(), "{bind}");
    }

    for bind in ["127.0.0.1:8080", "[::1]:8080"] {
        config.server.shared.listen = bind.parse().expect("an address");
        let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
        assert!(posture.gaps.is_empty(), "{bind} is loopback: {posture:?}");
        assert!(posture.permits_boot(), "{bind}");
    }

    config.server.shared.listen = "0.0.0.0:8080".parse().expect("an address");
    let accepted = accepting_the_plain_listener(&mut config);
    assert_eq!(accepted.gaps, [DeploymentGap::PlaintextListener]);
}

/// `production` refuses the separate management listener, which binds every
/// interface in plain HTTP even with `[server.tls]` on, and boots once
/// `plaintext_listener` is accepted by name (CRA Annex I Part I(2)(e)).
#[test]
fn production_refuses_a_plain_management_listener_on_its_own_port() {
    let mut config = production(separated());
    config.management.enabled = true;
    config.management.listen = Some(std::net::SocketAddr::from(([0, 0, 0, 0], 9090)));
    let posture = DeploymentPosture::evaluate(&config, &distinct_clusters());
    assert_eq!(posture.gaps, [DeploymentGap::PlaintextListener]);
    assert!(!posture.permits_boot());
    assert!(
        posture.refusal_message().contains("[management] port"),
        "{}",
        posture.refusal_message()
    );

    config.management.listen = None;
    let shared = DeploymentPosture::evaluate(&config, &distinct_clusters());
    assert!(
        shared.gaps.is_empty(),
        "shares the TLS listener: {shared:?}"
    );

    config.management.listen = Some(std::net::SocketAddr::from(([0, 0, 0, 0], 9090)));
    config.management.enabled = false;
    let off = DeploymentPosture::evaluate(&config, &distinct_clusters());
    assert!(
        off.gaps.is_empty(),
        "a disabled surface binds nothing: {off:?}"
    );

    config.management.enabled = true;
    accepting_the_plain_listener(&mut config);
}

/// `config` once it accepts `plaintext_listener` by name, which boots and
/// names the gap accepted.
fn accepting_the_plain_listener(config: &mut FerroEhrConfig) -> DeploymentPosture {
    config.deployment.accepts = vec![DeploymentGap::PlaintextListener];
    let accepted = DeploymentPosture::evaluate(config, &distinct_clusters());
    assert!(accepted.permits_boot());
    assert_eq!(accepted.accepted, [DeploymentGap::PlaintextListener]);
    accepted
}

/// `production` refuses an enabled audit FHIR feed on a plain `http://` URL and
/// an enabled multimedia store on an `http://` URL, passes `https://` and a
/// disabled section, and boots once each gap is accepted by name (CRA Annex I
/// Part I(2)(e)).
#[test]
fn production_refuses_a_plaintext_audit_feed_and_object_store_unless_accepted() {
    let feed = |enabled: bool, url: &str| {
        let mut config = production(separated());
        let mut feed = crate::system_log::config::FhirFeedConfig::default();
        feed.endpoint.url = SecretUrl::new(url);
        config.audit.fhir_feed = enabled.then_some(feed);
        config
    };
    for url in ["http://arr.internal/fhir", "HTTP://arr.internal/fhir"] {
        let posture = DeploymentPosture::evaluate(&feed(true, url), &distinct_clusters());
        assert_eq!(posture.gaps, [DeploymentGap::PlaintextAuditFeed], "{url}");
        assert!(!posture.permits_boot(), "{url}");
        assert!(
            posture.refusal_message().contains("plaintext_audit_feed"),
            "{url}"
        );
    }
    let tls = DeploymentPosture::evaluate(
        &feed(true, "https://arr.internal/fhir"),
        &distinct_clusters(),
    );
    assert!(tls.gaps.is_empty(), "https is no gap: {tls:?}");
    let off = DeploymentPosture::evaluate(
        &feed(false, "http://arr.internal/fhir"),
        &distinct_clusters(),
    );
    assert!(
        off.gaps.is_empty(),
        "a disabled feed sends nothing: {off:?}"
    );
    let accepted = accepting(
        feed(true, "http://arr.internal/fhir"),
        vec![DeploymentGap::PlaintextAuditFeed],
    );
    assert!(DeploymentPosture::evaluate(&accepted, &distinct_clusters()).permits_boot());

    let store = |enabled: bool, allow_http: bool| {
        let mut config = production(separated());
        config.record.multimedia.enabled = enabled;
        config.record.multimedia.url = Some(SecretUrl::new(if allow_http {
            "http://seaweedfs:8333"
        } else {
            "https://s3.example"
        }));
        config
    };
    let posture = DeploymentPosture::evaluate(&store(true, true), &distinct_clusters());
    assert_eq!(posture.gaps, [DeploymentGap::PlaintextObjectStore]);
    assert!(!posture.permits_boot());
    assert!(
        posture
            .refusal_message()
            .contains(DeploymentGap::PlaintextObjectStore.describe())
    );
    assert_eq!(
        DeploymentPosture::evaluate(&store(true, false), &distinct_clusters()).gaps,
        Vec::<DeploymentGap>::new()
    );
    assert_eq!(
        DeploymentPosture::evaluate(&store(false, true), &distinct_clusters()).gaps,
        Vec::<DeploymentGap>::new()
    );
    let accepted = accepting(store(true, true), vec![DeploymentGap::PlaintextObjectStore]);
    assert!(DeploymentPosture::evaluate(&accepted, &distinct_clusters()).permits_boot());
}

/// The category map is the audit category register, so a map that
/// classifies nothing is refused at the register write: production refuses it
/// while it audits, unless `no_category_map` is accepted by name (EHDS Annex
/// II 3.2(c)); a sandbox and a deployment that does not audit do not.
#[test]
fn production_requires_a_category_map_unless_accepted() {
    assert!(DeploymentPosture::requires_category_map(&production(
        separated()
    )));
    let accepted = accepting(production(separated()), vec![DeploymentGap::NoCategoryMap]);
    assert!(!DeploymentPosture::requires_category_map(&accepted));
    let mut silent = production(separated());
    silent.audit.enabled = false;
    assert!(!DeploymentPosture::requires_category_map(&silent));
    assert!(!DeploymentPosture::requires_category_map(
        &FerroEhrConfig::default()
    ));
    assert!(
        !DeploymentPosture::evaluate(&production(separated()), &distinct_clusters())
            .gaps
            .contains(&DeploymentGap::NoCategoryMap),
        "the configuration holds no category map, so boot never finds the gap"
    );
}
