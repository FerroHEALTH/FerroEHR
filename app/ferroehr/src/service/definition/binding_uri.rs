// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The code system and code an ADL2 term-binding target names.
//!
//! ADL2 binding targets "are expressed as URIs that follow the model for
//! terminology URIs published by IHTSDO or a similar model, in the case of
//! terminologies other than SNOMED CT" (AM ADL2
//! `master07.13-adl_terminology.adoc` §Terminology bindings, whose example
//! binds `<http://loinc.org/id/48334-7>`). That model puts the concept
//! identifier last, behind an `/id/` segment. A FHIR terminology server is
//! asked about a `(system, code)` pair, so VETDF has to take the target apart
//! before it can ask: the outer binding key (`SNOMED-CT`, `Snomed`, `LOINC`,
//! spelled however the author spelled it) routes to a provider but is not a
//! code system URI.
//!
//! No openEHR specification maps a binding URI to a FHIR `system`; the SNOMED
//! CT URI Standard does for SNOMED CT (<https://snomed.org/uri>: a concept is
//! `http://snomed.info/id/{sctid}`, the code system is `http://snomed.info/sct`),
//! and for every other terminology the URI up to the `/id/` segment is offered
//! as the system, which is right for LOINC (`http://loinc.org`, the FHIR
//! registry's system) and leaves any other answer to the server: one it does
//! not serve is reported back as unverifiable, never as a refusal.

use url::Url;

/// The FHIR code system URI of SNOMED CT (SNOMED CT URI Standard §3).
pub(crate) const SNOMED_CT_SYSTEM: &str = "http://snomed.info/sct";

/// The `(system, code)` a binding target names, when it follows the
/// `<terminology>/id/<code>` model; `None` for a plain code or a URI of
/// another shape, which the caller passes through unchanged.
///
/// `http://snomed.info/id/{sctid}` (and the `snomedct.info` host the ADL2
/// spec's own examples use) is SNOMED CT; anything else yields the URI before
/// `/id/` as the system.
pub(crate) fn code_system_and_code(target: &str) -> Option<(String, String)> {
    let url = Url::parse(target).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let mut segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let code = segments.pop()?;
    if segments.pop()? != "id" || code.is_empty() {
        return None;
    }
    let host = url.host_str()?.trim_start_matches("www.");
    if matches!(host, "snomed.info" | "snomedct.info") {
        return Some((SNOMED_CT_SYSTEM.to_owned(), code.to_owned()));
    }
    let mut system = url.clone();
    system.set_query(None);
    system.set_fragment(None);
    let path = if segments.is_empty() {
        String::new()
    } else {
        format!("/{}", segments.join("/"))
    };
    system.set_path(&path);
    let mut system = system.to_string();
    if system.ends_with('/') {
        system.pop();
    }
    Some((system, code.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(system: &str, code: &str) -> (String, String) {
        (system.to_owned(), code.to_owned())
    }

    #[test]
    fn snomed_concept_uris_name_the_snomed_code_system() {
        assert_eq!(
            code_system_and_code("http://snomed.info/id/50121007"),
            Some(pair(SNOMED_CT_SYSTEM, "50121007"))
        );
        assert_eq!(
            code_system_and_code("http://snomedct.info/id/271649006"),
            Some(pair(SNOMED_CT_SYSTEM, "271649006"))
        );
        assert_eq!(
            code_system_and_code("https://www.snomed.info/id/404684003"),
            Some(pair(SNOMED_CT_SYSTEM, "404684003"))
        );
    }

    #[test]
    fn loinc_and_other_id_uris_yield_the_uri_before_id() {
        assert_eq!(
            code_system_and_code("http://loinc.org/id/LA6742-6"),
            Some(pair("http://loinc.org", "LA6742-6"))
        );
        assert_eq!(
            code_system_and_code("http://umls.nlm.edu/id/C124305"),
            Some(pair("http://umls.nlm.edu", "C124305"))
        );
        assert_eq!(
            code_system_and_code("http://cnf.example.test/fhir/CodeSystem/sct-shaped/id/1000001"),
            Some(pair(
                "http://cnf.example.test/fhir/CodeSystem/sct-shaped",
                "1000001"
            ))
        );
    }

    #[test]
    fn a_plain_code_or_another_uri_shape_is_left_to_the_caller() {
        assert_eq!(code_system_and_code("50121007"), None);
        assert_eq!(code_system_and_code("http://snomed.info/sct"), None);
        assert_eq!(code_system_and_code("http://example.org/terms/12345"), None);
        assert_eq!(code_system_and_code("urn:oid:2.16.840.1.113883.6.96"), None);
        assert_eq!(code_system_and_code("http://loinc.org/id/"), None);
    }
}
