// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OPT 1.4 reader: an operational template is uploaded as XML on the
//! definition surface, and the server immediately walks the parsed template to
//! build its Web Template.
//!
//! Both halves are pure functions over the request bytes, so both are fuzzed
//! here: the generated `FromXml` tree for `OPERATIONAL_TEMPLATE`, and the
//! Web Template builder that reads whatever came out of it.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(template) = openehr_its::opt14::from_xml(text) else {
        return;
    };
    let _ = openehr_sdt::flat::webtemplate::builder::build_web_template(&template);
});
