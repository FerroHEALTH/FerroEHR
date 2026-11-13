// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The secret-bearing keys: inline and `_file` forms, the URL userinfo masking
//! and the redacted renderings.

use assert_fs::prelude::*;

use super::tests::{assemble_ok, env, rendered, standalone, toml_file};
use super::*;
use ferrohealth_secret::secret_url::SecretUrl;

#[test]
fn secrets_never_render_and_file_resolves() {
    let file = toml_file("[[auth.basic.users]]\nuser = \"alice\"\npassword_hash = \"topsecret\"\n");
    let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
    let rendered = rendered(&c);
    assert!(!rendered.contains("topsecret"), "secret leaked: {rendered}");
    assert!(
        rendered.contains("password_hash = \"***\""),
        "the hash renders masked: {rendered}"
    );
    assert!(!serde_json::to_string(&c).unwrap().contains("topsecret"));
    let secret = assert_fs::NamedTempFile::new("pass").expect("temp");
    secret.write_str("s3cret\n").expect("write");
    let file = toml_file(&format!(
        "[record.signing]\nkey_passphrase_file = \"{}\"\n",
        secret.path().display()
    ));
    let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
    assert_eq!(
        c.record
            .signing
            .key_passphrase
            .as_ref()
            .expect("resolved")
            .expose(),
        "s3cret"
    );
}

#[test]
fn secret_and_file_both_set_is_rejected() {
    let file = toml_file(
        "[record.signing]\nkey_passphrase = \"a\"\nkey_passphrase_file = \"/dev/null\"\n",
    );
    assert!(assemble(Some(file.path()), &env(&[]), &[]).is_err());
}

/// Every credential-bearing key reaches the same configuration through its
/// `*_file` sibling as it does inline.
///
/// These had no file route at all, so the most valuable secret in the
/// deployment — the database DSN — could only travel as an environment value,
/// readable through `/proc/<pid>/environ` and inherited by every child
/// process. The assertion is equivalence: the file route is an additional
/// delivery mechanism, not a second configuration path that might differ.
#[test]
fn the_credential_keys_resolve_identically_from_a_file_and_inline() {
    const DSN: &str = "postgres://u:p@db.internal:5432/ferroehr";
    const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2g";

    let secret_file = |name: &str, contents: &str| {
        let f = assert_fs::NamedTempFile::new(name).expect("temp");
        // A trailing newline is what `echo >` and a Kubernetes secret editor
        // both produce, so the reader must trim it.
        f.write_str(&format!("{contents}\n")).expect("write");
        f
    };

    let dsn = secret_file("dsn", DSN);
    let hash = secret_file("hash", HASH);

    let from_files = toml_file(&format!(
        "[database]\nurl_file = \"{}\"\n\
         [[auth.basic.users]]\nuser = \"alice\"\npassword_hash_file = \"{}\"\n",
        dsn.path().display(),
        hash.path().display(),
    ));
    let via_file = assemble_ok(Some(from_files.path()), &env(&[]), &[]);

    let inline = toml_file(&format!(
        "[database]\nurl = \"{DSN}\"\n\
         [[auth.basic.users]]\nuser = \"alice\"\npassword_hash = \"{HASH}\"\n"
    ));
    let via_inline = assemble_ok(Some(inline.path()), &env(&[]), &[]);

    assert_eq!(via_file.database.url.expose(), DSN);
    assert_eq!(
        via_file.database.url.expose(),
        via_inline.database.url.expose()
    );
    let user = &via_file.auth.shared.basic.as_ref().expect("basic").users[0];
    assert_eq!(
        user.password_hash
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some(HASH)
    );

    // And a file-delivered DSN redacts exactly like an inline one — the
    // whole point of the `SecretUrl` type is that redaction is a property of
    // the type rather than of how the value arrived.
    let rendered = rendered(&via_file);
    assert!(
        !rendered.contains("u:p@"),
        "a file-loaded DSN must redact its credentials: {rendered}"
    );
    assert!(
        !serde_json::to_string(&via_file)
            .expect("json")
            .contains(HASH),
        "a file-loaded password hash must not render"
    );
}

/// Setting a credential both inline and as a file is refused for each of the
/// six, rather than one silently winning.
///
/// Each of these fields has a non-empty dev default, so "the operator set it"
/// means "it differs from that default" — the default itself must NOT count
/// as a conflicting value, or the file route would be unusable.
#[test]
fn a_credential_set_both_inline_and_as_a_file_is_refused() {
    let cases = [
        "[database]\nurl = \"postgres://u:p@h:5432/d\"\nurl_file = \"/dev/null\"\n",
        "[database]\nmigrate_url = \"postgres://u:p@h:5432/d\"\n\
         migrate_url_file = \"/dev/null\"\n",
        "[database.party]\nurl = \"postgres://u:p@h:5432/d\"\n\
         url_file = \"/dev/null\"\n",
        "[database.audit]\nurl = \"postgres://u:p@h:5432/d\"\n\
         url_file = \"/dev/null\"\n",
        "[database.party]\nmigrate_url = \"postgres://u:p@h:5432/d\"\n\
         migrate_url_file = \"/dev/null\"\n",
        "[[auth.basic.users]]\nuser = \"a\"\npassword_hash = \"x\"\n\
         password_hash_file = \"/dev/null\"\n",
    ];
    for case in cases {
        let file = toml_file(case);
        let outcome = assemble(Some(file.path()), &env(&[]), &[]);
        assert!(
            outcome.is_err(),
            "both-set must be refused, but this was accepted: {case}"
        );
    }
}

/// The URL file routes are reachable through the environment grammar too,
/// which is how a container passes a mount path without a config file.
///
/// Without this the documented env form could ship dead — the whole reason
/// every section carries an env-mapping test.
#[test]
fn the_url_file_keys_are_reachable_through_the_env_grammar() {
    let dsn = assert_fs::NamedTempFile::new("dsn").expect("temp");
    dsn.write_str("postgres://u:p@h:5432/d\n").expect("write");
    // A DIFFERENT DSN, so the assertion below cannot pass by falling back
    // to `database.url` — which is what an unwired `migrate_url_file` would do.
    let migrate = assert_fs::NamedTempFile::new("migrate-dsn").expect("temp");
    migrate
        .write_str("postgres://m:p@h:5432/d\n")
        .expect("write");

    let c = assemble_ok(
        None,
        &env(&[
            (
                "FERROEHR__DATABASE__URL_FILE",
                &dsn.path().display().to_string(),
            ),
            (
                "FERROEHR__DATABASE__MIGRATE_URL_FILE",
                &migrate.path().display().to_string(),
            ),
            (
                "FERROEHR__DATABASE__PARTY__URL_FILE",
                &migrate.path().display().to_string(),
            ),
            (
                "FERROEHR__DATABASE__AUDIT__MIGRATE_URL_FILE",
                &migrate.path().display().to_string(),
            ),
        ]),
        &[],
    );
    assert_eq!(c.database.url.expose(), "postgres://u:p@h:5432/d");
    assert_eq!(c.database.migrate_dsn(), "postgres://m:p@h:5432/d");
    assert_eq!(
        c.database.dsn(crate::db::domain::Domain::Party),
        "postgres://m:p@h:5432/d",
        "the party domain's own DSN arrives through the env grammar, rather than \
         falling back to database.url as an unwired key would"
    );
    assert_eq!(
        c.database
            .domain_migrate_dsn(crate::db::domain::Domain::Audit),
        Some("postgres://m:p@h:5432/d"),
        "a relocated domain's migration credential arrives through the env grammar"
    );
}

/// The dev default is not a setting, so a `*_file` alone works without the
/// operator having to blank the default first.
#[test]
fn a_file_route_works_against_the_untouched_default() {
    let dsn = assert_fs::NamedTempFile::new("dsn").expect("temp");
    dsn.write_str("postgres://u:p@h:5432/d").expect("write");
    let file = toml_file(&format!(
        "[database]\nurl_file = \"{}\"\nmax_connections = 7\n",
        dsn.path().display()
    ));
    let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
    assert_eq!(c.database.url.expose(), "postgres://u:p@h:5432/d");
    assert_eq!(c.database.max_connections, 7);
}

/// Userinfo written into ANY URL key reaches neither the TOML rendering
/// (`ferroehr config check`), the TOML table (`ferroehr report`), the JSON
/// tree (`/management/env`, `GET /admin/config`) nor `Debug`; each URL
/// keeps its host and path with the userinfo masked.
#[test]
fn url_userinfo_reaches_no_rendering() {
    use ferrohealth_smart::config::credentials::OauthSection;

    let leaky = |n: u32| SecretUrl::new(format!("https://u{n}:URL_PW_SENTINEL_{n}@h{n}:1/p{n}"));
    let mut c = standalone();
    c.database.url = leaky(1);
    c.database.migrate_url = Some(leaky(2));
    c.database.clinical.url = Some(leaky(3));
    c.database.party.url = Some(leaky(4));
    c.database.clinical.migrate_url = Some(leaky(5));
    c.database.audit.url = Some(leaky(6));
    c.database.party.migrate_url = Some(leaky(15));
    c.audit.fhir_feed = Some(crate::system_log::config::FhirFeedConfig {
        endpoint: ferrohealth_smart::config::endpoint::EndpointSection {
            url: leaky(8),
            ..ferrohealth_smart::config::endpoint::EndpointSection::default()
        },
        ..crate::system_log::config::FhirFeedConfig::default()
    });
    c.terminology = Some(
        crate::service::terminology::config::TerminologyServerConfig {
            endpoint: ferrohealth_smart::config::endpoint::EndpointSection {
                url: leaky(9),
                credentials: ferrohealth_smart::config::credentials::CredentialsSection {
                    oauth: Some(OauthSection {
                        token_endpoint: SecretUrl::new("https://u10:URL_PW_SENTINEL_10@h10:1/p10"),
                        ..OauthSection::default()
                    }),
                    onward: Some(ferrohealth_smart::config::onward::OnwardSection {
                        token_endpoint: leaky(20),
                        ..ferrohealth_smart::config::onward::OnwardSection::default()
                    }),
                    ..ferrohealth_smart::config::credentials::CredentialsSection::default()
                },
                ..ferrohealth_smart::config::endpoint::EndpointSection::default()
            },
            ..crate::service::terminology::config::TerminologyServerConfig::default()
        },
    );
    c.authz.abac.remote.endpoint.url = leaky(12);
    c.telemetry.otlp = Some(ferrohealth_telemetry::section::OtlpSection {
        endpoint: leaky(13),
        ..ferrohealth_telemetry::section::OtlpSection::default()
    });
    c.auth.shared.issuer = vec![ferrohealth_smart::config::IssuerEntry {
        introspection: Some(ferrohealth_smart::config::IntrospectionSection {
            endpoint: leaky(21),
            ..ferrohealth_smart::config::IntrospectionSection::default()
        }),
        ..auth_section::machines("https://idp.example").remove(0)
    }];
    c.record.multimedia.url = Some(leaky(14));
    c.audit.syslog = Some(crate::system_log::config::SyslogConfig {
        url: leaky(16),
        ..crate::system_log::config::SyslogConfig::default()
    });
    c.auth.smart.platform_base_url = Some(leaky(17));
    c.server.public_base_url = Some(leaky(18));
    // URL 11 was the usage report endpoint, whose type refuses userinfo, so it
    // can hold no sentinel; URL 7 was the removed event broker; URL 19 was
    // `[auth.scim] public_base_url`, which URL 18 replaces.
    assert!(
        ferrohealth_usage_report::section::Endpoint::new(
            "https://u11:URL_PW_SENTINEL_11@h11:1/p11"
        )
        .is_err(),
        "the usage report endpoint refuses userinfo"
    );

    for (surface, rendered) in surfaces(&c) {
        assert!(
            !rendered.contains("URL_PW_SENTINEL"),
            "URL userinfo leaked into the {surface} rendering: {rendered}"
        );
        for n in (1..=21).filter(|n| ![7, 11, 19].contains(n)) {
            assert!(
                rendered.contains(&format!("https://***@h{n}:1/p{n}")),
                "URL {n} lost its host and path in the {surface} rendering: {rendered}"
            );
        }
    }
}

/// Every rendering of `c` an operator reads, by name: the TOML of `config
/// check`, the redacted table and JSON, and `Debug`.
fn surfaces(c: &FerroEhrConfig) -> [(&'static str, String); 4] {
    [
        ("toml", rendered(c)),
        ("table", c.to_redacted_table().expect("table").to_string()),
        (
            "json",
            serde_json::to_string(&c.to_redacted_json().expect("json")).expect("json"),
        ),
        ("debug", format!("{c:?}")),
    ]
}

/// A refusal of an issuer or the published public URL never prints the
/// userinfo it carries.
#[test]
fn an_issuer_refusal_masks_userinfo() {
    let mut c = standalone();
    c.auth.smart.discovery = true;
    c.server.public_base_url = Some(SecretUrl::new("https://a:SMART_PW_SENTINEL@cdr.example"));
    c.auth.shared.issuer = auth_section::machines("https://b:OIDC_PW_SENTINEL@idp.example/r");
    let text = c.validate().expect_err("refused").to_string();
    assert!(!text.contains("SMART_PW_SENTINEL"), "{text}");
    assert!(!text.contains("OIDC_PW_SENTINEL"), "{text}");
}

/// `to_redacted_json` masks EVERY secret-bearing leaf in the whole config
/// tree — the body `GET /admin/config` returns. Each secret is populated
/// with a unique high-entropy sentinel; none may appear in the rendered
/// JSON, while non-secret siblings (a Basic user's `user`/`scopes`) stay
/// visible. This is the standing enumeration of the current secret set: a
/// new secret field added without redaction is caught here once wired
/// into the fixture.
#[test]
fn redacted_json_masks_every_secret_field() {
    use ferrohealth_secret::secret::Secret;
    use ferrohealth_secret::secret_url::SecretUrl;
    use ferrohealth_smart::config::{BasicSection, BasicUser};

    // Each sentinel is unique so a leak is unambiguously attributable.
    const DB_PW: &str = "DB_PW_SENTINEL_9a1c";
    const BASIC_HASH: &str = "$argon2id$BASIC_HASH_SENTINEL_7b2d";
    const PASSPHRASE: &str = "PASSPHRASE_SENTINEL_1d5a";
    const S3_KEY: &str = "S3_SECRET_SENTINEL_8c6b";

    let mut c = standalone();
    c.database.url = SecretUrl::new(format!(
        "postgres://dbuser:{DB_PW}@db.internal:5432/ferroehr"
    ));
    c.auth.shared.basic = Some(BasicSection {
        users: vec![BasicUser {
            user: "alice".to_owned(),
            password_hash: Some(secrecy::SecretString::from(BASIC_HASH)),
            password_hash_file: None,
            scopes: vec!["user/composition-*.r".to_owned()],
            roles: Vec::new(),
        }],
        cache_ttl: ferrohealth_config::units::Duration::ZERO,
    });
    c.auth.shared.issuer = auth_section::machines("https://idp.example");
    c.record.signing.key_passphrase = Some(Secret::new(PASSPHRASE));
    c.record.multimedia.secret_access_key = Some(Secret::new(S3_KEY));
    c.record.multimedia.access_key_id = Some("AKIA_PUBLIC_ID".to_owned());

    let value = c.to_redacted_json().expect("render redacted json");
    let rendered = serde_json::to_string(&value).expect("stringify");

    for sentinel in [DB_PW, BASIC_HASH, PASSPHRASE, S3_KEY] {
        assert!(
            !rendered.contains(sentinel),
            "secret leaked into GET /admin/config body: {sentinel} in {rendered}"
        );
    }

    // Structural placeholders present where a secret was set.
    assert_eq!(value["auth"]["basic"]["users"][0]["password_hash"], "***");
    assert_eq!(value["record"]["signing"]["key_passphrase"], "***");
    assert_eq!(value["record"]["multimedia"]["secret_access_key"], "***");
    assert_eq!(
        value["database"]["url"],
        "postgres://***@db.internal:5432/ferroehr"
    );

    // Non-secret identifiers stay visible (they are not credentials).
    assert_eq!(value["auth"]["basic"]["users"][0]["user"], "alice");
    assert_eq!(
        value["auth"]["basic"]["users"][0]["scopes"][0],
        "user/composition-*.r"
    );
    assert_eq!(value["auth"]["issuer"][0]["issuer"], "https://idp.example");
    assert_eq!(
        value["record"]["multimedia"]["access_key_id"],
        "AKIA_PUBLIC_ID"
    );
}

/// The `[auth]` section every key that names an issuer is written under:
/// one trusted persons issuer, with no userinfo.
const NAMED_ISSUER: &str = "[auth]\nenabled = true\n\n[[auth.issuer]]\nissuer = \"https://idp.example/realms/ferroehr\"\nkind = \"persons\"\naudience = [\"ferroehr\"]\n";

/// Loads `[auth]` with `table` beside its trusted issuer, and asserts the
/// password `sentinel` in the table's issuer reaches no rendering of the
/// tree and no refusal, while the issuer still renders under `host` with
/// its userinfo masked (RFC 3986 §3.2.1).
fn assert_table_issuer_masked(table: &str, sentinel: &str, host: &str) {
    let refused = rendered_masked(table, sentinel, host)
        .validate()
        .expect_err("the issuer is not trusted")
        .to_string();
    assert!(
        refused.contains(host),
        "the refusal names the issuer: {refused}"
    );
    assert!(!refused.contains(sentinel), "{refused}");
}

/// Loads `[auth]` with `table` beside its trusted issuer, asserts every
/// rendering shows the issuer under `host` with its userinfo masked and
/// never the password `sentinel`, and returns the configuration.
fn rendered_masked(table: &str, sentinel: &str, host: &str) -> FerroEhrConfig {
    let file = toml_file(&format!("{NAMED_ISSUER}\n{table}"));
    let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
    for (surface, shown) in surfaces(&c) {
        assert!(
            !shown.contains(sentinel),
            "the issuer's password leaked into the {surface} rendering: {shown}"
        );
        assert!(
            shown.contains(&format!("https://***@{host}/realms/ferroehr")),
            "the issuer lost its host in the {surface} rendering: {shown}"
        );
    }
    c
}

/// An `[[auth.attributes]]` issuer renders and refuses with its userinfo
/// masked.
#[test]
fn an_attributes_issuer_never_renders_its_userinfo() {
    assert_table_issuer_masked(
        "[[auth.attributes]]\nissuer = \"https://u:ATTRIBUTES_PW_SENTINEL@attributes.example/realms/ferroehr\"\nmapping = \"iua\"\n",
        "ATTRIBUTES_PW_SENTINEL",
        "attributes.example",
    );
}

/// An `[[auth.federation]]` issuer renders and refuses with its userinfo
/// masked.
#[test]
fn a_federation_issuer_never_renders_its_userinfo() {
    assert_table_issuer_masked(
        "[[auth.federation]]\nissuer = \"https://u:FEDERATION_PW_SENTINEL@federation.example/realms/ferroehr\"\ncontact_point = false\n",
        "FEDERATION_PW_SENTINEL",
        "federation.example",
    );
}

/// An `[[auth.party]]` issuer renders and refuses with its userinfo masked.
#[test]
fn a_party_issuer_never_renders_its_userinfo() {
    assert_table_issuer_masked(
        "[[auth.party]]\nissuer = \"https://u:PARTY_PW_SENTINEL@party.example/realms/ferroehr\"\nidentifier_issuer = \"urn:example:login\"\nidentifier_type = \"guid\"\n",
        "PARTY_PW_SENTINEL",
        "party.example",
    );
}

/// An `auth.scopeless_subjects` issuer renders and refuses with its
/// userinfo masked.
#[test]
fn a_scopeless_subject_issuer_never_renders_its_userinfo() {
    assert_table_issuer_masked(
        "[[auth.scopeless_subjects]]\nissuer = \"https://u:SCOPELESS_PW_SENTINEL@scopeless.example/realms/ferroehr\"\nsub = \"svc\"\n",
        "SCOPELESS_PW_SENTINEL",
        "scopeless.example",
    );
}

/// An `[[authz.federation.agreement]]` issuer renders and refuses with its
/// userinfo masked.
#[test]
fn an_agreement_issuer_never_renders_its_userinfo() {
    assert_table_issuer_masked(
        "[[authz.federation.agreement]]\nissuer = \"https://u:AGREEMENT_PW_SENTINEL@agreement.example/realms/ferroehr\"\naccessor = \"person\"\nname = \"cross-border\"\n",
        "AGREEMENT_PW_SENTINEL",
        "agreement.example",
    );
}

/// An `[[authz.party_binding.issuer]]` issuer renders and refuses with its
/// userinfo masked.
#[test]
fn a_party_binding_issuer_never_renders_its_userinfo() {
    assert_table_issuer_masked(
        "[[authz.party_binding.issuer]]\nissuer = \"https://u:BINDING_PW_SENTINEL@binding.example/realms/ferroehr\"\njit = true\n",
        "BINDING_PW_SENTINEL",
        "binding.example",
    );
}

/// The removed `[[authz.system_client]]` table is refused as an unknown key,
/// and the refusal never carries its issuer's userinfo (#5436).
#[test]
fn a_removed_system_client_table_is_refused_without_its_userinfo() {
    let refused = tests::file_refusal(&format!(
        "{NAMED_ISSUER}\n[[authz.system_client]]\nissuer = \"https://u:SYSTEM_PW_SENTINEL@system.example/realms/ferroehr\"\nclient_id = \"lab\"\n"
    ));
    assert!(refused.contains("system_client"), "{refused}");
    assert!(!refused.contains("SYSTEM_PW_SENTINEL"), "{refused}");
}

/// An `[[auth.scim.client]]` issuer renders and refuses with its userinfo
/// masked.
#[test]
fn a_scim_client_issuer_never_renders_its_userinfo() {
    assert_table_issuer_masked(
        "[[auth.scim.client]]\nissuer = \"https://u:SCIM_PW_SENTINEL@scim.example/realms/ferroehr\"\nclient_id = \"idm\"\nscope = \"scim\"\nbinding_issuer = \"urn:example:login\"\n",
        "SCIM_PW_SENTINEL",
        "scim.example",
    );
}

/// An `authz.rbac.role_issuers` issuer renders and refuses with its userinfo
/// masked.
#[test]
fn a_role_issuer_never_renders_its_userinfo() {
    assert_table_issuer_masked(
        "[authz.rbac]\nrole_issuers = [\"https://u:ROLE_PW_SENTINEL@role.example/realms/ferroehr\"]\n",
        "ROLE_PW_SENTINEL",
        "role.example",
    );
}

/// An `auth.smart.patient_issuers` issuer renders with its userinfo masked,
/// and no refusal of the configuration carries it.
#[test]
fn a_patient_issuer_never_renders_its_userinfo() {
    let c = rendered_masked(
        "[auth.smart]\npatient_issuers = [\"https://u:PATIENT_PW_SENTINEL@patient.example/realms/ferroehr\"]\n",
        "PATIENT_PW_SENTINEL",
        "patient.example",
    );
    if let Err(refused) = c.validate() {
        assert!(
            !refused.to_string().contains("PATIENT_PW_SENTINEL"),
            "{refused}"
        );
    }
}
