// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The origin set of a version body: where the data came from (#3212).
//!
//! openEHR models provenance on the content itself. `FEEDER_AUDIT` "describes
//! the origin of data that have been transformed into openEHR form and
//! committed to the system", and its mandatory `originating_system_audit`
//! names "the IT system owned by the organisation legally responsible for
//! handling the data, and at which the data were previously created"
//! (RM common, `FEEDER_AUDIT` and `FEEDER_AUDIT_DETAILS.system_id`). It sits
//! on any `LOCATABLE`, so a body may carry several; a body carrying none was
//! created here, through this API, and its origin is this server.
//!
//! The set is derived once, at commit, and stored on `version.origins`
//! ([`crate::versioning::change`]), so a read aggregates over rows rather
//! than parsing bodies — the `stable_compatible` stamp's pattern. EHDS
//! Annex II 3.2(e) is the requirement it serves; the values are the
//! committer's provenance claims, recorded as such.

use std::collections::BTreeSet;

use serde_json::Value;

/// How many distinct origins one access record carries at most.
///
/// The record stores the true distinct count beside the capped set, so a
/// capped record is visibly capped rather than quietly short.
pub(crate) const RECORD_CAP: usize = 32;

/// The distinct origin set of `canonical`: every
/// `FEEDER_AUDIT.originating_system_audit.system_id` found anywhere in it,
/// sorted, or `system_id` alone when it carries none.
#[must_use]
pub(crate) fn origins(system_id: &str, canonical: &Value) -> Vec<String> {
    let mut found = BTreeSet::new();
    walk(canonical, &mut found);
    if found.is_empty() {
        found.insert(system_id.to_owned());
    }
    found.into_iter().collect()
}

/// The origins of one stored version at read time: the commit-time stamp
/// where one exists, otherwise assessed from the body in hand, otherwise the
/// creating system (a raw passthrough read of an unstamped row has no parsed
/// body to walk).
#[must_use]
pub(crate) fn of_stored(
    stamped: Option<&Value>,
    canonical: &Value,
    creating_system_id: &str,
) -> Vec<String> {
    if let Some(Value::Array(items)) = stamped {
        return items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
    }
    if canonical.is_null() {
        return vec![creating_system_id.to_owned()];
    }
    origins(creating_system_id, canonical)
}

/// Cap a set for the access record, keeping the true distinct count.
#[must_use]
pub(crate) fn recorded(mut set: Vec<String>) -> (Vec<String>, u64) {
    set.sort();
    set.dedup();
    let total = u64::try_from(set.len()).unwrap_or(u64::MAX);
    set.truncate(RECORD_CAP);
    (set, total)
}

fn walk(node: &Value, out: &mut BTreeSet<String>) {
    match node {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some("FEEDER_AUDIT")
                && let Some(id) = map
                    .get("originating_system_audit")
                    .and_then(|details| details.get("system_id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            {
                out.insert(id.to_owned());
            }
            for child in map.values() {
                walk(child, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                walk(child, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn feeder(system: &str) -> Value {
        json!({
            "_type": "FEEDER_AUDIT",
            "originating_system_audit": {
                "_type": "FEEDER_AUDIT_DETAILS",
                "system_id": system
            }
        })
    }

    /// A body with no `FEEDER_AUDIT` was created here: its one origin is this
    /// server.
    #[test]
    fn a_body_without_feeder_audit_originates_here() {
        let body = json!({ "_type": "COMPOSITION", "name": { "value": "x" } });
        assert_eq!(origins("cdr.example", &body), ["cdr.example"]);
    }

    /// Every `FEEDER_AUDIT` anywhere in the body counts once, sorted; a blank
    /// system id is not an origin.
    #[test]
    fn every_feeder_audit_is_an_origin_once() {
        let body = json!({
            "_type": "COMPOSITION",
            "feeder_audit": feeder("pacs.example"),
            "content": [
                { "_type": "OBSERVATION", "feeder_audit": feeder("lab.example") },
                { "_type": "OBSERVATION", "feeder_audit": feeder("pacs.example") },
                { "_type": "OBSERVATION", "feeder_audit": feeder("  ") }
            ]
        });
        assert_eq!(
            origins("cdr.example", &body),
            ["lab.example", "pacs.example"],
            "the server is not an origin of data it did not create"
        );
    }

    /// A stamp wins over the body; an unstamped void body reports the
    /// creating system; the record cap keeps the true count.
    #[test]
    fn stored_origins_prefer_the_stamp_and_the_cap_keeps_the_count() {
        let stamped = json!(["a.example", "b.example"]);
        assert_eq!(
            of_stored(Some(&stamped), &json!({"_type": "COMPOSITION"}), "cdr"),
            ["a.example", "b.example"]
        );
        assert_eq!(
            of_stored(None, &Value::Null, "cdr.example"),
            ["cdr.example"]
        );
        let many: Vec<String> = (0..40).map(|i| format!("sys{i:02}")).collect();
        let (set, total) = recorded(many);
        assert_eq!(set.len(), RECORD_CAP);
        assert_eq!(total, 40);
    }
}
