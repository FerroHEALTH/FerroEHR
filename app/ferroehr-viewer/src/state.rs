// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared server state, held in every page's request scope
//! ([`crate::scope`]) and handed to the plain axum handlers via `Extension`.

use std::sync::Arc;

use ferrohealth_session::routes::SignIn;
use ferrohealth_session::sessions::Sessions;
use ferrohealth_smart::oidc::RelyingParty;

use crate::session::ViewerPages;

/// Everything the BFF side needs per request.
#[derive(Debug, Clone)]
pub struct AppState {
    /// The loaded viewer configuration.
    pub config: Arc<crate::config::ViewerConfig>,
    /// The one CDR client.
    pub cdr: crate::cdr::CdrClient,
    /// The session store, the relying party when `auth.oidc_enabled`, and
    /// the pages the sign-in routes land on.
    pub sign_in: SignIn<ViewerPages>,
}

impl AppState {
    /// The state of a viewer with `config`, reaching the CDR through `cdr`
    /// and signing in at the provider through `party` when one is set.
    ///
    /// # Errors
    /// [`crate::config::ConfigError::Session`], the `ferrohealth-session`
    /// refusal of `[session]`.
    pub fn new(
        config: crate::config::ViewerConfig,
        cdr: crate::cdr::CdrClient,
        party: Option<RelyingParty>,
    ) -> Result<Self, crate::config::ConfigError> {
        let offline_access = config.auth.oidc_enabled && config.oidc.offline_access;
        let settings = config.session.check_with(offline_access)?;
        let sessions = Sessions::new(settings, crate::session::cookie_names());
        Ok(Self {
            config: Arc::new(config),
            cdr,
            sign_in: SignIn::new(sessions, party, ViewerPages),
        })
    }
}
