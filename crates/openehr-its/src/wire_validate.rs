// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The **wire-boundary RM class-invariant dispatcher** — the thin entry points
//! that compose the validation tiers at the codec boundary.
//!
//! - the generated five-crate structural fallthrough ([`structural_check`]),
//!   which spans every spec crate at once and so can only be emitted
//!   downstream of all of them;
//! - the undeclared-key refusal, which reads [`declared_fields`] and renders
//!   the reader's own [`crate::json::JsonParseError`];
//! - the entry points, which fix the ORDER the tiers and the orthogonal layers
//!   run in.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use serde_json::Value;

use openehr_base::validate::InvariantViolation;
use openehr_rm::v1_2::validate::typed_dispatch::record_type_mismatch;

use crate::json::JsonParseError;
use crate::json_codec::generated::structural::{declared_fields, structural_check};

/// Run the **core** (non-terminology) RM class invariants for a single
/// canonical-JSON node, dispatching on its `_type`.
pub fn validate_rm_invariants(value: &Value, out: &mut Vec<InvariantViolation>) {
    let Some(ty) = value.get("_type").and_then(Value::as_str) else {
        return;
    };
    validate_rm_invariants_as(ty, value, out);
}

/// [`validate_rm_invariants`] with the node's RM type supplied by the caller.
pub fn validate_rm_invariants_as(ty: &str, value: &Value, out: &mut Vec<InvariantViolation>) {
    // The fast path never decodes, so it checks the generated declared-key
    // table the strict reader refuses by, keeping both tiers identical.
    if let Some(violation) = undeclared_key(ty, value) {
        out.push(violation);
        return;
    }
    if !openehr_rm::v1_2::validate::try_fast_validate(ty, value, out) {
        validate_rm_value_typed(ty, value, out);
    }
}

/// [`validate_rm_invariants_as`] under the `553|incomplete|` **presence
/// relaxation** — the entry the relaxed commit path uses per node.
///
/// RM common `master06-change_control_package.adoc` §Incomplete Content
/// (NOTE): "In the `incomplete` state, a limited form of invalidity is
/// allowed: mandatory attributes may be absent … All other validity
/// requirements must be satisfied. In other words, in an `incomplete` commit,
/// data may be missing, but it may not be wrong."
pub fn validate_rm_invariants_relaxed_as(
    ty: &str,
    value: &Value,
    out: &mut Vec<InvariantViolation>,
) {
    if let Some(violation) = undeclared_key(ty, value) {
        out.push(violation);
        return;
    }
    if openehr_rm::v1_2::validate::incomplete::mandatory_data_present(ty, value) {
        if !openehr_rm::v1_2::validate::try_fast_validate(ty, value, out) {
            validate_rm_value_typed(ty, value, out);
        }
        return;
    }
    if openehr_rm::v1_2::validate::incomplete::contradicts_rm_type(ty, value) {
        validate_rm_value_typed(ty, value, out);
        return;
    }
    // Missing mandatory data is not wrong data: an invariant over an absent
    // attribute has nothing to judge (§Incomplete Content).
    let _vouched = openehr_rm::v1_2::validate::try_fast_validate(ty, value, out);
}

/// The violation for the first undeclared member of `value` under class `ty`,
/// or `None` when every member is declared (or `ty` names no emitted struct).
fn undeclared_key(ty: &str, value: &Value) -> Option<InvariantViolation> {
    let members = value.as_object()?;
    let declared = declared_fields(ty)?;
    let offending = members
        .keys()
        .find(|k| k.as_str() != "_type" && declared.binary_search(&k.as_str()).is_err())?;
    let mut out = Vec::new();
    record_type_mismatch(
        ty,
        &JsonParseError::unknown_field(offending, ty, declared).in_field(offending),
        &mut out,
    );
    out.pop()
}

/// Runs **all** RM class invariants for a single canonical-JSON node.
///
/// The set is the core
/// (fast/typed) invariants ([`validate_rm_invariants`]) plus the
/// terminology-backed invariants
/// ([`openehr_rm::v1_2::validate::terminology::validate_rm_terminology`], the
/// openEHR-terminology group and code-set membership checks the generated
/// invariant cores cannot express mechanically). This is the unified dispatcher
/// every consumer calls; the composition validator invokes it per node and
/// prefixes the absolute RM path onto each [`InvariantViolation`].
pub fn validate_rm_value(value: &Value, out: &mut Vec<InvariantViolation>) {
    let Some(ty) = value.get("_type").and_then(Value::as_str) else {
        return;
    };
    // The core path, then the mandatory-container lower bounds and the
    // terminology-backed invariants; the core tiers stay a pure pair.
    validate_rm_invariants_as(ty, value, out);
    openehr_rm::v1_2::validate::check_mandatory_containers(ty, value, out);
    openehr_rm::v1_2::validate::terminology::validate_rm_terminology(ty, value, out);
}

/// The typed dispatch tier of [`validate_rm_value`]: deserialize the node
/// into its concrete RM type and run that type's `Validate` impl.
///
/// Afterwards the inherited `LOCATABLE.Archetype_node_id_valid`
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.locatable.adoc`
/// §Invariants) is closed out for every concrete LOCATABLE descendant via
/// [`openehr_rm::v1_2::validate::locatable_node_id_violation`], whose applicable
/// set is the generated model's descendant closure rather than a hand-kept
/// list. It is appended only when this node's own pass did not already report
/// it, so the violation appears exactly once. Closing it out in this tier — not
/// above both — keeps the fast-vs-typed equivalence exact.
pub fn validate_rm_value_typed(ty: &str, value: &Value, out: &mut Vec<InvariantViolation>) {
    crate::conversion::record::<Value>("decode_value_node");
    // Checked ahead of the decode so the verdict does not depend on where the
    // offending member sits in document order.
    if let Some(violation) = undeclared_key(ty, value) {
        out.push(violation);
        return;
    }
    let before = out.len();
    if !openehr_rm::v1_2::validate::typed_dispatch::dispatch_typed(ty, value, out) {
        run_structural(ty, value, out);
    }
    if let Some(v) = openehr_rm::v1_2::validate::locatable_node_id_violation(ty, value)
        && !out.get(before..).is_some_and(|added| added.contains(&v))
    {
        out.push(v);
    }
}

/// The generated-dispatch fallthrough of [`validate_rm_value_typed`]: decode the
/// node as the class its `_type` names and record a structural violation when it
/// does not conform.
fn run_structural(ty: &str, value: &Value, out: &mut Vec<InvariantViolation>) {
    if let Some(Err(e)) = structural_check(ty, value) {
        record_type_mismatch(ty, &e, out);
    }
}
