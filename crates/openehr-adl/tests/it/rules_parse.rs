// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Structure tests for the AOM rules / slot-assertion composition
//! (`openehr_adl::rules`). Deep-structure assertions hand-derived from the
//! `AOM2` master05 / `master04.3` shapes.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::parse::Dialect;
use openehr_adl::rules::{parse_rules_body, parse_slot_assertions};
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::rules::expr_constraint::ExprConstraint;
use openehr_am::v2_4::beom::core::expr_value_ref::ExprValueRef;
use openehr_am::v2_4::beom::core::expression::Expression;
use openehr_am::v2_4::beom::core::statement::Statement;
use openehr_lang::v1_1::beom::types::expr_type_def::ExprTypeDef;

fn only_assertion(body: &str) -> Expression {
    let set = parse_rules_body(body).unwrap_or_else(|e| panic!("parse {body:?}: {e:?}"));
    assert_eq!(
        set.statement.as_ref().map_or(0, Vec::len),
        1,
        "expected one statement in {body:?}"
    );
    match set.statement.into_iter().flatten().next().unwrap() {
        Statement::Assertion(a) => *a.expression,
        other => panic!("expected assertion, got {other:?}"),
    }
}

fn binop(e: &Expression) -> (&str, &Expression, &Expression) {
    match e {
        Expression::ExprBinaryOperator(b) => {
            (b.operator.as_str(), &b.left_operand, &b.right_operand)
        }
        other => panic!("expected binary operator, got {other:?}"),
    }
}

fn archetype_ref_path(e: &Expression) -> &str {
    match e {
        Expression::ExprValueRef(ExprValueRef::ExprArchetypeRef(r)) => r.path.as_str(),
        other => panic!("expected EXPR_ARCHETYPE_REF, got {other:?}"),
    }
}

#[test]
fn path_leaf_becomes_expr_archetype_ref() {
    // A bare rule path is an EXPR_ARCHETYPE_REF (master05).
    let e = only_assertion("/data[id2]/items[id5]/value/magnitude = 10");
    let (op, l, _) = binop(&e);
    assert_eq!(op, "eq");
    assert_eq!(
        archetype_ref_path(l),
        "/data[id2]/items[id5]/value/magnitude"
    );
}

#[test]
fn dependency_rule_matches_implies_exists() {
    // Hand-derived from features/aom_structures/rules/…dependency_rule…: an
    // implication whose antecedent is a `matches` terminology constraint and
    // whose consequent is an `exists` over a path.
    let e = only_assertion(
        "/data[id2]/items[id21]/items[id15]/value[id50]/defining_code matches {[at19]} \
         implies exists /data[id2]/items[id21]/items[id20]",
    );
    let (op, lhs, rhs) = binop(&e);
    assert_eq!(op, "implies");

    // antecedent: EXPR_ARCHETYPE_REF matches EXPR_CONSTRAINT(C_TERMINOLOGY_CODE).
    let (mop, mlhs, mrhs) = binop(lhs);
    assert_eq!(mop, "matches");
    assert_eq!(
        archetype_ref_path(mlhs),
        "/data[id2]/items[id21]/items[id15]/value[id50]/defining_code"
    );
    match mrhs {
        Expression::ExprConstraint(ExprConstraint::ExprConstraint(c)) => {
            assert!(matches!(c.item, CPrimitiveObject::CTerminologyCode(_)));
        }
        other => panic!("expected EXPR_CONSTRAINT, got {other:?}"),
    }

    // consequent: exists <path> (a unary operator).
    match rhs {
        Expression::ExprUnaryOperator(u) => {
            assert_eq!(u.operator.as_str(), "exists");
            assert_eq!(
                archetype_ref_path(&u.operand),
                "/data[id2]/items[id21]/items[id20]"
            );
        }
        other => panic!("expected unary exists, got {other:?}"),
    }
}

#[test]
fn tagged_sum_assertion() {
    // The rules_sum shape: `tag: /path = /a + /b`.
    let set = parse_rules_body("score: /data[id3]/x = /data[id3]/a + /data[id3]/b").expect("parse");
    let Statement::Assertion(a) = set.statement.into_iter().flatten().next().unwrap() else {
        panic!("expected assertion");
    };
    assert_eq!(a.tag.as_deref(), Some("score"));
    let (op, _, r) = binop(&a.expression);
    assert_eq!(op, "eq");
    assert_eq!(binop(r).0, "plus");
}

#[test]
fn real_literal_and_precedence_in_formula() {
    // rules_formulae shape: 0.33 * ( … - … ) inside a sum.
    let e = only_assertion("/data/x = /data/a + 0.33 * (/data/b - /data/c)");
    let (op, _, rhs) = binop(&e);
    assert_eq!(op, "eq");
    let (plus, _, prod) = binop(rhs);
    assert_eq!(plus, "plus");
    assert_eq!(binop(prod).0, "multiply");
}

#[test]
fn slot_assertion_is_archetype_ref_matches_id_constraint() {
    // master04.3 §Archetype Slots: `archetype_id/value matches { /regex/ }`
    // parses to EXPR_ARCHETYPE_REF matches EXPR_ARCHETYPE_ID_CONSTRAINT.
    let text = "archetype_id/value matches {/openEHR-EHR-OBSERVATION\\..*\\.v1/}";
    let assertions = parse_slot_assertions(text).unwrap_or_else(|e| panic!("slot parse: {e:?}"));
    assert_eq!(assertions.len(), 1, "single assertion block");
    let assertion = &assertions[0];
    assert_eq!(assertion.string_expression.as_deref(), Some(text));
    let (op, lhs, rhs) = binop(&assertion.expression);
    assert_eq!(op, "matches");
    assert_eq!(archetype_ref_path(lhs), "archetype_id/value");
    assert!(
        matches!(
            rhs,
            Expression::ExprConstraint(ExprConstraint::ExprArchetypeIdConstraint(_))
        ),
        "slot RHS must be EXPR_ARCHETYPE_ID_CONSTRAINT, got {rhs:?}"
    );
}

#[test]
fn slot_assertion_block_splits_multiple_assertions() {
    // master04.3 §Archetype Slots + cADL grammar `c_includes : SYM_INCLUDE
    // assertion+`: an include/exclude block may carry more than one assertion;
    // each is parsed to its own EXPR_ARCHETYPE_REF matches
    // EXPR_ARCHETYPE_ID_CONSTRAINT tree.
    let text = "archetype_id/value matches {/openEHR-EHR-OBSERVATION\\.a\\.v1/}\n\
                archetype_id/value matches {/openEHR-EHR-OBSERVATION\\.b\\.v1/}";
    let assertions = parse_slot_assertions(text).unwrap_or_else(|e| panic!("slot parse: {e:?}"));
    assert_eq!(assertions.len(), 2, "both assertions retained");
    for a in &assertions {
        let (op, lhs, rhs) = binop(&a.expression);
        assert_eq!(op, "matches");
        assert_eq!(archetype_ref_path(lhs), "archetype_id/value");
        assert!(matches!(
            rhs,
            Expression::ExprConstraint(ExprConstraint::ExprArchetypeIdConstraint(_))
        ));
    }
}

#[test]
fn slot_assertion_string_form_is_rendered_per_assertion() {
    // `ASSERTION.string_expression` is the "String form of expression"
    // (`LANG/docs/BEL/master04-expression_object_model.adoc` §Core Package) —
    // each assertion of a multi-assertion block carries ITS OWN form, rendered
    // from its own tree, never the whole block's source text.
    let text = "archetype_id/value matches {/openEHR-EHR-OBSERVATION\\.a\\.v1/}\n\
                archetype_id/value matches {/openEHR-EHR-OBSERVATION\\.b\\.v1/}";
    let assertions = parse_slot_assertions(text).unwrap_or_else(|e| panic!("slot parse: {e:?}"));
    assert_eq!(
        assertions
            .iter()
            .map(|a| a.string_expression.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec![
            "archetype_id/value matches {/openEHR-EHR-OBSERVATION\\.a\\.v1/}".to_owned(),
            "archetype_id/value matches {/openEHR-EHR-OBSERVATION\\.b\\.v1/}".to_owned(),
        ]
    );
}

#[test]
fn symbolic_matches_renders_from_the_tree_in_its_keyword_form() {
    // `ADL2/master04.3` §Slots based on Lexical Archetype Identifiers writes
    // the operator symbolically (`archetype_id/value ∈ {/…/}`); the operator
    // is one node either way, so the rendered string form is the keyword
    // spelling the printer emits everywhere else.
    let assertions = parse_slot_assertions("archetype_id/value \u{2208} {/.*/}")
        .unwrap_or_else(|e| panic!("slot parse: {e:?}"));
    let a = &assertions[0];
    assert_eq!(binop(&a.expression).0, "matches");
    assert_eq!(
        a.string_expression.as_deref(),
        Some("archetype_id/value matches {/.*/}")
    );
}

#[test]
fn slot_assertion_prints_from_the_tree_not_the_string_form() {
    // The printer reads `ASSERTION.expression` (the "Root of expression
    // tree"), so an assertion whose string form is absent still renders.
    let text = "archetype_id/value matches {/openEHR-EHR-CLUSTER\\.device\\.v1/}";
    let mut assertions = parse_slot_assertions(text).unwrap_or_else(|e| panic!("slot: {e:?}"));
    assertions[0].string_expression = None;
    assert_eq!(
        openehr_adl::print::assertion_text(&assertions[0]).expect("render the assertion"),
        text
    );
}

#[test]
fn slot_assertion_accessors_read_the_expression_tree() {
    // The path and regex a slot constrains come from the tree, not from a
    // scan of the string form (`master04.3` §Slots based on Lexical Archetype
    // Identifiers).
    let mut assertions =
        parse_slot_assertions("archetype_id/value matches {/openEHR-EHR-CLUSTER\\..*\\.v1/}")
            .unwrap_or_else(|e| panic!("slot: {e:?}"));
    assertions[0].string_expression = None;
    assert_eq!(
        openehr_adl::rules::slot_assertion_path(&assertions[0]),
        Some("archetype_id/value")
    );
    assert_eq!(
        openehr_adl::rules::slot_assertion_regex(&assertions[0]),
        Some("openEHR-EHR-CLUSTER\\..*\\.v1")
    );

    // A non-`matches` assertion constrains no archetype-id regex.
    let existence = parse_rules_body("exists /items[id2]").expect("parse");
    let Statement::Assertion(a) = existence.statement.into_iter().flatten().next().unwrap() else {
        panic!("expected an assertion");
    };
    assert_eq!(openehr_adl::rules::slot_assertion_path(&a), None);
    assert_eq!(openehr_adl::rules::slot_assertion_regex(&a), None);
}

#[test]
fn statement_string_form_drops_only_the_outermost_parentheses() {
    // A statement's outermost operator needs no parentheses of its own; every
    // operand keeps them, so the form re-parses to the identical tree
    // whatever the precedence.
    let set = parse_rules_body("score: /data[id3]/x = /data[id3]/a + /data[id3]/b").expect("parse");
    let Statement::Assertion(a) = set.statement.into_iter().flatten().next().unwrap() else {
        panic!("expected an assertion");
    };
    let text = openehr_adl::print::assertion_text(&a).expect("render the assertion");
    assert_eq!(
        text, "score: /data[id3]/x = (/data[id3]/a + /data[id3]/b)",
        "the tag and the parenthesized operands are kept"
    );
    // Re-parsing the rendered form yields the same tree.
    let again = parse_rules_body(&text).expect("re-parse");
    let Statement::Assertion(b) = again.statement.into_iter().flatten().next().unwrap() else {
        panic!("expected an assertion");
    };
    assert_eq!(a.expression, b.expression);
    assert_eq!(a.tag, b.tag);
}

#[test]
fn declared_variable_type_is_recovered_by_a_later_reference() {
    // `LANG/docs/BEL/master03-language.adoc` §Typing: a declared variable's
    // type belongs to the declaration; a later `$name` reference carries the
    // declared `VARIABLE_DECLARATION`, not a type named after the variable.
    let set = parse_rules_body("$sys : Quantity\n$sys > 3").expect("parse");
    let statements: Vec<Statement> = set.statement.into_iter().flatten().collect();
    assert_eq!(statements.len(), 2);
    let Statement::Assertion(a) = &statements[1] else {
        panic!("expected the assertion, got {:?}", statements[1]);
    };
    let (op, lhs, _) = binop(&a.expression);
    assert_eq!(op, "gt");
    match lhs {
        Expression::ExprVariableRef(v) => {
            assert_eq!(v.item.name, "sys");
            match &v.item.r#type {
                ExprTypeDef::TypeDefObjectRef(r) => assert_eq!(r.type_name, "Quantity"),
                other => panic!("expected an object-reference type def, got {other:?}"),
            }
        }
        other => panic!("expected EXPR_VARIABLE_REF, got {other:?}"),
    }
}

#[test]
fn invalid_rule_expression_is_typed_error() {
    // An unparsable rule expression surfaces a typed error, never a panic.
    let err = parse_rules_body("exists").unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn archetype_ref_item_resolves_to_target_node() {
    // master05 §rules: after assembly, an EXPR_ARCHETYPE_REF's `item` is resolved
    // to the definition node its path addresses (not the empty parse-time
    // placeholder) — `openehr_adl::rules::resolve_archetype_refs`, run in assembly.
    use openehr_adl::assemble::parse_artefact;
    use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
    use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
    use openehr_am::v2_4::aom2::constraint_model::archetype_constraint::ArchetypeConstraint;
    use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;

    let src = "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
        \topenEHR-EHR-CLUSTER.rule_ref.v1.0.0\n\n\
        language\n\toriginal_language = <[ISO_639-1::en]>\n\n\
        description\n\tlifecycle_state = <\"draft\">\n\n\
        definition\n\tCLUSTER[id1] matches {\n\t\titems matches {\n\t\t\tELEMENT[id2] matches {*}\n\t\t}\n\t}\n\n\
        rules\n\t\texists /items[id2]\n\n\
        terminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>\n\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>\n\t\t>\n\t>\n";
    let art = parse_artefact(src, Dialect::Adl2).unwrap_or_else(|e| panic!("parse: {e:?}"));
    let rules = match art {
        Archetype::AuthoredArchetype(a) => match *a {
            AuthoredArchetype::AuthoredArchetype(d) => d.rules,
            other => panic!("expected authored archetype, got {other:?}"),
        },
        Archetype::TemplateOverlay(_) => panic!("expected an authored archetype, got an overlay"),
    };
    let stmt = rules
        .into_iter()
        .flatten()
        .next()
        .expect("a rules statement set")
        .statement
        .into_iter()
        .flatten()
        .next()
        .expect("a rule statement");
    let Statement::Assertion(a) = stmt else {
        panic!("expected an assertion, got {stmt:?}");
    };
    // `exists /items[id2]` → a unary operator over the EXPR_ARCHETYPE_REF.
    let Expression::ExprUnaryOperator(u) = *a.expression else {
        panic!("expected a unary `exists`, got {:?}", a.expression);
    };
    let Expression::ExprValueRef(ExprValueRef::ExprArchetypeRef(r)) = *u.operand else {
        panic!("expected an EXPR_ARCHETYPE_REF operand");
    };
    assert_eq!(r.path, "/items[id2]");
    // The item is resolved to the ELEMENT[id2] node (its rm_type_name is set),
    // not the empty `unresolved_ref_target` placeholder.
    match &r.item {
        ArchetypeConstraint::CComplexObject(cco) => {
            let rm = match cco.as_ref() {
                CComplexObject::CComplexObject(d) => &d.rm_type_name,
                CComplexObject::CArchetypeRoot(rt) => &rt.rm_type_name,
            };
            assert_eq!(rm, "ELEMENT", "resolved node RM type");
        }
        other => panic!("expected a resolved C_COMPLEX_OBJECT, got {other:?}"),
    }
}

#[test]
fn external_query_assignment_is_refused_by_the_printer() {
    // NOTE: neither `LANG/docs/BEL/masterAppA-syntax.adoc` nor
    // `AM/docs/ADL2/masterAppB-syntax_spec.adoc` has an EXTERNAL_QUERY
    // production, so the printer refuses it rather than invent syntax.
    use openehr_adl::assemble::parse_artefact;
    use openehr_adl::print::{PrintError, print};
    use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
    use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
    use openehr_am::v2_4::beom::core::assignment::Assignment;
    use openehr_am::v2_4::beom::core::expr_value::ExprValue;
    use openehr_am::v2_4::beom::core::statement_set::StatementSet;
    use openehr_lang::v1_1::beom::core::external_query::ExternalQuery;

    let src = "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
        \topenEHR-EHR-CLUSTER.external_query.v1.0.0\n\n\
        language\n\toriginal_language = <[ISO_639-1::en]>\n\n\
        description\n\tlifecycle_state = <\"draft\">\n\n\
        definition\n\tCLUSTER[id1] matches {*}\n\n\
        terminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>\n\t\t>\n\t>\n";
    let mut art = parse_artefact(src, Dialect::Adl2).unwrap_or_else(|e| panic!("parse: {e:?}"));
    print(&art).expect("the artefact prints before the EXTERNAL_QUERY is injected");

    // The target declaration comes from the parser, so only the assignment's
    // source is hand-built.
    let declaration = parse_rules_body("$dob : Date").expect("parse the declaration");
    let Some(Statement::VariableDeclaration(target)) =
        declaration.statement.and_then(|s| s.into_iter().next())
    else {
        panic!("expected a variable declaration");
    };
    let external = StatementSet {
        name: None,
        statement: Some(vec![Statement::Assignment(Assignment {
            target,
            source: ExprValue::ExternalQuery(ExternalQuery {
                context: "patient".to_owned(),
                query_id: "date_of_birth".to_owned(),
                query_args: None,
            }),
        })]),
    };
    match &mut art {
        Archetype::AuthoredArchetype(inner) => match inner.as_mut() {
            AuthoredArchetype::AuthoredArchetype(d) => d.rules = Some(vec![external]),
            other => panic!("expected an authored archetype, got {other:?}"),
        },
        Archetype::TemplateOverlay(_) => panic!("expected an authored archetype, got an overlay"),
    }

    assert_eq!(
        print(&art).expect_err("EXTERNAL_QUERY has no ADL surface syntax"),
        PrintError::ExternalQuery {
            target: "dob".to_owned()
        },
    );
}

#[test]
fn nameless_function_call_is_refused_by_the_printer() {
    // NOTE: the BEL function-call production requires an identifier name
    // (`LANG/docs/BEL/masterAppA-syntax.adoc`); a leaf `item` (`Any`, beom
    // `EXPR_LEAF`) with no string name has no spelling, so the printer refuses.
    use openehr_adl::print::{PrintError, assertion_text};
    use openehr_am::v2_4::beom::core::assertion::Assertion;
    use openehr_am::v2_4::beom::core::expr_function_call::ExprFunctionCall;
    use openehr_am::v2_4::beom::core::expression::Expression;

    let assertion = Assertion {
        tag: None,
        string_expression: None,
        expression: Box::new(Expression::ExprFunctionCall(ExprFunctionCall {
            item: None,
            arguments: None,
        })),
    };
    assert_eq!(
        assertion_text(&assertion).expect_err("a nameless function call has no ADL spelling"),
        PrintError::NamelessFunctionCall,
    );
}

#[test]
fn pathless_value_ref_is_refused_by_the_printer() {
    // NOTE: the BEL value-reference production requires a path
    // (`LANG/docs/BEL/masterAppA-syntax.adoc`); a leaf `item` (`Any`, beom
    // `EXPR_LEAF`) with no string path has no spelling, so the printer refuses.
    use openehr_adl::print::{PrintError, assertion_text};
    use openehr_am::v2_4::beom::core::assertion::Assertion;
    use openehr_am::v2_4::beom::core::expr_value_ref::{ExprValueRef, ExprValueRefData};
    use openehr_am::v2_4::beom::core::expression::Expression;

    let assertion = Assertion {
        tag: None,
        string_expression: None,
        expression: Box::new(Expression::ExprValueRef(ExprValueRef::ExprValueRef(
            ExprValueRefData { item: None },
        ))),
    };
    assert_eq!(
        assertion_text(&assertion).expect_err("a pathless value reference has no ADL spelling"),
        PrintError::PathlessValueRef,
    );
}
