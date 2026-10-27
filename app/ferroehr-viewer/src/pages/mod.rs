// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The pages of the viewer, each a route of `crate::app`.
//!
//! A page that draws from a serializable model compiles on both halves; the
//! others still draw on the server alone. No openEHR spec governs the viewer:
//! our own design.

// TODO(#4116): every page draws from a model and compiles for wasm32.
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
pub mod dashboard;
pub mod ehr;
pub mod ehrs;
#[cfg(not(target_arch = "wasm32"))]
pub mod errors;
pub mod find;
pub mod item_tags;
#[cfg(not(target_arch = "wasm32"))]
pub mod login;
pub mod parties;
#[cfg(not(target_arch = "wasm32"))]
pub mod preferences;
pub mod queries;
pub mod query;
pub mod results;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
#[cfg(not(target_arch = "wasm32"))]
pub mod subscriptions;
pub mod templates;
pub mod terminology;
