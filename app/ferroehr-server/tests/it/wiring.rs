// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binary's command-line seam: `--set` override parsing, the subcommand
//! shapes, the one dispatch branch of `run` that touches nothing external, and
//! the `[privacy]` policy the run path compiles out of the shipped defaults.
//!
//! `Cli`'s fields are private by design (visibility is deliberate — the type is
//! a `clap` parse target, not a record), so a parse result is observed through
//! its derived `Debug` rendering, the surface the type actually offers.

#![expect(
    clippy::panic_in_result_fn,
    reason = "the blessed test shape (the Rust Book ch11-01): `?` propagates \
              plumbing failures — here the clap parse — while the assertion \
              carries the behaviour under test and is meant to panic"
)]
#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only reaches \
              `#[test]`-annotated functions, so it misses this module's fixture helpers; a \
              failing fixture must panic at the fixture (the Rust Book ch11)"
)]

use assert_fs::prelude::{FileWriteStr as _, PathChild as _};
use clap::Parser as _;

use ferroehr_server::{Cli, run, write_report};

/// A `--set key=value` pair parses into the override list.
#[test]
fn set_override_accepts_a_key_value_pair() -> Result<(), clap::Error> {
    let cli = Cli::try_parse_from(["ferroehr", "--set", "db.max_connections=40"])?;
    assert!(
        format!("{cli:?}").contains(r#"("db.max_connections", "40")"#),
        "parsed override missing: {cli:?}"
    );
    Ok(())
}

/// `--set` is repeatable and keeps every pair, in the order given (the loader
/// applies them in sequence, so order is behaviour).
#[test]
fn set_override_is_repeatable_and_ordered() -> Result<(), clap::Error> {
    let cli = Cli::try_parse_from([
        "ferroehr",
        "--set",
        "db.max_connections=40",
        "--set",
        "server.bind=0.0.0.0:9000",
    ])?;
    let rendered = format!("{cli:?}");
    let first = rendered.find(r#"("db.max_connections", "40")"#);
    let second = rendered.find(r#"("server.bind", "0.0.0.0:9000")"#);
    assert!(
        matches!((first, second), (Some(a), Some(b)) if a < b),
        "overrides lost or reordered: {rendered}"
    );
    Ok(())
}

/// The key is trimmed and only the FIRST `=` separates key from value, so a
/// value may itself contain `=` (a DSN query string, a base64 tail).
#[test]
fn set_override_splits_on_the_first_equals_and_trims_the_key() -> Result<(), clap::Error> {
    let cli = Cli::try_parse_from(["ferroehr", "--set", "  db.url =postgres://h/db?a=b"])?;
    assert!(
        format!("{cli:?}").contains(r#"("db.url", "postgres://h/db?a=b")"#),
        "unexpected split: {cli:?}"
    );
    Ok(())
}

/// A `--set` argument without `=` is rejected, naming the expected form.
#[test]
fn set_override_rejects_a_pair_without_an_equals() {
    let err = Cli::try_parse_from(["ferroehr", "--set", "db.max_connections"])
        .expect_err("a bare key must not parse");
    let rendered = err.to_string();
    assert!(
        rendered.contains("expected key=value"),
        "unhelpful rejection: {rendered}"
    );
}

/// `ferroehr` with no subcommand is the serve path (`command: None`).
#[test]
fn no_subcommand_selects_the_serve_path() -> Result<(), clap::Error> {
    let cli = Cli::try_parse_from(["ferroehr"])?;
    assert!(
        format!("{cli:?}").contains("command: None"),
        "expected no subcommand: {cli:?}"
    );
    Ok(())
}

/// `--config` is global: accepted before or after the subcommand.
#[test]
fn config_path_is_global() -> Result<(), clap::Error> {
    const PATH: &str = "/etc/ferroehr/ferroehr.toml";
    for args in [
        ["ferroehr", "--config", PATH, "config", "check"],
        ["ferroehr", "config", "check", "--config", PATH],
    ] {
        let cli = Cli::try_parse_from(args)?;
        assert!(
            format!("{cli:?}").contains(PATH),
            "config path lost for {args:?}: {cli:?}"
        );
    }
    Ok(())
}

/// Both `config` utilities parse to their own variant.
#[test]
fn config_subcommands_parse() -> Result<(), clap::Error> {
    let default = Cli::try_parse_from(["ferroehr", "config", "default"])?;
    assert!(
        format!("{default:?}").contains("Default"),
        "not the Default utility: {default:?}"
    );
    let check = Cli::try_parse_from(["ferroehr", "config", "check"])?;
    assert!(
        format!("{check:?}").contains("Check"),
        "not the Check utility: {check:?}"
    );
    Ok(())
}

/// `config` without a utility is rejected (the inner subcommand is required).
#[test]
fn config_without_a_utility_is_rejected() {
    assert!(
        Cli::try_parse_from(["ferroehr", "config"]).is_err(),
        "`config` must require a utility"
    );
}

/// `healthcheck` takes an explicit URL and otherwise derives the local status
/// endpoint from the effective configuration, so the probe follows a
/// shortened `server.base_path` and a moved `server.bind` port.
#[test]
fn healthcheck_url_is_optional_with_a_derived_default() -> Result<(), clap::Error> {
    let explicit = Cli::try_parse_from(["ferroehr", "healthcheck", "--url", "http://h:8080/x"])?;
    assert!(
        format!("{explicit:?}").contains("http://h:8080/x"),
        "explicit URL lost: {explicit:?}"
    );
    let defaulted = Cli::try_parse_from(["ferroehr", "healthcheck"])?;
    assert!(
        format!("{defaulted:?}").contains("url: None"),
        "an absent URL must stay absent at parse time: {defaulted:?}"
    );
    // Built directly rather than loaded: the loader snapshots the process
    // environment, and a test runner's own `FERROEHR_*` variables are not
    // this test's subject.
    let mut config = ferroehr::config::FerroEhrConfig::default();
    let default_url = ferroehr_server::healthcheck_url(&config).expect("a port is configured");
    assert_eq!(default_url, "http://127.0.0.1:8080/ferroehr/rest/status");
    config.server.base_path = "/ferroehr/v1".to_owned();
    config.server.bind = "0.0.0.0:9090".to_owned();
    let shortened = ferroehr_server::healthcheck_url(&config).expect("a port is configured");
    assert_eq!(shortened, "http://127.0.0.1:9090/ferroehr/status");
    config.server.bind = "no-port".to_owned();
    assert!(
        ferroehr_server::healthcheck_url(&config).is_err(),
        "a bind address without a port must be refused, not probed"
    );
    Ok(())
}

/// `usage-report --print` parses with the start report as the default event,
/// takes `--event daily`, and refuses to run without `--print`, its only
/// action.
#[test]
fn usage_report_print_parses_and_requires_print() -> Result<(), clap::Error> {
    let start = Cli::try_parse_from(["ferroehr", "usage-report", "--print"])?;
    assert!(
        format!("{start:?}").contains("UsageReport { print: true, event: Start }"),
        "not the start report: {start:?}"
    );
    let daily = Cli::try_parse_from(["ferroehr", "usage-report", "--print", "--event", "daily"])?;
    assert!(
        format!("{daily:?}").contains("event: Daily"),
        "not the daily report: {daily:?}"
    );
    assert!(
        Cli::try_parse_from(["ferroehr", "usage-report"]).is_err(),
        "`usage-report` without --print must not parse"
    );
    assert!(
        Cli::try_parse_from(["ferroehr", "usage-report", "--print", "--event", "weekly"]).is_err(),
        "an unknown event must not parse"
    );
    Ok(())
}

/// An unknown subcommand is rejected rather than silently falling through to
/// the serve path.
#[test]
fn unknown_subcommand_is_rejected() {
    assert!(
        Cli::try_parse_from(["ferroehr", "migrate"]).is_err(),
        "an unknown subcommand must not parse"
    );
}

/// `ferroehr --version` names the manufacturer after the version: the name,
/// the postal address, the single point of contact and the website
/// (Regulation (EU) 2025/327, `docs/law/eu/ehds/text.html` Art. 30(1)(g)),
/// then the support period (Regulation (EU) 2024/2847,
/// `docs/law/eu/cra/text.html` Art. 13(19)).
#[test]
fn version_names_the_manufacturer() {
    let shown = Cli::try_parse_from(["ferroehr", "--version"])
        .expect_err("--version stops the parse to print")
        .to_string();
    assert_eq!(
        shown,
        format!(
            "ferroehr {}\nManufactured by Cadasto B.V., Comeniusstraat 2d, 1817 MS Alkmaar, The \
             Netherlands, info@cadasto.com\nhttps://www.cadasto.com/contact/\nSupport: {}\n",
            env!("CARGO_PKG_VERSION"),
            ferroehr::support::SupportPeriod::current().describe_on(ferroehr::support::today_utc())
        )
    );
}

/// `report` parses with and without a destination.
#[test]
fn report_parses_with_an_optional_output() -> Result<(), clap::Error> {
    let default = Cli::try_parse_from(["ferroehr", "report"])?;
    assert!(
        format!("{default:?}").contains("Report { output: None }"),
        "{default:?}"
    );
    let named = Cli::try_parse_from(["ferroehr", "report", "--output", "-"])?;
    assert!(
        format!("{named:?}").contains(r#"output: Some("-")"#),
        "{named:?}"
    );
    Ok(())
}

/// `ferroehr report` writes its file, through the seam its dispatch runs after
/// loading, against a database nothing answers on: the file names the
/// unreachable database in its manifest, and no credential of the
/// configuration reaches it.
#[tokio::test]
async fn run_report_writes_a_redacted_file_naming_an_unreachable_database() -> anyhow::Result<()> {
    const DB_PW: &str = "DB_PW_SENTINEL_31c7";
    const MIGRATE_PW: &str = "MIGRATE_PW_SENTINEL_84ad";
    const EVENTS_PW: &str = "EVENTS_PW_SENTINEL_5e02";
    let dir = assert_fs::TempDir::new()?;
    let config = dir.child("ferroehr.toml");
    config.write_str(&format!(
        "[db]\n\
         url = \"postgres://reportuser:{DB_PW}@127.0.0.1:1/ferroehr\"\n\
         migrate_url = \"postgres://migrator:{MIGRATE_PW}@127.0.0.1:1/ferroehr\"\n\
         [events]\n\
         url = \"amqp://mq:{EVENTS_PW}@broker:5672/vh\"\n"
    ))?;
    let output = dir.child("report.json");
    // Assembled with no environment, so a runner's own `FERROEHR_*` variables
    // stay out of the subject (the strict loader refuses unknown ones).
    let assembled =
        ferroehr::config::assemble(Some(config.path()), &std::collections::HashMap::new(), &[])
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    write_report(&assembled, Some(output.path())).await?;

    let written = std::fs::read_to_string(output.path())?;
    for leak in [
        DB_PW,
        MIGRATE_PW,
        EVENTS_PW,
        "reportuser",
        "migrator:",
        "mq:",
    ] {
        assert!(!written.contains(leak), "{leak} leaked:\n{written}");
    }
    assert!(
        written.contains(
            "\"part\": \"schema\",\n      \"status\": \"unavailable\",\n      \"reason\": \
             \"unreachable: "
        ),
        "{written}"
    );
    assert!(written.contains("\"name\": \"Cadasto B.V.\""), "{written}");
    Ok(())
}

/// `ferroehr config default` runs end to end through the real dispatch: it only
/// writes the annotated template to stdout, so it needs no database, listener,
/// or network.
#[tokio::test]
async fn run_config_default_is_a_pure_stdout_path() -> anyhow::Result<()> {
    let cli = Cli::try_parse_from(["ferroehr", "config", "default"])?;
    run(cli).await
}

/// Runs the built `ferroehr config check` against `toml` with an empty
/// environment, returning whether it succeeded and its stdout plus stderr.
fn config_check(toml: &str) -> (bool, String) {
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("a temp config path");
    file.write_str(toml).expect("write the config");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ferroehr"))
        .args(["config", "check", "--config"])
        .arg(file.path())
        .env_clear()
        .output()
        .expect("the binary runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

/// Userinfo in a URL key never reaches `ferroehr config check`: a secret-URL
/// key prints masked, and a key that refuses userinfo at boot refuses it
/// without quoting it.
#[test]
fn config_check_prints_no_url_userinfo() {
    let (ok, text) = config_check(
        r#"
[auth]
enabled = false

[db]
url = "postgres://u1:URL_PW_SENTINEL_1@h1:5432/p1"

[telemetry]
otlp_endpoint = "https://u2:URL_PW_SENTINEL_2@h2:4317/p2"

[multimedia]
endpoint = "https://u3:URL_PW_SENTINEL_3@h3:8333/p3"

[authz.abac.remote]
server = "https://u4:URL_PW_SENTINEL_4@h4:3001/p4/"

[terminology.external.providers.ts]
url = "https://u5:URL_PW_SENTINEL_5@h5:443/p5"

[terminology.external.oauth2_clients.client]
token_url = "https://u6:URL_PW_SENTINEL_6@h6:443/p6"
client_id = "client"
client_secret = "CLIENT_SECRET_SENTINEL"
"#,
    );
    assert!(ok, "the configuration checks: {text}");
    assert!(!text.contains("URL_PW_SENTINEL"), "{text}");
    assert!(!text.contains("CLIENT_SECRET_SENTINEL"), "{text}");
    for (n, port) in [
        (1, 5432),
        (2, 4317),
        (3, 8333),
        (4, 3001),
        (5, 443),
        (6, 443),
    ] {
        assert!(
            text.contains(&format!("://***@h{n}:{port}/p{n}")),
            "URL {n} lost its host and path: {text}"
        );
    }

    let (ok, text) = config_check(
        r#"
[auth.oidc]
issuer = "https://u7:URL_PW_SENTINEL_7@idp.example/realms/r"
audiences = ["ferroehr"]
hmac_secret = "an-hmac-secret-of-sufficient-length-for-hs256"
algorithms = ["HS256"]

[usage_report]
endpoint = "https://u8:URL_PW_SENTINEL_8@report.example/v1/report"

[smart.endpoints]
token_endpoint = "https://u9:URL_PW_SENTINEL_9@as.example/token"
"#,
    );
    assert!(!ok, "userinfo in these keys is refused: {text}");
    assert!(!text.contains("URL_PW_SENTINEL"), "{text}");
    assert!(text.contains("auth.oidc.issuer"), "{text}");
    assert!(text.contains("usage_report.endpoint"), "{text}");
    assert!(text.contains("smart.endpoints.token_endpoint"), "{text}");
}

// ── the boot-installed [privacy] policy ───────────────────────────────────────

/// The SHIPPED DEFAULT configuration, assembled through the loader the binary
/// runs, with no file, environment or override of this test's own.
///
/// [`ferroehr::config::assemble`] is the pure seam
/// [`load`](ferroehr::config::load) is a process-environment shim over, so the
/// shipped template travels through the real loader while a test runner's own
/// `FERROEHR_*` variables stay out of the subject.
fn shipped_default_config() -> ferroehr::config::FerroEhrConfig {
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("a temp config path");
    assert_fs::prelude::FileWriteStr::write_str(&file, ferroehr::config::DEFAULT_TEMPLATE)
        .expect("write the shipped template");
    ferroehr::config::assemble(Some(file.path()), &std::collections::HashMap::new(), &[])
        .expect("the shipped template assembles")
}

/// The [`WebTemplate`](openehr_sdt::flat::webtemplate::model::WebTemplate) of
/// the operational template the browser journey battery seeds.
///
/// The real generator is driven off this rather than a literal body: a
/// hand-written composition would drift away from what
/// `GET /definition/template/adl1.4/{id}/example` actually hands out, which is
/// the loop that has to hold.
fn seed_web_template() -> openehr_sdt::flat::webtemplate::model::WebTemplate {
    let opt = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ferroehr-viewer/tests/fixtures/minimal_evaluation.opt");
    let xml = std::fs::read_to_string(&opt).expect("the seed operational template reads");
    let parsed = openehr_its::opt14::from_xml(&xml).expect("the seed OPT parses");
    openehr_sdt::flat::webtemplate::builder::build_web_template(&parsed)
        .expect("the seed OPT builds a WebTemplate")
}

/// A server booted on the shipped defaults accepts the composition its own
/// example endpoint generates, at every detail level that endpoint offers.
///
/// The regression this pins is a wiring one, which is why it lives here: the
/// platform suites build the service with `FerroEhrService::new()`, which
/// carries the unenforced [`ferroehr::privacy::PrivacyPolicy`] default, and
/// only the binary compiles `[privacy]` and installs it
/// ([`ferroehr::privacy::PrivacyPolicy::compile`] + `with_privacy`). A default
/// posture that refuses `ctx/composer_name` therefore looks green everywhere
/// except in a real deployment.
#[test]
fn the_shipped_privacy_default_accepts_this_servers_own_example_composition() {
    let config = shipped_default_config();
    assert!(
        !config.privacy.allow_identified_parties_in_ehr,
        "the shipped default must stay the minimising posture"
    );
    let policy = ferroehr::privacy::PrivacyPolicy::compile(&config.privacy)
        .expect("the shipped [privacy] section compiles");
    let wt = seed_web_template();
    for (label, level) in [
        (
            "required",
            openehr_sdt::flat::example::DetailLevel::Required,
        ),
        ("medium", openehr_sdt::flat::example::DetailLevel::Medium),
        (
            "complete",
            openehr_sdt::flat::example::DetailLevel::Complete,
        ),
    ] {
        let example = openehr_sdt::flat::example::example_composition(&wt, level);
        let findings = policy.findings("COMPOSITION", &example);
        assert!(
            findings.is_empty(),
            "the shipped default refuses the example this server generates at \
             detail_level={label}: {findings:?}"
        );
    }
}
