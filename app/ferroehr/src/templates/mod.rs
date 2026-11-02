// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The identity law of archetype and template identifiers.
//!
//! `identity` applies the `ARCHETYPE_ID` / `TEMPLATE_ID` rule
//! (`BASE/docs/base_types/master05-identification_package.adoc` §Composite
//! Identifiers and Case: case-preserving and case-insensitive) at every cache
//! boundary, and reads the version axis of a `template_id`. The template
//! surface itself is `crate::service::definition`; its rows are
//! `crate::storage::template_repo`.

pub(crate) mod identity;
