// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The usage report to `FerroPULSE`, sent by `ferrohealth-usage-report`.
//!
//! One `start` report at each process start and one `daily` report per
//! installation per day, on `FerroPULSE`'s report envelope v2: the
//! installation id, the version, the licence grant type, the deployment kind,
//! coarse host size, FerroEHR's [`product::Status`] and, daily, latency
//! aggregates per route-template group. No patient data, request path, AQL
//! text or licence id can reach one: the envelope has no field that could hold
//! one, and the request log hands the window route templates only. On by
//! default; `[usage_report] enabled = false`
//! (`FERROEHR__USAGE_REPORT__ENABLED=false`) stops it, a CI run
//! (`GITHUB_ACTIONS` or `CI` set to `true` or `1`) stops it whatever the switch
//! says, and the boot line says which.
//!
//! No openEHR spec governs this: our own design. FerroEHR supplies two parts:
//!
//! - [`product`]: the status, the route groups, the identity and the preview.
//! - [`store`]: the crate's state store over the clinical database, so every
//!   replica of an installation shares one id, one start claim and one daily
//!   report.

pub mod product;
pub mod store;
