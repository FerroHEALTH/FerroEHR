// @generated-from-template templates/openehr-rm/support/terminology/code_set_access_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Why the `CODE_SET_ACCESS` spec functions are not realized on the value
//! (documentation only, by design).
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.support.code_set_access.adoc`
//! §Description — "Defines an object providing proxy access to A CODE_SET" —
//! and §Functions: `id`, `all_codes`, `has_lang` and `has_code`.
//!
//! The class declares no attributes, so the value carries neither which code
//! set it proxies ("External identifier of this code set") nor its codes. Each
//! function is a question about that code set's content, answered by the
//! service the proxy came from
//! (`org.openehr.rm.support.terminology_service.adoc` §Functions `code_set` /
//! `code_set_for_id`) — not by a stateless value.
//!
//! What the specification DOES fix, and what is therefore realized in this
//! crate, is which code set IDENTIFIERS exist:
//! `OPENEHR_CODE_SET_IDENTIFIERS.valid_code_set_id`.
