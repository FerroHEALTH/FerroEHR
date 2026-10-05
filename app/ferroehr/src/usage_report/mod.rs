// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The usage report to `FerroPULSE`.
//!
//! One `start` report at each process start and one `daily` report per instance
//! per day, carrying the installation id, the version, the licence grant type,
//! the deployment kind, coarse host size and latency aggregates per
//! route-template group.
//!
//! No openEHR spec governs this — our own design. The wire contract is
//! `FerroPULSE`'s report v1 ([`payload`]). It carries no patient data, no request
//! path, no AQL text and no licence id: the payload types have no field that
//! could hold one, and [`window`] records route templates only. On by default;
//! `[usage_report] enabled = false` (`FERROEHR__USAGE_REPORT__ENABLED=false`)
//! stops it, and the boot line says which ([`reporter::announce`]).
//!
//! - [`config`]: the `[usage_report]` section.
//! - [`payload`]: the typed report.
//! - [`window`]: the in-process metrics window and its rendering.
//! - [`store`]: the instance id, the cross-replica claims, the shared window.
//! - [`reporter`]: assembly, sending, scheduling.

pub mod config;
pub mod payload;
pub mod reporter;
pub mod store;
pub mod window;
