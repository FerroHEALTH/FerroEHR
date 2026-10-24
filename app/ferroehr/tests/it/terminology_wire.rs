// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The request side of the FHIR terminology wire, shared by the suites that
//! stub the terminology server with `wiremock`.
//!
//! The client invokes `$lookup`, `$validate-code`, `$expand` and `$subsumes`
//! as a `POST` of a `Parameters` resource, the form that carries every input
//! whatever its type (FHIR R4B `operations.html` §Executing an Operation,
//! <https://hl7.org/fhir/R4B/operations.html#executing>), so a stub matches
//! the operation by its path and its inputs by their `parameter` entries.

use serde_json::Value;
use wiremock::matchers::{header, method, path};
use wiremock::{Match, MockBuilder, Request};

/// The FHIR JSON media type (<https://hl7.org/fhir/R4B/http.html#mime-type>).
pub(crate) const FHIR_JSON: &str = "application/fhir+json";

/// A stub for the operation at `operation_path` (`/CodeSystem/$lookup`, …)
/// invoked as a `POST` of a FHIR JSON `Parameters` body.
pub(crate) fn operation(operation_path: &str) -> MockBuilder {
    wiremock::Mock::given(method("POST"))
        .and(path(operation_path))
        .and(header("content-type", FHIR_JSON))
        .and(Parameters)
}

/// Matches a request whose body is a FHIR `Parameters` resource.
#[derive(Debug)]
pub(crate) struct Parameters;

impl Match for Parameters {
    fn matches(&self, request: &Request) -> bool {
        body(request).is_some()
    }
}

/// Matches a `Parameters` body carrying the input `name` with the primitive
/// value `value`, in whichever `value[x]` type the operation declares.
#[derive(Debug)]
pub(crate) struct Parameter {
    name: &'static str,
    value: String,
}

/// The input `name` with the value `value`.
pub(crate) fn parameter(name: &'static str, value: impl Into<String>) -> Parameter {
    Parameter {
        name,
        value: value.into(),
    }
}

impl Match for Parameter {
    fn matches(&self, request: &Request) -> bool {
        body(request).is_some_and(|parameters| {
            parameters.iter().any(|entry| {
                entry.get("name").and_then(Value::as_str) == Some(self.name)
                    && entry.as_object().is_some_and(|members| {
                        members.iter().any(|(key, value)| {
                            key.starts_with("value") && value.as_str() == Some(self.value.as_str())
                        })
                    })
            })
        })
    }
}

/// Matches a `Parameters` body that carries no input named `name`.
#[derive(Debug)]
pub(crate) struct Absent(pub(crate) &'static str);

impl Match for Absent {
    fn matches(&self, request: &Request) -> bool {
        body(request).is_some_and(|parameters| {
            parameters
                .iter()
                .all(|entry| entry.get("name").and_then(Value::as_str) != Some(self.0))
        })
    }
}

/// The `parameter` entries of a `Parameters` request body, `None` when the
/// body is not one.
fn body(request: &Request) -> Option<Vec<Value>> {
    let value: Value = serde_json::from_slice(&request.body).ok()?;
    if value.get("resourceType").and_then(Value::as_str) != Some("Parameters") {
        return None;
    }
    Some(
        value
            .get("parameter")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    )
}
