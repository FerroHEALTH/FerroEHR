// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Pure readings of AQL **text** the query screens need before they can run a
//! query.
//!
//! Which `$placeholders` it declares, whether it carries its own row window,
//! and how typed parameter values become the `query_parameters` JSON object.
//! Component-free plain Rust with ordinary unit tests, compiled for both the
//! `ssr` and `hydrate` targets.
//!
//! The readings go through the real AQL lexer (`openehr_query::lexer`) rather
//! than a hand-rolled scan, so a `$name` inside a string literal is never
//! mistaken for a parameter.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use std::collections::BTreeMap;

/// The `query_parameters` names a query declares, in first-appearance order and
/// de-duplicated, **without** the `$` prefix.
///
/// AQL writes a parameter as `$name` (QUERY `docs/AQL/` — the `PARAMETER`
/// token); the wire form drops the sigil: "Provided query parameters SHOULD NOT
/// be prefixed with `$` sign. Instead, the server will (whenever necessary) add
/// the prefix" (ITS-REST `specifications/docs/query/Request.md` §Common Headers
/// and Query Parameters / Query parameters). The names returned here are
/// therefore exactly the JSON keys a run must send.
///
/// Text the lexer cannot tokenize yields NO names rather than a guess: such a
/// query cannot be executed either, and the run surfaces the CDR's own
/// diagnostic instead of a half-read parameter list.
#[must_use]
pub fn placeholders(aql: &str) -> Vec<String> {
    let Ok(tokens) = openehr_query::lexer::lex(aql) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for token in &tokens {
        if let openehr_query::lexer::Token::Parameter(raw) = token {
            let name = raw.strip_prefix('$').unwrap_or(raw);
            if !name.is_empty() && !out.iter().any(|seen| seen == name) {
                out.push(name.to_owned());
            }
        }
    }
    out
}

/// Does `aql` constrain its own row window — an AQL `LIMIT` or the deprecated
/// `TOP`?
///
/// ITS-REST keeps the request's paging window and the query's own window
/// mutually exclusive: `fetch` "cannot be combined with AQL-top" (ITS-REST
/// `specifications/docs/query/Request.md` §Common Headers and Query
/// Parameters). A query that windows itself is therefore executed bare, without
/// `fetch`/`offset`.
///
/// Text that does not parse is treated as carrying no window: the request then
/// looks exactly like every other paged run, and the CDR reports the real
/// problem with the query.
#[must_use]
pub fn carries_own_window(aql: &str) -> bool {
    openehr_query::parser::parse_str(aql)
        .is_ok_and(|query| query.limit.is_some() || query.select.top.is_some())
}

/// Turn the run surface's `(name, text)` parameter bindings into the
/// `query_parameters` JSON object.
///
/// Values are read as JSON first, so `38.5` binds as a number, `true` as a
/// boolean and `null` as null — the spec's own example binds a number and a
/// string side by side (`{"temperature": 38.5, "chills": "at0.64"}`, ITS-REST
/// `specifications/docs/query/Request.md` §Request details). Anything that is
/// not valid JSON binds as text, so `at0037`, `2026-07-01` and `1.0.0` need no
/// quoting; a value that must stay text despite looking numeric can be written
/// as a JSON string (`"0123"`).
///
/// A binding whose text is blank is OMITTED: an empty string is almost never
/// the value a query wants, and leaving the name out lets the CDR apply its own
/// default or say what is missing.
///
/// NOTE: the JSON-first reading is our own input convention — no openEHR spec
/// governs how the viewer collects a parameter value; only the resulting
/// object shape is spec-bound (cited above).
#[must_use]
pub fn parameters_json(bindings: &BTreeMap<String, String>) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    for (name, raw) in bindings {
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        let value = serde_json::from_str::<serde_json::Value>(text)
            .unwrap_or_else(|_| serde_json::Value::String(raw.clone()));
        object.insert(name.clone(), value);
    }
    serde_json::Value::Object(object)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::aql_text::{carries_own_window, parameters_json, placeholders};

    #[test]
    fn placeholders_reads_the_spec_example_query() {
        // The parameterised query from ITS-REST docs/query/Request.md
        // §Request details, shortened to its two parameter sites.
        let aql = "SELECT o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude \
                   AS temperature FROM EHR e CONTAINS OBSERVATION o \
                   WHERE o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/magnitude > $temperature \
                   AND o/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value/defining_code/code_string = $chills";
        assert_eq!(placeholders(aql), vec!["temperature", "chills"]);
    }

    #[test]
    fn placeholders_are_deduplicated_in_first_appearance_order() {
        let aql = "SELECT c FROM EHR e[ehr_id/value=$ehr] CONTAINS COMPOSITION c \
                   WHERE c/name/value=$name AND c/uid/value=$ehr";
        assert_eq!(placeholders(aql), vec!["ehr", "name"]);
    }

    #[test]
    fn placeholders_ignores_a_dollar_name_inside_a_string_literal() {
        // The lexer tokenizes the literal as one STRING, so its `$total` is
        // never read as a parameter.
        let aql = "SELECT c FROM COMPOSITION c WHERE c/name/value='cost in $total'";
        assert!(placeholders(aql).is_empty());
    }

    #[test]
    fn placeholders_of_untokenizable_text_is_empty() {
        // A stray character the lexer rejects: no guessed parameter list.
        assert!(placeholders("SELECT ↯ FROM COMPOSITION c WHERE x=$a").is_empty());
        assert!(placeholders("").is_empty());
    }

    #[test]
    fn placeholders_covers_every_parameter_site_the_grammar_allows() {
        // Predicate operand, node predicate, LIKE operand, MATCHES value list.
        let aql = "SELECT c FROM EHR e[ehr_id/value=$ehr] \
                   CONTAINS COMPOSITION c[$node] \
                   WHERE c/name/value LIKE $pattern \
                   AND c/context/other_context[at0001]/items[at0002]/value/defining_code/code_string MATCHES {$code}";
        assert_eq!(
            placeholders(aql),
            vec!["ehr", "node", "pattern", "code"],
            "every $parameter position must be reported"
        );
    }

    #[test]
    fn own_window_is_limit_or_top() {
        assert!(carries_own_window(
            "SELECT c FROM EHR e CONTAINS COMPOSITION c LIMIT 10"
        ));
        assert!(carries_own_window(
            "SELECT TOP 5 c FROM EHR e CONTAINS COMPOSITION c"
        ));
        assert!(!carries_own_window(
            "SELECT c FROM EHR e CONTAINS COMPOSITION c"
        ));
        // Unparseable text is windowed by the request, not by itself.
        assert!(!carries_own_window("NOT AQL AT ALL"));
    }

    /// Bindings as the run surface holds them.
    fn bindings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn parameters_json_types_the_spec_example_bindings() {
        let json = parameters_json(&bindings(&[("temperature", "38.5"), ("chills", "at0.64")]));
        assert_eq!(
            json,
            serde_json::json!({ "temperature": 38.5, "chills": "at0.64" })
        );
    }

    #[test]
    fn parameters_json_reads_json_scalars_and_falls_back_to_text() {
        let json = parameters_json(&bindings(&[
            ("count", "3"),
            ("flag", "true"),
            ("nothing", "null"),
            ("code", "at0037"),
            ("date", "2026-07-01"),
            ("version", "1.0.0"),
            ("forced", "\"0123\""),
        ]));
        assert_eq!(
            json,
            serde_json::json!({
                "count": 3,
                "flag": true,
                "nothing": null,
                "code": "at0037",
                "date": "2026-07-01",
                "version": "1.0.0",
                "forced": "0123",
            })
        );
    }

    #[test]
    fn parameters_json_omits_blank_bindings_and_keeps_inner_whitespace() {
        let json = parameters_json(&bindings(&[
            ("empty", ""),
            ("spaces", "   "),
            ("text", " two words "),
        ]));
        assert_eq!(json, serde_json::json!({ "text": " two words " }));
    }

    #[test]
    fn parameters_json_of_nothing_is_an_empty_object() {
        assert_eq!(parameters_json(&BTreeMap::new()), serde_json::json!({}));
    }
}
