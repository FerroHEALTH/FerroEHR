// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

#![allow(
    clippy::panic,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr,
    let_underscore_drop,
    reason = "integration-test assertions, corpus diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]
//! Parse the official openEHR AQL worked-example corpus (vendored under
//! `vendor/examples/`, from specifications-QUERY). Every `----` listing block
//! that is standard **AQL 1.1.0** must parse.
//!
//! Some blocks are intentionally NOT standard AQL and are excluded with a
//! recorded reason — this is faithful to the grammar, not a gap:
//! - the spec doc itself states *"the ADL expressions on the right-hand side of
//!   the `matches` operator are in ADL 1.4 format"* — those `matches { TYPE
//!   matches {…} }` / `{|range|}` blocks are cADL, not the AQL `matches`
//!   value-list operand;
//! - `NOT IN (subquery)` is a SQL-style construct absent from `AqlParser.g4`.

use std::fs;
use std::path::Path;

/// Classify a block that is not standard AQL 1.1.0, with the reason. `None`
/// means it should parse as standard AQL.
fn out_of_grammar_reason(q: &str) -> Option<&'static str> {
    // ADL 1.4 cADL on the matches RHS: `{|range|}`, or `matches { TYPE matches …}`.
    let adl_range = q.contains("{|") || q.contains("|}") || q.contains("|>") || q.contains("|<");
    let nested_typed_matches = {
        // a `matches {` whose operand opens with an UPPER_CASE RM/cADL type.
        let mut hit = false;
        for (i, keyword) in q.match_indices("matches") {
            let rest = q.get(i + keyword.len()..).unwrap_or_default().trim_start();
            let rest = rest.strip_prefix('{').map_or("", str::trim_start);
            if rest.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && rest.split_whitespace().nth(1) == Some("matches")
            {
                hit = true;
                break;
            }
        }
        hit
    };
    if adl_range || nested_typed_matches {
        return Some("ADL 1.4 cADL on the `matches` RHS (illustrative, not AQL)");
    }
    let lower = q.to_lowercase();
    if lower.contains("not in") || lower.contains(" in (") || lower.contains(" in(") {
        return Some("SQL-style IN/subquery — not in the AQL 1.1.0 grammar");
    }
    None
}

/// Extract the contents of **every** `AsciiDoc` `----` listing block.
///
/// Previously only SELECT-containing blocks were kept, which silently dropped
/// any bare `WHERE`/operator/function/`matches` fragment example — so the
/// `WHERE`/predicate/function surface went unexercised (audit hygiene note on
/// `08-aql-parser.md`). We now take all blocks and classify each in
/// [`official_aql_corpus_parses`]: a block that already contains a full
/// `SELECT` is parsed as-is; a bare fragment is wrapped in a minimal
/// `SELECT … FROM EHR e WHERE <fragment>` shell before parsing.
fn listing_blocks(doc: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block = false;
    let mut buf = String::new();
    for line in doc.lines() {
        if line.trim() == "----" {
            if in_block {
                if !buf.trim().is_empty() {
                    out.push(std::mem::take(&mut buf));
                }
                buf.clear();
                in_block = false;
            } else {
                in_block = true;
            }
        } else if in_block {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    out
}

/// Wrap a bare AQL fragment (a `WHERE`-style boolean/predicate expression that
/// is not itself a complete `SELECT`) in a minimal query shell so it can be
/// parse-tested.
fn wrap_fragment(fragment: &str) -> String {
    format!("SELECT e/ehr_id/value FROM EHR e WHERE {}", fragment.trim())
}

/// Every standard AQL 1.1.0 query of the official corpus, a bare fragment
/// wrapped in its query shell, for the topic modules that sweep the corpus.
pub(crate) fn standard_queries() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/examples");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .expect("examples dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "adoc"))
        .collect();
    files.sort();
    let mut queries = Vec::new();
    for path in files {
        let doc = fs::read_to_string(&path).expect("example file");
        for block in listing_blocks(&doc) {
            if out_of_grammar_reason(&block).is_some() {
                continue;
            }
            queries.push(if block.to_uppercase().contains("SELECT") {
                block
            } else {
                wrap_fragment(&block)
            });
        }
    }
    queries
}

/// Parses one query and round-trips it through the printer.
///
/// The whole official corpus must satisfy parse → print → parse with an
/// identical AST; the error is the failure message to record.
fn round_trip_failure(query: &str) -> Result<(), String> {
    let ast = openehr_query::parser::parse_str(query).map_err(|e| e.to_string())?;
    let printed = openehr_query::printer::to_aql(&ast);
    match openehr_query::parser::parse_str(&printed) {
        Ok(reparsed) if reparsed == ast => Ok(()),
        Ok(_) => Err(format!("printer round-trip drifted the AST via: {printed}")),
        Err(e) => Err(format!("printed AQL failed to reparse: {printed}\n  {e}")),
    }
}

#[test]
fn official_aql_corpus_parses() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/examples");
    let mut total = 0;
    let mut parsed = 0;
    let mut excluded: Vec<&'static str> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();

    let mut files: Vec<_> = fs::read_dir(&dir)
        .expect("examples dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "adoc"))
        .collect();
    files.sort();

    for path in files {
        let doc = fs::read_to_string(&path).unwrap();
        for block in listing_blocks(&doc) {
            total += 1;
            if let Some(reason) = out_of_grammar_reason(&block) {
                excluded.push(reason);
                continue;
            }
            // Full queries parse as-is; bare fragments get a SELECT/FROM shell.
            let query = if block.to_uppercase().contains("SELECT") {
                block.clone()
            } else {
                wrap_fragment(&block)
            };
            match round_trip_failure(&query) {
                Ok(()) => parsed += 1,
                Err(message) => failures.push((query, message)),
            }
        }
    }

    println!(
        "AQL corpus: {total} listing blocks — {parsed} parsed, {} out-of-grammar excluded, {} failed",
        excluded.len(),
        failures.len()
    );
    for r in &excluded {
        println!("  excluded: {r}");
    }
    for (q, e) in &failures {
        println!("\n--- FAILED (standard AQL that did not parse) ---\n{q}\n  error: {e}");
    }
    assert!(
        failures.is_empty(),
        "{} standard-AQL example(s) failed to parse",
        failures.len()
    );
    // Every standard-AQL block parsed, and the whole official corpus is
    // accounted for (parsed + excluded == total).
    assert!(parsed > 0, "no standard-AQL examples found");
    assert_eq!(
        parsed + excluded.len(),
        total,
        "some blocks unaccounted for"
    );
}
