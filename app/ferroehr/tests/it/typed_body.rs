// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared fixture→typed-value door for the service-seam suites.
//!
//! The commit seams take typed `openehr-rm` values, while the suites author
//! their bodies as raw canonical JSON — deliberately, because an
//! independently-authored wire body is what catches a codec defect a
//! typed-then-serialized value cannot (`.claude/rules/testing.md`
//! §Test-fixture construction, class 2). This module is the one place those
//! bodies cross into the typed world, through the same strict canonical
//! reader (`openehr_its::json::from_canonical_value`) a REST request body goes
//! through — so a fixture that is not a valid instance of its RM class fails
//! loudly, at the fixture, naming the offending member.

use serde_json::Value;

/// The fixture body as the typed RM value its commit seam takes.
///
/// # Panics
/// If `body` is not a valid canonical instance of `T` — a defective fixture,
/// which must be fixed rather than routed around.
#[expect(
    clippy::panic,
    reason = "a defective test fixture must fail loudly at the fixture, naming \
              the offending member — this helper is test scaffolding, not \
              production code, and its only sane failure mode is a panic"
)]
pub(crate) fn typed<T: serde::de::DeserializeOwned>(body: &Value) -> T {
    openehr_its::json::from_canonical_value(body)
        .unwrap_or_else(|e| panic!("the fixture body should decode as its RM type: {e}"))
}
