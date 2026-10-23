// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The pages of the viewer, drawn on the server alone (islands mode); each
//! is a route of `crate::app`. No openEHR spec governs the viewer: our own
//! design.

pub mod about;
pub mod access;
pub mod audit;
pub mod common;
pub mod composition;
pub mod configuration;
pub mod dashboard;
pub mod ehr;
pub mod ehrs;
pub mod errors;
pub mod find;
pub mod item_tags;
pub mod login;
pub mod parties;
pub mod preferences;
pub mod queries;
pub mod query;
pub mod results;
pub mod server;
pub mod subscriptions;
pub mod templates;
pub mod terminology;
