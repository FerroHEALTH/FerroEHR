// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared server state, provided to every server function via Leptos
//! context (`leptos_routes_with_context`) and to the plain axum handlers
//! via `Extension`.

use std::sync::Arc;

/// Everything the BFF side needs per request.
#[derive(Debug, Clone)]
pub struct AppState {
    /// The loaded viewer configuration.
    pub config: Arc<crate::config::ViewerConfig>,
    /// The one CDR client.
    pub cdr: crate::cdr::CdrClient,
    /// OIDC discovery result (present when `auth.oidc.enabled`).
    pub oidc: Option<Arc<crate::oidc::OidcState>>,
    /// The session-cookie sealing key (from `session.secret`, or ephemeral).
    pub session_keys: crate::session::SessionKeys,
}
