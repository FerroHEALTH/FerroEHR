// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Public-API tests for the pure-BEL parser (`openehr_lang::v1_1::bel`, the
//! [`BeomBuilder`] path). Structural assertions over the generated `beom` tree;
//! the AOM-extended path is exercised in `openehr-adl`.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_lang::v1_1::bel::{BelError, parse_statements};
use openehr_lang::v1_1::beom::core::expression::Expression;
use openehr_lang::v1_1::beom::core::statement::Statement;

fn one_assertion(src: &str) -> Expression {
    let stmts = parse_statements(src).unwrap_or_else(|e| panic!("parse {src:?}: {e}"));
    assert_eq!(stmts.len(), 1, "expected one statement in {src:?}");
    match stmts.into_iter().next().unwrap() {
        Statement::Assertion(a) => *a.expression,
        other => panic!("expected an assertion, got {other:?}"),
    }
}

fn binary(e: &Expression) -> (&str, &Expression, &Expression) {
    match e {
        Expression::ExprBinaryOperator(b) => {
            (b.operator.as_str(), &b.left_operand, &b.right_operand)
        }
        other => panic!("expected a binary operator, got {other:?}"),
    }
}

fn is_path(e: &Expression, path: &str) -> bool {
    matches!(e, Expression::ExprValueRef(r)
        if r.item.as_ref().and_then(|v| v.as_str()) == Some(path))
}

fn is_int(e: &Expression, n: i64) -> bool {
    matches!(e, Expression::ExprLiteral(l) if l.item.as_i64() == Some(n))
}

#[test]
fn arithmetic_precedence_multiply_binds_tighter_than_plus() {
    // 1 + 2 * 3  ==  1 + (2 * 3)
    let e = one_assertion("1 + 2 * 3");
    let (op, l, r) = binary(&e);
    assert_eq!(op, "plus");
    assert!(is_int(l, 1));
    let (op2, l2, r2) = binary(r);
    assert_eq!(op2, "multiply");
    assert!(is_int(l2, 2) && is_int(r2, 3));
}

#[test]
fn exponent_is_right_associative() {
    // 2 ^ 3 ^ 2  ==  2 ^ (3 ^ 2)
    let e = one_assertion("2 ^ 3 ^ 2");
    let (op, l, r) = binary(&e);
    assert_eq!(op, "exponent");
    assert!(is_int(l, 2));
    assert_eq!(binary(r).0, "exponent");
}

#[test]
fn equality_over_arithmetic_sum() {
    // tagged assertion: total: /data/x = /data/a + /data/b
    let stmts = parse_statements("total: /data/x = /data/a + /data/b").expect("parse");
    let Statement::Assertion(a) = stmts.into_iter().next().unwrap() else {
        panic!("expected assertion");
    };
    assert_eq!(a.tag.as_deref(), Some("total"));
    let (op, l, r) = binary(&a.expression);
    assert_eq!(op, "eq");
    assert!(is_path(l, "/data/x"));
    let (op2, l2, _) = binary(r);
    assert_eq!(op2, "plus");
    assert!(is_path(l2, "/data/a"));
}

#[test]
fn implies_is_lowest_precedence() {
    // a and b implies c  ==  (a and b) implies c
    let e = one_assertion("a and b implies c");
    let (op, l, r) = binary(&e);
    assert_eq!(op, "implies");
    assert!(is_path(r, "c"));
    assert_eq!(binary(l).0, "and");
}

#[test]
fn symbol_and_text_operator_forms_are_equivalent() {
    assert_eq!(binary(&one_assertion("a and b")).0, "and");
    assert_eq!(binary(&one_assertion("a \u{2227} b")).0, "and"); // ∧
    assert_eq!(binary(&one_assertion("a \u{2228} b")).0, "or"); // ∨
    assert_eq!(binary(&one_assertion("a -> b")).0, "implies");
}

#[test]
fn not_unary_both_forms() {
    for src in ["not a", "~a", "\u{00AC}a"] {
        match one_assertion(src) {
            Expression::ExprUnaryOperator(u) => assert_eq!(u.operator.as_str(), "not"),
            other => panic!("{src:?}: expected unary not, got {other:?}"),
        }
    }
}

#[test]
fn exists_path_is_unary() {
    match one_assertion("exists /data/events") {
        Expression::ExprUnaryOperator(u) => {
            assert_eq!(u.operator.as_str(), "exists");
            assert!(is_path(&u.operand, "/data/events"));
        }
        other => panic!("expected unary exists, got {other:?}"),
    }
}

#[test]
fn assignment_and_declaration() {
    let stmts = parse_statements("$x : Integer\n$x := /data/count").expect("parse");
    assert_eq!(stmts.len(), 2);
    assert!(matches!(&stmts[0], Statement::VariableDeclaration(v) if v.name == "x"));
    match &stmts[1] {
        Statement::Assignment(a) => assert_eq!(a.target.name, "x"),
        other => panic!("expected assignment, got {other:?}"),
    }
}

#[test]
fn parenthesised_overrides_precedence() {
    // (1 + 2) * 3
    let e = one_assertion("(1 + 2) * 3");
    let (op, l, r) = binary(&e);
    assert_eq!(op, "multiply");
    assert!(is_int(r, 3));
    assert_eq!(binary(l).0, "plus");
}

#[test]
fn matches_constraint_rejected_by_pure_builder() {
    // The AOM `matches { … }` leaf is EXPR_CONSTRAINT (not in beom); the pure
    // builder surfaces a typed Unsupported error rather than mis-parsing.
    let err = parse_statements("/data/x matches {|140..160|}").unwrap_err();
    assert!(matches!(err, BelError::Unsupported { .. }), "got {err:?}");
}

#[test]
fn lex_error_is_typed_and_located() {
    let err = parse_statements("a \u{00a7} b").unwrap_err(); // § is not a BEL token
    assert!(matches!(err, BelError::Lex { .. }), "got {err:?}");
}

/// `LANG/docs/BEL/master03-language.adoc` §Typing/§Statements (#1402):
/// generic `type_id`s in declarations (`base_expressions.g4` `type_id`'s
/// recursive form) and the chapter's own declaration examples.
#[test]
fn generic_type_ids_in_declarations() {
    for src in [
        "$heart_rate_history: List<Real>",
        "$table: Hash<String,Integer>",
        "$age_in_years: Integer := current_date() - $date_of_birth",
    ] {
        let stmts = parse_statements(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(
            matches!(&stmts[0], Statement::VariableDeclaration(_)),
            "{src}"
        );
    }
}

/// `master03-language.adoc` §Constants (#1402): `Name : Type = primitive_object`
/// parses as a constant declaration (never a silently mis-read assertion),
/// including the section's interval example; a UC-tagged assertion still
/// parses when the shape does not complete as a constant.
#[test]
fn constant_declarations_parse() {
    for src in [
        "Mph_to_kmh_factor: Real = 1.6",
        "Pounds_to_kg: Real = 0.4536",
        "Systolic_normal_range: Interval<Integer> = |105..135|",
    ] {
        let stmts = parse_statements(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        // the beom carries a constant as its one declaration shape (no
        // constant class exists — the NOTE at BeomBuilder::constant_declaration).
        assert!(
            matches!(&stmts[0], Statement::VariableDeclaration(_)),
            "{src}"
        );
    }
    // a UC tag whose body is not a `Type [= …]` shape stays an assertion.
    let stmts = parse_statements("Check_vs_vars: exists $heart_rate").expect("tagged assertion");
    assert!(matches!(&stmts[0], Statement::Assertion(_)));
}

/// `master03-language.adoc` §Container Operators (#1402): the quantifier
/// binding accepts the docs-text bare identifier alongside the grammar's
/// `$`-form, with both `:` and `in` separators and the optional `|`.
#[test]
fn quantifier_binding_spellings() {
    for src in [
        "Check: for_all v : $events | v/value > 0",
        "Check: for_all v in $events | v/value > 0",
        "Check: for_all $v : $events | $v/value > 0",
        "Check: there_exists v : $events | v/value > 0",
        "Check: \u{2200} v : $events | v/value > 0",
        "Check: \u{2203} v : $events | v/value > 0",
    ] {
        assert!(
            parse_statements(src).is_ok(),
            "{src}: {:?}",
            parse_statements(src).err()
        );
    }
}

/// `master03-language.adoc` §Literals (#1402): terminology-code literals are
/// BEL primitives; the boundary twins — container literals have no grammar
/// production and no beom destination (the BEOM-normative bound,
/// `master02-overview.adoc`) — stay refused.
#[test]
fn terminology_code_literals_and_container_literal_boundary() {
    let stmts =
        parse_statements("Check: $code = [snomed_ct::389086002]").expect("term-code equality");
    assert_eq!(stmts.len(), 1);

    for refused in [
        "$l: List<Integer> := [1, 2, 3]",
        "$s: Set<Integer> := {1, 2, 3}",
    ] {
        assert!(
            parse_statements(refused).is_err(),
            "{refused} must stay refused (no grammar production, no beom class)"
        );
    }
}

// ── the nesting bound ─────────────────────────────────────────────────────

/// Run `f` on a thread whose stack fits a walk at the bound: the bound is
/// set for the engine's 256 MiB thread, not the 2 MiB test thread.
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

fn parenthesised(levels: usize) -> String {
    format!("{}1{} > 0", "(".repeat(levels), ")".repeat(levels))
}

#[test]
fn expression_nesting_past_the_bound_is_a_typed_refusal() {
    // The parser recurses up to the bound before refusing, so the refusal
    // too needs the stack the bound is sized for.
    let err = on_big_stack(|| {
        parse_statements(&parenthesised(openehr_lang::nesting::MAX_NESTING_DEPTH + 1))
            .expect_err("an expression nested past the bound is refused, never recursed into")
    });
    assert!(
        matches!(err, BelError::NestingTooDeep { limit, .. }
            if limit == openehr_lang::nesting::MAX_NESTING_DEPTH),
        "got {err:?}"
    );
}

#[test]
fn expression_nesting_well_within_the_bound_parses() {
    on_big_stack(|| {
        let e = one_assertion(&parenthesised(
            openehr_lang::nesting::MAX_NESTING_DEPTH.div_ceil(2),
        ));
        let (op, l, r) = binary(&e);
        assert_eq!(op, "gt");
        assert!(is_int(l, 1) && is_int(r, 0));
    });
}

#[test]
fn a_not_chain_past_the_bound_is_a_typed_refusal() {
    let src = format!(
        "{}$flag",
        "not ".repeat(openehr_lang::nesting::MAX_NESTING_DEPTH + 1)
    );
    let err = parse_statements(&src).expect_err("a unary chain recurses per operator");
    assert!(
        matches!(err, BelError::NestingTooDeep { .. }),
        "got {err:?}"
    );
}
