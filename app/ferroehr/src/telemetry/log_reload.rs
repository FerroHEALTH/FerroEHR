// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `GET/POST/DELETE /management/loggers` — runtime log-filter control.
//!
//! The subscriber's `EnvFilter` sits behind a `tracing_subscriber::reload`
//! layer in the binary. [`LogReload`] is the type-erased handle onto that
//! layer: the concrete `reload::Handle<EnvFilter, S>` (whose `S` is the fully
//! assembled subscriber and awkward to name) is captured behind three closures
//! at telemetry init, so this crate can drive it without depending on the
//! subscriber's layer stack.
//!
//! - `GET`    → the effective filter directives + the boot filter.
//! - `POST`   `{"filter":"ferroehr=debug,sqlx=warn"}` → swap the live filter.
//! - `DELETE` → reset to the boot filter.

use std::sync::Arc;

/// Reads the effective filter directives from the subscriber's reload handle.
pub type ReadFilter = Arc<dyn Fn() -> String + Send + Sync>;
/// Applies a new filter directive set to the subscriber's reload handle.
pub type ApplyFilter = Arc<dyn Fn(&str) -> Result<(), FilterReloadError> + Send + Sync>;

/// Why a runtime log-filter swap did not take effect.
///
/// The two arms are the two distinguishable outcomes the `POST` handler must
/// tell apart: bad directives are the caller's fault (`400`), a refused reload
/// is the subscriber's (`503`).
#[derive(Debug, thiserror::Error)]
pub enum FilterReloadError {
    /// The submitted directives are not a valid `env-filter` expression.
    #[error("the filter directives are not valid: {0}")]
    Directives(#[from] tracing_subscriber::filter::ParseError),
    /// The subscriber's reload layer refused the new filter — it is gone.
    #[error("the subscriber's reload handle refused the new filter: {0}")]
    Handle(#[from] tracing_subscriber::reload::Error),
}

/// A type-erased handle onto the subscriber's reloadable `EnvFilter`.
#[derive(Clone)]
pub struct LogReload {
    boot_filter: Arc<str>,
    read: ReadFilter,
    apply: ApplyFilter,
}

impl std::fmt::Debug for LogReload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogReload")
            .field("boot_filter", &self.boot_filter)
            .finish_non_exhaustive()
    }
}

impl LogReload {
    /// Construct from the boot filter and the read/apply closures captured over
    /// the concrete `reload::Handle`.
    #[must_use]
    pub fn new(boot_filter: impl Into<Arc<str>>, read: ReadFilter, apply: ApplyFilter) -> Self {
        Self {
            boot_filter: boot_filter.into(),
            read,
            apply,
        }
    }

    /// The boot-time filter directives (the reset target).
    #[must_use]
    pub fn boot_filter(&self) -> &str {
        &self.boot_filter
    }

    /// The currently effective filter directives.
    #[must_use]
    pub fn current(&self) -> String {
        (self.read)()
    }

    /// Swap the live filter.
    ///
    /// # Errors
    /// [`FilterReloadError::Directives`] if `filter` is not a valid
    /// `EnvFilter` directive set, [`FilterReloadError::Handle`] if the
    /// subscriber's reload layer is gone.
    pub fn set(&self, filter: &str) -> Result<(), FilterReloadError> {
        (self.apply)(filter)
    }

    /// Reset the live filter to the boot filter.
    ///
    /// # Errors
    /// [`FilterReloadError::Handle`] if the subscriber's reload layer is gone.
    pub fn reset(&self) -> Result<(), FilterReloadError> {
        let boot = Arc::clone(&self.boot_filter);
        (self.apply)(&boot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// An in-memory stand-in for the reload handle: a `Mutex<String>` the apply
    /// closure mutates, so the round-trip logic is testable without a subscriber.
    fn fake(boot: &str) -> (LogReload, Arc<Mutex<String>>) {
        let state = Arc::new(Mutex::new(boot.to_owned()));
        let read_state = Arc::clone(&state);
        let apply_state = Arc::clone(&state);
        let reload = LogReload::new(
            boot,
            Arc::new(move || read_state.lock().expect("lock").clone()),
            Arc::new(move |f: &str| {
                tracing_subscriber::EnvFilter::try_new(f)?;
                *apply_state.lock().expect("lock") = f.to_owned();
                Ok(())
            }),
        );
        (reload, state)
    }

    #[test]
    fn set_then_reset_round_trip() {
        let (reload, _state) = fake("info,ferroehr=info");
        assert_eq!(reload.current(), "info,ferroehr=info");

        reload.set("ferroehr=debug,sqlx=warn").expect("apply");
        assert_eq!(reload.current(), "ferroehr=debug,sqlx=warn");

        reload.reset().expect("reset");
        assert_eq!(reload.current(), "info,ferroehr=info");
    }

    #[test]
    fn bad_filter_is_rejected_and_leaves_current_unchanged() {
        let (reload, _state) = fake("info");
        assert!(reload.set("ferroehr=!!bad").is_err());
        assert_eq!(reload.current(), "info");
    }
}
