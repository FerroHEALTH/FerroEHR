// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Why the `MEASUREMENT_SERVICE` spec functions are not realized on the value
//! (documentation only, by design).
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.support.measurement_service.adoc`
//! §Description — "Defines an object providing proxy access to a measurement
//! information service" — and §Functions: `is_valid_units_string (units)` and
//! `units_equivalent (units1, units2)`.
//!
//! Both defer their content to a specification openEHR does not publish and
//! this repository does not vendor: the answers are "valid … according to the
//! HL7 UCUM specification" and "correspond to the same measured property",
//! which are decided by the UCUM unit grammar and its commensurability
//! tables. The openEHR text states no rule of its own to implement — it names
//! the external authority — so a unit reader written here would be this
//! project's guess at UCUM rather than a realization of an openEHR function,
//! and in a clinical repository a guess that accepts an invalid unit (or
//! calls two units equivalent when they measure different properties) is the
//! silent wrong answer this codebase refuses to produce.
//!
//! A conforming platform realizes them against a real measurement service,
//! which is where the vendored specification puts them.
