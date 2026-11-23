// @generated-from-template templates/openehr-rm/validate.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! RM validation over a canonical-JSON node as a value: the fast path
//! ([`try_fast_validate`]) and the invariant helpers the `*_impl.rs` files
//! share; the per-node checks the typed model cannot express
//! ([`check_mandatory_containers`], [`check_archetyped_valid`],
//! [`check_data_structure_shapes`]); and the [`terminology`], [`incomplete`]
//! and [`typed_dispatch`] tiers.
//!
//! Violations read `Invariant <Name> failed on type <RM_TYPE>`, `<Name>` the
//! class-table name (`RM/docs/UML/classes/*.adoc` §Invariants). No openEHR spec
//! governs the wording: our own design, in `invariant_failed`.

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use openehr_base::validate::InvariantViolation;
use serde_json::Value;

mod fast;
/// The generated RM class-invariant cores (`emit-validate`), called by the
/// typed `Validate` impls and [`fast`].
pub(crate) mod generated;
// Module docs live in the module, so rustdoc resolves its links there.
pub mod incomplete;
// Module docs live in the module, so rustdoc resolves its links there.
pub mod terminology;
// Module docs live in the module, so rustdoc resolves its links there.
pub mod typed_dispatch;
// RM-instance validation over typed RM (its own `//!` module docs carry the
// detail), and the walk it runs, generated (`openehr-codegen -- emit-validate`).
pub mod tree;
mod tree_generated;

/// Runs the fast-path class-invariant check for one canonical-JSON node,
/// returning whether it vouched for the node; nothing is appended on `false`.
///
/// NOTE: no openEHR spec governs the fast path: our own design, with the
/// semantics of the typed dispatch.
#[must_use]
pub fn try_fast_validate(ty: &str, value: &Value, out: &mut Vec<InvariantViolation>) -> bool {
    fast::try_validate(ty, value, out)
}

/// The inherited `LOCATABLE.Archetype_node_id_valid` violation of a node of
/// type `ty`, for every concrete LOCATABLE descendant of the generated model.
///
/// Only a present, empty `archetype_node_id` violates; an absent one is a
/// structural defect reported by the decode.
#[must_use]
pub fn locatable_node_id_violation(ty: &str, value: &Value) -> Option<InvariantViolation> {
    // Cheap first: the overwhelming majority of nodes carry a non-empty id, so
    // the model lookup is paid only on an actual violation candidate.
    if value.get("archetype_node_id").and_then(Value::as_str) != Some("") {
        return None;
    }
    if !crate::v1_2::model::descendants("LOCATABLE").contains(&ty) {
        return None;
    }
    let mut out = Vec::with_capacity(1);
    generated::archetype_node_id_core(ty, "", &mut out);
    out.pop()
}

/// The mandatory-container check: each mandatory container the model declares
/// is present, and non-empty where its lower bound is 1
/// (`org.openehr.rm.data_structures.cluster.adoc`).
///
/// The reader reads an absent mandatory `Vec` as empty, so presence is
/// checked here; `List<Octet>` is a string and exempt. Run beside the
/// fast/typed pair.
pub fn check_mandatory_containers(ty: &str, value: &Value, out: &mut Vec<InvariantViolation>) {
    for attr in crate::v1_2::model::attributes(ty) {
        if !attr.is_mandatory
            || matches!(attr.container, crate::v1_2::model::Container::None)
            || attr.declared_type == "Octet"
        {
            continue;
        }
        match value.get(attr.name) {
            None | Some(Value::Null) => {
                out.push(InvariantViolation::here(format!(
                    "does not conform to RM type {ty}: mandatory container `{}` is absent",
                    attr.name
                )));
            }
            Some(Value::Array(a))
                if a.is_empty() && attr.cardinality.is_some_and(|c| c.lower >= 1) =>
            {
                out.push(InvariantViolation::here(format!(
                    "does not conform to RM type {ty}: mandatory container `{}` is empty \
                     (cardinality lower bound 1)",
                    attr.name
                )));
            }
            _ => {}
        }
    }
}

/// The declared-slot-type rule: a child's `_type` names the attribute's
/// declared type or a subtype, never relaxed ("data may be missing, but it may
/// not be wrong", RM `master06-change_control_package.adoc` §Incomplete
/// Content).
///
/// `None` for an unknown slot, a primitive type, or a `Hash`.
#[must_use]
pub fn check_declared_slot_type(
    parent_type: &str,
    field: &str,
    wire_type: &str,
) -> Option<InvariantViolation> {
    let attr = crate::v1_2::model::attribute(parent_type, field)?;
    if matches!(attr.container, crate::v1_2::model::Container::Hash) {
        return None;
    }
    crate::v1_2::model::class(attr.declared_type)?;
    if crate::v1_2::model::is_a(wire_type, attr.declared_type) {
        return None;
    }
    Some(InvariantViolation::here(format!(
        "does not conform to RM type {parent_type}: `{field}` is declared \
         {declared} and this member claims `_type` {wire_type}, which is not \
         a {declared}",
        declared = attr.declared_type,
    )))
}

/// The scalar-member arm of the declared-slot-type rule: no JSON scalar is an
/// RM class instance. `None` under the same guards.
#[must_use]
pub fn check_slot_member_is_object(parent_type: &str, field: &str) -> Option<InvariantViolation> {
    let attr = crate::v1_2::model::attribute(parent_type, field)?;
    if matches!(attr.container, crate::v1_2::model::Container::Hash) {
        return None;
    }
    crate::v1_2::model::class(attr.declared_type)?;
    Some(InvariantViolation::here(format!(
        "does not conform to RM type {parent_type}: `{field}` is declared \
         {declared} and this member is not a JSON object",
        declared = attr.declared_type,
    )))
}

/// `LOCATABLE.Archetyped_valid` (`is_archetype_root xor archetype_details =
/// Void`, `org.openehr.rm.common.locatable.adoc` §Invariants): a term-code node
/// must not carry `archetype_details`, and a node carrying it must have the
/// archetype id as its `archetype_node_id` (§Attributes).
///
/// NOTE: `locatable.adoc` §Functions gives `is_archetype_root` no derivation,
/// so the converse arm is not enforced.
#[must_use]
pub fn check_archetyped_valid(
    node_id: Option<&str>,
    has_archetype_details: bool,
    details_archetype_id: Option<&str>,
) -> Vec<InvariantViolation> {
    let mut out = Vec::new();
    let Some(node_id) = node_id else {
        return out;
    };
    if crate::v1_2::paths::archetype_node_id_is_term_code(node_id) && has_archetype_details {
        out.push(InvariantViolation::here(format!(
            "node {node_id:?} is not an archetype root (at/id term code) and must \
             not carry archetype_details (LOCATABLE.Archetyped_valid)"
        )));
    }
    if let Some(archetype_id) = details_archetype_id
        && archetype_id != node_id
    {
        out.push(InvariantViolation::here(format!(
            "archetype root archetype_node_id {node_id:?} is not the stringified \
             archetype_details.archetype_id {archetype_id:?} — at an archetype root \
             the two are always the same value (LOCATABLE.archetype_node_id)"
        )));
    }
    out
}

/// The `CLUSTER.items` presence duty, which `553|incomplete|` relaxes (RM
/// `master06-change_control_package.adoc` §Incomplete Content).
#[must_use]
pub fn check_cluster_items_present(
    obj: &serde_json::Map<String, Value>,
    ty: Option<&str>,
) -> Option<InvariantViolation> {
    (ty == Some("CLUSTER") && obj.get("items").and_then(Value::as_array).is_none()).then(|| {
        InvariantViolation::here("CLUSTER.items is mandatory (1..1 List<ITEM>, cluster.adoc)")
    })
}

/// The `HISTORY` shape duty: every event's `data` is the same
/// `ITEM_STRUCTURE` subtype (`master06-history_package.adoc`), reported at
/// `events[i]`.
#[must_use]
pub fn check_data_structure_shapes(
    obj: &serde_json::Map<String, Value>,
    ty: Option<&str>,
) -> Vec<InvariantViolation> {
    let mut out = Vec::new();
    let Some(ty) = ty else {
        return out;
    };
    if ty == "HISTORY"
        && let Some(events) = obj.get("events").and_then(Value::as_array)
    {
        let mut first: Option<&str> = None;
        for (i, event) in events.iter().enumerate() {
            let Some(data_ty) = event.pointer("/data/_type").and_then(Value::as_str) else {
                continue;
            };
            match first {
                None => first = Some(data_ty),
                Some(locked) if locked != data_ty => {
                    out.push(InvariantViolation::at(
                        format!("events[{i}]"),
                        format!(
                            "HISTORY events must all carry the same ITEM_STRUCTURE \
                             subtype in data — the history is HISTORY<{locked}> but \
                             this event carries {data_ty} (RM data_structures \
                             master06 §History)"
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
    }
    out
}

/// Builds a violation reading `"Invariant <name> failed on type <RM_TYPE>"`,
/// with an empty path.
///
/// NOTE: no openEHR spec governs the wording: our own design.
#[must_use]
pub(crate) fn invariant_failed(name: &str, rm_type: &str) -> InvariantViolation {
    InvariantViolation::here(format!("Invariant {name} failed on type {rm_type}"))
}

/// Whether a float is a whole number, for the DV_PROPORTION integrality
/// invariants (`dv_proportion.adoc` §Invariants).
#[must_use]
#[expect(
    clippy::float_cmp,
    reason = "an exact-integrality test is precisely a bit-equality question (`x.floor() == x`), not a tolerance comparison"
)]
pub(crate) fn is_integral(v: f64) -> bool {
    v.is_finite() && v.floor() == v
}

// Runtime realizations of the assertion-dialect predicates
// (`plan::overrides`).

/// `valid_magnitude_status (s)`: `s` is one of `= < > <= >= ~`.
#[must_use]
pub(crate) fn valid_magnitude_status(s: &str) -> bool {
    matches!(s, "=" | "<" | ">" | "<=" | ">=" | "~")
}

/// `valid_percentage (v)`: `0 <= v <= 100`.
#[must_use]
pub(crate) fn valid_percentage(v: f64) -> bool {
    (0.0..=100.0).contains(&v)
}

/// `valid_proportion_kind (k)`: `k` is a declared `PROPORTION_KIND` code.
#[must_use]
pub(crate) fn valid_proportion_kind(k: i32) -> bool {
    !matches!(
        crate::v1_2::data_types::quantity::proportion_kind::ProportionKind::from_value(k),
        crate::v1_2::data_types::quantity::proportion_kind::ProportionKind::Other(_)
    )
}

// ── ISO-8601 value validation ────────────────────────────────────────────────

// The temporal `Value_valid` invariants call BASE `Time_Definitions`.

pub(crate) use openehr_base::v1_3::foundation_types::time::time_definitions::{
    valid_iso8601_date, valid_iso8601_date_time, valid_iso8601_duration, valid_iso8601_time,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_date_forms() {
        assert!(valid_iso8601_date("2021"));
        assert!(valid_iso8601_date("2021-05"));
        assert!(valid_iso8601_date("2021-05-17"));
        assert!(valid_iso8601_date("20210517"));
        assert!(!valid_iso8601_date("2021-13"));
        assert!(!valid_iso8601_date("2021-05-32"));
        assert!(!valid_iso8601_date("not-a-date"));
        assert!(!valid_iso8601_date(""));
    }

    /// `Iso8601_date.Day_valid`, in both forms.
    #[test]
    fn iso_date_day_is_calendar_exact() {
        // 31-day months accept 31; 30-day months reject it.
        assert!(valid_iso8601_date("2021-01-31"));
        assert!(valid_iso8601_date("2021-12-31"));
        assert!(!valid_iso8601_date("2021-04-31")); // April has 30 days
        assert!(!valid_iso8601_date("2021-06-31"));
        assert!(!valid_iso8601_date("2021-09-31"));
        assert!(!valid_iso8601_date("2021-11-31"));
        assert!(valid_iso8601_date("2021-04-30"));

        // February: 28 in a common year, 29 in a leap year, never 30/31.
        assert!(!valid_iso8601_date("2021-02-31"));
        assert!(!valid_iso8601_date("2021-02-30"));
        assert!(!valid_iso8601_date("2021-02-29")); // 2021 is not a leap year
        assert!(valid_iso8601_date("2021-02-28"));
        assert!(valid_iso8601_date("2020-02-29")); // 2020 divisible by 4
        assert!(valid_iso8601_date("2000-02-29")); // 2000 divisible by 400
        assert!(!valid_iso8601_date("1900-02-29")); // 1900 century, not /400

        // Day 00 is never valid.
        assert!(!valid_iso8601_date("2021-05-00"));

        // Compact form is held to the same calendar rule.
        assert!(!valid_iso8601_date("20210431"));
        assert!(!valid_iso8601_date("20210229"));
        assert!(valid_iso8601_date("20200229"));
        assert!(valid_iso8601_date("20210131"));
    }

    #[test]
    fn iso_time_forms() {
        assert!(valid_iso8601_time("10"));
        assert!(valid_iso8601_time("10:30"));
        assert!(valid_iso8601_time("10:30:59"));
        assert!(valid_iso8601_time("10:30:59.250"));
        assert!(valid_iso8601_time("10:30:59Z"));
        assert!(valid_iso8601_time("10:30:59+01:00"));
        assert!(!valid_iso8601_time("25:00"));
        assert!(!valid_iso8601_time("10:61"));
        assert!(!valid_iso8601_time("abc"));
    }

    /// "only fractional seconds are supported" (BASE `master06-time_types.adoc`).
    #[test]
    fn iso_time_fraction_only_on_seconds() {
        // Fractional seconds (period or comma) is the sole permitted case.
        assert!(valid_iso8601_time("10:30:59.250"));
        assert!(valid_iso8601_time("10:30:59,5"));
        assert!(valid_iso8601_time("103059.250")); // compact HHMMSS
        assert!(valid_iso8601_time("10:30:59.5+01:00")); // fraction before timezone
        // A fractional hour or minute is rejected in every base form.
        assert!(!valid_iso8601_time("10.5")); // fractional hour
        assert!(!valid_iso8601_time("10:05.5")); // fractional minute (extended)
        assert!(!valid_iso8601_time("1005.5")); // fractional minute (compact HHMM)
        assert!(!valid_iso8601_time("10,5")); // fractional hour, comma
    }

    #[test]
    fn iso_date_time_forms() {
        assert!(valid_iso8601_date_time("2021-05-17T10:30:00"));
        assert!(valid_iso8601_date_time("2021-05-17T10:30:00+02:00"));
        assert!(valid_iso8601_date_time("2021-05-17"));
        assert!(!valid_iso8601_date_time("2021-05-17T99:00"));
        assert!(!valid_iso8601_date_time("nope"));
    }

    #[test]
    fn iso_duration_forms() {
        assert!(valid_iso8601_duration("P1Y"));
        assert!(valid_iso8601_duration("P1Y2M10D"));
        assert!(valid_iso8601_duration("PT2H30M"));
        assert!(valid_iso8601_duration("P1Y2M10DT2H30M"));
        assert!(valid_iso8601_duration("P2W"));
        assert!(valid_iso8601_duration("-P1D"));
        assert!(valid_iso8601_duration("PT0.5S"));
        assert!(!valid_iso8601_duration("P"));
        assert!(!valid_iso8601_duration("1Y"));
        assert!(!valid_iso8601_duration("P1X"));
        assert!(!valid_iso8601_duration("PT"));
    }

    /// This validator and BASE's `Iso8601Time` accept the same strings.
    #[test]
    fn time_validity_agrees_with_the_base_reader() {
        for value in [
            "10:30:00", "10:30:59", "23:59:59", "10:30:60", "103060", "25:00:00",
        ] {
            let base = openehr_base::v1_3::foundation_types::time::iso8601_time::Iso8601Time {
                value: value.to_owned(),
            }
            .hour()
            .is_some();
            assert_eq!(
                valid_iso8601_time(value),
                base,
                "{value:?}: the RM validator and the BASE reader disagree",
            );
        }
    }

    /// This validator and BASE's `Iso8601Duration` accept the same strings.
    #[test]
    fn duration_validity_agrees_with_the_base_reader() {
        for value in [
            "P1Y",
            "P1Y2M10DT2H30M",
            "P2W",
            "PT0.5S",
            "P1Y1Y",
            "P2Y1Y",
            "P1D1M",
            "PT1S1H",
            "P1YT",
            "PT1H2H",
            "P1X",
            "PT",
        ] {
            let base =
                openehr_base::v1_3::foundation_types::time::iso8601_duration::Iso8601Duration {
                    value: value.to_owned(),
                }
                .to_seconds()
                .is_some();
            assert_eq!(
                valid_iso8601_duration(value),
                base,
                "{value:?}: the RM validator and the BASE reader disagree",
            );
        }
    }

    /// A fraction on a duration component other than seconds is invalid.
    #[test]
    fn iso_duration_fraction_only_on_seconds() {
        // Fraction on seconds (period or comma) is the sole permitted case.
        assert!(valid_iso8601_duration("PT2H30M0.5S"));
        assert!(valid_iso8601_duration("PT0,5S"));
        // Fraction on any other component is rejected.
        assert!(!valid_iso8601_duration("P1Y3M4DT2.5H"));
        assert!(!valid_iso8601_duration("PT2H14.5M"));
        assert!(!valid_iso8601_duration("P1.5Y"));
        assert!(!valid_iso8601_duration("P1.5M"));
        assert!(!valid_iso8601_duration("P1.5W"));
        assert!(!valid_iso8601_duration("P1.5D"));
        assert!(!valid_iso8601_duration("PT1.5H"));
        assert!(!valid_iso8601_duration("PT2H14,5M"));
    }

    // The `_type`-dispatch tests are in `openehr-its/tests/it/rm_validation.rs`.

    /// BASE `Iso8601_timezone`: `+` offsets reach +14:00, `-` offsets stop at
    /// -12:00; ±00:00 is accepted (see `is_valid_tz`).
    #[test]
    fn timezone_bounds_are_asymmetric() {
        assert!(valid_iso8601_time("10:00:00+14:00"));
        assert!(valid_iso8601_time("10:00:00-12:00"));
        assert!(valid_iso8601_time("10:00:00+00:00"));
        assert!(valid_iso8601_time("10:00:00-00:00"));
        assert!(
            !valid_iso8601_time("10:00:00+15:00"),
            "+15 exceeds Max_timezone_hour"
        );
        assert!(
            !valid_iso8601_time("10:00:00-13:00"),
            "-13 exceeds Min_timezone_hour"
        );
    }
}
