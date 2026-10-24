// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The methods each mounted API route serves, so the access gates answer an
//! unserved method `405` before they judge the caller.
//!
//! ITS-REST `overview/Requests_and_responses` §HTTP Methods answers a method
//! the resource does not serve `405`, and RFC 9110 §15.5.6 makes `Allow`
//! mandatory on it. Whether a method is served is a property of the resource,
//! never of the credential, so the RBAC and SMART on openEHR checks judge only
//! a served method (no openEHR spec orders the checks: our own design).
//!
//! The table is read from the API's `OpenAPI` document, which `utoipa-axum`
//! builds from the same `routes!` registrations that mount each route.

use std::collections::HashMap;

use axum::response::Response;
use http::{HeaderValue, Method, StatusCode, header};

use crate::overview::error::status_error_response;

/// The served methods of every mounted API route template.
#[derive(Debug, Default)]
pub(crate) struct ServedMethods {
    /// The methods of each full route template, in the order `Allow` lists
    /// them.
    routes: HashMap<String, Vec<Method>>,
}

impl ServedMethods {
    /// Returns the table of the API mounted under `base_path`.
    pub(crate) fn new(base_path: &str) -> Self {
        let document = crate::api::api_doc(base_path);
        let mut routes = HashMap::new();
        for (path, item) in &document.paths.paths {
            let declared = [
                (Method::GET, item.get.is_some()),
                // NOTE: axum `routing::get` serves HEAD as well (docs.rs axum
                // 0.8 `MethodRouter::get`), so a GET route serves HEAD.
                (Method::HEAD, item.head.is_some() || item.get.is_some()),
                (Method::POST, item.post.is_some()),
                (Method::PUT, item.put.is_some()),
                (Method::PATCH, item.patch.is_some()),
                (Method::DELETE, item.delete.is_some()),
                (Method::OPTIONS, item.options.is_some()),
                (Method::TRACE, item.trace.is_some()),
            ];
            let methods: Vec<Method> = declared
                .into_iter()
                .filter_map(|(method, served)| served.then_some(method))
                .collect();
            if !methods.is_empty() {
                routes.insert(path.clone(), methods);
            }
        }
        Self { routes }
    }

    /// Returns the `405` for `method` on the matched route template `matched`
    /// when the route does not serve it, with the `Allow` field naming the
    /// methods it does; `None` when the method is served or the route is not
    /// in the table, which leaves the request to the access checks.
    pub(crate) fn refusal(&self, method: &Method, matched: Option<&str>) -> Option<Response> {
        let methods = self.routes.get(matched?)?;
        if methods.contains(method) {
            return None;
        }
        let allow = methods
            .iter()
            .map(Method::as_str)
            .collect::<Vec<_>>()
            .join(",");
        let mut resp = status_error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "the request method is not allowed on this resource",
        );
        if let Ok(value) = HeaderValue::from_str(&allow) {
            resp.headers_mut().insert(header::ALLOW, value);
        }
        Some(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "/ferroehr/rest/openehr/v1";

    #[test]
    fn an_unserved_method_is_refused_with_the_served_set() {
        let served = ServedMethods::new(BASE);
        let ehr = format!("{BASE}/ehr");
        let resp = served
            .refusal(&Method::DELETE, Some(&ehr))
            .expect("DELETE /ehr is not served");
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        let allow = resp
            .headers()
            .get(header::ALLOW)
            .and_then(|v| v.to_str().ok())
            .expect("an Allow field");
        assert!(allow.contains("POST"), "{allow}");
        assert!(!allow.contains("DELETE"), "{allow}");
    }

    #[test]
    fn a_served_method_and_an_unknown_route_are_left_to_the_checks() {
        let served = ServedMethods::new(BASE);
        let ehr = format!("{BASE}/ehr");
        assert!(served.refusal(&Method::POST, Some(&ehr)).is_none());
        let one = format!("{BASE}/ehr/{{ehr_id}}");
        assert!(served.refusal(&Method::GET, Some(&one)).is_none());
        assert!(served.refusal(&Method::HEAD, Some(&one)).is_none());
        assert!(
            served
                .refusal(&Method::DELETE, Some("/elsewhere"))
                .is_none()
        );
        assert!(served.refusal(&Method::DELETE, None).is_none());
    }
}
