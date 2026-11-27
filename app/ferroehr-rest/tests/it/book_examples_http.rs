// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The requests of the book's "Your first integration" tutorial, run in page
//! order against a fresh server, each held to the status its marker names.
//!
//! A request is a fenced `http` block after a `<!-- book-request: NAME ... -->`
//! marker whose attributes are `expect=STATUS`, `body-file=PATH` (relative to
//! the repository root), `capture=VAR:/json/pointer` (with `|object` for the
//! versioned object id before `::`), `json-has=/json/pointer` and
//! `header=NAME:VALUE`. A `{VAR}` in the request is a value an earlier answer
//! captured. The page shows no credentials, so the server runs without
//! authentication.

use std::collections::BTreeMap;
use std::path::PathBuf;

use http::StatusCode;
use serde_json::Value;

use crate::common;

const PAGE: &str = "website/book/src/tutorials/first-integration.md";
const MARKER: &str = "<!-- book-request: ";

/// One request of the page, with what its marker asks of the answer.
#[derive(Debug)]
struct Example {
    name: String,
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
    body_file: Option<String>,
    expect: u16,
    captures: Vec<(String, String)>,
    json_has: Vec<String>,
    header_has: Vec<(String, String)>,
}

fn repo_file(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()))
}

/// Every marked request of the page, in page order.
fn examples() -> Vec<Example> {
    let page = repo_file(PAGE);
    let mut found = Vec::new();
    for chunk in page.split(MARKER).skip(1) {
        let (attributes, rest) = chunk
            .split_once(" -->\n")
            .expect("a marker closes on its line");
        let block = rest
            .strip_prefix("```http\n")
            .unwrap_or_else(|| panic!("an http block follows the marker {attributes}"));
        let block = block.split_once("\n```").expect("the http block closes").0;
        let (head, body) = block.split_once("\n\n").unwrap_or((block, ""));
        let mut lines = head.lines();
        let (method, path) = lines
            .next()
            .and_then(|line| line.split_once(' '))
            .expect("a request line");
        let headers = lines
            .map(|line| {
                let (name, value) = line.split_once(": ").expect("a header line");
                (name.to_owned(), value.to_owned())
            })
            .collect();
        let mut words = attributes.split_whitespace();
        let name = words.next().expect("a marker names its request").to_owned();
        let mut example = Example {
            name,
            method: method.to_owned(),
            path: path.to_owned(),
            headers,
            body: body.to_owned(),
            body_file: None,
            expect: 0,
            captures: Vec::new(),
            json_has: Vec::new(),
            header_has: Vec::new(),
        };
        for word in words {
            let (key, value) = word.split_once('=').expect("an attribute is key=value");
            match key {
                "expect" => example.expect = value.parse().expect("a status code"),
                "body-file" => example.body_file = Some(value.to_owned()),
                "capture" => {
                    let (var, pointer) = value.split_once(':').expect("capture=VAR:/pointer");
                    example.captures.push((var.to_owned(), pointer.to_owned()));
                }
                "json-has" => example.json_has.push(value.to_owned()),
                "header" => {
                    let (name, wanted) = value.split_once(':').expect("header=NAME:VALUE");
                    example
                        .header_has
                        .push((name.to_owned(), wanted.to_owned()));
                }
                other => panic!("unknown marker attribute {other}"),
            }
        }
        found.push(example);
    }
    found
}

/// `text` with each `{VAR}` replaced by its captured value.
fn fill(text: &str, vars: &BTreeMap<String, String>) -> String {
    vars.iter().fold(text.to_owned(), |text, (var, value)| {
        text.replace(&format!("{{{var}}}"), value)
    })
}

#[tokio::test]
async fn the_first_integration_tutorial_runs_as_written() {
    let (_pg, app) = common::app(common::api_config(false)).await;
    let examples = examples();
    assert!(examples.len() >= 8, "the tutorial holds its eight requests");
    let mut vars = BTreeMap::new();
    for example in examples {
        let path = fill(&example.path, &vars);
        let mut req = common::req(&example.method, path.as_str());
        for (name, value) in &example.headers {
            req = req.header(name.as_str(), fill(value, &vars));
        }
        let body = match &example.body_file {
            Some(file) => repo_file(file),
            None => fill(&example.body, &vars),
        };
        let (status, headers, text) = common::send(&app, req.body(body).build()).await;
        assert_eq!(
            status,
            StatusCode::from_u16(example.expect).expect("a status"),
            "{} answers {}: {text}",
            example.name,
            example.expect
        );
        for (name, wanted) in &example.header_has {
            let got = headers
                .get(name.as_str())
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            assert!(
                got.starts_with(wanted.as_str()),
                "{} carries {name}: {wanted}, got {got}",
                example.name
            );
        }
        if example.captures.is_empty() && example.json_has.is_empty() {
            continue;
        }
        let json: Value = serde_json::from_str(&text).expect("a JSON answer");
        for pointer in &example.json_has {
            assert!(
                json.pointer(pointer).is_some(),
                "{} holds {pointer}: {json}",
                example.name
            );
        }
        for (var, pointer) in &example.captures {
            let (pointer, object) = match pointer.strip_suffix("|object") {
                Some(pointer) => (pointer, true),
                None => (pointer.as_str(), false),
            };
            let value = json
                .pointer(pointer)
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("{} holds {pointer}", example.name));
            let value = if object {
                value.split_once("::").map_or(value, |(id, _)| id)
            } else {
                value
            };
            vars.insert(var.clone(), value.to_owned());
        }
    }
}
