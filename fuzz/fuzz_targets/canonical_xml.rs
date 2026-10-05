// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The canonical-XML reader: the same REST writes as JSON, reached by content
//! negotiation, plus the AOM2 archetype-XML doors the definition surface reads.
//!
//! This is the reader that already shipped a defect — a document nested deeper
//! than the recursive-descent `FromXml` impls could take recursed off the
//! stack, and a Rust stack overflow ABORTS instead of unwinding, so the
//! catch-panic layer could not render its clean `500`. The lexical drain below
//! runs on every input for exactly that reason.

#![no_main]

use libfuzzer_sys::fuzz_target;

/// Drive the lexical reader to exhaustion — the layer that holds the DOCTYPE,
/// entity and depth refusals, independent of any typed door.
fn drain(xml: &str) -> Result<(), openehr_its::xml::runtime::XmlError> {
    let mut reader = openehr_its::xml::runtime::XmlReader::new(xml);
    loop {
        if matches!(reader.read()?, openehr_its::xml::runtime::XmlEvent::Eof) {
            return Ok(());
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    // The lexical layer sees every input; a mutated byte string almost always
    // dies here, which keeps the typed doors below on structurally valid XML.
    if drain(text).is_err() {
        return;
    }

    let _ = openehr_its::xml::from_canonical_xml::<openehr_rm::prelude::Composition>(text);
    let _ = openehr_its::xml::from_canonical_xml::<openehr_rm::prelude::EhrStatus>(text);
    let _ = openehr_its::xml::from_canonical_xml::<openehr_rm::prelude::Folder>(text);
    let _ = openehr_its::xml::from_canonical_xml::<openehr_rm::prelude::Contribution>(text);
    let _ = openehr_its::aom2::from_xml(text);
    let _ = openehr_its::aom2_model::from_xml(text);
});
