// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The metrics of a request's access decisions, counted once it ended.

use std::collections::BTreeMap;
use std::sync::PoisonError;

use openehr_rm::draft::access_control::terminology::{Refusal, Term as _};
use opentelemetry::KeyValue;

use super::RequestAccess;
use crate::access_eval::procedure::Lifted;
use crate::ids::EhrId;

impl RequestAccess {
    /// Count the request's decisions once it ended: one per EHR it judged or
    /// refused, refused under the first code noted for it, and each
    /// directive or step an emergency lifted. The labels are the closed sets
    /// of the outcome, the `refused-*` code and [`Lifted`], never the EHR.
    /// No openEHR spec governs the metrics: our own design (#5591).
    pub(super) fn count_decisions(&self) {
        let metrics = crate::telemetry::access_path::metrics();
        let mut refused: BTreeMap<Option<EhrId>, Refusal> = self
            .recorded
            .refusals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(ehr_id, noted)| (*ehr_id, noted.refusal))
            .collect();
        let verdicts = self.verdicts.lock().unwrap_or_else(PoisonError::into_inner);
        for (ehr_id, judged) in verdicts.iter() {
            let decision = match refused.remove(&Some(*ehr_id)) {
                Some(refusal) => [("outcome", "refused"), ("code", refusal.code())],
                None => [("outcome", "admitted"), ("code", "none")],
            };
            metrics
                .decisions
                .add(1, &decision.map(|(key, value)| KeyValue::new(key, value)));
            for lifted in &judged.verdict.lifted {
                let lifted = match lifted {
                    Lifted::OptOut => "opt_out",
                    Lifted::Restriction => "restriction",
                    Lifted::Consent => "consent",
                    Lifted::Rule => "rule",
                    Lifted::Policy => "policy",
                };
                metrics
                    .emergency_overrides
                    .add(1, &[KeyValue::new("lifted", lifted)]);
            }
        }
        for refusal in refused.into_values() {
            metrics.decisions.add(
                1,
                &[
                    KeyValue::new("outcome", "refused"),
                    KeyValue::new("code", refusal.code()),
                ],
            );
        }
    }
}
