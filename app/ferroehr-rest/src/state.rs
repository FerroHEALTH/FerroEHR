// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared application state.
//!
//! [`AppState`] is the router state every group dispatcher in [`crate::api`]
//! receives. It holds the concrete platform service (`FerroEhrService`)
//! (no trait objects, no stub backend — the binary monomorphizes it
//! over the DB-backed `FerroEhrService`). It is cheap to
//! clone (an `Arc` inside) and carries the configuration, the service `S` the
//! dispatchers call into, the optional authorization handle, and the
//! [`Observability`] bundle (management surface + telemetry handles + health
//! registry) — which defaults to fully off, so a server without observability
//! is the clean default. ATNA auditing lives in the platform service `S` (the
//! SM `SystemLog` component), reached through `AppState::backend`.
//!
//! The REST layer holds **no caches of its own** — in particular, `WebTemplate`
//! resolution is a single service-owned concern reached through
//! `ferroehr::service::WebTemplateService`.

use std::sync::Arc;

use ferroehr::service::FerroEhrService;

use crate::config::AppConfig;
use crate::extensions::access::authz::AuthzHandle;
use crate::extensions::management::Observability;

/// Cheaply-cloneable application state, generic over the platform service `S`:
/// the configuration, the service the HTTP dispatcher calls into, and the
/// observability bundle.
#[derive(Debug)]
pub struct AppState {
    inner: Arc<Inner>,
}

// Hand-written so `Clone` does not spuriously require `S: Clone` — the state is
// always shared through the inner `Arc`.
impl Clone for AppState {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[derive(Debug)]
struct Inner {
    config: AppConfig,
    backend: Arc<FerroEhrService>,
    /// The authorization handle (the RBAC gate), when access control is wired in
    /// (the binary supplies it); `None` restores authentication-only behaviour.
    authz: Option<Arc<AuthzHandle>>,
    /// The observability bundle (management config + telemetry handles).
    observability: Observability,
}

impl AppState {
    /// Construct state with a concrete service (the `ferroehr` application injects
    /// its DB-backed service here).
    #[must_use]
    pub fn with_backend(config: AppConfig, backend: Arc<FerroEhrService>) -> Self {
        Self::with_parts(config, backend, None, Observability::default())
    }

    /// Construct state from all parts, including the authorization handle and the
    /// observability bundle.
    #[must_use]
    pub fn with_parts(
        config: AppConfig,
        backend: Arc<FerroEhrService>,
        authz: Option<Arc<AuthzHandle>>,
        observability: Observability,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                backend,
                authz,
                observability,
            }),
        }
    }

    /// The server configuration.
    #[must_use]
    pub fn config(&self) -> &AppConfig {
        &self.inner.config
    }

    /// The service the group dispatchers call into.
    pub(crate) fn backend(&self) -> &FerroEhrService {
        &self.inner.backend
    }

    /// An owned handle on the service, for work that outlives the handler.
    ///
    /// A handler that answers with a STREAM returns its response head while
    /// the work behind the body is still running, so that work cannot borrow
    /// the state it was called with. Every other dispatcher takes
    /// [`Self::backend`] instead.
    pub(crate) fn backend_handle(&self) -> Arc<FerroEhrService> {
        Arc::clone(&self.inner.backend)
    }

    /// The authorization handle (RBAC gate), if access control is wired.
    pub(crate) fn authz(&self) -> Option<Arc<AuthzHandle>> {
        self.inner.authz.clone()
    }

    /// The observability bundle (management + telemetry handles).
    pub(crate) fn observability(&self) -> &Observability {
        &self.inner.observability
    }
}
