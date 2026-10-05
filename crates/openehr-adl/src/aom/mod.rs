// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared AOM2 substrate: one home for reading, building, and doing
//! interval arithmetic over the generated `openehr_am::v2_4::aom2` constraint
//! model.
//!
//! The AOM2 `C_OBJECT` hierarchy is a closed 13-variant subtype set
//! (`docs/specs/openehr/AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Class Definitions), so every read of a field the abstract `C_OBJECT`
//! declares (`node_id`, `rm_type_name`, `occurrences`, `sibling_order`) is a
//! 13-arm match. Those matches live ONCE, in [`access`]; the constructors that
//! build the same model live in [`build`]; the multiplicity/interval arithmetic
//! the validity rules share lives in [`interval`].
//!
//! Nothing here validates, flattens, or prints — it is the layer all three
//! stand on.

pub mod access;
pub mod build;
pub mod interval;
pub mod nesting;
