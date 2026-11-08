// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What an operational template states about a node that the Web Template
//! document has no member for, kept on the node the builder made from it.
//!
//! A consumer that needs one of these facts reads it from the node it already
//! holds, so it never walks the operational template a second time and never
//! re-spells the builder's `aqlPath` to join the two. The builder sets them
//! where it builds the node, which settles the walk: an OPT 1.4
//! `T_COMPLEX_OBJECT` extends `C_COMPLEX_OBJECT` (openEHR OPT 1.4
//! `Template.xsd`), so it is walked as one, and every node under it carries
//! its facts like any other.

use std::collections::BTreeMap;

use openehr_am::v2_4::aom2;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_its::opt14::types::CArchetypeRoot;
use openehr_its::opt14::types::CObject;

use super::model::WebTemplateCodedValue;

/// The `items` key of an OPT 1.4 `ARCHETYPE_TERM` that holds the term's text.
const TEXT: &str = "text";
/// The `items` key of an OPT 1.4 `ARCHETYPE_TERM` that holds its description.
const DESCRIPTION: &str = "description";

/// The operational-template facts of one Web Template node.
///
/// Every member is empty below an archetype root. No openEHR specification
/// governs the Web Template model, so the set is our own design; each member
/// cites the template construct it is read from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebTemplateNodeFacts {
    /// The template an OPT 1.4 `C_ARCHETYPE_ROOT` states it was included from,
    /// its optional `template_id` (OPT 1.4 `Template.xsd`, `C_ARCHETYPE_ROOT`).
    /// An OPT2 records no such member (AOM2 `master03-archetype_package.adoc`),
    /// so it is `None` there.
    pub embedded_template_id: Option<String>,
    /// At an archetype root, every term the archetype it roots defines, by
    /// code, in the template's default language: an OPT 1.4
    /// `C_ARCHETYPE_ROOT.term_definitions`, or the `term_definitions` of the
    /// component terminology an OPT2 carries for the root (AM OPT2
    /// `master03-opt_raw.adoc` §Terminology). It holds the codes of the nodes
    /// the template leaves out, which the Web Template carries nowhere.
    pub archetype_terms: BTreeMap<String, WebTemplateTerm>,
}

/// One term an archetype defines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebTemplateTerm {
    /// The term's text.
    pub text: String,
    /// The term's description, when the archetype gives one.
    pub description: Option<String>,
}

impl WebTemplateNodeFacts {
    /// The facts an OPT 1.4 constraint object states, which only a
    /// `C_ARCHETYPE_ROOT` does.
    pub(super) fn of_v1_4(co: &CObject) -> Self {
        let CObject::CArchetypeRoot(root) = co else {
            return Self::default();
        };
        Self {
            embedded_template_id: root.template_id.as_ref().map(|id| id.value.clone()),
            archetype_terms: terms_v1_4(root),
        }
    }

    /// The facts an OPT2 constraint object states, which only a
    /// `C_ARCHETYPE_ROOT` does, its component terminology being `terminology`.
    pub(super) fn of_v2_4(
        co: &aom2::constraint_model::c_object::CObject,
        terminology: &ArchetypeTerminology,
        language: &str,
    ) -> Self {
        match co {
            aom2::constraint_model::c_object::CObject::CComplexObject(
                aom2::constraint_model::c_complex_object::CComplexObject::CArchetypeRoot(_),
            ) => Self::of_v2_4_root(terminology, language),
            _ => Self::default(),
        }
    }

    /// The facts of an OPT2 archetype root, or of the template's own root,
    /// whose terminology is `terminology`, read in `language`.
    pub(super) fn of_v2_4_root(terminology: &ArchetypeTerminology, language: &str) -> Self {
        let archetype_terms = terminology
            .term_definitions
            .get(language)
            .into_iter()
            .flatten()
            .map(|(code, term)| {
                let entry = WebTemplateTerm {
                    text: term.text.clone(),
                    description: Some(term.description.clone()).filter(|d| !d.is_empty()),
                };
                (code.clone(), entry)
            })
            .collect();
        Self {
            embedded_template_id: None,
            archetype_terms,
        }
    }
}

/// The terms of one OPT 1.4 archetype root, the first definition of a code
/// winning.
fn terms_v1_4(root: &CArchetypeRoot) -> BTreeMap<String, WebTemplateTerm> {
    let mut terms = BTreeMap::new();
    for term in &root.term_definitions {
        terms
            .entry(term.code.clone())
            .or_insert_with(|| WebTemplateTerm {
                text: term.items.get(TEXT).cloned().unwrap_or_default(),
                description: term.items.get(DESCRIPTION).cloned(),
            });
    }
    terms
}

impl WebTemplateCodedValue {
    /// This option with the terminology its code is drawn from.
    #[must_use]
    pub fn with_terminology(mut self, terminology: &str) -> Self {
        self.terminology = Some(terminology.to_owned());
        self
    }
}
