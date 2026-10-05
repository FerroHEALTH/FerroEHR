// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Simplified Formats readers (`openehr_sdt::flat`): FLAT and STRUCTURED
//! composition bodies, which the ITS-REST Formats sub-spec accepts on a
//! composition write.
//!
//! Only the template-free half is fuzzable as a pure parse: the FLAT path
//! grammar, the STRUCTURED tree reader, and the two transforms between them.
//! Building a COMPOSITION needs a Web Template, which is server state, not
//! request bytes.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };

    // A FLAT body is a JSON object of path→value; a STRUCTURED body is a tree.
    if let serde_json::Value::Object(map) = &value {
        let _ = openehr_sdt::flat::sim::flat::parse_flat(map);
        let _ = openehr_sdt::flat::convert::flat_to_structured(map);
    }
    let _ = openehr_sdt::flat::sim::structured::parse_structured(&value);
    let _ = openehr_sdt::flat::convert::structured_to_flat(&value);
});
