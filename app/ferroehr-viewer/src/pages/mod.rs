// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The pages of the viewer, each a route of `crate::app`.
//!
//! A page whose model loads apart from its view compiles for both halves;
//! the others are drawn on the server alone (islands mode). No openEHR spec
//! governs the viewer: our own design.

#[cfg(not(target_arch = "wasm32"))]
pub mod about;
#[cfg(not(target_arch = "wasm32"))]
pub mod access;
#[cfg(not(target_arch = "wasm32"))]
pub mod audit;
pub mod common;
pub mod composition;
#[cfg(not(target_arch = "wasm32"))]
pub mod configuration;
#[cfg(not(target_arch = "wasm32"))]
pub mod dashboard;
pub mod ehr;
pub mod ehrs;
#[cfg(not(target_arch = "wasm32"))]
pub mod errors;
#[cfg(not(target_arch = "wasm32"))]
pub mod find;
pub mod item_tags;
#[cfg(not(target_arch = "wasm32"))]
pub mod login;
pub mod parties;
#[cfg(not(target_arch = "wasm32"))]
pub mod preferences;
#[cfg(not(target_arch = "wasm32"))]
pub mod queries;
#[cfg(not(target_arch = "wasm32"))]
pub mod query;
#[cfg(not(target_arch = "wasm32"))]
pub mod results;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
#[cfg(not(target_arch = "wasm32"))]
pub mod subscriptions;
#[cfg(not(target_arch = "wasm32"))]
pub mod templates;
#[cfg(not(target_arch = "wasm32"))]
pub mod terminology;
