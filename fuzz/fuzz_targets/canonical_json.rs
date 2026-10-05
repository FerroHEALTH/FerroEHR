// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The canonical-JSON reader: every write on the REST surface arrives here.
//!
//! A pure parse — the typed strict reader (`openehr_its::json`), then, once the
//! bytes are well-formed JSON, the wire door `ferroehr-rest` runs during
//! content negotiation and the RM/terminology validators a commit runs before
//! touching storage. No I/O, no database, no global mutable state.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    // The typed doors, on the raw text: each REST route deserializes into one
    // concrete RM root, so each is its own reachable strict-reader path.
    let _ = openehr_its::json::from_canonical_json::<openehr_rm::prelude::Composition>(text);
    let _ = openehr_its::json::from_canonical_json::<openehr_rm::prelude::EhrStatus>(text);
    let _ = openehr_its::json::from_canonical_json::<openehr_rm::prelude::Folder>(text);
    let _ = openehr_its::json::from_canonical_json::<openehr_rm::prelude::Contribution>(text);

    // The untyped doors take a tree; building one is cheap and gates them out
    // for the overwhelming majority of mutated inputs.
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let _ = openehr_its::json::reject_undeclared_keys(&value);
    let mut violations = Vec::new();
    openehr_its::wire_validate::validate_rm_invariants(&value, &mut violations);
    violations.clear();
    openehr_its::wire_validate::validate_rm_value(&value, &mut violations);
    let _ = openehr_sdt::rm_instance::validate_rm_and_terminology(&value);
    let _ = openehr_its::json::from_canonical_value::<openehr_rm::prelude::Composition>(&value);
});
