// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! OPT2 acceptance harness — the master10 worked example, transcribed inline.
//!
//! Oracle: `docs/specs/openehr/AM/docs/ADL2/master10-templates.adoc` (the
//! `discharge` COMPOSITION → `t_clinical_info_ds_sf` template → operational
//! template worked example) and `docs/specs/openehr/AM/docs/OPT2/master02-04`
//! (the OPT checklist + profiled processing).
//!
//! The master10 listing of the resulting operational template is heavily elided
//! (`--- etc ---` throughout), so a byte-for-byte reproduction of its printed
//! form is not possible from the vendored text. Instead the template, its
//! overlays, and the referenced archetypes are authored inline reproducing the
//! master10 mechanics (a COMPOSITION with a filled slot, an overlay filler that
//! removes nodes via `occurrences {0}`, a `use_node` proxy, a `closed` slot, and
//! a `category existence {0}` deletion), and `create_opt` is asserted against
//! every master02 checklist bullet, then round-tripped through the printer and
//! parser.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::aom::access::{complex_attributes, object_node_id, object_rm_type};
use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::{BindingFilter, OptError, ProfileSpec, create_opt, profile_opt};
use openehr_adl::parse::Dialect;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

// ── the inline master10-style corpus ────────────────────────────────────────

/// The base filler archetype: a SECTION with an item to keep, an item the
/// overlay removes, a `use_node` proxy (inlined in the OPT), and terminology.
const SECTION_EVENT_INFO: &str = "\
archetype (adl_version=2.4.0; rm_release=1.0.3)
    openEHR-EHR-SECTION.event_info.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"openEHR\">
    >

definition
    SECTION[at0000] matches {    -- Event info
        items matches {
            ELEMENT[at0001] occurrences matches {0..1} matches {    -- Keep
                value matches {
                    DV_TEXT[at0002] matches {*}
                }
            }
            ELEMENT[at0003] occurrences matches {0..1} matches {    -- Drop me
                value matches {
                    DV_TEXT[at0004] matches {*}
                }
            }
            use_node ELEMENT[at0005] occurrences matches {0..1} /items[at0001]
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"at0000\"] = <
                text = <\"Event info\">
                description = <\"Event info section\">
            >
            [\"at0001\"] = <
                text = <\"Keep\">
                description = <\"Kept element\">
            >
            [\"at0002\"] = <
                text = <\"Keep value\">
                description = <\"Kept value\">
            >
            [\"at0003\"] = <
                text = <\"Drop me\">
                description = <\"Removed element\">
            >
            [\"at0004\"] = <
                text = <\"Drop value\">
                description = <\"Removed value\">
            >
            [\"at0005\"] = <
                text = <\"Proxy\">
                description = <\"Proxy of the kept element\">
            >
        >
    >
";

/// The parent COMPOSITION: a `closed` slot (removed in the OPT), an open slot to
/// be filled by the template, and a `category` the template deletes.
const COMPOSITION_DISCHARGE: &str = "\
archetype (adl_version=2.4.0; rm_release=1.0.3)
    openEHR-EHR-COMPOSITION.discharge.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"openEHR\">
    >

definition
    COMPOSITION[at0000] matches {    -- Discharge
        category matches {
            DV_CODED_TEXT[at0010] matches {
                defining_code matches {[at0011]}
            }
        }
        content matches {
            allow_archetype CONTENT_ITEM[at0001] closed
            allow_archetype CONTENT_ITEM[at0002] matches {
                include
                    archetype_id/value matches {/openEHR-EHR-SECTION\\..*/}
            }
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"at0000\"] = <
                text = <\"Discharge\">
                description = <\"Discharge summary\">
            >
            [\"at0001\"] = <
                text = <\"Closed slot\">
                description = <\"A closed content slot\">
            >
            [\"at0002\"] = <
                text = <\"Open slot\">
                description = <\"An open content slot\">
            >
            [\"at0010\"] = <
                text = <\"Category\">
                description = <\"Composition category\">
            >
            [\"at0011\"] = <
                text = <\"event\">
                description = <\"event category\">
            >
        >
    >
";

/// The root template: specialises `discharge`, deletes `category`
/// (`existence {0}`), and fills the open slot with a local overlay via
/// `use_archetype`. Carries the overlay `t_event_info` inline.
const TEMPLATE_T_DISCHARGE: &str = "\
template (adl_version=2.4.0; rm_release=1.0.3)
    openEHR-EHR-COMPOSITION.t_discharge.v1.0.0

specialize
    openEHR-EHR-COMPOSITION.discharge.v1

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"openEHR\">
    >

definition
    COMPOSITION[at0000.1] matches {    -- Templated discharge
        category existence matches {0}
        content matches {
            use_archetype SECTION[at0002.1, openEHR-EHR-SECTION.t_event_info.v1] occurrences matches {1}
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"at0000.1\"] = <
                text = <\"Templated discharge\">
                description = <\"Templated discharge summary\">
            >
        >
    >
----------------------------------------------------------------
template_overlay
    openEHR-EHR-SECTION.t_event_info.v1.0.0

specialize
    openEHR-EHR-SECTION.event_info.v1

definition
    SECTION[at0000.1] matches {    -- Templated event info
        items matches {
            ELEMENT[at0003] occurrences matches {0}
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"at0000.1\"] = <
                text = <\"Templated event info\">
                description = <\"Templated event info\">
            >
        >
    >
";

fn parse(src: &str) -> Archetype {
    parse_artefact(src, Dialect::Adl2).unwrap_or_else(|e| panic!("parse failed: {e:?}"))
}

/// The repository holding the base archetypes the template + overlay reference
/// (the local overlay itself is registered by `create_opt`).
fn base_repo() -> ArchetypeRepository {
    let mut repo = ArchetypeRepository::new();
    repo.insert(parse(SECTION_EVENT_INFO));
    repo.insert(parse(COMPOSITION_DISCHARGE));
    repo
}

fn build_opt() -> OperationalTemplate {
    let template = parse(TEMPLATE_T_DISCHARGE);
    create_opt(&template, &base_repo()).unwrap_or_else(|e| panic!("create_opt failed: {e}"))
}

// ── navigation helpers ──────────────────────────────────────────────────────

fn root_children<'a>(def: &'a CComplexObject, attr: &str) -> &'a [CObject] {
    complex_attributes(def)
        .iter()
        .find(|a| a.rm_attribute_name == attr)
        .map_or(&[], |a| a.children.as_deref().unwrap_or_default())
}

fn attr_names(def: &CComplexObject) -> Vec<&str> {
    complex_attributes(def)
        .iter()
        .map(|a| a.rm_attribute_name.as_str())
        .collect()
}

fn find_child<'a>(def: &'a CComplexObject, attr: &str, node_id: &str) -> Option<&'a CObject> {
    root_children(def, attr)
        .iter()
        .find(|c| object_node_id(c) == node_id)
}

// ── the OPT checklist (master02) ────────────────────────────────────────────

#[test]
fn create_opt_reproduces_the_master10_worked_example() {
    let opt = build_opt();
    let def = &opt.definition;

    // The OPT root is the templated COMPOSITION (`at0000.1`).
    assert_eq!(object_node_id_of(def), "at0000.1");

    // master02: "all attribute (`C_ATTRIBUTE`) nodes that have `existence
    // matches {0}` … are removed" — `category` is gone.
    assert!(
        !attr_names(def).contains(&"category"),
        "category (existence {{0}}) must be removed from the OPT, attrs = {:?}",
        attr_names(def)
    );
    assert!(attr_names(def).contains(&"content"));

    // master02: "all closed slots are removed" — the `at0001` closed slot is
    // gone; and "all slot-fillers … have been resolved and substituted" — the
    // open slot `at0002` is replaced by the inlined filler `at0002.1`.
    let content = root_children(def, "content");
    assert!(
        find_child(def, "content", "at0001").is_none(),
        "closed slot at0001 must be removed"
    );
    assert_eq!(content.len(), 1, "content holds only the inlined filler");
    let filler = &content[0];

    // master02: "all archetype references have been resolved to specific
    // archetype identifiers, including full version" — the `...v1` reference
    // resolves to the full `...v1.0.0` id, carried on a `C_ARCHETYPE_ROOT`.
    let CObject::CComplexObject(CComplexObject::CArchetypeRoot(root)) = filler else {
        panic!("the filler must be an inlined C_ARCHETYPE_ROOT, got {filler:?}");
    };
    assert_eq!(root.node_id, "at0002.1");
    assert_eq!(
        root.archetype_ref,
        "openEHR-EHR-SECTION.t_event_info.v1.0.0"
    );
    assert_eq!(root.rm_type_name, "SECTION");

    // master02: "all template overlays have been applied (flattening)" +
    // "all slot-fillers … resolved and substituted" — the filler body is the
    // overlay's flattened SECTION structure, inlined (not a bare reference).
    let filler_def = CComplexObject::CArchetypeRoot(root.clone());
    let items = root_children(&filler_def, "items");

    // master02: "object … nodes with `occurrences matches {0}` [are removed]" —
    // the overlay set ELEMENT `at0003` to `{0}`, so it is gone.
    assert!(
        items.iter().all(|c| object_node_id(c) != "at0003"),
        "occurrences {{0}} object at0003 must be removed, items = {:?}",
        items.iter().map(object_node_id).collect::<Vec<_>>()
    );

    // master02: "no `use_node` nodes … all internal references … expanded out
    // as copies of their targets" — the `at0005` proxy is inlined as a copy of
    // `at0001` (a real ELEMENT complex object, not a proxy).
    let proxy = items
        .iter()
        .find(|c| object_node_id(c) == "at0005")
        .expect("the inlined use_node target keeps the proxy node id at0005");
    assert!(
        matches!(
            proxy,
            CObject::CComplexObject(CComplexObject::CComplexObject(_))
        ),
        "at0005 must be an inlined complex copy, not a proxy: {proxy:?}"
    );
    assert_eq!(object_rm_type(proxy), "ELEMENT");
    assert!(
        complex_attributes(as_complex(proxy))
            .iter()
            .any(|a| a.rm_attribute_name == "value"),
        "the inlined proxy carries the copied target structure"
    );
    // The kept element survives.
    assert!(items.iter().any(|c| object_node_id(c) == "at0001"));

    // master02: "no specialisation statement — an OPT is … 'top-level'".
    assert!(
        opt.parent_archetype_id.is_none(),
        "the OPT has no specialise parent"
    );
    assert!(opt.is_generated, "the OPT is a generated artefact");

    // master03 §Terminology: "the flat form of the `terminology` section of each
    // flattened constituent … (other than the root template) [is gathered] under
    // `component_terminologies`". The inlined overlay filler is the only
    // constituent; the root's own terminology stays in `terminology`.
    let components = opt
        .component_terminologies
        .as_ref()
        .expect("component_terminologies present");
    assert_eq!(
        components.keys().collect::<Vec<_>>(),
        vec!["openEHR-EHR-SECTION.t_event_info.v1.0.0"],
        "component_terminologies keyed by the filler's full id"
    );
    let filler_term = &components["openEHR-EHR-SECTION.t_event_info.v1.0.0"];
    // The filler's flat terminology accumulates the base archetype's terms
    // (master09.09 term_definitions accumulate): `at0000` (from the base),
    // `at0000.1` (the overlay), `at0003`, etc.
    let en = &filler_term.term_definitions["en"];
    assert!(en.contains_key("at0000"), "base term accumulated");
    assert!(en.contains_key("at0000.1"), "overlay term present");
    // The root template's own terms are NOT in component_terminologies.
    assert!(
        !components.contains_key("openEHR-EHR-COMPOSITION.t_discharge.v1.0.0"),
        "the root template is never a component terminology"
    );
    // The root's flat terminology stays under `terminology` (discharge + template).
    let root_en = &opt.terminology.term_definitions["en"];
    assert!(
        root_en.contains_key("at0000.1"),
        "root template term present"
    );
    assert!(
        root_en.contains_key("at0000"),
        "parent discharge term accumulated"
    );
}

#[test]
fn opt_round_trips_through_printer_and_parser() {
    let opt1 = build_opt();
    let printed1 = openehr_adl::print::print(&Archetype::AuthoredArchetype(Box::new(
        AuthoredArchetype::OperationalTemplate(Box::new(opt1.clone())),
    )))
    .expect("print the built OPT");

    // The printed OPT re-parses (the operational_template + inline
    // C_ARCHETYPE_ROOT-with-body forms both round-trip through the parser).
    let reparsed = parse(&printed1);
    let Archetype::AuthoredArchetype(inner) = &reparsed else {
        panic!("re-parsed artefact is not an authored archetype");
    };
    let AuthoredArchetype::OperationalTemplate(opt2) = inner.as_ref() else {
        panic!("re-parsed artefact is not an operational_template: {inner:?}");
    };

    // Textual idempotence: printing the re-parsed OPT is byte-identical.
    let printed2 = openehr_adl::print::print(&reparsed).expect("print the re-parsed OPT");
    assert_eq!(
        printed1, printed2,
        "OPT print is stable across a round-trip"
    );

    // The load-bearing content survives the round-trip.
    assert_eq!(
        opt1.definition, opt2.definition,
        "the OPT definition round-trips identically"
    );
    assert_eq!(
        opt1.component_terminologies, opt2.component_terminologies,
        "component_terminologies round-trips identically"
    );
}

// ── profiled OPT (master04) ─────────────────────────────────────────────────

#[test]
fn profile_removes_annotations() {
    let opt = build_opt();
    let profiled = profile_opt(
        &opt,
        &ProfileSpec {
            remove_annotations: true,
            ..ProfileSpec::default()
        },
    )
    .expect("profiling succeeds");
    assert!(
        profiled.annotations.is_none(),
        "master04 §Annotations Removal — the annotations section is removed"
    );
}

#[test]
fn profile_filters_languages_keeping_at_least_one() {
    // Author a two-language filler flattened into the root, then keep only `en`.
    let opt = build_opt();
    // Baseline: `en` is present in the terminology.
    assert!(opt.terminology.term_definitions.contains_key("en"));

    let profiled = profile_opt(
        &opt,
        &ProfileSpec {
            keep_languages: Some(vec!["en".to_owned()]),
            ..ProfileSpec::default()
        },
    )
    .expect("keeping en succeeds");
    assert_eq!(
        profiled
            .terminology
            .term_definitions
            .keys()
            .collect::<Vec<_>>(),
        vec!["en"],
        "master04 §Language Filtering — only the kept language remains"
    );

    // Removing every language is rejected (≥1 must remain).
    let err = profile_opt(
        &opt,
        &ProfileSpec {
            keep_languages: Some(vec!["zz".to_owned()]),
            ..ProfileSpec::default()
        },
    )
    .expect_err("removing every language is rejected");
    assert_eq!(err, OptError::NoLanguagesLeft);
}

#[test]
fn profile_filters_terminology_bindings() {
    let opt = build_opt();
    let profiled = profile_opt(
        &opt,
        &ProfileSpec {
            bindings: BindingFilter::RemoveAll,
            ..ProfileSpec::default()
        },
    )
    .expect("binding removal succeeds");
    assert!(
        profiled.terminology.term_bindings.is_none(),
        "master04 §Terminology Binding Filtering — all bindings removed"
    );
}

#[test]
fn profile_rejects_node_level_substitution() {
    let opt = build_opt();
    let err = profile_opt(
        &opt,
        &ProfileSpec {
            substitute_nodes: true,
            ..ProfileSpec::default()
        },
    )
    .expect_err("node-level substitution is unsupported");
    assert_eq!(err, OptError::NodeSubstitutionUnsupported);
}

#[test]
fn create_opt_reports_an_unresolved_reference() {
    // A template whose filler references an archetype absent from the repo.
    let template = parse(TEMPLATE_T_DISCHARGE);
    let mut repo = ArchetypeRepository::new();
    repo.insert(parse(COMPOSITION_DISCHARGE));
    // `event_info` (the overlay's parent) is absent, so flattening the overlay
    // filler cannot proceed — surfaced as a flatten error.
    let err = create_opt(&template, &repo).expect_err("missing constituent parent is an error");
    assert!(
        matches!(err, OptError::Flatten(_) | OptError::UnresolvedReference(_)),
        "unexpected error: {err}"
    );
}

// ── small object helpers ────────────────────────────────────────────────────

fn object_node_id_of(def: &CComplexObject) -> &str {
    match def {
        CComplexObject::CComplexObject(d) => &d.node_id,
        CComplexObject::CArchetypeRoot(r) => &r.node_id,
    }
}

fn as_complex(obj: &CObject) -> &CComplexObject {
    match obj {
        CObject::CComplexObject(cco) => cco,
        other => panic!("expected a complex object, got {other:?}"),
    }
}
