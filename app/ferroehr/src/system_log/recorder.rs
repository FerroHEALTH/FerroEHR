// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The request's write recorder: what a request-scoped write's audit records
//! are built from at its commit, and the reservations its commits took
//! (#5762).
//!
//! The protocol adapter primes the recorder before the operation runs with a
//! builder: everything the request's operation record names except what the
//! write itself decides (the EHR, the object, the content and the origins of
//! what it wrote). The one commit choke point,
//! `FerroEhrService::commit_recorded`, completes the
//! record with those facts, writes it into the audit intake held under a
//! reservation id, inserts the id in the write's own transaction and commits;
//! the adapter then releases what the request reserved. No openEHR spec
//! governs the audit trail: our own design.

use std::sync::{Arc, Mutex, PoisonError};

use tokio::task_local;
use uuid::Uuid;

use crate::system_log::categories::AccessedContent;
use crate::system_log::event::{AccessDomain, AuditEvent};

task_local! {
    static RECORDER: WriteRecorder;
}

/// What a write decides of its operation's record: the EHR and the object it
/// wrote, what they hold and where it came from.
#[derive(Debug, Clone, Default)]
pub struct WriteFacts {
    /// The EHR the write changed.
    pub ehr_id: Option<String>,
    /// The object the write produced: a version uid, a party, a template.
    pub object_id: Option<String>,
    /// The origins of the data written.
    pub origins: Vec<String>,
    /// What the write wrote, for the record's EHDS priority category.
    pub content: Option<AccessedContent>,
    /// The pseudonymisation domain the write changed, when the operation's
    /// resource class does not say it.
    pub domain: Option<AccessDomain>,
}

impl WriteFacts {
    /// The facts of a demographic change-set: the `PERSON` or `GROUP` it
    /// wrote first, which the SCIM endpoint names as the access's object.
    #[must_use]
    pub fn of_parties(committed: &crate::versioning::change::CommittedContribution) -> Self {
        let party = committed.versions.iter().find(|version| {
            matches!(
                version.kind,
                crate::versioning::Kind::Person | crate::versioning::Kind::Group
            )
        });
        Self {
            object_id: party.map(|party| party.vo_id.to_string()),
            domain: Some(AccessDomain::Demographic),
            ..Self::default()
        }
    }
}

/// Builds the operation records of a write from what the write decided.
pub type Builder = Arc<dyn Fn(&WriteFacts) -> Vec<AuditEvent> + Send + Sync>;

/// One reservation a commit of the request took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reserved {
    /// The reservation id the held records name and the write's domain
    /// holds.
    pub id: Uuid,
}

#[derive(Default)]
struct State {
    builder: Option<Builder>,
    /// Whether a commit of the request holds its operation record.
    built: bool,
    /// The reservations the request's commits took.
    reserved: Vec<Reserved>,
    /// The act's own records its next commit holds.
    deferred: Vec<AuditEvent>,
}

/// The recorder of one request.
#[derive(Clone, Default)]
pub struct WriteRecorder {
    state: Arc<Mutex<State>>,
}

impl std::fmt::Debug for WriteRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.lock();
        f.debug_struct("WriteRecorder")
            .field("primed", &state.builder.is_some())
            .field("built", &state.built)
            .field("reserved", &state.reserved)
            .field("deferred", &state.deferred.len())
            .finish()
    }
}

impl WriteRecorder {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Prime the recorder with the builder of the request's operation record.
    pub fn prime(&self, builder: Builder) {
        self.lock().builder = Some(builder);
    }

    /// The operation records of a write that decided `facts`; empty when the
    /// recorder holds no builder or an earlier commit of the request holds
    /// them, since a request has one operation record whatever it commits.
    pub(crate) fn records_for(&self, facts: &WriteFacts) -> Vec<AuditEvent> {
        let mut state = self.lock();
        if state.built {
            return Vec::new();
        }
        let Some(build) = state.builder.clone() else {
            return Vec::new();
        };
        state.built = true;
        drop(state);
        let records = build(facts);
        // NOTE: our own design (#5762), a commit before the operation is known (a party the
        // authentication layer provisions) builds nothing, so a later commit records it.
        if records.is_empty() {
            self.unbuild();
        }
        records
    }

    /// Hand the operation records back after their commit failed, so a later
    /// commit of the request records them.
    pub(crate) fn unbuild(&self) {
        self.lock().built = false;
    }

    /// Hand `records`, the act's own records, to the request's next commit,
    /// which holds them with its operation record.
    pub(crate) fn defer(&self, records: Vec<AuditEvent>) {
        self.lock().deferred.extend(records);
    }

    /// Take the records handed to the next commit.
    pub(crate) fn take_deferred(&self) -> Vec<AuditEvent> {
        std::mem::take(&mut self.lock().deferred)
    }

    /// Note a reservation a commit of the request took.
    pub(crate) fn committed(&self, reserved: Reserved) {
        self.lock().reserved.push(reserved);
    }

    /// Whether a commit of the request holds its operation record, so the
    /// adapter writes none of its own.
    #[must_use]
    pub fn holds_operation_record(&self) -> bool {
        self.lock().built
    }

    /// What the request's commits reserved.
    #[must_use]
    pub fn reserved(&self) -> Vec<Reserved> {
        self.lock().reserved.clone()
    }
}

/// Run `fut` with `recorder` as the request's write recorder.
pub async fn with_recorder<F>(recorder: WriteRecorder, fut: F) -> F::Output
where
    F: Future,
{
    RECORDER.scope(recorder, fut).await
}

/// The current request's write recorder; `None` outside a request.
#[must_use]
pub fn current() -> Option<WriteRecorder> {
    RECORDER.try_with(Clone::clone).ok()
}
