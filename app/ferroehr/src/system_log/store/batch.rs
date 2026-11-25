// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The parallel column arrays of the batched intake of the system chain, one
//! entry per record, which the `unnest` of the statement reads.

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
use crate::system_log::content_digest::{self, Record};
use crate::system_log::event::ActingMode;
use openehr_rm::draft::access_control::terminology::Confidentiality;

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
    ids: uuid::Uuid,
    recorded_at: Timestamp,
    stored_at: Timestamp,
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
    reviews: Option<&'static str>,
    contents: Option<serde_json::Value>,
    bindings: Option<&'static str>,
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
    /// states none), written at `stored_at`, to every column; both instants
    /// at the microsecond, the precision the store keeps.
    ///
    /// # Errors
    /// [`AuditError::Render`] when the classified content cannot be written as
    /// JSON.
    pub(super) fn push(
        &mut self,
        record: &'a AuditRecord,
        end: Option<&RecordEnd>,
        stored_at: jiff::Timestamp,
    ) -> Result<(), AuditError> {
        let AuditRecord { event, subject, .. } = record;
        self.ids.push(uuid::Uuid::now_v7());
        self.recorded_at
            .push(Timestamp::from(content_digest::to_microsecond(
                event.timestamp,
            )));
        self.stored_at.push(Timestamp::from(stored_at));
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
        self.reviews.push(event.review.map(Confidentiality::code));
        self.contents.push(category_content_json(event)?);
        let (binding, json) = binding_columns(event);
        self.bindings.push(binding);
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

impl BatchColumns<'_> {
    /// The content digest of each record ([`content_digest`]), every one
    /// rendered under the policy `policy` names and naming the search tokens
    /// whose array text `tokens` holds, one per record.
    ///
    /// # Errors
    /// [`AuditError::Digest`] when a record has no RFC 8785 form.
    pub(super) fn content_digests(
        &self,
        policy: &[u8],
        tokens: &[String],
    ) -> Result<Vec<Vec<u8>>, AuditError> {
        let digests = (0..self.ids.len()).map(|i| {
            let record = self.record(i, policy, tokens.get(i).map_or("{}", String::as_str));
            content_digest::of(&record)
                .map(|digest| digest.to_vec())
                .map_err(AuditError::Digest)
        });
        digests.collect()
    }

    /// The columns of record `i` as its content digest reads them.
    #[expect(
        clippy::indexing_slicing,
        reason = "i is below self.ids.len(), and push appends one entry to every column together"
    )]
    fn record<'b>(&'b self, i: usize, policy: &'b [u8], tokens: &'b str) -> Record<'b> {
        Record {
            id: self.ids[i],
            recorded_at: self.recorded_at[i].to_jiff(),
            stored_at: self.stored_at[i].to_jiff(),
            action: &self.actions[i],
            outcome: self.outcomes[i],
            event_code: self.event_codes[i],
            operation: self.operations[i],
            principal: self.principals[i],
            patient_id: self.patient_ids[i],
            resource_class: self.resource_classes[i],
            resource_id: self.resource_ids[i],
            client_ip: self.client_ips[i],
            token_id: self.token_ids[i],
            domain: self.domains[i],
            purpose: self.purposes[i],
            legal_basis: self.legal_bases[i],
            result_count: self.result_counts[i],
            request_id: self.request_ids[i],
            roles: self.roles[i].as_ref(),
            origins: self.origins[i].as_ref(),
            origin_count: self.origin_counts[i],
            categories: self.categories[i].as_ref(),
            category_basis: self.bases[i],
            category_evidence: self.evidence[i].as_ref(),
            category_map_digest: self.digests[i].as_deref(),
            acting_mode: self.modes[i],
            assurance_level: self.levels[i],
            assurance_value: self.values[i],
            professional_id: self.professionals[i],
            emergency_access: self.emergency[i],
            retention_ends: self.ends[i].as_deref(),
            retention_stated: self.stated[i],
            permit: self.permits[i],
            issuer: self.issuers[i],
            client_id: self.clients[i],
            ehr_id: self.ehr_ids[i],
            subject_namespace: self.namespaces[i],
            subject_lookup_failed: self.lookups_failed[i],
            review: self.reviews[i],
            category_content: self.contents[i].as_ref(),
            render_policy: policy,
            search_tokens: tokens,
            binding: self.bindings[i],
            party: self.parties[i].as_ref(),
            party_groups: self.groups[i].as_ref(),
            provider: self.providers[i].as_ref(),
            representation: self.representations[i].as_ref(),
            external_fields: self.externals[i].as_ref(),
            agreement: self.agreements[i].as_ref(),
            actor_chain: self.actor_chains[i].as_ref(),
            facilities: self.facilities[i].as_ref(),
            binding_detail: self.details[i].as_ref(),
            summary: self.summaries[i].as_ref(),
            digest_version: content_digest::VERSION,
            token_attributes: self.token_attributes[i].as_ref(),
        }
    }
}
