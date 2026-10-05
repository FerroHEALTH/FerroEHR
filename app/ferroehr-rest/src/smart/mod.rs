// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR **SMART App Launch and Service Discovery** (Release-1.1.0,
//! DEVELOPMENT status)
//! — `docs/specs/openehr/ITS-REST/docs/smart_app_launch/master00-09*.adoc`.
//!
//! This server is
//! the SMART *Platform's openEHR CDR* (the `org.openehr.rest` resource
//! server) — the three capabilities that belong here (master02 §Overview
//! capability split):
//!
//! - [`discovery`] — the `/.well-known/smart-configuration` document
//!   (master04 §Service Discovery), served pre-auth;
//! - the master08 resource-scope grammar
//!   (`compartment/resource.permission`, `*`/`**`/`ns::*` patterns) —
//!   `openehr_sdt::smart_scopes`, parsed from the validated token's
//!   `scope` claim;
//! - [`enforce`] — scope enforcement riding the existing ABAC PEP
//!   ([`crate::extensions::access::pep`]), AND-composed after RBAC/Cedar, plus the
//!   `ehrId`/`patient` launch-context binding (master07/master09).
//!
//! Registration (master03), token issuance/grants/PKCE (master06), and
//! launch-sequence UI (master07) are Authorization-Server/Launcher duties —
//! out of scope for a CDR, a settled adjudication (out of scope for a CDR).
//! Config-gated (`ferroehr::config::smart::SmartConfig`): off by default, zero
//! wire drift when disabled.

pub mod discovery;
pub mod enforce;
// The scope grammar itself lives in `openehr_sdt::smart_scopes` (one
// grammar for the gate here and for REST clients previewing grants).
