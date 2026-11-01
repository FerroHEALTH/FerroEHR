// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Slot assertions: the `include`/`exclude` expressions of a slot and the
//! archetype-id include that replaces an embedded root.

/// A `C_CODE_REFERENCE` names an external reference set by URI. With no
/// inline code list, that is exactly the AOM2 ac-code term-binding pattern:
/// an ac-code constraint whose binding URI "will designate a ref-set or value
/// set" (`AOM2/master07-terminology_package.adoc` §Overview; keys per VTCBK) —
/// a minted ac-code + definition + binding is emitted, and the converter core
/// shifts all three consistently. When an inline code list is ALSO present
/// the (more concrete) list constraint wins and the URI is carried in
/// `conversion_details` — a dual-constrained node has no single ADL2 form (no
/// openEHR spec governs 1.4→2 conversion — our own design).
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::terminology::archetype_term::ArchetypeTerm;
use openehr_am::v2_4::beom::core::assertion::Assertion;
use openehr_its::opt14;

use super::RootCx;
use super::primitive::terminology_code;
use super::value::code_constraint;

pub(super) fn map_code_reference(c: &opt14::types::CCodeReference, cx: &mut RootCx) -> CObject {
    if c.referenceSetUri.is_empty() || !c.code_list.is_empty() {
        if !c.referenceSetUri.is_empty() {
            cx.note(
                format!("reference_set_uri.{}", c.node_id),
                format!(
                    "node {} carries both a code list and referenceSetUri {}; \
                     the code list was converted, the URI is recorded here",
                    c.node_id, c.referenceSetUri
                ),
            );
        }
        return terminology_code(
            &c.rm_type_name,
            &c.node_id,
            &c.occurrences,
            code_constraint(
                c.terminology_id.as_ref().map(|t| t.value.as_str()),
                &c.code_list,
                c.assumed_value.as_ref().map(|a| a.code_string.as_str()),
            ),
        );
    }
    let ac = cx.alloc_ac();
    cx.extra_terms.push(ArchetypeTerm {
        code: ac.clone(),
        text: "Reference set".to_owned(),
        description: format!("External reference set {}", c.referenceSetUri),
        other_items: None,
    });
    // NOTE: the 1.4 `referenceSetUri` carries no terminology name, so an
    // undeclared one keys under a generic bucket (no openEHR spec governs the
    // key choice — our own design).
    let term_key = c
        .terminology_id
        .as_ref()
        .map_or_else(|| "external".to_owned(), |t| t.value.clone());
    cx.extra_bindings
        .push((term_key, ac.clone(), c.referenceSetUri.clone()));
    terminology_code(&c.rm_type_name, &c.node_id, &c.occurrences, ac)
}

/// The `include` assertion naming the archetype that filled a slot: the
/// canonical `archetype_id/value matches {/…/}` form
/// (`org.openehr.am.aom2.archetype_slot.adoc`), built through the BEL slot
/// parser so the tree matches what parsing the printed ADL2 would yield.
/// A parse failure (unreachable for a valid `ARCHETYPE_HRID`) degrades to an
/// open slot, reported — the fill edge in [`OptConversion::structure`] still
/// carries the identity.
pub(super) fn archetype_id_include(
    child_archetype_id: &str,
    slot_node_id: &str,
    cx: &mut RootCx,
) -> Vec<Assertion> {
    let text = format!(
        "archetype_id/value matches {{/{}/}}",
        regex_escape(child_archetype_id)
    );
    match crate::rules::parse_slot_assertions(&text) {
        Ok(list) if !list.is_empty() => list,
        _ => {
            cx.note(
                format!("slot_assertion.{slot_node_id}.include.fill"),
                format!(
                    "the include naming filled archetype {child_archetype_id} did not parse; \
                     the slot is emitted open (the fill edge still records the identity)"
                ),
            );
            Vec::new()
        }
    }
}

/// Escape an archetype id for use inside a slot-assertion regex (`.` is the
/// only regex metacharacter a valid `ARCHETYPE_HRID` contains).
pub(super) fn regex_escape(id: &str) -> String {
    id.replace('.', "\\.")
}

/// Map the 1.4 slot `ASSERTION`s (`AOM1.4/master05-assertion_package.adoc`
/// expression trees) onto AOM2 `beom` assertions: the verbatim
/// `string_expression` is preferred (both generations share the
/// `archetype_id/value matches {/…/}` surface syntax), else the expression
/// tree is rendered to that syntax; the result re-parses through the BEL slot
/// parser. An assertion that cannot be rendered or parsed is dropped —
/// reported in `conversion_details` so the slot's weakening is visible.
pub(super) fn map_slot_assertions(
    assertions: &[opt14::types::Assertion],
    node_id: &str,
    kind: &str,
    cx: &mut RootCx,
) -> Vec<Assertion> {
    let mut out = Vec::new();
    for (idx, a) in assertions.iter().enumerate() {
        let text = a
            .string_expression
            .clone()
            .or_else(|| render_expr(&a.expression));
        let parsed = text
            .as_deref()
            .and_then(|t| crate::rules::parse_slot_assertions(t).ok());
        match parsed {
            Some(mut list) if !list.is_empty() => out.append(&mut list),
            _ => {
                cx.note(
                    format!("slot_assertion.{node_id}.{kind}.{idx}"),
                    format!(
                        "slot {node_id} {kind} assertion could not be converted \
                         (source: {:?}); the slot is emitted without it",
                        text.unwrap_or_else(|| "unrenderable expression tree".to_owned())
                    ),
                );
            }
        }
    }
    out
}

/// Render a 1.4 `EXPR_ITEM` tree to the ADL slot-assertion surface syntax.
/// Returns `None` for a shape outside the supported forms (binary/unary
/// operators over attribute paths, constants and constraint leaves).
pub(super) fn render_expr(e: &opt14::types::ExprItem) -> Option<String> {
    match e {
        opt14::types::ExprItem::ExprBinaryOperator(b) => {
            let l = render_expr(&b.left_operand)?;
            let r = render_expr(&b.right_operand)?;
            let op = binary_symbol(b.operator)?;
            Some(format!("{l} {op} {r}"))
        }
        opt14::types::ExprItem::ExprUnaryOperator(u) => {
            let inner = render_expr(&u.operand)?;
            let op = unary_symbol(u.operator)?;
            Some(format!("{op} ({inner})"))
        }
        opt14::types::ExprItem::ExprLeaf(leaf) => render_leaf(leaf),
    }
}

/// The ADL surface symbol of a binary `OPERATOR_KIND`, or `None` for a kind
/// with no infix rendering.
///
/// The relational, arithmetic and logical renderings are the textual column of
/// `LANG/docs/BEL/master03-language.adoc` §Operators; `matches` is the
/// archetype-slot constraint operator
/// (`AM/docs/ADL2/master07.10-adl_definition.adoc` §Archetype Slots). The two
/// quantifiers (`for_all`, `exists`) are not infix operators — their syntax is
/// `there_exists v : c | …` / `for_all v : c | …` (BEL master03 §Container
/// Operators) — so a tree that carries one as a binary operator has no surface
/// form here and is reported unconverted rather than rendered into text the
/// slot-assertion parser would refuse.
pub(super) fn binary_symbol(op: opt14::types::OperatorKind) -> Option<&'static str> {
    Some(match op {
        opt14::types::OperatorKind::Equal => "=",
        opt14::types::OperatorKind::NotEqual => "!=",
        opt14::types::OperatorKind::LessThanOrEqual => "<=",
        opt14::types::OperatorKind::LessThan => "<",
        opt14::types::OperatorKind::GreaterThanOrEqual => ">=",
        opt14::types::OperatorKind::GreaterThan => ">",
        opt14::types::OperatorKind::Matches => "matches",
        opt14::types::OperatorKind::And => "and",
        opt14::types::OperatorKind::Or => "or",
        opt14::types::OperatorKind::Xor => "xor",
        opt14::types::OperatorKind::Implies => "implies",
        opt14::types::OperatorKind::Plus => "+",
        opt14::types::OperatorKind::Minus => "-",
        opt14::types::OperatorKind::Multiply => "*",
        opt14::types::OperatorKind::Divide => "/",
        opt14::types::OperatorKind::Exponent => "^",
        opt14::types::OperatorKind::Not
        | opt14::types::OperatorKind::ForAll
        | opt14::types::OperatorKind::Exists => return None,
    })
}

/// The ADL surface symbol of a prefix `OPERATOR_KIND`, or `None` for a kind
/// with no prefix rendering.
///
/// `not` "can be applied as a prefix operator to all operators returning a
/// Boolean result as well as a parenthesised Boolean expression"
/// (`LANG/docs/BEL/master03-language.adoc` §Logical Negation); the unary
/// arithmetic signs are the same chapter's §Operators table.
pub(super) fn unary_symbol(op: opt14::types::OperatorKind) -> Option<&'static str> {
    match op {
        opt14::types::OperatorKind::Not => Some("not"),
        opt14::types::OperatorKind::Plus => Some("+"),
        opt14::types::OperatorKind::Minus => Some("-"),
        _ => None,
    }
}

/// Render a 1.4 `EXPR_LEAF`: an attribute path verbatim, a constant literal,
/// or a constraint (`C_STRING` pattern/list) as the `{/…/}`/`{"…"}` block.
pub(super) fn render_leaf(leaf: &opt14::types::ExprLeaf) -> Option<String> {
    let item = &leaf.item;
    let text = item.text();
    match leaf.reference_type.to_lowercase().as_str() {
        "attribute" => (!text.is_empty()).then_some(text),
        "constant" => {
            if text.is_empty() {
                None
            } else if item.xsi_type() == Some("string") || text.trim().parse::<i64>().is_err() {
                Some(format!("{text:?}"))
            } else {
                Some(text.trim().to_owned())
            }
        }
        "constraint" => {
            // The XML leaf item is a C_STRING: a regex `pattern` or a literal
            // string list.
            if let Some(p) = item.child("pattern") {
                let p = p.text();
                let body = p.trim_matches('/');
                return Some(format!("{{/{body}/}}"));
            }
            let items: Vec<String> = item
                .children_named("list")
                .map(|v| format!("{:?}", v.text()))
                .collect();
            (!items.is_empty()).then(|| format!("{{{}}}", items.join(", ")))
        }
        _ => None,
    }
}
