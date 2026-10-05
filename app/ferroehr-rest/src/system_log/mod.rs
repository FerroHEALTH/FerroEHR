// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SM **System Log** component at the wire — the IHE ATNA audit
//! middleware and the operation → audit-event classification.
//!
//! Spec-governed, not an extension. The openEHR platform model defines this
//! component by one normative line — "System Log | IHE ATNA-compliant system
//! log" (SM `master02-overview.adoc` §openEHR Platform Model) — with an empty
//! interface stub (`i_system_log.adoc` has no calls); logging is a
//! STANDARD-profile capability ("STANDARD … adds AQL querying and logging to
//! the CORE", CNF `profiles/master03-profiles.adoc` §Default Profiles). The
//! audit-record shape is therefore governed by the external standards openEHR
//! points at: IHE **ATNA**, whose payload is the DICOM Audit Message
//! (DICOM PS3.15 §A.5).
//!
//! This module is the capability's request-path half, over the
//! `ferroehr::service::SystemLog` seam (`ferroehr::service::system_log`),
//! with the DICOM/syslog rendering in the platform crate (`ferroehr::system_log`):
//!
//! - [`middleware`] resolves each request into an [`AuditEvent`](ferroehr::system_log::event::AuditEvent)
//!   — caller identity, client network address, and DICOM outcome from the HTTP
//!   status — and hands it to the platform emitter (non-blocking).
//! - [`classify`] maps every operation id to its DICOM `EventActionCode` +
//!   resource class. Every generated ITS-REST operation is explicitly
//!   classified (a completeness test guards this); an unrecognised operation id
//!   (extension route or future op) fails **closed** to a documented default and
//!   is still audited — never silently unaudited.

pub mod classify;
pub mod middleware;
