// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The hospital scenario of #5611 as in-process integration tests: one
//! controller, "Sint Example Ziekenhuis", its staff provisioned over SCIM 2.0
//! (RFC 7643, RFC 7644), its clients and care relationships written through
//! the Demographic API, its policy through the admin `ACCESS_POLICY` route,
//! and the patient's record and directives committed through ITS-REST.
//!
//! Every rule of the scenario is one test over the world
//! [`build::world`] builds, with the access-log entry each access writes
//! asserted beside its answer. The decision procedure, its codes and the
//! scenario are our own design (#5487); EHDS Art. 8, 9, 10 and 11(5) ground
//! the restriction, the log, the opt-out and the emergency
//! (`docs/law/eu/ehds/text.html`).

pub(crate) mod abac;
pub(crate) mod build;
pub(crate) mod ledger;
mod matrix;
pub(crate) mod world;

mod probe;
