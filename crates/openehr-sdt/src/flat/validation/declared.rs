// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Which attribute names the reference model the walk validates against
//! declares.

/// Whether the RM model, or a FerroEHR draft class extending it, declares an
/// attribute named `attr` on any class; the name alone decides
/// (AOM2 `master11-rm_adaptation.adoc` for the draft classes).
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
