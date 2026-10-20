// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR **SMART App Launch and Service Discovery** (Release-1.1.0,
//! DEVELOPMENT status)
//! — `docs/specs/openehr/ITS-REST/docs/smart_app_launch/master00-09*.adoc`.
//!
//! This server is the SMART platform's openEHR CDR (the `org.openehr.rest`
//! resource server), on the FerroHEALTH family's `ferrohealth-smart` crate,
//! which parses the master08 scope grammar and checks every requirement:
//!
//! - [`discovery`] — the `/.well-known/smart-configuration` document
//!   (master04 §Service Discovery), served pre-auth when `smart.enabled`;
//! - [`routes`] — the SMART on openEHR requirement of every route, checked by
//!   the authentication middleware after RBAC, and for a COMPOSITION by the
//!   policy-enforcement point once its template is known, with the `ehrId`
//!   launch-context binding of a `patient/` grant (master07 §Context
//!   Selection, master08 §Resource Scopes).
//!
//! Registration (master03), token issuance, grants and PKCE (master06), and
//! the launch-sequence UI (master07) are the authorization server's and the
//! launcher's; the CDR checks tokens and never issues them.

pub mod discovery;
pub(crate) mod routes;
