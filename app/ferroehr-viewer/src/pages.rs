// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The routed screens. One module
//! per screen; components stay thin — data flows through the `#[server]`
//! fns each module co-locates.

pub mod audit;
pub mod composition;
pub mod dashboard;
pub mod demographics;
pub mod ehr_detail;
pub mod ehr_tags;
pub mod ehrs;
pub mod login;
pub mod operations;
pub mod queries;
pub mod query_aql;
pub mod query_builder;
pub mod query_stored;
pub mod shell;
pub mod subscriptions;
pub mod system;
pub mod template_adl2;
pub mod template_detail;
pub mod templates;
pub mod terminology;
