// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Which attribute names the reference model the walk validates against
//! declares.

/// Does the generated RM model, or a class of FerroEHR's draft packages that
/// extend it, declare an attribute named `attr` on ANY class?
///
/// Name-only by design: an OPT constrains attributes at several RM levels and
/// the walker does not carry the declaring class here, while the defect this
/// answers — a constraint on an attribute NO RM class declares — is fully
/// decided by the name. With the `draft` feature, a template of a draft
/// class, such as an `ACCESS_POLICY` profile, constrains the draft's
/// attributes, which the RM adapted by the draft class tables declares (AOM2
/// `master11-rm_adaptation.adoc`).
pub(super) fn rm_declares_attribute(attr: &str) -> bool {
    static DECLARED: std::sync::LazyLock<std::collections::BTreeSet<&str>> =
        std::sync::LazyLock::new(|| {
            let classes = openehr_rm::v1_2::model::classes();
            #[cfg(feature = "draft")]
            let classes = classes
                .chain(openehr_rm::draft::access_control::model::CLASSES)
                .chain(openehr_rm::draft::access_log::model::CLASSES);
            classes
                .flat_map(|c| c.attributes.iter().map(|a| a.name))
                .collect()
        });
    DECLARED.contains(attr)
}

#[cfg(test)]
mod tests {
    use super::rm_declares_attribute;

    #[test]
    fn an_rm_attribute_is_declared_and_a_misspelling_is_not() {
        assert!(rm_declares_attribute("null_flavour"));
        assert!(!rm_declares_attribute("null_flavor"));
    }

    #[cfg(feature = "draft")]
    #[test]
    fn a_draft_class_attribute_is_declared_under_the_draft_feature() {
        // NOTE: AOM2 master11-rm_adaptation.adoc, an ACCESS_POLICY profile constrains draft attributes.
        assert!(rm_declares_attribute("self_declared_care"));
        assert!(rm_declares_attribute("concealed_confidentialities"));
    }
}
