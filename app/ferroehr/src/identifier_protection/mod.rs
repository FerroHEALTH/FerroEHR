// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Protected national identifiers in the demographic domain.
//!
//! **No openEHR spec governs this — our own design/extension.** The RM models
//! identifiers generically (`PARTY_IDENTITY`, RM demographic
//! `UML/classes/org.openehr.rm.demographic.party_identity.adoc`; `DV_IDENTIFIER`,
//! RM `data_types` `UML/classes/org.openehr.rm.data_types.dv_identifier.adoc`)
//! and says nothing about protecting the value. A national identifier needs
//! more than that generic slot: GDPR Art. 32(1)(a) names encryption as an
//! appropriate measure, and UAVG Art. 46 with the Wabvpz permit BSN processing
//! in care only for identification and under the act's conditions.
//!
//! The value therefore leaves the versioned body and lives in
//! `party.national_identifier`, sealed under a per-domain key, with a
//! keyed digest beside it for equality lookup without decryption. The body
//! keeps a reference in its place. [`engine`] is the one object the commit
//! path holds; [`crate::storage::identifier_repo`] reads and writes the rows and only ever
//! sees sealed bytes.

pub mod body;
pub mod config;
pub mod crypto;
pub mod engine;
pub mod store;
