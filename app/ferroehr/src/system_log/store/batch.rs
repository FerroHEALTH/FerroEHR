// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The parallel column arrays of the batched audit insert, one entry per
//! record, which the `unnest` of the statement reads.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the store carries an \
              already-rendered FHIR document, never a typed resource"
)]

use ferrohealth_smart::assurance::AssuranceLevel;
use jiff_sqlx::Timestamp;

use super::{
    AuditRecord, RecordEnd, access_ehr_id, action_str, binding_columns, categories_json,
    category_basis, category_content_json, category_evidence_json, category_map_digest, event_code,
    nonempty_opt, operation, origin_count, origins_json, outcome_smallint, resource_class,
    result_count, roles_json, token_attributes_json,
};
use crate::system_log::AuditError;
use crate::system_log::event::ActingMode;

/// Declares [`BatchColumns`] with one `Vec` per column and its
/// `with_capacity`, so a column is named once.
macro_rules! batch_columns {
    ($($column:ident: $ty:ty),+ $(,)?) => {
        /// The parallel column arrays of a batched insert, one entry per
        /// record, which the `unnest` of the statement reads.
        pub(super) struct BatchColumns<'a> {
            $(pub(super) $column: Vec<$ty>,)+
        }

        impl BatchColumns<'_> {
            pub(super) fn with_capacity(n: usize) -> Self {
                Self {
                    $($column: Vec::with_capacity(n),)+
                }
            }
        }
    };
}

batch_columns! {
    recorded_at: Timestamp,
    actions: String,
    outcomes: i16,
    event_codes: &'a str,
    operations: Option<&'a str>,
    principals: Option<&'a str>,
    patient_ids: Option<&'a str>,
    resource_classes: &'a str,
    resource_ids: Option<&'a str>,
    client_ips: Option<&'a str>,
    token_ids: Option<&'a str>,
    domains: &'a str,
    purposes: Option<&'a str>,
    legal_bases: Option<&'a str>,
    result_counts: Option<i64>,
    request_ids: Option<&'a str>,
    roles: Option<serde_json::Value>,
    origins: Option<serde_json::Value>,
    origin_counts: Option<i64>,
    categories: Option<serde_json::Value>,
    bases: Option<&'static str>,
    evidence: Option<serde_json::Value>,
    digests: Option<String>,
    modes: Option<&'static str>,
    levels: Option<&'static str>,
    values: Option<&'a str>,
    professionals: Option<&'a str>,
    emergency: bool,
    ends: Option<String>,
    stated: bool,
    permits: Option<&'a str>,
    issuers: Option<&'a str>,
    clients: Option<&'a str>,
    ehr_ids: Option<&'a str>,
    namespaces: Option<&'a str>,
    lookups_failed: bool,
    contents: Option<serde_json::Value>,
    bindings: Option<&'static str>,
    guests: bool,
    parties: Option<serde_json::Value>,
    groups: Option<serde_json::Value>,
    providers: Option<serde_json::Value>,
    representations: Option<serde_json::Value>,
    externals: Option<serde_json::Value>,
    agreements: Option<serde_json::Value>,
    actor_chains: Option<serde_json::Value>,
    facilities: Option<serde_json::Value>,
    details: Option<serde_json::Value>,
    summaries: Option<serde_json::Value>,
    token_attributes: Option<serde_json::Value>,
}

impl<'a> BatchColumns<'a> {
    /// Append one record, with the retention end it states (`None` when it
    /// states none), to every column.
    ///
    /// # Errors
    /// [`AuditError::Render`] when the classified content cannot be written as
    /// JSON.
    pub(super) fn push(
        &mut self,
        record: &'a AuditRecord,
        end: Option<&RecordEnd>,
    ) -> Result<(), AuditError> {
        let AuditRecord { event, subject, .. } = record;
        self.recorded_at.push(Timestamp::from(event.timestamp));
        self.actions.push(action_str(event));
        self.outcomes.push(outcome_smallint(event));
        self.event_codes.push(event_code(event));
        self.operations.push(operation(event));
        self.principals.push(nonempty_opt(&event.user_id));
        self.patient_ids.push(subject.as_deref());
        self.resource_classes.push(resource_class(event.object));
        self.resource_ids.push(event.object_id.as_deref());
        self.client_ips.push(event.client_ip.as_deref());
        self.token_ids.push(event.token_id.as_deref());
        self.domains.push(event.domain.as_str());
        self.purposes.push(event.purpose.as_deref());
        self.legal_bases.push(event.legal_basis.as_deref());
        self.result_counts.push(result_count(event));
        self.request_ids.push(event.request_id.as_deref());
        self.roles.push(roles_json(event));
        self.origins.push(origins_json(event));
        self.origin_counts.push(origin_count(event));
        self.categories.push(categories_json(event));
        self.bases.push(category_basis(event));
        self.evidence.push(category_evidence_json(event));
        self.digests.push(category_map_digest(event));
        self.modes.push(event.actor.mode.map(ActingMode::as_str));
        self.levels
            .push(event.actor.assurance_level.map(AssuranceLevel::as_str));
        self.values.push(event.actor.assurance_value.as_deref());
        self.professionals
            .push(event.actor.professional_id.as_deref());
        self.emergency.push(event.emergency_access);
        self.ends.push(match end {
            Some(RecordEnd::On(date)) => Some(date.to_string()),
            Some(RecordEnd::Never) | None => None,
        });
        self.stated.push(end.is_some());
        self.permits.push(event.permit.as_deref());
        self.issuers.push(event.issuer.as_deref());
        self.clients.push(event.client_id.as_deref());
        self.ehr_ids.push(access_ehr_id(event));
        self.namespaces.push(event.subject_namespace.as_deref());
        self.lookups_failed.push(event.subject_lookup_failed);
        self.contents.push(category_content_json(event)?);
        let (binding, guest, json) = binding_columns(event);
        self.bindings.push(binding);
        self.guests.push(guest);
        let [
            party,
            groups,
            provider,
            representation,
            external,
            agreement,
            chain,
            facility,
            detail,
            summary,
        ] = json;
        self.parties.push(party);
        self.groups.push(groups);
        self.providers.push(provider);
        self.representations.push(representation);
        self.externals.push(external);
        self.agreements.push(agreement);
        self.actor_chains.push(chain);
        self.facilities.push(facility);
        self.details.push(detail);
        self.summaries.push(summary);
        self.token_attributes.push(token_attributes_json(event));
        Ok(())
    }
}
