// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Run one codec micro-path once, for an instruction count under callgrind.
//!
//! Usage: `codec_instructions <path> <composition.json>`, where `<path>` is
//! one of `json_to_rm`, `rm_to_json`, `xml_to_rm` and `rm_to_xml`. The input
//! is read and every other representation the path needs is built before the
//! path runs, inside [`measured`] alone, so `valgrind --tool=callgrind
//! --toggle-collect='*measured*'` counts that path and nothing else
//! (`scripts/perf/instructions.sh`, #6007).

use std::error::Error;

use openehr_its::xml::runtime::Namespace;
use openehr_rm::v1_2::composition::composition::Composition;

/// Runs `path` and keeps its result opaque to the optimizer.
#[inline(never)]
fn measured<R>(path: impl FnOnce() -> R) -> R {
    std::hint::black_box(path())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(input)) = (args.next(), args.next()) else {
        return Err("usage: codec_instructions <path> <composition.json>".into());
    };
    let json = std::fs::read_to_string(input)?;
    let composition: Composition = openehr_its::json::from_canonical_json(&json)?;
    match path.as_str() {
        "json_to_rm" => {
            measured(|| openehr_its::json::from_canonical_json::<Composition>(&json))?;
        }
        "rm_to_json" => {
            measured(|| openehr_its::json::to_canonical_json(&composition));
        }
        "xml_to_rm" => {
            let xml =
                openehr_its::xml::runtime::to_xml(&composition, "composition", Namespace::V2)?;
            measured(|| openehr_its::xml::from_canonical_xml::<Composition>(&xml))?;
        }
        "rm_to_xml" => {
            measured(|| {
                openehr_its::xml::runtime::to_xml(&composition, "composition", Namespace::V2)
            })?;
        }
        other => return Err(format!("unknown path `{other}`").into()),
    }
    Ok(())
}
