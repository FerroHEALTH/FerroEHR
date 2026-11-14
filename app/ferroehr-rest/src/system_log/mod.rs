// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The **system log** at the wire — the IHE ATNA audit middleware and the
//! operation → audit-event classification.
//!
//! Logging is a STANDARD-profile capability ("STANDARD … adds AQL querying and
//! logging to the CORE", CNF `profiles/master03-profiles.adoc` §Default
//! Profiles). openEHR fixes no audit-record shape, so the shape follows IHE
//! **ATNA**, whose payload is the DICOM Audit Message (DICOM PS3.15 §A.5).
//!
//! This module is the capability's request-path half, over the
//! `ferroehr::service::SystemLog` seam (`ferroehr::service::system_log`),
//! with the DICOM/syslog rendering in the platform crate (`ferroehr::system_log`):
//!
//! - [`middleware`] resolves each request into an [`AuditEvent`](ferroehr::system_log::event::AuditEvent)
//!   — caller identity, client network address, and DICOM outcome from the HTTP
//!   status — and hands it to the platform emitter (non-blocking).
//! - [`classify`] maps every operation id to its DICOM `EventActionCode` +
//!   resource class. Every mounted route's operation is classified, which the
//!   completeness and route-coverage tests hold; an operation without a class
//!   is refused, never recorded under a guessed one.
//! - [`refusals`] records every refused request within its budget, with the
//!   summary records beyond it.

pub mod classify;
pub mod middleware;
pub mod refusals;
pub mod served;
