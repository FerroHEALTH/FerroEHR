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
#[cfg(not(target_arch = "wasm32"))]
pub mod composition;
#[cfg(not(target_arch = "wasm32"))]
pub mod configuration;
pub mod dashboard;
#[cfg(not(target_arch = "wasm32"))]
pub mod ehr;
#[cfg(not(target_arch = "wasm32"))]
pub mod ehrs;
#[cfg(not(target_arch = "wasm32"))]
pub mod errors;
pub mod find;
#[cfg(not(target_arch = "wasm32"))]
pub mod item_tags;
#[cfg(not(target_arch = "wasm32"))]
pub mod login;
#[cfg(not(target_arch = "wasm32"))]
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
