// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The pages of the viewer, each a route of `crate::app`.
//!
//! A page that draws from a serializable model compiles on both halves; the
//! others still draw on the server alone. No openEHR spec governs the viewer:
//! our own design.

// TODO(#4116): every page draws from a model and compiles for wasm32.
pub mod about;
pub mod access;
pub mod audit;
pub mod common;
pub mod composition;
pub mod configuration;
pub mod dashboard;
pub mod ehr;
pub mod ehrs;
#[cfg(not(target_arch = "wasm32"))]
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
