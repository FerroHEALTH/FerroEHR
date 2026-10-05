// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Why the `TERMINOLOGY_ACCESS` spec functions are not realized on the value
//! (documentation only, by design).
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.support.terminology_access.adoc`
//! §Description — "Defines an object providing proxy access to A
//! TERMINOLOGY" — and §Functions: `id`, `all_codes`, `codes_for_group_id`,
//! `codes_for_group_name`, `has_code_for_group_id` and `rubric_for_code`.
//!
//! The class declares no attributes, so the value carries neither WHICH
//! terminology it proxies nor that terminology's content. `id ()` is the
//! plainest case: "Identification of this Terminology" has no answer on a
//! value that holds no identification, and returning a fixed one would claim
//! every proxy is the same terminology. The rest read that terminology's
//! codes, groups and rubrics, which likewise live in the service.
//!
//! A conforming platform realizes them in its terminology layer, over the
//! terminology the proxy was obtained for
//! (`org.openehr.rm.support.terminology_service.adoc` §Functions
//! `terminology`).
