// @generated-from-template templates/openehr-rm/support/terminology/terminology_service_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Where the `TERMINOLOGY_SERVICE` spec functions are realized — and why none
//! of them is realized HERE (documentation only, by design).
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.support.terminology_service.adoc`
//! §Description defines the class as "an object providing proxy access to a
//! terminology service", and §Functions declares eight operations over it:
//! `terminology`, `code_set`, `code_set_for_id`, `has_terminology`,
//! `has_code_set`, `terminology_identifiers`, `openehr_code_sets` and
//! `code_set_identifiers`.
//!
//! None of them is a function of the generated value. The class declares no
//! attributes at all, so every answer comes from the SERVICE the object
//! proxies — which terminologies it knows ("allowable names include openehr,
//! centc251, any name from … the US NLM UMLS meta-data list"), which code sets
//! it has, and what each of them contains. A stateless value cannot know that,
//! and answering from whatever terminology happens to be compiled in would
//! report "this terminology is not known" about a terminology the deployment
//! serves — a wrong answer, not an unavailable one.
//!
//! Its two inherited classes are different, and ARE realized: the identifier
//! sets `OPENEHR_CODE_SET_IDENTIFIERS` and
//! `OPENEHR_TERMINOLOGY_GROUP_IDENTIFIERS` are fixed by the specification
//! rather than by a deployment, so their `valid_code_set_id` /
//! `valid_terminology_group_id` predicates live on those classes.
//!
//! The same holds for the two proxy interfaces this service hands out,
//! `TERMINOLOGY_ACCESS` and `CODE_SET_ACCESS` (see their own modules), and for
//! `MEASUREMENT_SERVICE`.
