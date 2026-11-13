// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Wabvpz Art. 15f check of a request under an NL agreement
//! (`docs/law/nl/wabvpz/text.html`): an insurer has no access to an exchange
//! system (paragraph 1), and neither has a care provider acting as an
//! occupational or insurance physician (paragraph 2).
//!
//! The codes come from the lists `scripts/vendor/nl-role-codes.sh` fetches:
//! the UZI role codes of the `uzi-rolcode` `CodeSystem`, whose source is
//! Bijlage 3 of the UZI-register CPS v11.7 and which, like the CPS, is cache
//! only (`.vendor-cache/nl-role-codes/`), and the care provider types of the
//! `OrganisatieTypeCodelijst` `ValueSet` in `docs/specs/nl-role-codes/`. The
//! URA carries no organisation type and no published list types an insurer,
//! so an organisation is admitted only when it asserts a care provider type
//! of that list. The check runs under an agreement whose `jurisdiction` is
//! `NL` (`ferroehr::party_binding::grants::agreement`). Which codes the
//! article covers is our own reading of it.

use ferrohealth_smart::federation::{Attribute, External, Field};

/// The UZI role codes of paragraph 2: 01.008 Bedrijfsarts and 01.048
/// Verzekeringsarts.
const REFUSED_ROLES: [&str; 2] = ["01.008", "01.048"];

/// The care provider types of the `OrganisatieTypeCodelijst` `ValueSet`
/// (2.16.840.1.113883.2.4.3.11.60.40.2.17.2.3, 2020-09-01).
const CARE_PROVIDER_TYPES: [&str; 29] = [
    "A1", "B2", "BRA", "DIA", "G3", "G5", "G6", "H1", "H2", "J8", "JGZ", "K3", "K4", "K9", "L1",
    "M1", "N6", "P4", "P6", "R5", "R8", "T2", "V4", "V5", "V6", "X3", "Z3", "Z4", "Z5",
];

/// The code systems a coded organisation type may name: the
/// `organization-type` `CodeSystem` by its canonical URL and by its OID.
const ORGANISATION_TYPE_SYSTEMS: [&str; 2] = [
    "http://nictiz.nl/fhir/NamingSystem/organization-type",
    "urn:oid:2.16.840.1.113883.2.4.15.1060",
];

/// Why Art. 15f refuses the request `external` asserts, as the binding code
/// and the message the caller reads; `None` admits it.
pub(super) fn refusal(external: Option<&External>) -> Option<(&'static str, &'static str)> {
    if external
        .and_then(|external| external.field(Field::NlRole))
        .is_some_and(refused_role)
    {
        return Some((
            "refused-wabvpz-role",
            "an occupational or insurance physician has no access under an NL agreement \
             (Wabvpz Art. 15f(2))",
        ));
    }
    let care_provider = external
        .and_then(|external| external.field(Field::NlOrganisationType))
        .is_some_and(care_provider_type);
    if !care_provider {
        return Some((
            "refused-wabvpz-organisation-type",
            "an NL agreement admits only an organisation asserting a care provider type \
             (Wabvpz Art. 15f(1))",
        ));
    }
    None
}

/// Whether a role `asserted` carries is one of [`REFUSED_ROLES`].
fn refused_role(asserted: &Attribute) -> bool {
    asserted
        .values()
        .iter()
        .any(|code| REFUSED_ROLES.contains(code))
}

/// Whether every organisation type `asserted` carries is a care provider type,
/// each coded one in the `organization-type` system, and it carries one.
fn care_provider_type(asserted: &Attribute) -> bool {
    match asserted {
        Attribute::Text(code) => CARE_PROVIDER_TYPES.contains(&code.as_str()),
        Attribute::Codes(codes) => {
            !codes.is_empty()
                && codes.iter().all(|code| {
                    CARE_PROVIDER_TYPES.contains(&code.code.as_str())
                        && code
                            .system
                            .as_deref()
                            .is_none_or(|system| ORGANISATION_TYPE_SYSTEMS.contains(&system))
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use ferrohealth_smart::federation::Code;
    use serde_json::Value;

    use super::*;

    /// The cache-only `uzi-rolcode` `CodeSystem`, under the workspace root.
    const ROLE_CODES: &str =
        ".vendor-cache/nl-role-codes/CodeSystem-RoleCodeNLZorgverlenertypen-uzi-rolcode.json";
    const ORGANISATION_TYPES: &str = include_str!(
        "../../../../../../docs/specs/nl-role-codes/ValueSet-OrganisatieTypeCodelijst-2.16.840.1.113883.2.4.3.11.60.40.2.17.2.3--20200901000000.json"
    );

    fn text(value: &str) -> Attribute {
        Attribute::Text(value.to_owned())
    }

    fn coded(codes: &[(Option<&str>, &str)]) -> Attribute {
        Attribute::Codes(
            codes
                .iter()
                .map(|(system, code)| Code {
                    system: system.map(str::to_owned),
                    code: (*code).to_owned(),
                })
                .collect(),
        )
    }

    #[test]
    fn the_refused_roles_are_the_vendored_bedrijfsarts_and_verzekeringsarts() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(ROLE_CODES);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "read {}: {e}; run scripts/vendor/nl-role-codes.sh --test-inputs",
                path.display()
            )
        });
        let system: Value = serde_json::from_str(&text).expect("the CodeSystem");
        let display = |code: &str| {
            system["concept"]
                .as_array()
                .expect("concepts")
                .iter()
                .find(|concept| concept["code"] == code)
                .and_then(|concept| concept["display"].as_str())
                .map(str::to_owned)
        };
        assert_eq!(display(REFUSED_ROLES[0]).as_deref(), Some("Bedrijfsarts"));
        assert_eq!(
            display(REFUSED_ROLES[1]).as_deref(),
            Some("Verzekeringsarts")
        );
    }

    #[test]
    fn the_care_provider_types_are_the_vendored_value_set() {
        let set: Value = serde_json::from_str(ORGANISATION_TYPES).expect("the ValueSet");
        let includes = set["compose"]["include"].as_array().expect("includes");
        assert_eq!(includes.len(), 1);
        assert_eq!(includes[0]["system"], ORGANISATION_TYPE_SYSTEMS[0]);
        let vendored: BTreeSet<&str> = includes[0]["concept"]
            .as_array()
            .expect("concepts")
            .iter()
            .map(|concept| concept["code"].as_str().expect("a code"))
            .collect();
        assert_eq!(vendored, CARE_PROVIDER_TYPES.into_iter().collect());
    }

    #[test]
    fn a_role_is_refused_when_any_of_its_codes_is_refused() {
        assert!(refused_role(&text("01.008")));
        assert!(refused_role(&coded(&[(None, "01.015"), (None, "01.048")])));
        assert!(!refused_role(&text("01.015")));
    }

    #[test]
    fn an_organisation_type_is_admitted_only_when_every_code_is_a_care_provider() {
        let oid = Some(ORGANISATION_TYPE_SYSTEMS[1]);
        assert!(care_provider_type(&text("V4")));
        assert!(care_provider_type(&coded(&[(oid, "Z3"), (None, "A1")])));
        assert!(!care_provider_type(&text("ZV")));
        assert!(!care_provider_type(&coded(&[])));
        assert!(!care_provider_type(&coded(&[(None, "V4"), (None, "ZV")])));
        assert!(!care_provider_type(&coded(&[(
            Some("urn:example:other"),
            "V4"
        )])));
    }
}
