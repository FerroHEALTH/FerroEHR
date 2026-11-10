// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The category map: the declared tables and their refusals, the digest, the
//! classification and the shipped map.

use super::*;

const LAB: &str = "http://hl7.eu/fhir/health-data-api/CodeSystem/eehrxf-document-priority-category-cs|\
     Laboratory-Reports";
const SUMMARY: &str = "http://hl7.eu/fhir/health-data-api/CodeSystem/eehrxf-document-priority-category-cs|\
     Patient-Summaries";
const IMAGING: &str = "http://hl7.eu/fhir/health-data-api/CodeSystem/eehrxf-document-priority-category-cs|\
     Medical-Imaging";
const BEELD: &str = "urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld";

fn map(toml_text: &str) -> Classifier {
    let config: CategoryMapConfig = toml::from_str(toml_text).expect("parse");
    config.compile().expect("compile")
}

fn object(template: Option<&str>, archetype: Option<&str>) -> ContentIds {
    ContentIds {
        template_id: template.map(str::to_owned),
        archetype_id: archetype.map(str::to_owned),
        resource_kind: None,
    }
}

const MAP: &str = r#"
    national = ["urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld"]
    [templates]
    "Lab Report" = ["Laboratory-Reports"]
    "IPS" = ["Patient-Summaries"]
    "Admin Note" = "none"
    [archetypes]
    "openEHR-EHR-COMPOSITION.imaging_report.v1" = ["Medical-Imaging", "urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld"]
"#;

#[test]
fn the_config_round_trips_through_its_serialized_form() {
    let config: CategoryMapConfig = toml::from_str(MAP).expect("parse");
    let text = toml::to_string(&config).expect("serialize");
    let again: CategoryMapConfig = toml::from_str(&text).expect("parse again");
    assert_eq!(config, again);
}

#[test]
fn the_boot_validation_names_every_defect() {
    let config: CategoryMapConfig = toml::from_str(
        r#"
        [templates]
        "a" = []
        "b" = ["none", "Medical-Imaging"]
        "C" = ["Medical-Imaging"]
        "c" = ["Medical-Imaging"]
        [archetypes]
        "not-an-archetype" = ["Medical-Imaging"]
        "#,
    )
    .expect("parse");
    let errors = config.errors();
    assert_eq!(errors.len(), 4, "{errors:#?}");
    assert!(errors.iter().any(|e| e.contains("maps to no category")));
    assert!(errors.iter().any(|e| e.contains("\"none\"")));
    assert!(errors.iter().any(|e| e.contains("differing only in case")));
    assert!(errors.iter().any(|e| e.contains("is not an archetype id")));
    let unknown: CategoryMapConfig =
        toml::from_str("[templates]\n\"a\" = [\"lab\"]").expect("parse");
    assert!(
        unknown
            .errors()
            .iter()
            .any(|e| e.contains("neither a priority category nor a declared national one")),
        "an unknown category is refused"
    );
}

#[test]
fn the_digest_is_stable_over_case_and_order_and_moves_with_the_content() {
    let a = map(MAP);
    let b = map(r#"
        national = ["urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld"]
        [archetypes]
        "openEHR-EHR-COMPOSITION.IMAGING_REPORT.v1" = ["urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld", "Medical-Imaging"]
        [templates]
        "admin note" = "none"
        "ips" = ["Patient-Summaries"]
        "lab report" = ["Laboratory-Reports"]
        "#);
    assert_eq!(a.digest(), b.digest());
    assert_ne!(a.digest(), Classifier::default().digest());
    assert!(a.digest().to_string().starts_with("sha256:"));
}

#[test]
fn objects_are_classified_template_first_then_root_archetype() {
    let m = map(MAP);
    let by_template = m.classify(
        &AccessedContent::Objects(vec![object(
            Some("lab report"),
            Some("openEHR-EHR-COMPOSITION.imaging_report.v1"),
        )]),
        false,
    );
    assert_eq!(by_template.stored_categories(), [LAB]);
    assert_eq!(by_template.read_from, ReadFrom::Returned);

    let by_archetype = m.classify(
        &AccessedContent::Objects(vec![object(
            Some("unmapped"),
            Some("openehr-ehr-composition.imaging_report.v1"),
        )]),
        true,
    );
    assert_eq!(by_archetype.stored_categories(), [IMAGING, BEELD]);
    assert_eq!(by_archetype.read_from, ReadFrom::Written);
}

#[test]
fn an_unmapped_object_is_unclassified_with_its_ids_as_evidence() {
    let m = map(MAP);
    let record = m.classify(
        &AccessedContent::Objects(vec![
            object(Some("IPS"), None),
            object(Some("Vitals"), Some("openEHR-EHR-COMPOSITION.encounter.v1")),
        ]),
        false,
    );
    assert_eq!(record.stored_categories(), [SUMMARY, UNCLASSIFIED]);
    // NOTE: openEHR BASE 1.1.0, Composite Identifiers and Case: ids keep their case.
    assert_eq!(
        record.stored_evidence(),
        ["IPS", "Vitals", "openEHR-EHR-COMPOSITION.encounter.v1"]
    );
}

#[test]
fn none_gives_way_to_a_real_category_and_kinds_classify_as_none() {
    let m = map(MAP);
    let mixed = m.classify(
        &AccessedContent::Objects(vec![
            object(Some("Admin Note"), None),
            object(Some("Lab Report"), None),
            ContentIds::of_kind("EHR_STATUS", None, None),
        ]),
        false,
    );
    assert_eq!(mixed.stored_categories(), [LAB]);
    let kind = m.classify(&AccessedContent::ResourceKind(ResourceKind::Ehr), false);
    assert_eq!(kind.stored_categories(), [NONE]);
    assert_eq!(kind.read_from, ReadFrom::ResourceKind);
    assert_eq!(kind.stored_evidence(), ["EHR"]);
}

/// Every queried id must map: an unmapped one leaves the access
/// unclassified beside what did map (Annex II 3.2(c) records the
/// categories of all the data accessed).
#[test]
fn a_query_without_objects_reads_its_positive_constraints() {
    let m = map(MAP);
    let constraints = QueryConstraints {
        template_ids: BTreeSet::new(),
        archetype_ids: BTreeSet::from([
            "openEHR-EHR-COMPOSITION.imaging_report.v1".to_owned(),
            "openEHR-EHR-OBSERVATION.unmapped.v1".to_owned(),
        ]),
        reaches_content: true,
        every_root_bound: true,
        resource_kinds: BTreeSet::new(),
    };
    let record = m.classify(&AccessedContent::Query(constraints), false);
    assert_eq!(record.stored_categories(), [IMAGING, UNCLASSIFIED, BEELD]);
    assert_eq!(record.read_from, ReadFrom::Queried);

    let mapped = m.classify(
        &AccessedContent::Query(QueryConstraints {
            archetype_ids: BTreeSet::from(["openEHR-EHR-COMPOSITION.imaging_report.v1".to_owned()]),
            reaches_content: true,
            every_root_bound: true,
            ..QueryConstraints::default()
        }),
        false,
    );
    assert_eq!(mapped.stored_categories(), [IMAGING, BEELD]);

    let none_mapped = m.classify(
        &AccessedContent::Query(QueryConstraints {
            template_ids: BTreeSet::from(["Vitals".to_owned()]),
            reaches_content: true,
            every_root_bound: true,
            ..QueryConstraints::default()
        }),
        false,
    );
    assert_eq!(none_mapped.stored_categories(), [UNCLASSIFIED]);
    assert_eq!(none_mapped.stored_evidence(), ["Vitals"]);

    let ehr_only = m.classify(
        &AccessedContent::Query(QueryConstraints {
            resource_kinds: BTreeSet::from([ResourceKind::Ehr]),
            ..QueryConstraints::default()
        }),
        false,
    );
    assert_eq!(ehr_only.stored_categories(), [NONE]);
    assert_eq!(ehr_only.read_from, ReadFrom::ResourceKind);
}

/// The shipped map applies while no key is declared, a national category
/// alone keeps it, any declared key replaces it whole, and turning it off
/// with nothing declared leaves no map (EHDS Annex II 3.2(c)).
#[test]
fn the_shipped_map_compiles_and_classifies() {
    let shipped = CategoryMapConfig::default();
    assert_eq!(shipped.source(), MapSource::Shipped);
    let classifier = shipped.compile().expect("the shipped map compiles");
    assert!(!classifier.is_empty());
    let ips = classifier.classify(
        &AccessedContent::Objects(vec![object(
            Some("International Patient Summary"),
            Some("openEHR-EHR-COMPOSITION.health_summary.v1"),
        )]),
        false,
    );
    assert_eq!(ips.stored_categories(), [SUMMARY]);
    let discharge = classifier.classify(
        &AccessedContent::Objects(vec![object(
            Some("a local discharge template"),
            Some("openEHR-EHR-COMPOSITION.transfer_summary.v1"),
        )]),
        true,
    );
    assert_eq!(
        discharge.stored_categories(),
        [
            "http://hl7.eu/fhir/health-data-api/CodeSystem/eehrxf-document-priority-category-cs|\
          Discharge-Reports"
        ]
    );
    let generic = classifier.classify(
        &AccessedContent::Objects(vec![object(
            Some("a local result template"),
            Some("openEHR-EHR-COMPOSITION.report-result.v1"),
        )]),
        false,
    );
    assert_eq!(
        generic.stored_categories(),
        [UNCLASSIFIED],
        "a container shared by several categories is never guessed"
    );

    let national: CategoryMapConfig =
        toml::from_str(r#"national = ["urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld"]"#)
            .expect("parse");
    assert_eq!(national.source(), MapSource::Shipped);
    assert_eq!(
        national.compile().expect("compile").rows(),
        classifier.rows()
    );

    let declared = map(MAP);
    assert_eq!(
        declared
            .classify(
                &AccessedContent::Objects(vec![object(
                    Some("unmapped"),
                    Some("openEHR-EHR-COMPOSITION.health_summary.v1"),
                )]),
                false,
            )
            .stored_categories(),
        [UNCLASSIFIED],
        "a declared map replaces the shipped one whole"
    );

    let off: CategoryMapConfig = toml::from_str("shipped_map = false").expect("parse");
    assert_eq!(off.source(), MapSource::None);
    assert!(off.compile().expect("compile").is_empty());
}

#[test]
fn the_mirror_rows_carry_tokens_and_none() {
    let classifier = map(MAP);
    let rows = classifier.rows();
    assert!(rows.contains(&("template", "admin note".to_owned(), NONE.to_owned())));
    assert!(rows.contains(&("template", "lab report".to_owned(), LAB.to_owned())));
    assert!(rows.contains(&(
        "archetype",
        "openehr-ehr-composition.imaging_report.v1".to_owned(),
        BEELD.to_owned()
    )));
}
