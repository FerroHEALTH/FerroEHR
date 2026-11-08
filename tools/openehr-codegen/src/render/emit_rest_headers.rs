// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The impls emitted on each generated response-headers struct: the
//! `ResponseHeaders` seam the server half writes through, and the typed
//! `ETag` accessor the client half reads through.

use std::fmt::Write as _;

use crate::render::emit_rest::{LIST_HEADERS, field_id};

/// The `ResponseHeaders` impl of one headers struct (a `Some` value is one
/// field, every item of a list header one field line), and, where the struct
/// carries an `ETag`, its typed accessor.
pub(crate) fn emit_response_headers_impl(b: &mut String, name: &str, headers: &[String]) {
    let _ = write!(
        b,
        "impl crate::rest::runtime::ResponseHeaders for {name} {{\n    \
         fn into_header_map(\n        self,\n    \
         ) -> Result<http::HeaderMap, crate::rest::runtime::HeaderError> {{\n        \
         let mut map = http::HeaderMap::new();\n"
    );
    for h in headers {
        let call = if LIST_HEADERS.contains(&h.as_str()) {
            "append_headers"
        } else {
            "set_header"
        };
        let _ = writeln!(
            b,
            "        crate::rest::runtime::{call}(&mut map, \"{h}\", self.{})?;",
            field_id(h)
        );
    }
    b.push_str("        Ok(map)\n    }\n}\n\n");
    if headers.iter().any(|h| h == "ETag") {
        emit_entity_tag(b, name);
    }
}

/// The accessor that reads a headers struct's raw `ETag` as an ITS-REST
/// entity tag (`openehr-its` `rest::etag`).
fn emit_entity_tag(b: &mut String, name: &str) {
    let _ = write!(
        b,
        "impl {name} {{\n    \
         /// The `ETag` read as an entity tag, `None` when the answer carries none.\n    \
         ///\n    \
         /// # Errors\n    \
         ///\n    \
         /// The inner [`crate::rest::etag::EntityTagError`] when the value is no\n    \
         /// RFC 9110 entity tag.\n    \
         pub fn entity_tag(\n        &self,\n    \
         ) -> Option<Result<crate::rest::etag::EntityTag, crate::rest::etag::EntityTagError>> {{\n        \
         self.etag.as_deref().map(str::parse)\n    \
         }}\n}}\n\n"
    );
}
