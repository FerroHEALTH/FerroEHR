// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operation record of a request that writes, built at its commit
//! (#5762).
//!
//! The audit middleware primes the request's write recorder with a builder
//! before the operation runs. The layers inside publish what only they know:
//! the dispatch names the operation and the authentication layer how the
//! caller authenticated. The service's commit choke point
//! (`FerroEhrService::commit_recorded` in `ferroehr`) calls the
//! builder with what the write decided, so the operation record is the one
//! the middleware would have written for a successful response, written and
//! held before the write commits. No openEHR spec governs the audit trail:
//! our own design.

use std::sync::{Arc, Mutex, PoisonError};

use http::StatusCode;

use ferroehr::system_log::event::{ActorAuthentication, AuditEvent};
use ferroehr::system_log::recorder::{Builder, WriteFacts, WriteRecorder};

use crate::state::AppState;
use crate::system_log::classify::audit_for;
use crate::system_log::middleware::{AuditObject, Caller, RequestFacts, operation_record};

/// What the layers inside the audit middleware learn of the request.
#[derive(Debug, Default)]
struct Learned {
    op: Option<&'static str>,
    actor: Option<ActorAuthentication>,
}

type Slot = Arc<Mutex<Learned>>;

tokio::task_local! {
    static LEARNED: Slot;
}

fn learn(apply: impl FnOnce(&mut Learned)) {
    let _outside = LEARNED.try_with(|slot| {
        apply(&mut slot.lock().unwrap_or_else(PoisonError::into_inner));
    });
}

/// Publish the matched operation of the running request.
pub(crate) fn learn_op(op: &'static str) {
    learn(|learned| learned.op = Some(op));
}

/// Publish how the running request's caller authenticated.
pub(crate) fn learn_actor(actor: &ActorAuthentication) {
    learn(|learned| learned.actor = Some(actor.clone()));
}

/// Run `fut` under `recorder`, with a fresh slot for what the layers inside
/// learn.
pub(super) async fn run<F>(recorder: WriteRecorder, fut: F) -> F::Output
where
    F: Future,
{
    let slot = Slot::default();
    LEARNED
        .scope(
            slot,
            ferroehr::system_log::recorder::with_recorder(recorder, fut),
        )
        .await
}

/// The builder of the request's operation record from what a write decided.
pub(super) fn builder(state: &AppState, facts: &RequestFacts) -> Builder {
    let state = state.clone();
    let facts = facts.clone();
    Arc::new(move |written: &WriteFacts| build(&state, &facts, written))
}

/// The operation record of a successful write that decided `written`; none
/// outside a classified operation.
fn build(state: &AppState, facts: &RequestFacts, written: &WriteFacts) -> Vec<AuditEvent> {
    let Ok((Some(op), actor)) = LEARNED.try_with(|slot| {
        let learned = slot.lock().unwrap_or_else(PoisonError::into_inner);
        (learned.op, learned.actor.clone())
    }) else {
        return Vec::new();
    };
    let Some(classes) = audit_for(op) else {
        return Vec::new();
    };
    let principal = crate::extensions::access::authn::current_principal();
    let binding = ferroehr::system_log::access_context::current_binding().map(|mut binding| {
        binding.facilities = ferroehr::system_log::access_context::current_facilities();
        binding
    });
    let access = ferroehr::access_eval::condition::current();
    let caller = Caller {
        principal: principal.as_deref(),
        actor: actor.as_ref(),
        binding: binding.as_ref(),
        access: access.as_ref(),
    };
    let object = AuditObject {
        ehr_id: written.ehr_id.clone(),
        uid: written.object_id.clone(),
        origins: written.origins.clone(),
        origin_count: (!written.origins.is_empty())
            .then(|| u64::try_from(written.origins.len()).unwrap_or(u64::MAX)),
        content: written.content.clone(),
        domain: written.domain,
        ..AuditObject::default()
    };
    vec![operation_record(
        state,
        op,
        classes,
        StatusCode::OK,
        false,
        caller,
        facts,
        Some(&object),
    )]
}
