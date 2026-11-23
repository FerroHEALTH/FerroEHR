// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The database error chain, the domain layout and the pool defaults.

use super::*;
use crate::db::pool::POOL_MIN;

/// Every `DbError` variant wrapping a cause keeps it as the source and out
/// of its own message, so a chain printer shows it once (#3684).
#[test]
fn wrapped_causes_are_sources_and_not_repeated() {
    use std::error::Error as _;
    let error = DbError::from(sqlx::Error::Protocol("driver detail".to_owned()));
    let source = error.source().expect("the driver error is the source");
    assert!(source.downcast_ref::<sqlx::Error>().is_some());
    assert!(!error.to_string().contains("driver detail"), "{error}");

    let error = DbError::from(sqlx::migrate::MigrateError::VersionMissing(7));
    let source = error.source().expect("the migrator error is the source");
    assert!(
        source
            .downcast_ref::<sqlx::migrate::MigrateError>()
            .is_some()
    );
    assert!(!error.to_string().contains('7'), "{error}");

    let mismatch = SchemaMismatch::NeverMigrated {
        schema: "clinical".to_owned(),
    };
    let mismatch_text = mismatch.to_string();
    let error = DbError::SchemaNotReady(mismatch);
    let source = error.source().expect("the divergence is the source");
    assert!(source.downcast_ref::<SchemaMismatch>().is_some());
    assert!(!error.to_string().contains(&mismatch_text), "{error}");
}

/// With no domain relocated and no domain migrator named, the plan is the
/// one database `[database]` prepares, and it opens no connection to find that
/// out (the DSN here reaches nothing).
#[tokio::test]
async fn a_co_located_layout_plans_one_database_without_connecting() {
    let settings = DbConfig::new("postgres://nobody@unreachable.invalid:1/none");
    let plan = preparation_plan(&settings)
        .await
        .expect("no connection is needed");
    assert_eq!(plan.len(), 1);
    let only = plan.first().expect("one group");
    assert_eq!(only.domains, Domain::ALL);
    assert_eq!(only.credential, PreparationCredential::Migrator);
}

#[test]
fn defaults_applied() {
    let s = DbConfig::new("postgres://localhost/ferroehr");
    assert_eq!(s.url.expose(), "postgres://localhost/ferroehr");
    assert_eq!(s.max_connections, None);
    assert_eq!(s.min_connections, None);
    assert!((POOL_MIN..=s.pool_share()).contains(&s.pool_size()));
    assert_eq!(s.pool_kept(), 0);
    assert_eq!(s.acquire_timeout, units::Duration::from_secs(30));
    assert!(!s.is_dev_default());
}

#[test]
fn default_url_is_the_dev_dsn() {
    assert!(DbConfig::default().is_dev_default());
}

/// The server bounds an unset pool size by what it admits at once and keeps
/// every connection of the pool open under an unset floor, so it opens none
/// under load (#5774); a set size and a set floor are kept, and a pool never
/// keeps more than its size.
#[test]
fn the_server_sizes_an_unset_pool_for_its_concurrency_and_keeps_all_of_it_open() {
    let unset = DbConfig::new("postgres://localhost/ferroehr");
    let small = unset.for_serving(1);
    assert_eq!(
        (small.max_connections, small.min_connections),
        (Some(1), Some(1))
    );
    assert_eq!(unset.for_serving(0).max_connections, Some(1));
    let derived = unset.for_serving(256);
    assert_eq!(derived.max_connections, Some(unset.pool_size()));
    assert_eq!(derived.pool_kept(), unset.pool_size());
    let mut sized = unset.clone();
    sized.max_connections = Some(20);
    assert_eq!(sized.for_serving(256).pool_kept(), 20);
    let mut set = unset.clone();
    set.max_connections = Some(50);
    set.min_connections = Some(80);
    assert_eq!(set.for_serving(4).max_connections, Some(50));
    assert_eq!(set.pool_kept(), 50);
    set.min_connections = Some(5);
    assert_eq!(set.for_serving(4).pool_kept(), 5);
}

/// One instance with every default opens at most its budget of 40: one
/// listener session for every channel, the audit runtime's four and three
/// pools of at most 11, so two instances fit `PostgreSQL`'s default
/// `max_connections` of 100 less its three `superuser_reserved_connections`
/// (#5845).
#[test]
fn two_default_instances_fit_a_default_postgresql() {
    let settings = DbConfig::default();
    let serving = settings.for_serving(256);
    assert_eq!(settings.connection_budget, 40);
    assert_eq!(
        serving.listen_sessions(),
        1,
        "one session carries every channel"
    );
    assert_eq!(serving.pool_share(), 11);
    assert!(serving.pool_size() <= 11);
    assert_eq!(serving.connections(), serving.pool_size() * 3 + 4 + 1);
    assert!(serving.connections() <= 40, "{}", serving.connections());
    assert!(serving.connections() * 2 <= 100 - 3);
    assert_eq!(serving.budget_errors(), Vec::<String>::new());
}

/// A domain listening on a DSN of its own takes a second session, which the
/// pools' share gives up; one listen DSN for both domains keeps one.
#[test]
fn each_listen_dsn_takes_one_session_out_of_the_budget() {
    let mut settings = DbConfig::new("postgres://localhost/ferroehr");
    settings.listen_url = Some(SecretUrl::new("postgres://primary/ferroehr"));
    assert_eq!(settings.listen_sessions(), 1);
    settings.party.listen_url = Some(SecretUrl::new("postgres://party/ferroehr"));
    assert_eq!(settings.listen_sessions(), 2);
    assert_eq!(settings.pool_share(), 11);
    settings.connection_budget = 100;
    assert_eq!(settings.pool_share(), 31);
    assert!(settings.connections() <= 100);
}

/// A budget that leaves a pool fewer than two connections, and a set pool
/// size whose pools and listener exceed the budget, are refused naming the
/// budget; a set size inside it is kept.
#[test]
fn a_pool_outside_the_budget_is_refused_naming_it() {
    let mut settings = DbConfig::new("postgres://localhost/ferroehr");
    settings.connection_budget = 6;
    let errors = settings.budget_errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors
            .iter()
            .all(|e| e.contains("database.connection_budget = 6") && e.contains("at least 11")),
        "{errors:?}"
    );
    settings.connection_budget = 40;
    settings.max_connections = Some(30);
    let errors = settings.budget_errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors
            .iter()
            .all(|e| e.contains("95 in all") && e.contains("database.connection_budget = 40")),
        "{errors:?}"
    );
    settings.max_connections = Some(11);
    assert_eq!(settings.budget_errors(), Vec::<String>::new());
    assert_eq!(settings.connections(), 38);
}

/// A write takes an audit connection before its own (#5964), so the audit
/// pool is never smaller than the clinical or the party pool: every domain
/// pool is built at one size, set or derived.
#[test]
fn the_audit_pool_is_never_smaller_than_a_domain_pool() {
    let unset = DbConfig::new("postgres://localhost/ferroehr");
    let mut set = unset.clone();
    set.max_connections = Some(7);
    for settings in [
        unset.clone(),
        unset.for_serving(3),
        unset.for_serving(256),
        set,
    ] {
        let size = |domain| pool::options(&settings, domain).get_max_connections();
        let audit = size(Domain::Audit);
        assert!(audit >= size(Domain::Clinical), "{settings:?}");
        assert!(audit >= size(Domain::Party), "{settings:?}");
    }
}

/// A pool whose connections all opened at one instant, as the server opens
/// them before it answers ready (#5774), retires them one at a time across
/// [`pool::RETIRE_SPREAD`], each released every second (#6341).
#[test]
fn a_full_pool_opened_at_once_retires_its_connections_across_the_spread() {
    let size: u32 = 20;
    let retirement = pool::Retirement::new(size);
    let start = std::time::Instant::now();
    let mut born = vec![std::time::Duration::ZERO; usize::try_from(size).expect("fits")];
    let mut retired = Vec::new();
    for second in 0..2 * pool::RETIRE_SPREAD.as_secs() {
        let now = pool::LIFETIME + std::time::Duration::from_secs(second);
        for birth in &mut born {
            if retirement.retires(now.saturating_sub(*birth), start + now) {
                retired.push(now);
                *birth = now;
            }
        }
    }
    assert_eq!(retired.len(), born.len(), "each connection retires once");
    let spacing = pool::RETIRE_SPREAD / size;
    for pair in retired.windows(2) {
        assert!(
            pair[1].saturating_sub(pair[0]) >= spacing,
            "{pair:?} at least {spacing:?} apart"
        );
    }
    let first = retired.first().copied().expect("a retirement");
    let last = retired.last().copied().expect("a retirement");
    assert_eq!(first, pool::LIFETIME, "the first retires at the lifetime");
    assert!(
        last.saturating_sub(first) < pool::RETIRE_SPREAD,
        "{last:?} within the spread"
    );
    assert!(
        last.saturating_sub(first) >= pool::RETIRE_SPREAD.saturating_sub(spacing),
        "{last:?} spread over the window"
    );
}

/// A connection past [`pool::LIFETIME_CAP`] retires on its next release even
/// while the pacing holds the others back.
#[test]
fn a_connection_past_the_cap_retires_whatever_the_pacing() {
    let retirement = pool::Retirement::new(1);
    let now = std::time::Instant::now();
    assert!(!retirement.retires(
        pool::LIFETIME.saturating_sub(std::time::Duration::from_secs(1)),
        now
    ));
    assert!(retirement.retires(pool::LIFETIME, now));
    assert!(!retirement.retires(pool::LIFETIME, now), "paced");
    assert!(retirement.retires(pool::LIFETIME_CAP, now), "capped");
}
