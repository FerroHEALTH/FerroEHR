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

use clap::Parser as _;

use ferroehr_server::{Cli, run};

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

/// `ferroehr config default` runs end to end through the real dispatch: it only
/// writes the annotated template to stdout, so it needs no database, listener,
/// or network.
#[tokio::test]
async fn run_config_default_is_a_pure_stdout_path() -> anyhow::Result<()> {
    let cli = Cli::try_parse_from(["ferroehr", "config", "default"])?;
    run(cli).await
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
