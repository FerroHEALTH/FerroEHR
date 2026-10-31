// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AQL `TERMINOLOGY()` seam: [`FerroEhrService`] implements the
//! [`TerminologyExpander`] trait (`crate::aql::terminology`) so the AQL
//! semantic-analysis pass (`aql/terminology.rs::expand_matches`) can resolve
//! `TERMINOLOGY('expand', …)` operands, the Boolean operations, and
//! `matches { terminology-uri }` through the in-process bundle or the one
//! terminology server (QUERY master03 §TERMINOLOGY, §matches).
//!
//! QUERY master03 §TERMINOLOGY defines `service_api` as "an identifier of a type
//! of terminology service. This is not the service endpoint as such", so a FHIR
//! operand of any FHIR flavour is answered by the configured server.

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
    /// set's codes: by `service_api` (FHIR → the terminology server; `"openehr"`
    /// → the in-process bundle), fetch the expansion via the SM `get_value_set`, and
    /// return its code keys.
    ///
    /// # Errors
    ///
    /// A missing terminology server or an unrecognised `service_api` is a 400
    /// ([`AqlFeatureError::UnknownTerminologyService`]); an unknown value set
    /// is a 400 ([`AqlFeatureError::TerminologyValueSetNotFound`]); any other
    /// (server/transport) failure is a 500 ([`ExecError::Terminology`]).
    async fn expand(&self, service_api: &str, params_uri: &str) -> Result<Vec<String>, AqlError> {
        let extract = if is_fhir_service_api(service_api) {
            let server = self.terminology_server().ok_or_else(|| {
                AqlFeatureError::UnknownTerminologyService(format!(
                    "{service_api} (no FHIR terminology server configured)"
                ))
            })?;
            // The server ignores `terminology_id` for `$expand`; the value set
            // is identified by its URL, which `params_uri` carries.
            let url = expand_value_set_url(params_uri)?;
            server.get_value_set(service_api, &url).await
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
        let args = params_uri_args(params_uri);
        let arg = |name: &'static str| -> Result<&str, AqlError> {
            args.get(name)
                .map(String::as_str)
                .ok_or_else(|| AqlFeatureError::TerminologyParams(name).into())
        };
        let fhir = is_fhir_service_api(service_api);
        if !fhir && service_api != BUNDLE_SERVICE_API {
            return Err(AqlFeatureError::UnknownTerminologyService(service_api.to_owned()).into());
        }
        let fhir_server = || {
            self.terminology_server().ok_or_else(|| {
                AqlFeatureError::UnknownTerminologyService(format!(
                    "{service_api} (no FHIR terminology server configured)"
                ))
            })
        };
        let result = match operation.to_ascii_lowercase().as_str() {
            "validate" => {
                let (system, url, code) = (arg("system")?, arg("url")?, arg("code")?);
                if fhir {
                    fhir_server()?
                        .value_set_validate(system, url, code, None)
                        .await
                } else {
                    bundle::value_set_validate(OPENEHR, url, code)
                }
            }
            "subsumes" => {
                let (system, code_a, code_b) = (arg("system")?, arg("codeA")?, arg("codeB")?);
                if fhir {
                    fhir_server()?.subsumes(system, code_a, code_b).await
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
    /// `terminology://openehr/<set>` URI, else to the terminology server (the
    /// URI is the value-set identifier).
    ///
    /// # Errors
    ///
    /// As [`TerminologyExpander::expand`]: no terminology server configured for
    /// a non-bundle URI is a 400; an unknown value set is a 400; any other
    /// server failure is a 500.
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
        let server = self.terminology_server().ok_or_else(|| {
            AqlFeatureError::UnknownTerminologyService(format!(
                "{uri} (no FHIR terminology server configured for URI operands)"
            ))
        })?;
        let extract = server
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

/// The value-set URL a FHIR `TERMINOLOGY('expand', …)` names. QUERY master03
/// §TERMINOLOGY gives `params_uri` as "URI path and query sections" and shows
/// two forms: the bare value-set URL (`http://snomed.info/sct?fhir_vs=…`), and
/// the `$expand` query whose `url` argument is that URL
/// (`url=http://snomed.info/sct?fhir_vs=…`, AQL examples §TERMINOLOGY).
///
/// NOTE: master03 §TERMINOLOGY; a value-set URL starts with a scheme, so a
/// `params_uri` that starts with a `name=` pair whose name holds no `:` or `/`
/// is the query form.
fn expand_value_set_url(params_uri: &str) -> Result<String, AqlError> {
    let query = params_uri.strip_prefix('?').unwrap_or(params_uri);
    let is_query_form = query
        .split_once('=')
        .is_some_and(|(name, _)| !name.is_empty() && !name.contains([':', '/', '?', '&']));
    if !is_query_form {
        return Ok(params_uri.to_owned());
    }
    params_uri_args(params_uri)
        .remove("url")
        .ok_or_else(|| AqlFeatureError::TerminologyParams("url").into())
}

/// The query arguments of a `params_uri` (`system=…&code=…&url=…`), which
/// QUERY master03 §TERMINOLOGY gives as "URI path and query sections". A path
/// ends at a `?` that comes before the first `=`; with no such path the whole
/// string is the query. Each argument runs from its name's `=` to the next
/// `&`, so a value-set URL keeps its own `?` and `=` (master03's `validate`
/// example passes `url=http://snomed.info/sct?fhir_vs`). Values are
/// form-urlencoded (`+` → space) and percent-decoded (`urlencoding`, WHATWG
/// recovery semantics).
fn params_uri_args(params_uri: &str) -> BTreeMap<String, String> {
    let query = match (params_uri.find('?'), params_uri.find('=')) {
        (Some(q), Some(eq)) if q < eq => params_uri.get(q + 1..).unwrap_or_default(),
        (Some(q), None) => params_uri.get(q + 1..).unwrap_or_default(),
        _ => params_uri,
    };
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
    fn expand_names_the_value_set_in_either_params_uri_form() {
        let url = |p: &str| expand_value_set_url(p).ok();
        // The bare value-set URL (master03 §TERMINOLOGY) passes through.
        assert_eq!(
            url("http://snomed.info/sct?fhir_vs=isa/50697003").as_deref(),
            Some("http://snomed.info/sct?fhir_vs=isa/50697003")
        );
        // The `$expand` query form (AQL examples §TERMINOLOGY) names it by `url`.
        assert_eq!(
            url("url=http://snomed.info/sct?fhir_vs=isa/50697003").as_deref(),
            Some("http://snomed.info/sct?fhir_vs=isa/50697003")
        );
        assert_eq!(
            url("url=http://hl7.org/fhir/ValueSet/administrative-gender").as_deref(),
            Some("http://hl7.org/fhir/ValueSet/administrative-gender")
        );
        assert_eq!(
            url("?count=10&url=http%3A%2F%2Fexample.test%2Fvs").as_deref(),
            Some("http://example.test/vs")
        );
        // A query form without `url` names no value set.
        assert!(matches!(
            expand_value_set_url("count=10"),
            Err(AqlError::Feature(AqlFeatureError::TerminologyParams("url")))
        ));
    }

    #[test]
    fn params_uri_args_decode_form_encoding() {
        let args = params_uri_args("fhir/ValueSet?system=http%3A%2F%2Fsnomed.info&code=a+b");
        assert_eq!(
            args.get("system").map(String::as_str),
            Some("http://snomed.info")
        );
        assert_eq!(args.get("code").map(String::as_str), Some("a b"));
        // No `?` → the whole string is the query.
        let bare = params_uri_args("system=s&codeA=x&codeB=y");
        assert_eq!(bare.get("codeA").map(String::as_str), Some("x"));
        // A literal `+` survives as %2B.
        let plus = params_uri_args("code=a%2Bb");
        assert_eq!(plus.get("code").map(String::as_str), Some("a+b"));
    }

    #[test]
    fn validate_params_uri_keeps_a_value_set_url_whole() {
        // QUERY master03 §TERMINOLOGY, the `validate` example.
        let args = params_uri_args(
            "system=http://snomed.info/sct&code=122298005\
             &url=http://snomed.info/sct?fhir_vs&display=Astrovirus RNA assay",
        );
        let arg = |name: &str| args.get(name).map(String::as_str);
        assert_eq!(arg("system"), Some("http://snomed.info/sct"));
        assert_eq!(arg("code"), Some("122298005"));
        assert_eq!(arg("url"), Some("http://snomed.info/sct?fhir_vs"));
        assert_eq!(arg("display"), Some("Astrovirus RNA assay"));
        // A value-set URL with its own query argument stays whole.
        let refset = params_uri_args(
            "system=http://snomed.info/sct&code=1\
             &url=http://snomed.info/sct?fhir_vs=refset/1200161000168100",
        );
        assert_eq!(
            refset.get("url").map(String::as_str),
            Some("http://snomed.info/sct?fhir_vs=refset/1200161000168100")
        );
        // A leading `?` is an empty path.
        let leading = params_uri_args("?url=http://snomed.info/sct?fhir_vs&code=1");
        assert_eq!(
            leading.get("url").map(String::as_str),
            Some("http://snomed.info/sct?fhir_vs")
        );
    }

    #[test]
    fn subsumes_params_uri_keeps_a_system_url_whole() {
        let args =
            params_uri_args("system=http://snomed.info/sct?fhir_vs&codeA=235856003&codeB=3738000");
        let arg = |name: &str| args.get(name).map(String::as_str);
        assert_eq!(arg("system"), Some("http://snomed.info/sct?fhir_vs"));
        assert_eq!(arg("codeA"), Some("235856003"));
        assert_eq!(arg("codeB"), Some("3738000"));
    }
}
