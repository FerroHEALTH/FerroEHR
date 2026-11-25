// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The unit tests of the party mapping and its selectors.

use super::*;

fn keyed(issuers: &[&str]) -> BindingMapping {
    let key = IdentifierSelector::new("/details[at0001]/items[at0002]", "guid")
        .expect("parses")
        .with_issuers(issuers.iter().copied());
    BindingMapping::default().with_person("Person", key)
}

#[test]
fn a_key_issuer_moves_no_digest_and_is_a_key_space() {
    let one = keyed(&["urn:a"]);
    let two = keyed(&["urn:a", "https://login"]);
    assert_eq!(one.digest(), two.digest(), "a key issuer is a key space");
    assert_eq!(
        two.key_spaces(),
        BTreeSet::from(["https://login".to_owned(), "urn:a".to_owned()])
    );
    assert_ne!(
        keyed(&[]).digest(),
        one.digest(),
        "a key selector holding every issuer derives other rows than one holding a list"
    );
    assert!(keyed(&[]).key_spaces().is_empty());
}

#[test]
fn a_selector_or_template_change_moves_the_digest() {
    let base = keyed(&["urn:a"]);
    assert_ne!(
        base.digest(),
        base.clone().with_professional_role("Role").digest()
    );
    let moved = IdentifierSelector::new("/details[at0001]/items[at0003]", "guid")
        .expect("parses")
        .with_issuers(["urn:a"]);
    assert_ne!(
        base.digest(),
        BindingMapping::default()
            .with_person("Person", moved)
            .digest()
    );
}

#[test]
fn the_policy_moves_no_digest() {
    assert_eq!(
        BindingMapping::default().with_policy().digest(),
        BindingMapping::default().digest(),
        "the policy derives no row, so it moves no digest"
    );
}

#[test]
fn a_path_parses_into_its_steps_and_reads_the_element_value() {
    let selector = IdentifierSelector::new(
        "/details[at0001]/items[openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1]/items[at0001]",
        "guid",
    )
    .expect("parses");
    let step = |attribute: &str, node_id: Option<&str>| Step {
        attribute: attribute.to_owned(),
        node_id: node_id.map(str::to_owned),
    };
    assert_eq!(
        selector.parsed,
        vec![
            step("details", Some("at0001")),
            step(
                "items",
                Some("openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1")
            ),
            step("items", Some("at0001")),
            step("value", None),
        ]
    );
}

#[test]
fn a_path_ending_at_the_value_reads_it_once() {
    let selector =
        IdentifierSelector::new("/details[at0001]/items[at0002]/value", "guid").expect("parses");
    assert_eq!(
        selector
            .parsed
            .iter()
            .filter(|step| step.attribute == "value")
            .count(),
        1
    );
}

#[test]
fn a_name_path_compiles_with_its_node_ids_as_variables() {
    let selector =
        NameSelector::new("/identities[at0001]/details[at0002]/items[at0003]").expect("compiles");
    assert_eq!(
        selector.json_path,
        "$.identities[*] ? (@.archetype_node_id == $p0).details[*] ? (@.archetype_node_id \
         == $p1).items[*] ? (@.archetype_node_id == $p2).value[*] ? (@._type == \"DV_TEXT\" \
         || @._type == \"DV_CODED_TEXT\")"
    );
    assert_eq!(
        selector.variables,
        r#"{"p0":"at0001","p1":"at0002","p2":"at0003"}"#
    );
}

#[test]
fn a_path_that_is_not_attribute_steps_is_refused() {
    for path in [
        "details[at0001]",
        "/details[at0001",
        "/Details[at0001]",
        "/details[at0001 and name/value='x']",
        "/details[at0001\"]",
        "//items",
        "/",
    ] {
        assert_eq!(
            IdentifierSelector::new(path, "guid"),
            Err(SelectorError::Path {
                path: path.to_owned()
            }),
            "{path}"
        );
    }
    assert!(matches!(
        IdentifierSelector::new("/details[at0001]", " "),
        Err(SelectorError::Type { .. })
    ));
}
