// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The lax-mode SQL/JSON path reading the store's search applies to a
//! rendered `AuditEvent` document, in Rust where the column search cannot
//! answer.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the store's search \
              reads an already-rendered FHIR document as a JSON value"
)]

/// The items `steps` reaches from `from`, read as an SQL/JSON path in lax mode
/// reads them: an array is unwrapped before each member step and once at the
/// end, and a missing member yields nothing (ISO/IEC 9075-2 SQL/JSON path, as
/// PostgreSQL 18 documents it in "The SQL/JSON Path Language",
/// <https://www.postgresql.org/docs/18/functions-json.html#FUNCTIONS-SQLJSON-PATH>).
pub(crate) fn lax_path<'a>(
    from: &'a serde_json::Value,
    steps: &[&str],
) -> Vec<&'a serde_json::Value> {
    let unwrap = |items: Vec<&'a serde_json::Value>| -> Vec<&'a serde_json::Value> {
        items
            .into_iter()
            .flat_map(|item| match item {
                serde_json::Value::Array(elements) => elements.iter().collect::<Vec<_>>(),
                other => vec![other],
            })
            .collect()
    };
    let mut items = vec![from];
    for step in steps {
        items = unwrap(items)
            .into_iter()
            .filter_map(|item| item.get(*step))
            .collect();
    }
    unwrap(items)
}

/// Whether any item `steps` reaches from `from` is `value`.
pub(super) fn any_is(from: &serde_json::Value, steps: &[&str], value: &serde_json::Value) -> bool {
    lax_path(from, steps).into_iter().any(|item| item == value)
}

/// Whether any item `steps` reaches from `from` is the string `text`.
pub(super) fn any_equals(from: &serde_json::Value, steps: &[&str], text: &str) -> bool {
    lax_path(from, steps)
        .into_iter()
        .any(|item| item.as_str() == Some(text))
}
