// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AQL `TERMINOLOGY()` seam: [`FerroEhrService`] implements the
//! [`TerminologyExpander`] trait (`crate::aql::terminology`) so the AQL
//! semantic-analysis pass (`aql/terminology.rs::expand_matches`) can resolve
//! `TERMINOLOGY('expand', …)` operands, the Boolean operations, and
//! `matches { terminology-uri }` through this service's providers
//! (QUERY master03 §TERMINOLOGY, §matches).
//!
//! A FHIR operand is answered by the terminology server its routing keys
//! select ([`crate::service::terminology::router::TerminologyRouter`]) — the
//! value set / `system` argument first, then the `service_api` flavour, then
//! the default provider — so several servers resolve different value sets in
//! one instance.

use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::aql::error::{AqlError, AqlFeatureError, ExecError};
use crate::aql::terminology::TerminologyExpander;
use crate::service::FerroEhrService;
use crate::service::status::{CallStatusType, SmError};
use crate::versioning::audit::OPENEHR;

use super::bundle;

/// The `service_api` identifier for the in-process openEHR terminology bundle.
///
/// NOTE: master03 §TERMINOLOGY's `service_api` examples are all
/// *external* servers (FHIR, Ocean, Better, Apelon) and the spec defines it as
/// "an identifier for the kind/flavour of terminology service" with an
/// implementation-defined value set — there is no standard identifier for a
/// local in-process bundle. We adopt the openEHR terminology's own id
/// `"openehr"`, and route the value set (`params_uri`) as a group / code-set
/// id within that terminology.
const BUNDLE_SERVICE_API: &str = "openehr";

/// Whether a `service_api` names a FHIR terminology service (any FHIR
/// version): the master03 examples are `hl7.org/fhir/4.0`, `/3.0`, `/1.0`,
/// `/r4`.
fn is_fhir_service_api(service_api: &str) -> bool {
    service_api.to_ascii_lowercase().starts_with("hl7.org/fhir")
}

#[async_trait]
impl TerminologyExpander for FerroEhrService {
    /// Resolve `TERMINOLOGY('expand', service_api, params_uri)` to the value
    /// set's codes: route by `service_api` (FHIR → the terminology server the
    /// value-set URL or the `service_api` routes to; `"openehr"` → the
    /// in-process bundle), fetch the expansion via the SM `get_value_set`, and
    /// return its code keys.
    ///
    /// # Errors
    ///
    /// A missing FHIR provider or an unrecognised `service_api` is a 400
    /// ([`AqlFeatureError::UnknownTerminologyService`]); an unknown value set
    /// is a 400 ([`AqlFeatureError::TerminologyValueSetNotFound`]); any other
    /// (server/transport) failure is a 500 ([`ExecError::Terminology`]).
    async fn expand(&self, service_api: &str, params_uri: &str) -> Result<Vec<String>, AqlError> {
        let extract = if is_fhir_service_api(service_api) {
            // Routed by the value set first (a deployment may point a specific
            // value-set URL at a specific server), then by the `service_api`
            // flavour, else the default provider.
            let provider = self
                .terminology_route(params_uri)
                .or_else(|| self.terminology_route(service_api))
                .or_else(|| self.terminology_default_provider())
                .ok_or_else(|| {
                    AqlFeatureError::UnknownTerminologyService(format!(
                        "{service_api} (no FHIR terminology server configured)"
                    ))
                })?;
            // The FHIR provider ignores `terminology_id` for `$expand`; the
            // value set is identified by `params_uri` (its URL).
            provider.get_value_set(service_api, params_uri).await
        } else if service_api == BUNDLE_SERVICE_API {
            // The bundle's value sets live under the `"openehr"` terminology;
            // `params_uri` is the group / code-set id.
            bundle::get_value_set(OPENEHR, params_uri)
        } else {
            return Err(AqlFeatureError::UnknownTerminologyService(service_api.to_owned()).into());
        };
        let extract = extract.map_err(|e| map_expand_error(e, service_api, params_uri))?;
        Ok(codes_of(extract))
    }

    /// Evaluates the Boolean `TERMINOLOGY()` operations (QUERY master03
    /// §TERMINOLOGY, third usage): `validate` calls `value_set_validate` with
    /// the `params_uri` arguments `url`, `system` and `code`, and `subsumes`
    /// calls `subsumes` with `system`, `codeA` and `codeB`. `lookup` and `map`
    /// return complex structures with no boolean semantics and are typed
    /// rejects.
    ///
    /// # Errors
    ///
    /// A missing required `params_uri` argument is a 400
    /// ([`AqlFeatureError::TerminologyParams`]); an operation without boolean
    /// semantics is a 400
    /// ([`AqlFeatureError::UnsupportedTerminologyOperation`]); otherwise as
    /// [`TerminologyExpander::expand`].
    async fn boolean_operation(
        &self,
        operation: &str,
        service_api: &str,
        params_uri: &str,
    ) -> Result<bool, AqlError> {
        let args = uri_query_args(params_uri);
        let arg = |name: &'static str| -> Result<&str, AqlError> {
            args.get(name)
                .map(String::as_str)
                .ok_or_else(|| AqlFeatureError::TerminologyParams(name).into())
        };
        let fhir = is_fhir_service_api(service_api);
        if !fhir && service_api != BUNDLE_SERVICE_API {
            return Err(AqlFeatureError::UnknownTerminologyService(service_api.to_owned()).into());
        }
        // The FHIR operations name their terminology in the `system` argument,
        // which is the routing key; the `service_api` flavour and the value-set
        // URL are the fallbacks.
        let fhir_provider = |primary: &str, secondary: &str| {
            self.terminology_route(primary)
                .or_else(|| self.terminology_route(secondary))
                .or_else(|| self.terminology_route(service_api))
                .or_else(|| self.terminology_default_provider())
                .ok_or_else(|| {
                    AqlFeatureError::UnknownTerminologyService(format!(
                        "{service_api} (no FHIR terminology server configured)"
                    ))
                })
        };
        let result = match operation.to_ascii_lowercase().as_str() {
            "validate" => {
                let (system, url, code) = (arg("system")?, arg("url")?, arg("code")?);
                if fhir {
                    fhir_provider(system, url)?
                        .value_set_validate(system, url, code, None)
                        .await
                } else {
                    bundle::value_set_validate(OPENEHR, url, code)
                }
            }
            "subsumes" => {
                let (system, code_a, code_b) = (arg("system")?, arg("codeA")?, arg("codeB")?);
                if fhir {
                    fhir_provider(system, system)?
                        .subsumes(system, code_a, code_b)
                        .await
                } else {
                    bundle::subsumes(system, code_a, code_b)
                }
            }
            other => {
                return Err(
                    AqlFeatureError::UnsupportedTerminologyOperation(other.to_owned()).into(),
                );
            }
        };
        result.map_err(|e| map_expand_error(e, service_api, params_uri))
    }

    /// Expand a terminology URI operand (`matches { terminology://… }` —
    /// QUERY master03 §matches/URI): the URI identifies a value set; matching
    /// is membership of its expansion. Routed to the in-process bundle for a
    /// `terminology://openehr/<set>` URI, else to the FHIR server the URI
    /// routes to (the URI is both the value-set identifier and the routing
    /// key).
    ///
    /// # Errors
    ///
    /// As [`TerminologyExpander::expand`]: no FHIR provider configured for a
    /// non-bundle URI is a 400; an unknown value set is a 400; any other
    /// provider failure is a 500.
    async fn expand_uri(&self, uri: &str) -> Result<Vec<String>, AqlError> {
        if let Some(rest) = uri
            .strip_prefix("terminology://openehr/")
            .or_else(|| uri.strip_prefix("terminology://openEHR/"))
        {
            let set = rest.split('?').next().unwrap_or(rest);
            let extract = bundle::get_value_set(OPENEHR, set)
                .map_err(|e| map_expand_error(e, "openehr", uri))?;
            return Ok(codes_of(extract));
        }
        let provider = self.terminology_provider(uri).ok_or_else(|| {
            AqlFeatureError::UnknownTerminologyService(format!(
                "{uri} (no FHIR terminology server configured for URI operands)"
            ))
        })?;
        let extract = provider
            .get_value_set("", uri)
            .await
            .map_err(|e| map_expand_error(e, "fhir", uri))?;
        Ok(codes_of(extract))
    }
}

/// The code keys of an extract's `_terms_` map (the expansion's membership
/// view), empty when the extract carries no terms.
fn codes_of(extract: super::types::TerminologyExtract) -> Vec<String> {
    extract
        .terms
        .map(|terms| terms.into_keys().collect())
        .unwrap_or_default()
}

/// The query args of a `params_uri` (`system=…&code=…&url=…`), from the part
/// after the first `?` when present, else the whole string. Values are
/// form-urlencoded (`+` → space) and percent-decoded (`urlencoding`, WHATWG
/// recovery semantics).
fn uri_query_args(params_uri: &str) -> BTreeMap<String, String> {
    let query = params_uri.split_once('?').map_or(params_uri, |(_, q)| q);
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| {
            let plus_decoded = v.replace('+', " ");
            (
                k.trim().to_owned(),
                String::from_utf8_lossy(&urlencoding::decode_binary(plus_decoded.as_bytes()))
                    .into_owned(),
            )
        })
        .collect()
}

/// Map a terminology-service [`SmError`] raised during expansion onto the AQL
/// error taxonomy: a "does not exist" status is a bad query (400 — the value
/// set is unknown); anything else is an upstream server fault (500).
fn map_expand_error(e: SmError, service_api: &str, params_uri: &str) -> AqlError {
    match e.status {
        CallStatusType::VersionedObjectDoesNotExist => {
            AqlFeatureError::TerminologyValueSetNotFound {
                service_api: service_api.to_owned(),
                value_set: params_uri.to_owned(),
            }
            .into()
        }
        _ => ExecError::Terminology(e.message).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fhir_service_api_recognised_case_insensitively() {
        assert!(is_fhir_service_api("hl7.org/fhir/4.0"));
        assert!(is_fhir_service_api("HL7.org/FHIR/r4"));
        assert!(!is_fhir_service_api("openehr"));
        assert!(!is_fhir_service_api("ocean"));
    }

    #[test]
    fn uri_query_args_decode_form_encoding() {
        let args = uri_query_args("fhir/ValueSet?system=http%3A%2F%2Fsnomed.info&code=a+b");
        assert_eq!(
            args.get("system").map(String::as_str),
            Some("http://snomed.info")
        );
        assert_eq!(args.get("code").map(String::as_str), Some("a b"));
        // No `?` → the whole string is the query.
        let bare = uri_query_args("system=s&codeA=x&codeB=y");
        assert_eq!(bare.get("codeA").map(String::as_str), Some("x"));
        // A literal `+` survives as %2B.
        let plus = uri_query_args("code=a%2Bb");
        assert_eq!(plus.get("code").map(String::as_str), Some("a+b"));
    }
}
