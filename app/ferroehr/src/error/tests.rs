// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The tests of the error model.

use openehr_base::validate::InvariantViolation;

use super::{ServiceError, Violation};

/// A [`Violation`] renders `[<path> ]<detail>[: <cause>; …][ (<invariant>)]`
/// — the ONE place the facts become prose. Each fact is independently
/// readable back off the value, which is what the refusal tests assert.
#[test]
fn violation_renders_its_facts_and_keeps_them_queryable() {
    let plain = Violation::new("contribution must contain versions");
    assert_eq!(plain.to_string(), "contribution must contain versions");
    assert_eq!(plain.path(), None);
    assert_eq!(plain.invariant(), None);
    assert_eq!(plain.causes(), []);

    let rule = Violation::new("must be a non-empty list when present")
        .with_path("ATTESTATION.items")
        .with_invariant("ATTESTATION.Items_valid");
    assert_eq!(
        rule.to_string(),
        "ATTESTATION.items must be a non-empty list when present \
         (ATTESTATION.Items_valid)"
    );
    assert_eq!(rule.path(), Some("ATTESTATION.items"));
    assert_eq!(rule.invariant(), Some("ATTESTATION.Items_valid"));

    let nested = Violation::new("is not a valid PARTY_PROXY")
        .with_path("AUDIT_DETAILS.committer")
        .with_causes(vec![
            InvariantViolation::here("Invariant Basic_validity failed"),
            InvariantViolation::at("name", "Invariant Name_valid failed"),
        ]);
    assert_eq!(
        nested.to_string(),
        "AUDIT_DETAILS.committer is not a valid PARTY_PROXY: \
         Invariant Basic_validity failed; Invariant Name_valid failed"
    );
    assert_eq!(nested.causes().len(), 2);
    assert_eq!(nested.causes()[1].path, "name");
}

/// A carried cause stays WALKABLE — [`std::error::Error::source`] reaches
/// the concrete underlying error type, which is the whole point of RFC 0201
/// chaining. The protocol adapter's tests pin that it never reaches a body.
#[test]
fn a_carried_cause_is_walkable() {
    use std::error::Error;

    let err = ServiceError::internal("read a stored version row", cause());
    let first = Error::source(&err).expect("a carried cause must be reachable");

    let mut hops = Vec::new();
    let mut found = None;
    let mut next = Some(first);
    while let Some(step) = next {
        hops.push(step.to_string());
        if let Some(io) = step.downcast_ref::<std::io::Error>() {
            found = Some(io.kind());
        }
        next = step.source();
    }
    assert_eq!(
        found,
        Some(std::io::ErrorKind::PermissionDenied),
        "walking the chain must reach the underlying std::io::Error, got {hops:?}"
    );
}

/// A [`Violation`] with a carried cause renders EXACTLY as one without —
/// the 422 body is the refusal's own facts, and the cause is chain-only.
#[test]
fn a_violation_cause_is_chain_only() {
    use std::error::Error;

    let plain = Violation::new("is not a valid PARTY_PROXY").with_path("AUDIT_DETAILS.committer");
    let carried = Violation::new("is not a valid PARTY_PROXY")
        .with_path("AUDIT_DETAILS.committer")
        .with_source(cause());
    assert_eq!(carried.to_string(), plain.to_string());
    assert_eq!(carried, plain, "equality is over the facts, not the cause");
    assert!(Error::source(&plain).is_none());
    // The hop must be the CONCRETE cause, not a smart-pointer wrapper
    // around it — otherwise no caller can branch on what actually failed.
    assert_eq!(
        Error::source(&carried)
            .and_then(|c| c.downcast_ref::<std::io::Error>())
            .map(std::io::Error::kind),
        Some(std::io::ErrorKind::PermissionDenied)
    );
}

/// A stand-in underlying failure whose `Display` is exactly the kind of
/// internal detail a `500` body must never carry.
fn cause() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "the node table is not readable",
    )
}

/// A signing failure reaches its concrete cause; the protocol adapter's
/// tests pin that it still renders as a server fault.
///
/// Downcasting rather than asserting `source().is_some()` is deliberate:
/// `Display` forwards either way, so a chain that is broken at the seam
/// reads correctly in a log while `downcast_ref` returns `None` — the exact
/// silent loss `reliability.md` records, and the only thing that catches it.
#[test]
fn a_signing_failure_reaches_its_concrete_cause() {
    use std::error::Error;

    let pgp = crate::versioning::signature::key::PgpSignError::from(pgp::errors::Error::Message {
        message: "no secret key".to_owned(),
        backtrace: None,
    });
    let signing =
        ServiceError::SigningFailed(crate::versioning::signature::signer::SignError::Pgp(pgp));
    assert!(
        !signing.to_string().contains("no secret key"),
        "the cause is the source, not part of the message: {signing}"
    );

    // Walked to the ROOT cause, not to a wrapper. `SignError::Pgp` is
    // `#[error(transparent)]`, and transparent forwards `source()` as well
    // as `Display` — so the wrapper is not a hop in the chain at all, and a
    // test looking for it would fail while the chain was perfectly intact.
    let mut found = false;
    let mut next = Error::source(&signing);
    while let Some(step) = next {
        if step.downcast_ref::<pgp::errors::Error>().is_some() {
            found = true;
        }
        next = step.source();
    }
    assert!(
        found,
        "a signing failure must keep the underlying OpenPGP error reachable, or a \
         caller cannot tell a key problem from a configuration problem without \
         parsing prose"
    );
}
