// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ADL2 template identity, links, and REST paths — component-free.
//!
//! The viewer's Template Manager serves two families: the ADL 1.4
//! operational templates and the ADL2 ones, switched by `?family=` on
//! `/templates`. An ADL2 artefact is keyed by its AOM2 archetype HRID
//! (`openEHR-EHR-COMPOSITION.concept.v1.0.0`), whose trailing `.v{semver}`
//! carries the release version, and the ITS-REST Definition API exposes a
//! version segment beside it (`specifications/operations/
//! definition_template_adl2_version_get.yaml` under
//! `docs/specs/openehr/ITS-REST/`). Every derivation the two screens need
//! from those facts — the family stem, the release version, the versions one
//! listing holds for a family, the wire paths, the viewer hrefs — lives
//! here as pure functions with unit tests, so the views stay thin.
//!
//! NOTE: no openEHR spec governs the viewer's routes or family switch — our
//! own design / product extension; only the wire paths are spec-bound.

use crate::example_options::{ExampleDetail, ExampleType};

/// Which template family a Template Manager view is showing.
///
/// Serializable because the listing resource carries the family it fetched
/// alongside the rows: the screen then reads the family from the DATA rather
/// than from a signal inside the suspense, which is what keeps the rendered
/// rows and their family in step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum TemplateFamily {
    /// ADL 1.4 operational templates (`definition/template/adl1.4`) — the
    /// default when `?family=` is absent or unrecognized.
    #[default]
    Adl14,
    /// ADL2 operational templates (`definition/template/adl2`).
    Adl2,
}

impl TemplateFamily {
    /// The `?family=` value naming this family.
    #[must_use]
    pub fn as_query(self) -> &'static str {
        match self {
            Self::Adl14 => "adl14",
            Self::Adl2 => "adl2",
        }
    }

    /// The family switch's visible label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Adl14 => "ADL 1.4",
            Self::Adl2 => "ADL 2",
        }
    }

    /// The upload trigger's label — what the header button offers.
    #[must_use]
    pub fn upload_label(self) -> &'static str {
        match self {
            Self::Adl14 => "Upload OPT",
            Self::Adl2 => "Upload ADL2",
        }
    }

    /// The upload dialog's heading.
    #[must_use]
    pub fn upload_title(self) -> &'static str {
        match self {
            Self::Adl14 => "Upload an operational template",
            Self::Adl2 => "Upload an ADL2 operational template",
        }
    }

    /// The upload dialog's one-paragraph help: what the CDR ingests, and that
    /// a refusal is shown verbatim.
    #[must_use]
    pub fn upload_help(self) -> &'static str {
        match self {
            Self::Adl14 => {
                "The CDR ingests an ADL 1.4 operational template as OPT/XML. Choose a file or \
                 paste the source below; the CDR's validation diagnostics are shown verbatim on \
                 refusal."
            }
            Self::Adl2 => {
                "The CDR ingests the ADL2 artefact SOURCE as text/plain. Choose a file or paste \
                 the source below; the openEHR-ADL engine's diagnostics are shown verbatim on \
                 refusal."
            }
        }
    }

    /// The upload picker's `accept` list.
    #[must_use]
    pub fn upload_accept(self) -> &'static str {
        match self {
            Self::Adl14 => ".opt,.xml",
            Self::Adl2 => ".adls,.adl,.opt2,.txt",
        }
    }

    /// The paste area's placeholder: a recognizable first line of the format.
    #[must_use]
    pub fn upload_placeholder(self) -> &'static str {
        match self {
            Self::Adl14 => "<template xmlns=\"http://schemas.openehr.org/v1\"> …",
            Self::Adl2 => "operational_template (adl_version=2.0.6; rm_release=1.0.2; generated) …",
        }
    }

    /// The file picker's button label.
    #[must_use]
    pub fn upload_choose_label(self) -> &'static str {
        match self {
            Self::Adl14 => " Choose an OPT/XML file",
            Self::Adl2 => " Choose an ADL2 file",
        }
    }

    /// Read the family out of a `?family=` value.
    ///
    /// Anything other than `adl2` is the ADL 1.4 default: the parameter is
    /// user input, so a typo lands on the screen's normal view rather than on
    /// an error.
    #[must_use]
    pub fn from_query(value: &str) -> Self {
        if value.eq_ignore_ascii_case("adl2") {
            Self::Adl2
        } else {
            Self::Adl14
        }
    }

    /// The `/templates` href showing this family.
    ///
    /// Deliberately carries NOTHING else across: the two families are
    /// different listings, so a page index or window size from the other one
    /// would point at rows that are not there.
    #[must_use]
    pub fn href(self) -> String {
        match self {
            Self::Adl14 => "/templates".to_owned(),
            Self::Adl2 => "/templates?family=adl2".to_owned(),
        }
    }
}

/// Which pane of the ADL2 detail screen is showing (`?tab=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Adl2Tab {
    /// The stored ADL2 source, read as `text/plain` — the default.
    #[default]
    Source,
    /// The `OperationalTemplateV2` canonical JSON, read as `application/json`.
    Json,
    /// The Web-Template path catalog, built viewer-side from that JSON.
    Catalog,
    /// The CDR-generated example composition.
    Example,
}

impl Adl2Tab {
    /// The `?tab=` value naming this pane. Every spelling is a URL-safe
    /// lowercase word, so a href never has to encode it.
    #[must_use]
    pub fn as_query(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Json => "json",
            Self::Catalog => "catalog",
            Self::Example => "example",
        }
    }

    /// The tab's visible label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::Json => "AOM2 JSON",
            Self::Catalog => "Path catalog",
            Self::Example => "Example",
        }
    }

    /// Read the pane out of a `?tab=` value; anything unrecognized is the
    /// default source pane (the parameter is user input).
    #[must_use]
    pub fn from_query(value: &str) -> Self {
        match value {
            "json" => Self::Json,
            "catalog" => Self::Catalog,
            "example" => Self::Example,
            _ => Self::Source,
        }
    }
}

/// Split an archetype HRID into its family stem and its release version.
///
/// `openEHR-EHR-COMPOSITION.cnf_vitals.v1.0.0` splits into
/// `("openEHR-EHR-COMPOSITION.cnf_vitals", Some("1.0.0"))`. An id with no
/// `.v{digit}` tail yields the whole id and `None`.
#[must_use]
pub fn split_hrid(hrid: &str) -> (&str, Option<&str>) {
    // A release version carries no `v`, so the LAST `.v` followed by a digit
    // is the version marker even when a namespace prefix contains dots.
    // `.get(..)` throughout, never `&s[..]`: the id is CDR-supplied text and a
    // panicking slice on request-path data is denied.
    if let Some(at) = hrid.rfind(".v")
        && let Some(version) = hrid.get(at.saturating_add(2)..)
        && version.starts_with(|c: char| c.is_ascii_digit())
        && let Some(stem) = hrid.get(..at)
    {
        return (stem, Some(version));
    }
    (hrid, None)
}

/// The release versions one listing holds for the HRID family `stem`, ordered
/// oldest first and de-duplicated.
///
/// Ordering is by the numeric `major.minor.patch` triple, with the literal
/// string as the tie-break, so `1.10.0` sorts after `1.9.0`. A component that
/// is not a plain integer counts as `0` — the viewer orders what the CDR
/// listed, it does not validate SEMVER.
#[must_use]
pub fn family_versions(template_ids: &[String], stem: &str) -> Vec<String> {
    let mut versions = Vec::new();
    for id in template_ids {
        let (id_stem, version) = split_hrid(id);
        // NOTE: an id the CDR listed without a `.v{semver}` tail names no
        // version to offer — legitimately absent, not a defect (no openEHR
        // spec governs the viewer's version picker — our own design).
        if id_stem == stem
            && let Some(version) = version
        {
            versions.push(version.to_owned());
        }
    }
    versions.sort_by(|a, b| version_key(a).cmp(&version_key(b)).then_with(|| a.cmp(b)));
    versions.dedup();
    versions
}

/// The numeric ordering key of a release version, saturating at three
/// components and treating a non-integer component as `0`.
fn version_key(version: &str) -> (u32, u32, u32) {
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// The ITS-REST path of one stored ADL2 template, optionally pinned to
/// `version`.
///
/// `definition/template/adl2/{template_id}` — the plain get, which resolves a
/// partial id to its latest matching version — or
/// `definition/template/adl2/{template_id}/{version}`, the versioned get. The
/// version segment is "an exact version (e.g. `1.7.1`), or a pattern as partial
/// prefix, in a form of `{major}` or `{major}.{minor}` … in which case the
/// highest (latest) version matching the prefix will be considered"
/// (`docs/specs/openehr/ITS-REST/specifications/parameters/path/version.yaml`).
/// Both segments are percent-encoded with the `urlencoding` crate — an HRID is
/// CDR-supplied text.
#[must_use]
pub fn template_path(template_id: &str, version: Option<&str>) -> String {
    let id = urlencoding::encode(template_id);
    match version.map(str::trim).filter(|value| !value.is_empty()) {
        Some(version) => format!(
            "definition/template/adl2/{id}/{}",
            urlencoding::encode(version)
        ),
        None => format!("definition/template/adl2/{id}"),
    }
}

/// The ITS-REST path of one stored ADL2 ARTEFACT
/// (`definition/artefact/adl2/{artefact_id}`).
///
/// The artefact resource addresses the whole AOM2 store — archetype, template
/// or OPT — by the same HRID the template routes use, and it is the only route
/// that DELETES one. NOTE: the released ITS-REST Definition API declares no
/// artefact route at all, so this is the CDR's own extension realizing SM
/// `I_DEFINITION_ADL2.delete_artefact`
/// (`docs/specs/openehr/SM/docs/UML/classes/i_definition_adl2.adoc`), whose
/// `artefact_does_not_exist` error is the `404`.
///
/// The id is percent-encoded with the `urlencoding` crate — an HRID is
/// CDR-supplied text.
#[must_use]
pub fn artefact_path(artefact_id: &str) -> String {
    format!(
        "definition/artefact/adl2/{}",
        urlencoding::encode(artefact_id)
    )
}

/// The ITS-REST path of the CDR-generated example composition for one stored
/// ADL2 template, at `detail` and `kind`.
///
/// The Definition API declares no versioned example resource, so the artefact
/// the example is generated from is the one `template_id` itself resolves to.
/// The two example options ride the query string
/// ([`crate::example_options::example_query`]); the id is percent-encoded with
/// the `urlencoding` crate.
#[must_use]
pub fn example_path(template_id: &str, detail: ExampleDetail, kind: ExampleType) -> String {
    format!(
        "definition/template/adl2/{}/example{}",
        urlencoding::encode(template_id),
        crate::example_options::example_query(detail, kind)
    )
}

/// The `/templates/adl2/{template_id}` detail href for one list row.
///
/// The id is percent-encoded because it is CDR-supplied text: a reserved
/// character would otherwise split the path segment. `leptos_router`
/// percent-decodes route params on both targets, so the read side needs no
/// decode.
#[must_use]
pub fn detail_href(template_id: &str) -> String {
    format!("/templates/adl2/{}", urlencoding::encode(template_id))
}

/// The ADL2 detail screen's self-link with `tab` selected and `version`
/// pinned (`None` = the artefact the route names).
#[must_use]
pub fn view_href(template_id: &str, tab: Adl2Tab, version: Option<&str>) -> String {
    let mut href = format!(
        "/templates/adl2/{}?tab={}",
        urlencoding::encode(template_id),
        tab.as_query()
    );
    if let Some(version) = version.map(str::trim).filter(|value| !value.is_empty()) {
        href.push_str("&version=");
        href.push_str(&urlencoding::encode(version));
    }
    href
}

#[cfg(test)]
mod tests {
    use crate::adl2::{
        Adl2Tab, TemplateFamily, artefact_path, detail_href, example_path, family_versions,
        split_hrid, template_path, view_href,
    };
    use crate::example_options::{ExampleDetail, ExampleType};

    #[test]
    fn the_family_switch_reads_and_writes_its_query_value() {
        assert_eq!(TemplateFamily::from_query("adl2"), TemplateFamily::Adl2);
        assert_eq!(TemplateFamily::from_query("ADL2"), TemplateFamily::Adl2);
        // Absent, empty, or unrecognized all land on the default listing.
        assert_eq!(TemplateFamily::from_query(""), TemplateFamily::Adl14);
        assert_eq!(TemplateFamily::from_query("adl14"), TemplateFamily::Adl14);
        assert_eq!(TemplateFamily::from_query("adl-2"), TemplateFamily::Adl14);
        assert_eq!(TemplateFamily::default(), TemplateFamily::Adl14);
        assert_eq!(TemplateFamily::Adl14.href(), "/templates");
        assert_eq!(TemplateFamily::Adl2.href(), "/templates?family=adl2");
        assert_eq!(TemplateFamily::Adl2.as_query(), "adl2");
    }

    #[test]
    fn the_detail_tabs_round_trip_their_query_value() {
        for tab in [
            Adl2Tab::Source,
            Adl2Tab::Json,
            Adl2Tab::Catalog,
            Adl2Tab::Example,
        ] {
            assert_eq!(Adl2Tab::from_query(tab.as_query()), tab);
        }
        assert_eq!(Adl2Tab::from_query("nonsense"), Adl2Tab::Source);
        assert_eq!(Adl2Tab::default(), Adl2Tab::Source);
    }

    #[test]
    fn an_hrid_splits_into_its_family_stem_and_release_version() {
        assert_eq!(
            split_hrid("openEHR-EHR-COMPOSITION.cnf_adl2_versioned.v1.0.0"),
            ("openEHR-EHR-COMPOSITION.cnf_adl2_versioned", Some("1.0.0"))
        );
        // A namespaced id keeps every dot before the version marker.
        assert_eq!(
            split_hrid("org.example::openEHR-EHR-COMPOSITION.vitals.v2.1.3"),
            ("org.example::openEHR-EHR-COMPOSITION.vitals", Some("2.1.3"))
        );
        // No `.v{digit}` tail: the whole id is the stem.
        assert_eq!(
            split_hrid("openEHR-EHR-COMPOSITION.vitals"),
            ("openEHR-EHR-COMPOSITION.vitals", None)
        );
        // `.v` NOT followed by a digit is part of the concept, not a version.
        assert_eq!(
            split_hrid("openEHR-EHR-COMPOSITION.vitals.version"),
            ("openEHR-EHR-COMPOSITION.vitals.version", None)
        );
        assert_eq!(split_hrid(""), ("", None));
    }

    #[test]
    fn family_versions_orders_numerically_and_ignores_other_families() {
        let ids = vec![
            "openEHR-EHR-COMPOSITION.fam.v1.10.0".to_owned(),
            "openEHR-EHR-COMPOSITION.fam.v1.9.0".to_owned(),
            "openEHR-EHR-COMPOSITION.fam.v1.0.0".to_owned(),
            "openEHR-EHR-COMPOSITION.other.v3.0.0".to_owned(),
            "openEHR-EHR-COMPOSITION.fam".to_owned(),
        ];
        assert_eq!(
            family_versions(&ids, "openEHR-EHR-COMPOSITION.fam"),
            vec!["1.0.0", "1.9.0", "1.10.0"]
        );
        // A family nothing in the listing belongs to yields nothing.
        assert!(family_versions(&ids, "openEHR-EHR-COMPOSITION.absent").is_empty());
    }

    #[test]
    fn family_versions_deduplicates_repeated_versions() {
        let ids = vec![
            "openEHR-EHR-COMPOSITION.fam.v1.0.0".to_owned(),
            "openEHR-EHR-COMPOSITION.fam.v1.0.0".to_owned(),
        ];
        assert_eq!(
            family_versions(&ids, "openEHR-EHR-COMPOSITION.fam"),
            vec!["1.0.0"]
        );
    }

    #[test]
    fn the_wire_paths_encode_every_supplied_segment() {
        assert_eq!(
            template_path("openEHR-EHR-COMPOSITION.cnf.v1.0.0", None),
            "definition/template/adl2/openEHR-EHR-COMPOSITION.cnf.v1.0.0"
        );
        assert_eq!(
            template_path("openEHR-EHR-COMPOSITION.cnf", Some("1.1.0")),
            "definition/template/adl2/openEHR-EHR-COMPOSITION.cnf/1.1.0"
        );
        // A blank version is "as stored", not an empty path segment.
        assert_eq!(
            template_path("id", Some("  ")),
            "definition/template/adl2/id"
        );
        // A slash in either segment would otherwise route elsewhere.
        assert_eq!(
            template_path("a/b", Some("1/2")),
            "definition/template/adl2/a%2Fb/1%2F2"
        );
        assert_eq!(
            example_path("a b", ExampleDetail::default(), ExampleType::default()),
            "definition/template/adl2/a%20b/example?detail_level=required&type=input"
        );
    }

    #[test]
    fn the_example_path_carries_the_selected_options() {
        // Both parameters ride every request, so the CDR generates exactly
        // what the pane's controls show.
        assert_eq!(
            example_path(
                "openEHR-EHR-COMPOSITION.cnf.v1.0.0",
                ExampleDetail::Complete,
                ExampleType::Output
            ),
            "definition/template/adl2/openEHR-EHR-COMPOSITION.cnf.v1.0.0/example?detail_level=complete&type=output"
        );
        assert_eq!(
            example_path("id", ExampleDetail::Medium, ExampleType::Input),
            "definition/template/adl2/id/example?detail_level=medium&type=input"
        );
    }

    #[test]
    fn the_artefact_path_addresses_the_aom2_store_by_hrid() {
        assert_eq!(
            artefact_path("openEHR-EHR-COMPOSITION.cnf_adl2_versioned.v1.0.0"),
            "definition/artefact/adl2/openEHR-EHR-COMPOSITION.cnf_adl2_versioned.v1.0.0"
        );
        // A slash would otherwise address a different resource entirely.
        assert_eq!(artefact_path("a/b"), "definition/artefact/adl2/a%2Fb");
        assert_eq!(artefact_path("a b#c"), "definition/artefact/adl2/a%20b%23c");
    }

    #[test]
    fn the_viewer_hrefs_encode_the_template_id() {
        assert_eq!(
            detail_href("openEHR-EHR-COMPOSITION.cnf.v1.0.0"),
            "/templates/adl2/openEHR-EHR-COMPOSITION.cnf.v1.0.0"
        );
        assert_eq!(detail_href("a/b#c"), "/templates/adl2/a%2Fb%23c");
        assert_eq!(
            view_href("id", Adl2Tab::Json, None),
            "/templates/adl2/id?tab=json"
        );
        assert_eq!(
            view_href("id", Adl2Tab::Source, Some("1.1.0")),
            "/templates/adl2/id?tab=source&version=1.1.0"
        );
        // A blank version is dropped rather than sent as `version=`.
        assert_eq!(
            view_href("id", Adl2Tab::Example, Some(" ")),
            "/templates/adl2/id?tab=example"
        );
    }

    #[test]
    fn a_detail_href_round_trips_through_the_router_unescape() {
        // `ParamsMap::insert` percent-decodes every route param on both
        // targets, so the segment this builder emits must decode back.
        for id in [
            "openEHR-EHR-COMPOSITION.cnf_adl2_versioned.v1.0.0",
            "a/b",
            "a#b",
            "a%2Fb",
            "temperatur-°C",
        ] {
            let href = detail_href(id);
            let segment = href
                .strip_prefix("/templates/adl2/")
                .expect("the builder always emits /templates/adl2/<segment>");
            assert_eq!(
                urlencoding::decode(segment).expect("valid UTF-8 percent-encoding"),
                id
            );
        }
    }
}
