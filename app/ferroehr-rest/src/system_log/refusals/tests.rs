// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusal budgets and the writing of refusal records.

use ferroehr::system_log::binding::BindingCode;
use ferroehr::system_log::event::{EventActionCode, EventOutcome, ObjectClass};

use super::*;

fn limits() -> RefusalLimits {
    RefusalLimits {
        window: Duration::from_secs(60),
        per_caller: 2,
        budget: 3,
    }
}

fn event(code: Option<BindingCode>, issuer: &str, user: &str) -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Execute,
        ObjectClass::Authentication,
        EventOutcome::MinorFailure,
    );
    event.issuer = Some(issuer.to_owned());
    event.user_id = user.to_owned();
    event.binding = code.map(BindingRecord::of);
    event
}

fn caller(subject: &str) -> Charged {
    Charged::Caller {
        issuer: "https://idp".to_owned(),
        subject: subject.to_owned(),
    }
}

fn charge(
    ledger: &RefusalLedger,
    now: Instant,
    charged: Charged,
    ehr: Option<&str>,
    event: &AuditEvent,
) -> Charge {
    ledger.charge(
        now,
        &Refusal {
            charged,
            outcome: event
                .binding
                .as_ref()
                .map_or("invalid_token", |binding| binding.code.as_str()),
            ehr_id: ehr,
            event,
            summarised: false,
        },
    )
}

#[test]
fn over_its_limit_a_callers_refusals_become_one_summary_with_the_ehr_ids() {
    let ledger = RefusalLedger::new(limits());
    let now = Instant::now();
    let refused = event(Some(BindingCode::RefusedInactive), "https://idp", "u1");
    let mut dispositions = Vec::new();
    let mut close_at = Vec::new();
    for ehr in ["e1", "e2", "e3", "e4", "e3"] {
        let charged = charge(&ledger, now, caller("u1"), Some(ehr), &refused);
        dispositions.push(charged.disposition);
        close_at.extend(charged.close_at);
    }
    assert_eq!(
        dispositions,
        [
            Disposition::Record,
            Disposition::Record,
            Disposition::Summarised,
            Disposition::Summarised,
            Disposition::Summarised
        ]
    );
    assert_eq!(close_at, [now + limits().window], "one close per window");
    assert!(ledger.close_due(now).is_empty(), "the window is still open");
    let due = ledger.close_due(now + limits().window);
    assert_eq!(due.len(), 1);
    assert_eq!(
        due[0].summary,
        Some(Summary {
            count: 3,
            ehr_ids: vec!["e3".to_owned(), "e4".to_owned()],
        })
    );
    assert_eq!(due[0].user_id, "u1");
    assert_eq!(
        due[0].binding.as_ref().map(|b| b.code),
        Some(BindingCode::RefusedInactive)
    );
}

#[test]
fn a_flood_of_invalid_tokens_never_stops_a_verified_callers_record() {
    let ledger = RefusalLedger::new(limits());
    let now = Instant::now();
    let invalid = event(None, "", "");
    for host in 0..50u8 {
        let address = IpAddr::from([198, 51, host, 7]);
        charge(
            &ledger,
            now,
            Charged::network(Some(address)),
            None,
            &invalid,
        );
    }
    let probing = event(Some(BindingCode::RefusedUnbound), "https://idp", "u1");
    assert_eq!(
        charge(&ledger, now, caller("u1"), Some("e1"), &probing).disposition,
        Disposition::Record
    );
}

#[test]
fn beyond_the_budget_one_summary_is_written_per_issuer_and_outcome() {
    let ledger = RefusalLedger::new(limits());
    let now = Instant::now();
    let refused = |user: &str| event(Some(BindingCode::RefusedUnbound), "https://idp", user);
    let mut dispositions = Vec::new();
    for user in ["u1", "u2", "u3", "u4", "u5"] {
        dispositions.push(charge(&ledger, now, caller(user), None, &refused(user)).disposition);
    }
    let other = event(Some(BindingCode::RefusedGuest), "https://idp", "u6");
    dispositions.push(charge(&ledger, now, caller("u6"), Some("e9"), &other).disposition);
    assert_eq!(
        dispositions,
        [
            Disposition::Record,
            Disposition::Record,
            Disposition::Record,
            Disposition::Summarised,
            Disposition::Summarised,
            Disposition::Summarised
        ]
    );
    // A caller recorded before the budget ran out keeps its own tally.
    assert_eq!(
        charge(&ledger, now, caller("u1"), None, &refused("u1")).disposition,
        Disposition::Record
    );
    let due = ledger.close_due(now + limits().window);
    let summaries: Vec<(Option<BindingCode>, u64, &str)> = due
        .iter()
        .map(|event| {
            (
                event.binding.as_ref().map(|b| b.code),
                event.summary.as_ref().map_or(0, |s| s.count),
                event.user_id.as_str(),
            )
        })
        .collect();
    assert_eq!(
        summaries,
        [
            (Some(BindingCode::RefusedGuest), 1, ""),
            (Some(BindingCode::RefusedUnbound), 2, ""),
        ],
        "an issuer-and-outcome summary names no caller"
    );
}

#[test]
fn a_security_event_is_recorded_for_every_answered_request() {
    let ledger = RefusalLedger::new(limits());
    let now = Instant::now();
    let mismatch = event(
        Some(BindingCode::RefusedTrcPatientMismatch),
        "https://ncp",
        "u1",
    );
    for _ in 0..20 {
        let charged = charge(&ledger, now, caller("u1"), Some("e1"), &mismatch);
        assert_eq!(charged.disposition, Disposition::Record);
        assert_eq!(charged.close_at, None);
    }
    assert!(ledger.close_due(now + limits().window).is_empty());
}

#[test]
fn a_closed_window_hands_its_summaries_to_the_next_refusal() {
    let ledger = RefusalLedger::new(limits());
    let now = Instant::now();
    let refused = event(Some(BindingCode::RefusedAmbiguous), "https://idp", "u1");
    for _ in 0..3 {
        charge(&ledger, now, caller("u1"), None, &refused);
    }
    let later = charge(&ledger, now + limits().window, caller("u1"), None, &refused);
    assert_eq!(later.disposition, Disposition::Record, "a new window");
    assert_eq!(later.due.len(), 1);
    assert_eq!(later.due[0].summary.as_ref().map(|s| s.count), Some(1));
}

/// A refusal recorded one by one comes back for its request to write
/// with its other records, so it is durable before the refusal is
/// answered; a summarised one does not.
#[tokio::test]
async fn a_recorded_refusal_is_written_with_its_request() {
    let state = state(None);
    let refused = event(Some(BindingCode::RefusedUnbound), "https://idp", "u1");
    let own = record(
        &state,
        &Refusal {
            charged: caller("u1"),
            outcome: "refused-unbound",
            ehr_id: None,
            event: &refused,
            summarised: false,
        },
    );
    assert!(own.is_some(), "the refusal's own record");
    let summarised = record(
        &state,
        &Refusal {
            charged: caller("u2"),
            outcome: "caller-rate",
            ehr_id: None,
            event: &refused,
            summarised: true,
        },
    );
    assert!(summarised.is_none(), "it goes into the caller's summary");
}

/// A summary the system log cannot write is counted and logged.
#[tokio::test]
async fn a_summary_the_system_log_cannot_write_is_counted() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(200))
        .connect_lazy_with(
            sqlx::postgres::PgConnectOptions::new()
                .host("127.0.0.1")
                .port(9),
        );
    let config = ferroehr::system_log::config::AuditConfig {
        store: ferroehr::system_log::config::StoreConfig {
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
        },
        ..ferroehr::system_log::config::AuditConfig::default()
    };
    let (sender, handle) = ferroehr::system_log::sender::start(config, None, pool)
        .await
        .expect("the sender starts on a lazy pool");
    let state = state(Some(sender));
    let summary = event(Some(BindingCode::RefusedUnbound), "https://idp", "u1");
    write_summaries(&state, vec![summary]).await;
    assert_eq!(state.refusals().unwritten(), 1);
    handle.shutdown(Duration::from_secs(1)).await;
}

/// The state of an app whose service records through `sender`, when one
/// is given.
fn state(sender: Option<ferroehr::system_log::sender::AuditSender>) -> AppState {
    let service =
        ferroehr::service::FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
            &sqlx::postgres::PgPoolOptions::new()
                .connect_lazy_with(sqlx::postgres::PgConnectOptions::new()),
        ));
    let service = match sender {
        Some(sender) => service.with_audit(sender),
        None => service,
    };
    AppState::with_backend(
        crate::config::AppConfig::default(),
        std::sync::Arc::new(service),
    )
}

#[test]
fn a_refusal_above_a_caller_rate_goes_into_the_callers_summary_from_the_first() {
    let ledger = RefusalLedger::new(limits());
    let now = Instant::now();
    let over = event(Some(BindingCode::External), "https://federation", "hp-1");
    let charged = Charged::Caller {
        issuer: "https://federation".to_owned(),
        subject: "hp-1".to_owned(),
    };
    let mut close_at = Vec::new();
    for ehr in ["e1", "e2", "e1"] {
        let charge = ledger.charge(
            now,
            &Refusal {
                charged: charged.clone(),
                outcome: "caller-rate",
                ehr_id: Some(ehr),
                event: &over,
                summarised: true,
            },
        );
        assert_eq!(charge.disposition, Disposition::Summarised);
        close_at.extend(charge.close_at);
    }
    assert_eq!(
        close_at.len(),
        1,
        "the first summarised refusal closes the window"
    );
    let due = ledger.close_due(now + limits().window);
    assert_eq!(due.len(), 1);
    assert_eq!(
        due[0].summary,
        Some(Summary {
            count: 3,
            ehr_ids: vec!["e1".to_owned(), "e2".to_owned()],
        })
    );
    assert_eq!(due[0].user_id, "hp-1", "the summary names its caller");
}

#[test]
fn a_network_is_its_ipv4_slash_24_or_its_ipv6_slash_64() {
    assert_eq!(
        Charged::network(Some(IpAddr::from([203, 0, 113, 9]))),
        Charged::Network("203.0.113.0/24".to_owned())
    );
    let v6: IpAddr = "2001:db8:1:2:3:4:5:6".parse().expect("an address");
    assert_eq!(
        Charged::network(Some(v6)),
        Charged::Network("2001:db8:1:2::/64".to_owned())
    );
    assert_eq!(
        Charged::network(None),
        Charged::Network("unknown".to_owned())
    );
}
