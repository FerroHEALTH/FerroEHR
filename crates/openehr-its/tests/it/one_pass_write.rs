// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: Apache-2.0

//! The one-pass writers render the same bytes as the string writers, into a
//! caller's buffer (#6002).

#![expect(
    clippy::expect_used,
    reason = "fixture helpers outside a #[test] fn: a missing fixture or a refused write is a test failure"
)]

use openehr_its::xml::runtime::Namespace;
use openehr_rm::prelude::Composition;

fn ips() -> Composition {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json"
    ))
    .expect("the IPS fixture should be readable");
    openehr_its::json::from_canonical_json(&text).expect("the IPS fixture should read")
}

#[test]
fn json_into_a_buffer_is_the_string_writer_bytes_after_a_prefix() {
    let comp = ips();
    let mut buffer = b"prefix".to_vec();
    openehr_its::json::write_canonical_json(&comp, &mut buffer);
    let expected = openehr_its::json::to_canonical_json(&comp);
    assert_eq!(buffer.get(..6), Some(&b"prefix"[..]));
    assert_eq!(buffer.get(6..), Some(expected.as_bytes()));

    let mut written = Vec::new();
    openehr_its::json::write_canonical_json_to(&mut written, &comp).expect("writes");
    assert_eq!(written, expected.as_bytes());
}

#[test]
fn xml_into_a_buffer_is_the_string_writer_bytes_after_a_prefix() {
    let comp = ips();
    let bytes = openehr_its::xml::runtime::write_xml_into(
        &comp,
        "composition",
        Namespace::V2,
        b"prefix".to_vec(),
    )
    .expect("writes");
    let expected =
        openehr_its::xml::runtime::to_xml(&comp, "composition", Namespace::V2).expect("writes");
    assert_eq!(bytes.get(..6), Some(&b"prefix"[..]));
    assert_eq!(bytes.get(6..), Some(expected.as_bytes()));
}
