// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Refusals the XML reader must keep making, each pinned as an asserted
//! negative test.
//!
//! Two properties here are held by a DEPENDENCY's current behaviour rather than
//! by this code, which is exactly the kind that changes silently: `quick-xml`
//! declares that it does not parse DTDs (<https://docs.rs/quick-xml/>), so no
//! entity declaration is ever recorded and an external entity cannot resolve.
//! Rather than rely on that, the reader refuses a DOCTYPE outright and refuses
//! any entity outside the five predefined names — and these tests assert the
//! refusals, so the guarantee stops depending on an upstream implementation
//! detail.
//!
//! The depth bound is different in kind: without it, the recursive descent in
//! the generated `FromXml` impls overflows the stack, and a Rust stack overflow
//! ABORTS the process instead of unwinding — so the catch-panic layer that
//! renders this server's clean `500` cannot intercept it, and one request would
//! take the process down for every caller.
//!
//! No openEHR spec governs these bounds — our own design.

#![expect(clippy::panic_in_result_fn, reason = "test assertions and fixtures")]

use openehr_its::xml::runtime::MAX_DEPTH;
use openehr_its::xml::runtime::XmlError;
use openehr_its::xml::runtime::XmlEvent;
use openehr_its::xml::runtime::XmlReader;

/// Drive the reader to exhaustion, returning the first error if any.
fn drain(xml: &str) -> Result<(), XmlError> {
    let mut reader = XmlReader::new(xml);
    loop {
        if matches!(reader.read()?, XmlEvent::Eof) {
            return Ok(());
        }
    }
}

/// A DOCTYPE is refused rather than skipped.
///
/// The classic XXE shape: the declaration is where an attacker names an external
/// entity. Canonical openEHR XML has no use for a DOCTYPE, so refusing the
/// declaration removes the whole question.
#[test]
fn a_doctype_declaration_is_refused() {
    let xml = concat!(
        r#"<?xml version="1.0"?>"#,
        r#"<!DOCTYPE composition [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>"#,
        r#"<composition><name>&xxe;</name></composition>"#,
    );
    let err = drain(xml).expect_err("a DOCTYPE must be refused");
    let message = err.to_string();
    assert!(
        message.contains("DOCTYPE"),
        "the refusal must name what it refused: {message}"
    );
}

/// A DOCTYPE naming an external DTD by URL is refused for the same reason,
/// before any question of fetching it arises.
#[test]
fn an_external_dtd_reference_is_refused() {
    let xml = concat!(
        r#"<!DOCTYPE composition SYSTEM "http://example.invalid/evil.dtd">"#,
        r#"<composition/>"#,
    );
    assert!(drain(xml).is_err(), "an external DTD must be refused");
}

/// An entity outside the five XML predefined names does not resolve — it is an
/// error, never an empty string or a silently dropped node.
///
/// This is the second half of the XXE defence: even if a declaration were ever
/// parsed, the reference could not resolve to anything.
#[test]
fn an_undeclared_entity_reference_is_refused() {
    let err = drain("<composition><name>&secret;</name></composition>")
        .expect_err("an undeclared entity must be refused");
    let message = err.to_string();
    assert!(
        message.contains("unknown entity"),
        "the refusal must name the entity: {message}"
    );
}

/// The five predefined entities and character references still resolve, so the
/// refusal above is narrow rather than a blanket rejection of escaped text.
#[test]
fn the_predefined_entities_still_resolve() -> Result<(), XmlError> {
    let mut reader = XmlReader::new("<v>a &amp; b &lt; c &#65;</v>");
    let mut text = String::new();
    loop {
        match reader.read()? {
            XmlEvent::Text(t) => text.push_str(&t),
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    assert!(text.contains('&'), "&amp; must resolve: {text:?}");
    assert!(text.contains('<'), "&lt; must resolve: {text:?}");
    assert!(text.contains('A'), "&#65; must resolve: {text:?}");
    Ok(())
}

/// Nesting past the limit is refused.
///
/// Built from the RM's own recursion (`CLUSTER.items` holds `Item`, which
/// includes `CLUSTER`), because that is the shape an attacker has available —
/// unknown element names are skipped iteratively and never recurse.
#[test]
fn nesting_past_the_limit_is_refused() {
    let depth = usize::try_from(MAX_DEPTH).unwrap() + 8;
    let mut xml = String::from(r#"<composition xmlns="http://schemas.openehr.org/v1">"#);
    for _ in 0..depth {
        xml.push_str(r#"<items xsi:type="CLUSTER">"#);
    }
    for _ in 0..depth {
        xml.push_str("</items>");
    }
    xml.push_str("</composition>");

    let err = drain(&xml).expect_err("nesting past the limit must be refused");
    let message = err.to_string();
    assert!(
        message.contains("nesting"),
        "the refusal must say what it refused: {message}"
    );
}

/// And nesting within the limit is still accepted, so the bound is a ceiling
/// rather than a new restriction on real documents. Depth is also RELEASED as
/// elements close, so a wide document of shallow siblings is unaffected —
/// otherwise a long list would false-positive.
#[test]
fn deep_but_legal_nesting_and_wide_documents_are_accepted() -> Result<(), XmlError> {
    let legal = usize::try_from(MAX_DEPTH).unwrap() - 2;
    let mut xml = String::from("<composition>");
    for _ in 0..legal {
        xml.push_str("<items>");
    }
    for _ in 0..legal {
        xml.push_str("</items>");
    }
    xml.push_str("</composition>");
    drain(&xml)?;

    // Ten times the depth limit in SIBLINGS, one level deep.
    let mut wide = String::from("<composition>");
    for _ in 0..(MAX_DEPTH * 10) {
        wide.push_str("<items/>");
    }
    wide.push_str("</composition>");
    drain(&wide)?;
    Ok(())
}
