// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The **wire-boundary RM class-invariant dispatcher** — the thin entry points
//! that compose the validation tiers at the codec boundary.
//!
//! The tiers themselves are RM model semantics and live upstream in
//! [`openehr_rm::v1_2::validate`]: the allocation-free fast path and the
//! authoritative typed dispatch. Three things stay here because they need this
//! crate:
//!
//! - the generated five-crate structural fallthrough ([`structural_check`]),
//!   which spans every spec crate at once and so can only be emitted
//!   downstream of all of them;
//! - the undeclared-key refusal, which reads [`declared_fields`] and renders
//!   the reader's own [`crate::json::JsonParseError`];
//! - the entry points, which fix the ORDER the tiers and the orthogonal layers
//!   run in.
//!
//! A node that does not deserialize into its declared concrete RM type surfaces
//! `does not conform to RM type …`.

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
///
/// A node with no (or an unrecognised) `_type` runs no invariants.
///
/// Two tiers, because the pass visits every `_type` node of a commit (~1.5k for
/// a populated composition), so per-node cost is load-bearing:
///
/// 1. [`openehr_rm::v1_2::validate::try_fast_validate`] checks conformance
///    directly against the live JSON node through the same invariant cores the
///    typed impls call — no deserialization, byte-identical output;
/// 2. anything it cannot vouch for falls back to [`validate_rm_value_typed`].
///
/// The equivalence battery pins this tier: the fast path may vouch only when it
/// is byte-identical to the typed oracle. The terminology-backed invariants are
/// an orthogonal layer [`validate_rm_value`] adds.
pub fn validate_rm_invariants(value: &Value, out: &mut Vec<InvariantViolation>) {
    let Some(ty) = value.get("_type").and_then(Value::as_str) else {
        return;
    };
    validate_rm_invariants_as(ty, value, out);
}

/// [`validate_rm_invariants`] with the node's RM type supplied by the caller.
///
/// This is the entry a tree walker uses for a node whose wire `_type` is
/// legitimately absent (canonical JSON requires `_type` only where the declared
/// attribute type is abstract; see
/// [`openehr_rm::v1_2::model::declared_concrete_type`]). The `_type`-reading
/// wrapper stays for callers with no declaration context.
pub fn validate_rm_invariants_as(ty: &str, value: &Value, out: &mut Vec<InvariantViolation>) {
    // The strict reader refuses an undeclared wire key, so a node carrying one
    // is not a conformant instance — and the FAST path never decodes, so it
    // would not see it. Checking here, against the generated declared-key table
    // (`declared_fields`, emitted from the SAME field view as the reader's own
    // refusal), keeps the two tiers byte-identical without paying for a decode.
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
///
/// The three branches realize that sentence:
///
/// 1. an undeclared member is wrong, never missing — refused first;
/// 2. a node with nothing missing runs the strict tiers unchanged;
/// 3. a node that IS missing mandatory data skips typed construction, because
///    that tier's refusal is the presence rule the state lifts. Wrongness is
///    still judged: a positive type contradiction goes through the typed tier,
///    and the class invariants run wherever the fast path can still vouch for
///    the node as it stands.
///
/// Only the mandatory-presence and cardinality-lower-bound layers relax. The
/// orthogonal layers the caller runs beside this one — terminology,
/// `LOCATABLE.Archetyped_valid`, the data-structure shape duties and the
/// archetype-conformance pass — do not.
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
    // The node is missing mandatory data and is not wrong: its class
    // invariants are evaluated where the fast path can still read them off the
    // node as it stands, and a decline is the honest answer — an invariant
    // over an absent attribute has nothing to judge, which is exactly the
    // "data may be missing" case §Incomplete Content admits.
    let _vouched = openehr_rm::v1_2::validate::try_fast_validate(ty, value, out);
}

/// The violation for the first undeclared member of `value` under class `ty`,
/// or `None` when every member is declared (or `ty` names no emitted struct).
///
/// Byte-identical to what the typed tier reports for the same node: it builds
/// the reader's own error and runs it through [`record_type_mismatch`].
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
    // The core path (fast/typed), then the two orthogonal layers — the
    // model-driven mandatory-container lower bounds and the
    // terminology-backed invariants
    // (each dispatches on the same `_type`). The core tiers stay a pure pair so
    // the fast-vs-typed equivalence property holds exactly.
    validate_rm_invariants_as(ty, value, out);
    openehr_rm::v1_2::validate::check_mandatory_containers(ty, value, out);
    openehr_rm::v1_2::validate::terminology::validate_rm_terminology(ty, value, out);
}

/// The typed dispatch tier of [`validate_rm_value`]: deserialize the node
/// into its concrete RM type and run that type's `Validate` impl.
///
/// Authoritative for every node — the fast path may only skip it when the
/// result is provably identical — and the oracle its equivalence tests compare
/// against. Composed from three parts, in this order:
///
/// 1. the undeclared-key refusal, ahead of any decode;
/// 2. the typed table
///    ([`openehr_rm::v1_2::validate::typed_dispatch::dispatch_typed`]), for the
///    concrete types carrying a non-terminology class invariant;
/// 3. for every class the table declines, the generated [`structural_check`],
///    which decodes the node and discards it: the codec is the
///    structural-conformance authority for the whole emitted model, so a class
///    with no invariant is still refused when it is structurally defective.
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
    // Raised again ahead of the decode so this tier's verdict does not depend
    // on WHERE the offending member sits: the reader streams members in
    // document order, so a node that is both structurally defective and carries
    // an undeclared key would otherwise report whichever came first.
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
///
/// The classes the typed table
/// ([`openehr_rm::v1_2::validate::typed_dispatch::dispatch_typed`]) handles already
/// decode there — its arms report the same `does not conform to RM type …` on
/// failure — and the table returns `true` for them, so they never reach here: no
/// node is decoded twice. A `_type` naming no emitted class
/// ([`structural_check`] returns `None`) runs no check, unchanged: an
/// unrecognised type is not a structural claim this layer can adjudicate.
fn run_structural(ty: &str, value: &Value, out: &mut Vec<InvariantViolation>) {
    if let Some(Err(e)) = structural_check(ty, value) {
        record_type_mismatch(ty, &e, out);
    }
}
