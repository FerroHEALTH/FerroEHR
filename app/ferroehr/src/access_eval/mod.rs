// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access evaluator: the fixed decision procedure over the openEHR access
//! facts.
//!
//! `EHR_ACCESS` is the access-decision authority of an EHR: "All access
//! decisions to data in the EHR must be made in accordance with the policies
//! and rules in this object" (RM ehr `org.openehr.rm.ehr.ehr_access.adoc`
//! §`EHR_ACCESS` Class). Its settings are the subject's `access_directives`
//! (`ACCESS_DIRECTIVE_SETTINGS` of the draft access control package); the
//! caller's facts are the demographic `CAPABILITY` codes and care
//! relationships ([`crate::party_binding::grants::caller`]); the controller's
//! are its `ACCESS_POLICY` ([`crate::access_policy`]). The evaluator reads
//! only coded content of the access control terminology
//! (`openehr_rm::draft::access_control::terminology`), never an archetype id
//! (BASE Architecture Overview `master07-security.adoc` §Access Control: each
//! access control model is a subtype of `ACCESS_CONTROL_SETTINGS`, and the
//! scheme in use is named in the EHR Access object).
//!
//! [`procedure::decide`] is a pure function over the three fact sets, the
//! request and the moment, run after authentication, the capability check
//! and the SMART scope layer, per EHR an operation reaches. It yields a
//! [`procedure::Verdict`]: an ordered list of rules over the data of the
//! EHR ([`scope::Pred`]), whose first match decides, so one verdict answers a
//! named object, the EHR itself, and, rendered as SQL, every row of a query
//! ([`crate::aql::sql::access`]). The decision procedure, its order and its
//! refusal codes are our own design (#5487, #5513): no openEHR specification
//! gives one, and FHIR `Consent` leaves enforcement out of scope
//! (`docs/specs/fhir-consent/` §6.2.3).

pub mod condition;
pub mod procedure;
pub mod scope;

#[cfg(test)]
mod tests;
