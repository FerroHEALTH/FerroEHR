// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Whether an archetype slot admits a filler.
//!
//! A 1.4 slot constrains its fillers with `include` and `exclude` assertions
//! on `archetype_id/value` (`AM/docs/ADL1.4/master05-cadl.adoc` §Archetype
//! Slots). ADL 1.4 states no rule for combining the two lists; the only
//! specification text that does is ADL 2's (`AM/docs/ADL2/
//! master04.3-cadl_complex_types.adoc` §Formal Semantics of include and
//! exclude Constraints), which this module applies: a lone list is a
//! recommendation that admits every archetype, and a substantive list paired
//! with an open one (`.*`) is binding.

use regex::Regex;

use crate::opt14::types::{ArchetypeSlot, Assertion, ExprItem, OperatorKind};
use crate::xml::runtime::XmlAny;

/// What one assertion matches: a regular expression or a list of ids.
#[derive(Debug)]
enum Matcher {
    /// `archetype_id/value matches {/pattern/}`.
    Pattern { source: String, regex: Regex },
    /// `archetype_id/value matches {"id", …}`.
    List(Vec<String>),
}

impl Matcher {
    /// Returns whether the matcher admits every archetype id.
    fn is_open(&self) -> bool {
        match self {
            Self::Pattern { source, .. } => matches!(source.trim(), ".*" | ".+"),
            Self::List(_) => false,
        }
    }

    fn matches(&self, archetype_id: &str) -> bool {
        match self {
            Self::Pattern { regex, .. } => regex.is_match(archetype_id),
            Self::List(ids) => ids.iter().any(|id| id == archetype_id),
        }
    }
}

/// Why a slot cannot be evaluated.
#[derive(Debug)]
pub(super) enum SlotError {
    /// An assertion has a form other than an archetype-id match.
    Unsupported,
    /// An assertion's pattern does not compile.
    Pattern {
        /// The pattern as written.
        pattern: String,
        /// The compiler's error.
        source: regex::Error,
    },
}

/// Reads the `archetype_id/value matches {…}` constraint of one assertion.
fn matcher(assertion: &Assertion) -> Result<Matcher, SlotError> {
    let ExprItem::ExprBinaryOperator(op) = assertion.expression.as_ref() else {
        return Err(SlotError::Unsupported);
    };
    let (ExprItem::ExprLeaf(left), ExprItem::ExprLeaf(right)) =
        (op.left_operand.as_ref(), op.right_operand.as_ref())
    else {
        return Err(SlotError::Unsupported);
    };
    if op.operator != OperatorKind::Matches || left.item.text().trim() != "archetype_id/value" {
        return Err(SlotError::Unsupported);
    }
    c_string(&right.item)
}

/// Reads the `C_STRING` constraint leaf on the right of the match.
fn c_string(item: &XmlAny) -> Result<Matcher, SlotError> {
    if item.xsi_type() != Some("C_STRING") {
        return Err(SlotError::Unsupported);
    }
    if let Some(pattern) = item.child("pattern") {
        let source = pattern.text();
        // NOTE: ADL2 master04.5-cadl_primitive_types.adoc §Regular Expression: `...` "matches
        // any 3 characters" and `ab?` "the strings "a" and "ab"", so a pattern matches the whole id.
        let regex =
            Regex::new(&format!("^(?:{source})$")).map_err(|source_error| SlotError::Pattern {
                pattern: source.clone(),
                source: source_error,
            })?;
        return Ok(Matcher::Pattern { source, regex });
    }
    let ids: Vec<String> = item.children_named("list").map(XmlAny::text).collect();
    if ids.is_empty() {
        return Err(SlotError::Unsupported);
    }
    Ok(Matcher::List(ids))
}

/// Returns whether `slot` admits the archetype `archetype_id`.
pub(super) fn admits(slot: &ArchetypeSlot, archetype_id: &str) -> Result<bool, SlotError> {
    let includes = slot
        .includes
        .iter()
        .map(matcher)
        .collect::<Result<Vec<_>, _>>()?;
    let excludes = slot
        .excludes
        .iter()
        .map(matcher)
        .collect::<Result<Vec<_>, _>>()?;
    if includes.is_empty() || excludes.is_empty() {
        return Ok(true);
    }
    let include_open = includes.iter().any(Matcher::is_open);
    let exclude_open = excludes.iter().any(Matcher::is_open);
    let any_include = includes.iter().any(|m| m.matches(archetype_id));
    let any_exclude = excludes.iter().any(|m| m.matches(archetype_id));
    Ok(match (include_open, exclude_open) {
        (_, true) => any_include,
        (true, false) => !any_exclude,
        (false, false) => any_include && !any_exclude,
    })
}

#[cfg(test)]
mod tests {
    use super::admits;
    use crate::opt14::types::ArchetypeSlot;

    fn slot(includes: &[&str], excludes: &[&str]) -> ArchetypeSlot {
        let xml = format!(
            "<children xmlns=\"http://schemas.openehr.org/v1\" \
             xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"ARCHETYPE_SLOT\">\
             <rm_type_name>CLUSTER</rm_type_name><occurrences><lower_included>true</lower_included>\
             <lower_unbounded>false</lower_unbounded><upper_unbounded>true</upper_unbounded>\
             <lower>0</lower></occurrences><node_id>at0001</node_id>{}{}</children>",
            includes
                .iter()
                .map(|p| assertion("includes", p))
                .collect::<String>(),
            excludes
                .iter()
                .map(|p| assertion("excludes", p))
                .collect::<String>()
        );
        crate::xml::runtime::from_xml(&xml).expect("a well-formed slot")
    }

    fn assertion(tag: &str, pattern: &str) -> String {
        format!(
            "<{tag}><expression xsi:type=\"EXPR_BINARY_OPERATOR\"><type>Boolean</type>\
             <operator>2007</operator><precedence_overridden>false</precedence_overridden>\
             <left_operand xsi:type=\"EXPR_LEAF\"><type>String</type>\
             <item xsi:type=\"xsd:string\">archetype_id/value</item>\
             <reference_type>attribute</reference_type></left_operand>\
             <right_operand xsi:type=\"EXPR_LEAF\"><type>C_STRING</type>\
             <item xsi:type=\"C_STRING\"><pattern>{pattern}</pattern></item>\
             <reference_type>constraint</reference_type></right_operand></expression></{tag}>"
        )
    }

    const DEVICE: &str = "openEHR-EHR-CLUSTER.device.v1";
    const ADDRESS: &str = "openEHR-EHR-CLUSTER.address.v1";

    #[test]
    fn a_lone_list_is_a_recommendation() {
        let only_device = slot(&["openEHR-EHR-CLUSTER\\.device\\.v1"], &[]);
        assert!(admits(&only_device, ADDRESS).expect("evaluable"));
        let no_device = slot(&[], &["openEHR-EHR-CLUSTER\\.device\\.v1"]);
        assert!(admits(&no_device, DEVICE).expect("evaluable"));
    }

    #[test]
    fn a_substantive_include_with_an_open_exclude_binds() {
        let binding = slot(
            &["openEHR-EHR-CLUSTER\\.device(-[a-zA-Z0-9_]+)*\\.v1"],
            &[".*"],
        );
        assert!(admits(&binding, DEVICE).expect("evaluable"));
        assert!(admits(&binding, "openEHR-EHR-CLUSTER.device-pump.v1").expect("evaluable"));
        assert!(!admits(&binding, ADDRESS).expect("evaluable"));
    }

    #[test]
    fn a_substantive_exclude_with_an_open_include_binds() {
        let binding = slot(&[".*"], &["openEHR-EHR-CLUSTER\\.address\\.v1"]);
        assert!(admits(&binding, DEVICE).expect("evaluable"));
        assert!(!admits(&binding, ADDRESS).expect("evaluable"));
    }

    #[test]
    fn a_pattern_matches_the_whole_id() {
        let binding = slot(&["openEHR-EHR-CLUSTER\\.device\\.v1"], &[".*"]);
        assert!(!admits(&binding, "openEHR-EHR-CLUSTER.device.v12").expect("evaluable"));
        assert!(!admits(&binding, "x-openEHR-EHR-CLUSTER.device.v1").expect("evaluable"));
    }

    #[test]
    fn an_invalid_pattern_is_refused() {
        let broken = slot(&["openEHR-EHR-CLUSTER\\.device(\\.v1"], &[".*"]);
        assert!(admits(&broken, DEVICE).is_err());
    }
}
