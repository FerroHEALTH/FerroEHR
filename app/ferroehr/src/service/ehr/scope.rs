// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SMART `composition-` scope a COMPOSITION delete is judged on (#6271).
//!
//! A delete names no template in its request, so its scope is judged on the
//! template of the version it addresses, which its gate read returns beside
//! everything else the delete is judged on, before any other pre-check (SMART
//! on openEHR `master08-scopes.adoc` §Resource Scopes). The credential is the
//! REST layer's, so the service asks it through [`ScopeJudge`].

/// The caller's SMART `composition-` scope for the operation it runs.
pub trait ScopeJudge: Sync {
    /// Whether the scope covers a COMPOSITION of `template`; `None` is a
    /// template the store does not name, which only a grant on every template
    /// covers.
    fn covers(&self, template: Option<&str>) -> bool;
}

/// A caller no SMART scope binds: a caller of the service outside the REST
/// surface, or a request on a surface with authentication off.
#[derive(Debug, Clone, Copy)]
pub struct Unscoped;

impl ScopeJudge for Unscoped {
    fn covers(&self, _template: Option<&str>) -> bool {
        true
    }
}
