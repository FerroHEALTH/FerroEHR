// @generated-from-template templates/openehr-rm/data_types/time_specification/dv_general_time_specification_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Why the `DV_GENERAL_TIME_SPECIFICATION` extraction functions are NOT
//! realized (documentation only, by design).
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_general_time_specification.adoc`
//! §Functions effects three functions of its abstract parent
//! (`org.openehr.rm.data_types.dv_time_specification.adoc` §Functions):
//! `calendar_alignment ()`, `event_alignment ()` and `institution_specified
//! ()`, each documented only as "extracted from value".
//!
//! The value is a `DV_PARSABLE` in the HL7v3 GTS syntax, and the grammar the
//! released text gives for it
//! (`docs/specs/openehr/RM/docs/data_types/master08-time_specification_package.adoc`
//! §General Time Specification Syntax) does not determine an answer:
//!
//! - `general_time_spec = symbol | union | exclusion` opens on a `symbol`
//!   production the section never defines, so a conformant value cannot even
//!   be tokenised from the released text;
//! - `union` and `intersection` admit SEVERAL `phase_linked_time_spec`
//!   factors, each carrying its own `[ "@" alignment ]` and `[ "IST" ]`, while
//!   the functions return `1..1` — the text states no rule for combining or
//!   choosing among them;
//! - the `hull` production is declared and then referenced by nothing, so the
//!   grammar does not close over its own rules.
//!
//! Answering anyway would mean choosing one reading of an underdetermined
//! grammar and writing the result into clinical timing. The per-factor
//! extractions themselves ARE determined, and are realized on the sibling
//! subtype that owns a single factor —
//! `DV_PERIODIC_TIME_SPECIFICATION.calendar_alignment` /
//! `.event_alignment` / `.institution_specified`
//! (`org.openehr.rm.data_types.dv_periodic_time_specification.adoc`
//! §Functions).
//!
//! The abstract `DV_TIME_SPECIFICATION` dispatch waits on the same fact: it
//! can only be as total as its two subtypes, and one of them has no
//! determined answer to give.
