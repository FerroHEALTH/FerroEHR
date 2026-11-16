// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusal records: every refused request becomes an access record through
//! the gated path, within two budgets and with summary records beyond them.
//!
//! A refusal of a token that failed verification is charged to the source
//! network it came from, and a refusal of a verified caller to its issuer and
//! subject. The two never share a budget, so a flood of invalid tokens cannot
//! crowd out the record of a verified caller's probing. Within one window a
//! caller's first refusals are recorded one by one; past
//! [`RefusalLimits::per_caller`] they become one summary record carrying the
//! count and the EHR ids they named. Past a class's
//! [`RefusalLimits::budget`] the refusals of every caller not yet tracked
//! become one summary record per issuer and outcome. A security event is
//! recorded once per answered request and never summarised (XCA 8.0.1
//! §4.2.2). The limits, windows and budgets are kept per replica.
//!
//! No openEHR spec governs access logging: our own design.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use axum::response::Response;
use ferroehr::system_log::binding::{BindingRecord, Summary};
use ferroehr::system_log::event::{
    AccessDecision, AuditEvent, EntryScope, EventActionCode, EventType, ObjectClass,
};
use http::StatusCode;
use tokio::time::{Duration, Instant};

use crate::state::AppState;
use crate::system_log::middleware::{
    AccessContext, Caller, fill_access, fill_common, outcome_from_status,
};

/// The most EHR ids one summary record names; the count stays exact.
const SUMMARY_EHR_IDS: usize = 64;

/// The outcome an authentication refusal was answered with, which the
/// authentication layer puts on its response.
#[derive(Debug, Clone, Copy)]
pub struct Reason(pub &'static str);

/// The mark of a refusal of a federation caller above its agreement's
/// `caller_rate`, which goes into the caller's summary record and is never
/// recorded by itself.
#[derive(Debug, Clone, Copy)]
pub struct OverRate;

/// The refusal limits of one replica.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefusalLimits {
    /// The length of one summary window.
    pub window: Duration,
    /// The refusals of one caller recorded one by one in a window.
    pub per_caller: u64,
    /// The refusals of one class recorded one by one in a window.
    pub budget: u64,
}

impl Default for RefusalLimits {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(60),
            per_caller: 10,
            budget: 1000,
        }
    }
}

/// Whom a refusal is charged to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Charged {
    /// A token that failed verification, by the network it came from: the
    /// `/24` of an IPv4 address, the `/64` of an IPv6 one.
    Network(String),
    /// A verified caller, by its token's issuer and subject.
    Caller {
        /// The issuer; empty for a Basic user.
        issuer: String,
        /// The subject.
        subject: String,
    },
}

impl Charged {
    /// The network `address` belongs to; `unknown` for a request served over
    /// no connection.
    #[must_use]
    pub fn network(address: Option<IpAddr>) -> Self {
        Self::Network(match address {
            Some(IpAddr::V4(v4)) => {
                let [a, b, c, _] = v4.octets();
                format!("{a}.{b}.{c}.0/24")
            }
            Some(IpAddr::V6(v6)) => {
                let [a, b, c, d, ..] = v6.segments();
                format!("{a:x}:{b:x}:{c:x}:{d:x}::/64")
            }
            None => "unknown".to_owned(),
        })
    }
}

/// One refused request, as the access log sees it.
#[derive(Debug)]
pub struct Refusal<'a> {
    /// Whom it is charged to.
    pub charged: Charged,
    /// The outcome, such as a binding code or `invalid_token`.
    pub outcome: &'a str,
    /// The EHR the request named.
    pub ehr_id: Option<&'a str>,
    /// Its access record.
    pub event: &'a AuditEvent,
    /// Whether it goes into its caller's summary record whatever the caller's
    /// count: a refusal above an agreement's `caller_rate`.
    pub summarised: bool,
}

/// Whether a refusal is recorded by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Recorded by itself.
    Record,
    /// Counted into a summary record.
    Summarised,
}

/// What charging one refusal decided.
#[derive(Debug)]
pub struct Charge {
    /// The refusal's own disposition.
    pub disposition: Disposition,
    /// The summary records of the window that closed before it.
    pub due: Vec<AuditEvent>,
    /// When to close the window, when this refusal opened its first summary.
    pub close_at: Option<Instant>,
}

/// The refusals one summary record stands for.
#[derive(Debug)]
struct Pending {
    event: AuditEvent,
    count: u64,
    ehr_ids: BTreeSet<String>,
}

impl Pending {
    fn new(event: AuditEvent) -> Self {
        Self {
            event,
            count: 0,
            ehr_ids: BTreeSet::new(),
        }
    }

    fn add(&mut self, ehr_id: Option<&str>) {
        self.count = self.count.saturating_add(1);
        if let Some(ehr_id) = ehr_id
            && self.ehr_ids.len() < SUMMARY_EHR_IDS
        {
            self.ehr_ids.insert(ehr_id.to_owned());
        }
    }

    fn into_event(self) -> AuditEvent {
        let mut event = self.event;
        event.summary = Some(Summary {
            count: self.count,
            ehr_ids: self.ehr_ids.into_iter().collect(),
        });
        event.request_id = None;
        event.ehr_id = None;
        event.timestamp = jiff::Timestamp::now();
        event
    }
}

/// One caller's refusals in the window.
#[derive(Debug, Default)]
struct Tally {
    recorded: u64,
    summary: Option<Pending>,
}

/// One budget class in the window.
#[derive(Debug, Default)]
struct Class {
    recorded: u64,
    callers: BTreeMap<Charged, Tally>,
    overflow: BTreeMap<(String, String), Pending>,
}

impl Class {
    fn drain(&mut self, into: &mut Vec<AuditEvent>) {
        let callers = std::mem::take(&mut self.callers);
        into.extend(
            callers
                .into_values()
                .filter_map(|tally| tally.summary)
                .map(Pending::into_event),
        );
        let overflow = std::mem::take(&mut self.overflow);
        into.extend(overflow.into_values().map(Pending::into_event));
        self.recorded = 0;
    }
}

/// The current window.
#[derive(Debug)]
struct Window {
    end: Instant,
    /// The refusals of tokens that failed verification.
    failed: Class,
    /// The refusals of verified callers.
    verified: Class,
    close_scheduled: bool,
}

impl Window {
    /// The budget class `charged` draws on.
    fn class(&mut self, charged: &Charged) -> &mut Class {
        match charged {
            Charged::Network(_) => &mut self.failed,
            Charged::Caller { .. } => &mut self.verified,
        }
    }

    fn drain(&mut self, into: &mut Vec<AuditEvent>) {
        self.failed.drain(into);
        self.verified.drain(into);
    }
}

/// The refusal budgets of this replica.
#[derive(Debug)]
pub struct RefusalLedger {
    limits: RefusalLimits,
    window: Mutex<Option<Window>>,
    unwritten: AtomicU64,
}

impl RefusalLedger {
    /// A ledger under `limits`.
    #[must_use]
    pub fn new(limits: RefusalLimits) -> Self {
        Self {
            limits,
            window: Mutex::new(None),
            unwritten: AtomicU64::new(0),
        }
    }

    /// Charges `refusal` at `now`.
    pub fn charge(&self, now: Instant, refusal: &Refusal<'_>) -> Charge {
        let mut due = Vec::new();
        if refusal
            .event
            .binding
            .as_ref()
            .is_some_and(|binding| binding.code.is_security_event())
        {
            return Charge {
                disposition: Disposition::Record,
                due,
                close_at: None,
            };
        }
        let mut guard = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        let window = Self::current(&mut guard, now, self.limits.window, &mut due);
        let end = window.end;
        let close_scheduled = window.close_scheduled;
        let class = window.class(&refusal.charged);
        let tracked = class.callers.contains_key(&refusal.charged);
        let disposition = if !tracked && class.recorded >= self.limits.budget {
            let issuer = refusal.event.issuer.clone().unwrap_or_default();
            class
                .overflow
                .entry((issuer, refusal.outcome.to_owned()))
                .or_insert_with(|| Pending::new(anonymous(refusal.event)))
                .add(refusal.ehr_id);
            Disposition::Summarised
        } else {
            let tally = class.callers.entry(refusal.charged.clone()).or_default();
            if !refusal.summarised && tally.recorded < self.limits.per_caller {
                tally.recorded += 1;
                class.recorded = class.recorded.saturating_add(1);
                Disposition::Record
            } else {
                tally
                    .summary
                    .get_or_insert_with(|| Pending::new(refusal.event.clone()))
                    .add(refusal.ehr_id);
                Disposition::Summarised
            }
        };
        let close_at = (disposition == Disposition::Summarised && !close_scheduled).then_some(end);
        if close_at.is_some() {
            window.close_scheduled = true;
        }
        Charge {
            disposition,
            due,
            close_at,
        }
    }

    /// The summary records of a window that ended by `now`.
    pub fn close_due(&self, now: Instant) -> Vec<AuditEvent> {
        let mut due = Vec::new();
        let mut guard = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.as_ref().is_some_and(|window| now >= window.end)
            && let Some(mut window) = guard.take()
        {
            window.drain(&mut due);
        }
        due
    }

    /// How many refusal records the system log could not take.
    #[must_use]
    pub fn unwritten(&self) -> u64 {
        self.unwritten.load(Ordering::Relaxed)
    }

    /// The window `now` falls in, opening a new one and draining the closed
    /// one's summaries into `due` when the last has ended.
    fn current<'w>(
        slot: &'w mut Option<Window>,
        now: Instant,
        length: Duration,
        due: &mut Vec<AuditEvent>,
    ) -> &'w mut Window {
        if let Some(window) = slot.as_mut()
            && now >= window.end
        {
            window.drain(due);
            *slot = None;
        }
        slot.get_or_insert_with(|| Window {
            end: now + length,
            failed: Class::default(),
            verified: Class::default(),
            close_scheduled: false,
        })
    }
}

/// The template of a summary over many callers: the record without what
/// names one caller or one request.
fn anonymous(event: &AuditEvent) -> AuditEvent {
    let mut event = event.clone();
    event.user_id.clear();
    event.client_ip = None;
    event.client_id = None;
    event.token_id = None;
    event.roles.clear();
    event.actor = ferroehr::system_log::event::ActorAuthentication::default();
    event.binding = event
        .binding
        .as_ref()
        .map(|binding| BindingRecord::of(binding.code));
    event
}

/// Charges `refusal` to its budget and writes the summaries a closed window
/// left; the refusal's own record when it is recorded one by one, for the
/// request to write with its other records before it answers.
pub(super) fn record(state: &AppState, refusal: &Refusal<'_>) -> Option<AuditEvent> {
    let ledger = state.refusals();
    let charge = ledger.charge(Instant::now(), refusal);
    if !charge.due.is_empty() {
        let state = state.clone();
        drop(tokio::spawn(async move {
            write_summaries(&state, charge.due).await;
        }));
    }
    if let Some(at) = charge.close_at {
        let state = state.clone();
        drop(tokio::spawn(async move {
            tokio::time::sleep_until(at).await;
            let due = state.refusals().close_due(Instant::now());
            write_summaries(&state, due).await;
        }));
    }
    (charge.disposition == Disposition::Record).then(|| refusal.event.clone())
}

/// Writes the summary records of closed windows, counting and logging at
/// `error` the ones the system log could not write.
///
/// A summary is the count of refusals already answered, kept in this
/// replica's memory until its window closes; no specification governs it:
/// our own design.
pub(super) async fn write_summaries(state: &AppState, summaries: Vec<AuditEvent>) {
    if summaries.is_empty() {
        return;
    }
    let count = u64::try_from(summaries.len()).unwrap_or(u64::MAX);
    if let Err(error) = state.backend().record_events(summaries).await {
        state
            .refusals()
            .unwritten
            .fetch_add(count, Ordering::Relaxed);
        ferroehr::telemetry::metrics::metrics()
            .atna_audit_refusals_unwritten
            .add(count, &[opentelemetry::KeyValue::new("kind", "summary")]);
        tracing::error!(
            error = %ferroehr::error::ErrorChain::new(&error),
            "the system log could not write the refusal summaries"
        );
    }
}

/// Whether `resp` refused the request: a `401`, a `403`, a caller above its
/// agreement's `caller_rate` ([`OverRate`]), or a security event such as a TRC
/// patient mismatch, whatever its status.
pub(super) fn is_refusal(resp: &Response) -> bool {
    let status = resp.status();
    status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || resp.extensions().get::<OverRate>().is_some()
        || resp
            .extensions()
            .get::<BindingRecord>()
            .is_some_and(|binding| binding.code.is_security_event())
}

/// Records the refusal `resp` answered as a failed user authentication
/// (DICOM PS3.15 §A.5.1 `EventID` 110114, `EventTypeCode` 110122), charged
/// to the verified caller when there is one and to the client's network
/// otherwise, and counted into the caller's summary when it is [`OverRate`];
/// its own record when it is recorded one by one. A refusal no
/// operation record of the request covers (`op_recorded` false) is the
/// access-log entry of the refused access.
#[expect(
    clippy::too_many_arguments,
    reason = "the request facts the middleware read once, handed to the one refusal recorder"
)]
pub(super) fn refused(
    state: &AppState,
    resp: &Response,
    caller: Caller<'_>,
    client_ip: Option<String>,
    timestamp: jiff::Timestamp,
    access: &AccessContext,
    ehr_id: Option<String>,
    op_recorded: bool,
) -> Option<AuditEvent> {
    let mut event = AuditEvent::new(
        EventActionCode::Execute,
        ObjectClass::Authentication,
        outcome_from_status(resp.status()),
    );
    event.event_type = Some(EventType::Login);
    event.decision = AccessDecision::Refused;
    if !op_recorded {
        event.entry = EntryScope::Refusal;
    }
    // NOTE: no specification governs this: our own design; the address was
    // rendered from an `IpAddr`, so only its absence leaves it unparsed.
    let address = client_ip.as_deref().and_then(|ip| ip.parse().ok());
    // A refused credential has no principal, so the caller is `UNKNOWN`; a
    // 403 and a patient-data step-up 401 carry one.
    fill_common(&mut event, caller, client_ip, timestamp);
    fill_access(&mut event, access, state);
    event.ehr_id = ehr_id;
    caller.stamp_decision(&mut event);
    let charged = caller.principal.map_or_else(
        || Charged::network(address),
        |principal| Charged::Caller {
            issuer: event.issuer.clone().unwrap_or_default(),
            subject: principal.subject.clone(),
        },
    );
    let summarised = resp.extensions().get::<OverRate>().is_some();
    let outcome = if summarised {
        "caller-rate".to_owned()
    } else {
        caller.binding.map_or_else(
            || {
                resp.extensions().get::<Reason>().map_or_else(
                    || resp.status().as_str().to_owned(),
                    |Reason(r)| (*r).to_owned(),
                )
            },
            |binding| binding.code.as_str().to_owned(),
        )
    };
    record(
        state,
        &Refusal {
            charged,
            outcome: &outcome,
            ehr_id: event.ehr_id.as_deref(),
            event: &event,
            summarised,
        },
    )
}

#[cfg(test)]
mod tests;
