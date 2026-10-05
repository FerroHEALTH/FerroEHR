// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The Expression Language parser and the `BMM_ASSERTION` materialisation it
//! feeds.
//!
//! Two levels are pinned here: the GRAMMAR, over every assertion string the
//! pinned production schemas carry, and the MATERIALISATION, over a small
//! self-contained schema that the v3 transform can build.

#![expect(
    clippy::panic_in_result_fn,
    reason = "the Book ch11 test shape: `?` propagates the read/parse plumbing while the assertions ARE the test — an assertion panic is how these tests fail"
)]
#![expect(
    clippy::panic,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::PathBuf;

use openehr_lang::v1_1::bmm_persistence::create_bmm3_model::create_bmm3_model_reporting;
use openehr_lang::v1_1::bmm_persistence::error::PBmmReadError;
use openehr_lang::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use openehr_lang::v1_1::bmm_persistence::reader::read_schema;
use openehr_lang::v1_1::bmm_persistence::validate::AssertionKind;
use openehr_lang::v1_1::bmm_persistence::validate::PBmmValidityFinding;
use openehr_lang::v1_1::bmm3::core::entity::bmm_class::BmmClass;
use openehr_lang::v1_1::bmm3::core::entity::bmm_simple_class::BmmSimpleClass;
use openehr_lang::v1_1::bmm3::expression::el_expression::ElExpression;
use openehr_lang::v1_1::el::ElBuilder;
use openehr_lang::v1_1::el::ElError;
use openehr_lang::v1_1::el::ElLiteral;
use openehr_lang::v1_1::el::ElOperator;
use openehr_lang::v1_1::el::parse_boolean_expression_with;

/// A builder that only proves the grammar accepts a text.
struct Syntax;

impl ElBuilder for Syntax {
    type Expr = String;

    fn literal(&mut self, literal: ElLiteral, _at: usize) -> Result<String, ElError> {
        Ok(literal.value_literal())
    }
    fn self_ref(&mut self, _at: usize) -> Result<String, ElError> {
        Ok("Self".to_owned())
    }
    fn result_ref(&mut self, _at: usize) -> Result<String, ElError> {
        Ok("Result".to_owned())
    }
    fn bound_variable(&mut self, name: &str, _at: usize) -> Result<String, ElError> {
        Ok(format!("${name}"))
    }
    fn type_ref(&mut self, type_id: &str, _at: usize) -> Result<String, ElError> {
        Ok(format!("{{{type_id}}}"))
    }
    fn feature_ref(
        &mut self,
        scoper: Option<String>,
        name: &str,
        args: Option<Vec<String>>,
        _at: usize,
    ) -> Result<String, ElError> {
        let call = match args {
            None => name.to_owned(),
            Some(args) => format!("{name}({})", args.join(",")),
        };
        Ok(match scoper {
            None => call,
            Some(scoper) => format!("{scoper}.{call}"),
        })
    }
    fn binary(
        &mut self,
        operator: ElOperator,
        left: String,
        right: String,
        _at: usize,
    ) -> Result<String, ElError> {
        Ok(format!("({} {left} {right})", operator.function))
    }
    fn unary(
        &mut self,
        operator: ElOperator,
        operand: String,
        _at: usize,
    ) -> Result<String, ElError> {
        Ok(format!("({} {operand})", operator.function))
    }
    fn attached(&mut self, operand: String, _at: usize) -> Result<String, ElError> {
        Ok(format!("(attached {operand})"))
    }
    fn quantified(
        &mut self,
        universal: bool,
        variable: &str,
        collection: String,
        condition: String,
        _at: usize,
    ) -> Result<String, ElError> {
        let keyword = if universal { "for_all" } else { "there_exists" };
        Ok(format!("({keyword} {variable} {collection} {condition})"))
    }
    fn tuple(&mut self, items: Vec<String>, _at: usize) -> Result<String, ElError> {
        Ok(format!("[{}]", items.join(",")))
    }
    fn constraint(&mut self, raw: &str, _at: usize) -> Result<String, ElError> {
        Ok(format!("constraint{raw}"))
    }
}

/// Parses `src` under [`Syntax`].
fn syntax(src: &str) -> Result<String, ElError> {
    parse_boolean_expression_with(src, &mut Syntax)
}

// ── the grammar, over the pinned production schemas ──────────────────────

/// The ODIN serialisations of the component schemas `docs/VERSIONS.md` pins.
const PINNED_SCHEMAS: &[&str] = &[
    "BASE/odin/openehr_base_1.3.0.bmm",
    "RM/odin/openehr_rm_1.2.0.bmm",
    "AM/odin/openehr_am_1.4.0.bmm",
    "AM/odin/openehr_am_2.4.0.bmm",
    "TERM/odin/openehr_term_3.1.0.bmm",
    "LANG/odin/openehr_lang_1.1.0.bmm",
    "LANG/odin/openehr_lang_1.1.0-bmm3.bmm",
];

/// How many of the pinned schemas' assertion strings the EL grammar accepts.
///
/// The number only ever ratchets UP: a grammar change that accepts fewer is a
/// regression, one that accepts more updates this line together with
/// [`UNPARSABLE`].
const PARSING_ASSERTIONS: usize = 319;

/// Every DISTINCT assertion string in the pinned schemas that the EL grammar
/// refuses, adjudicated.
///
/// These are not EL. openEHR's published BMM schemas write their invariants in
/// an Eiffel-flavoured surface syntax, and the vendored normative EL grammar
/// (`vendor/grammar/v1_1/{ElLexer.g4, ElParser.g4}`, which
/// `LANG/docs/EL/masterAppA-syntax.adoc` includes) admits none of the following
/// forms. Each row falls into exactly one class:
///
/// * **`and then` / `or else`** — Eiffel's short-circuit operators.
///   `ElLexer.g4` declares `SYM_AND`/`SYM_OR` and a bare `SYM_THEN` that no
///   `ElParser.g4` production uses, so the two-word forms have no production.
/// * **Typographic quotes** (`“…”`, `‘…’`) — `STRING` and `CHARACTER` are
///   delimited by `"` and `'` in the base lexical layer; the curly forms are
///   not lexable.
/// * **`for_all v in c | …` and `c.for_all (v: T | …)`** — `ElParser.g4`
///   `elForAllExpr` is `SYM_FOR_ALL elLocalVariableId ':' elValueGenerator
///   '¦' elBooleanExpr`: the binder is `:` and the body separator is the
///   BROKEN BAR `¦`, never `in` and never `|`.
/// * **Chained comparisons** (`Result = a < b`) —
///   `elArithmeticComparisonExpr` takes exactly one `elComparisonBinop`
///   between two arithmetic expressions, so a second comparison in the same
///   expression is trailing input.
/// * **Assignment and interval forms in an assertion position** (`Result :=
///   …`, `arity in |1..2|`, `s in {"=", …}`) — `ElParser.g4` `assertion` is
///   `LC_ID ':' elBooleanExpr`; `:=` belongs to `assignment` and `in` outside
///   a quantifier has no production.
/// * **Truncated multi-line ODIN values** — two rows are the fragments the
///   published schemas leave after a line break inside the quoted string.
///
/// The register is exhaustive: any other refusal fails the sweep.
const UNPARSABLE: &[&str] = &[
    " items.forall (i:ITEM | i.type = \"ELEMENT\")",
    "(current_revision /= Void and not is_controlled) implies current_revision.is_equal (“(uncontrolled)”)",
    "(events /= Void and then not events.is_empty) or summary /= Void",
    "(normal_range /= Void and normal_status /= Void) implies (normal_status.code_string.is_equal (“N”) xor not normal_range.has (self))",
    "(range.lower_unbounded or else range.lower.is_simple) and (range.upper_unbounded or else range.upper.is_simple)",
    "(reason.generating_type.is_equal (“DV_CODED_TEXT”) implies terminology (Terminology_id_openehr).has_code_for_group_id (Group_id_attestation_reason, reason.defining_code))",
    "Result /= Void implies (not Result.empty and then Result.for_all (item | repository (\"demographics\").all_party_relationships.has_object (item) and then repository (\"demographics\").all_party_relationships.object (item).target = self))",
    "Result := c = ‘>’ or c = ‘=’ or c = ‘<’ or c = ‘?’",
    "Result := children = Void or else children.is_empty",
    "Result := children.is_empty and not is_prohibited",
    "Result := constraint.is_empty",
    "Result := generic_parameter_defs /= Void",
    "Result = constraint.is_empty or else constraint.count = 1 and constraint.first.is_equal (Regex_any_string)",
    "Result = d >= 1 and d <= days_in_month (m, y)",
    "Result = existence /= Void and then existence.is_mandatory",
    "Result = existence /= Void and then existence.is_prohibited",
    "Result = existence = Void and ((is_single and other.is_single) or (is_multiple and other.is_multiple and cardinality = Void))",
    "Result = fs >= 0.0 and fs < 1.0",
    "Result = is_at_code (a_code) or else is_id_code (a_code) or else is_value_code (a_code) or else is_value_set_code (a_code)",
    "Result = item = Void",
    "Result = m >= 0 and m < Minutes_in_hour",
    "Result = m >= 1 and m <= Months_in_year",
    "Result = magnitude < other.magnitude",
    "Result = occurrences /= Void and then occurrences.is_prohibited",
    "Result = open_arguments = Void",
    "Result = s >= 0 and s < Seconds_in_minute",
    "Result = s in {\"=\", \"<\", \">\", \"<=\", \">=\", \"~\"}",
    "Result = soc_parent /= Void or parent.soc_parent /= Void",
    "Result = y >= 0",
    "Result implies subject.generating_type = “PARTY_SELF”",
    "all_version_ids.has (a_preceding_version_uid) or else version_count = 0",
    "alternatives /= Void and then alternatives.for_all(co: C_OBJECT | co.occurrences.upper <= 1)",
    "branch_number /= Void implies branch_number.is_integer and then branch_number.as_integer >= 1",
    "branch_version /= Void implies branch_version.is_integer and then branch_version.as_integer >= 1",
    "expression /= Void and then expression.type.is_equal(“BOOLEAN”)",
    "folders /= Void implies for_all f in folders | f.type.is_equal(\"VERSIONED_FOLDER\")",
    "for_all c in compositions | c.type.is_equal (\"VERSIONED_COMPOSITION\")",
    "for_all c in contributions | c.type.is_equal(\"CONTRIBUTION\")",
    "for_all p in converters : creators.has(p) and p.arity() = 1",
    "for_all p in creators : procedures.has(p)",
    "for_all v in all_versions | v.archetype_node_id.is_equal (all_versions.first.archetype_node_id)",
    "for_all v in all_versions | v.is_persistent = all_versions.first.data.is_persistent",
    "function /= Void and then function.generating_type.is_equal (“DV_CODED_TEXT”) implies terminology (Terminology_id_openehr).has_code_for_group_id (Group_id_participation_function, function.defining_code)",
    "is_periodic implies events.for_all (e: EVENT | e.offset. to_seconds.mod(period.to_seconds) = 0)",
    "lifecycle_state /= Void and then terminology (Term_id_openehr).has_code_for_group_id (Group_id_version_lifecycle_state, lifecycle_state.defining_code)",
    "match = ‘<’ implies Result",
    "match = ‘>’ implies Result",
    "match = ‘?’ implies Result",
    "media_type /= Void and then code_set (Code_set_id_media_types).has_code (media_type)",
    "normal_status /= Void implies normal_status.code_string.is_equal (“N”)",
    "not is_first xor trunk_version.is_equal(“1”)",
    "offset /= Void and then offset = time.diff (parent.origin)",
    "operator_def /= Void implies arity in |1..2|",
    "original_language /= void and then language /= Void",
    "parent_resource /= Void implies details.for_all (d | parent_resource.languages_available.has (d.language.code_string))",
    "relationships /= Void implies (not relationships.is_empty and then relationships.for_all (r | r.source = self)",
    "reverse_relationships /= Void implies (not reverse_relationships.empty and then reverse_relationships.for_all (item | repository (\"demographics\").all_party_relationships.has_object (item) and then repository(\"demographics\").all_party_relationships.object (item).target = self))",
    "rows.for_all (items.for_all (instance_of (\"ELEMENT\")))",
    "second_validity = {VALIDITY_KIND}.disallowed implies millisecond_validity = {VALIDITY_KIND}.disallowed Validity_is_range: validity_is_range = (range /= Void)",
    "soc_parent /= Void or else (parent /= Void and then parent.is_second_order_constrained)",
    "source /= Void and then source.relationships.has (self)",
    "subject_is_self implies subject.generating_type = “PARTY_SELF”",
    "target /= Void and then not target.reverse_relationships.has (self)",
    "target_path /= Void and then not target_path.is_empty",
    "translations /= Void implies (description.details.for_all (d &#124;\ntranslations.has_key (d.language.code_string)))",
    "translations /= Void implies (description.details.for_all (d |\ntranslations.has_key (d.language.code_string)))",
    "trunk_version /= Void and then trunk_version.is_integer and then trunk_version.as_integer >= 1",
    "type.is_equal (“ACCESS_GROUP”)",
    "type.is_equal(“PERSON”) or type.is_equal(“ORGANISATION”) or type.is_equal(“GROUP”) or type.is_equal(“AGENT”)or type.is_equal(“ROLE”) or type.is_equal(“PARTY”) or type.is_equal(“ACTOR”)",
    "value.formalism.is_equal (“HL7:PIVL”) or value.formalism. is_equal (“HL7:EIVL”)",
    "version /= Void and then version.is_equal(archetype_id.version_id)",
];

fn read_pinned(file: &str) -> PBmmSchema {
    let full = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/openehr-codegen/vendor/bmm/components")
        .join(file);
    let src =
        std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("read {}: {e}", full.display()));
    read_schema(&src).unwrap_or_else(|e| panic!("{file}: the pinned schema reads: {e}"))
}

/// Every assertion string of `schema`: class invariants plus routine pre- and
/// post-conditions.
fn assertion_strings(schema: &PBmmSchema) -> Vec<String> {
    let mut out = Vec::new();
    for class in schema.class_definitions.iter().flatten() {
        out.extend(
            class
                .invariants()
                .into_iter()
                .flatten()
                .map(|(_, text)| text.clone()),
        );
        for function in class.functions().into_iter().flatten().map(|(_, f)| f) {
            out.extend(
                function
                    .pre_conditions
                    .iter()
                    .flatten()
                    .map(|(_, text)| text.clone()),
            );
            out.extend(
                function
                    .post_conditions
                    .iter()
                    .flatten()
                    .map(|(_, text)| text.clone()),
            );
        }
    }
    out
}

#[test]
fn every_pinned_schema_assertion_parses_or_is_adjudicated() {
    let mut parsed = 0usize;
    let mut refused: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for file in PINNED_SCHEMAS {
        for text in assertion_strings(&read_pinned(file)) {
            match syntax(&text) {
                Ok(_) => parsed += 1,
                Err(_) => {
                    refused.insert(text);
                }
            }
        }
    }
    let registered: std::collections::BTreeSet<String> =
        UNPARSABLE.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(
        refused, registered,
        "the set of refused assertion strings drifted from the adjudicated register"
    );
    assert_eq!(
        parsed, PARSING_ASSERTIONS,
        "the number of pinned-schema assertions the EL grammar accepts changed"
    );
}

// ── the grammar, unit by unit ────────────────────────────────────────────

#[test]
fn logical_precedence_follows_the_el_operator_table_not_the_parser_grammar() -> Result<(), ElError>
{
    // `LANG/docs/EL/master05-expressions.adoc` §Primitive Operators lists the
    // Logical Operators in descending precedence NOT > AND > OR > XOR >
    // IMPLIES, and §Precedence and Parentheses makes that table normative.
    // `ElParser.g4` `elBooleanExpr` lists `xor` before `or`, which ANTLR reads
    // as the tighter binding; the docs text wins.
    assert_eq!(
        syntax("a xor b or c")?,
        "(exclusive_disjunction a (disjunction b c))"
    );
    assert_eq!(syntax("a or b and c")?, "(disjunction a (conjunction b c))");
    assert_eq!(
        syntax("a implies b xor c")?,
        "(implication a (exclusive_disjunction b c))"
    );
    assert_eq!(syntax("not a and b")?, "(conjunction (not a) b)");
    Ok(())
}

#[test]
fn arithmetic_precedence_and_associativity_follow_the_grammar() -> Result<(), ElError> {
    assert_eq!(syntax("a + b * c = d")?, "(equal (add a (multiply b c)) d)");
    // `<assoc=right>` on the `^` alternative of `elArithmeticExpr`.
    assert_eq!(
        syntax("a ^ b ^ c = d")?,
        "(equal (exponent a (exponent b c)) d)"
    );
    Ok(())
}

#[test]
fn every_operator_maps_to_its_documented_function() -> Result<(), ElError> {
    // The three operator tables of `master05-expressions.adoc` §Primitive
    // Operators, which are what `EL_OPERATOR.call` carries.
    assert_eq!(syntax("a != b")?, "(not_equal a b)");
    assert_eq!(syntax("a \u{2260} b")?, "(not_equal a b)");
    assert_eq!(syntax("a <= b")?, "(less_than_or_equal a b)");
    assert_eq!(syntax("a \u{2265} b")?, "(greater_than_or_equal a b)");
    assert_eq!(syntax("a % b = c")?, "(equal (modulus a b) c)");
    // The word and symbol spellings `ElLexer.g4` declares.
    assert_eq!(syntax("a AND b")?, syntax("a \u{2227} b")?);
    assert_eq!(syntax("a implies b")?, syntax("a \u{21D2} b")?);
    assert_eq!(syntax("a implies b")?, syntax("a \u{2192} b")?);
    assert_eq!(syntax("NOT a")?, syntax("\u{00AC} a")?);
    Ok(())
}

#[test]
fn value_generators_scope_left_to_right() -> Result<(), ElError> {
    assert_eq!(syntax("Self.name")?, "Self.name");
    assert_eq!(syntax("a.b.c")?, "a.b.c");
    assert_eq!(syntax("f(x, y).g")?, "f(x,y).g");
    // `elScoper : '{' typeId '}' '.' …`
    assert_eq!(
        syntax("{VALIDITY_KIND}.disallowed = x")?,
        "(equal {VALIDITY_KIND}.disallowed x)"
    );
    assert_eq!(
        syntax("{Hash<String,Integer>}.count = 1")?,
        "(equal {Hash<String,Integer>}.count 1)"
    );
    Ok(())
}

#[test]
fn the_quantifiers_and_predicates_take_their_grammar_forms() -> Result<(), ElError> {
    // `elForAllExpr : SYM_FOR_ALL elLocalVariableId ':' elValueGenerator '¦'
    // elBooleanExpr` — the body separator is the BROKEN BAR.
    assert_eq!(
        syntax("for_all v : items \u{00A6} v.is_valid")?,
        "(for_all v items v.is_valid)"
    );
    assert_eq!(
        syntax("\u{2200} v : items \u{00A6} v.is_valid")?,
        "(for_all v items v.is_valid)"
    );
    assert_eq!(
        syntax("there_exists v : items \u{00A6} v.is_valid")?,
        "(there_exists v items v.is_valid)"
    );
    // `SYM_EXISTS elValueGenerator` — `ElLexer.g4` calls it the "Non-null
    // assertion operator", which is `EL_ATTACHED`.
    assert_eq!(syntax("exists uid")?, "(attached uid)");
    assert_eq!(syntax("\u{25A1} uid")?, "(attached uid)");
    Ok(())
}

#[test]
fn a_tuple_needs_at_least_two_items() -> Result<(), ElError> {
    // `elTuple : '[' elExpression ( ',' elExpression )+ ']'`.
    assert_eq!(syntax("[1, 2] = c")?, "(equal [1,2] c)");
    assert!(syntax("[1] = c").is_err());
    // `[name]` is one `LOCAL_TERM_CODE_REF` token, not a one-item tuple
    // (`ElLexer.g4` `LOCAL_TERM_CODE_REF : '[' ALPHANUM_US_CHAR+ ']'`).
    assert_eq!(syntax("[heart_rate] = c")?, "(equal [heart_rate] c)");
    Ok(())
}

#[test]
fn a_decision_table_is_refused_as_unsupported() {
    let error = syntax("choice in").expect_err("a decision table has no reader");
    assert!(
        matches!(error, ElError::Unsupported { .. }),
        "expected an Unsupported refusal, got {error}"
    );
}

#[test]
fn the_el_reading_reserves_only_what_el_lexer_declares() -> Result<(), ElError> {
    // `existence`, `occurrences` and `infinity` are cADL constraint keywords,
    // reachable in EL only inside a `matches { … }` block; in expression
    // position they are ordinary feature names.
    assert_eq!(
        syntax("existence.lower >= 0 and occurrences.upper <= 1")?,
        "(conjunction (greater_than_or_equal existence.lower 0) \
         (less_than_or_equal occurrences.upper 1))"
    );
    // `Self`, `Result`, `case`, `choice` and `assert` ARE declared there.
    assert_eq!(syntax("Self = Result")?, "(equal Self Result)");
    Ok(())
}

// ── the materialisation ──────────────────────────────────────────────────

/// A self-contained schema whose invariants exercise each resolution branch.
const ASSERTION_SCHEMA: &str = r#"
    bmm_version = <"2.4">
    rm_publisher = <"test">
    schema_name = <"assertions">
    rm_release = <"1.0.0">
    packages = <
        ["org.test"] = <
            name = <"org.test">
            classes = <"Integer", "Boolean", "String", "THING">
        >
    >
    primitive_types = <
        ["Integer"] = < name = <"Integer"> >
        ["Boolean"] = < name = <"Boolean"> >
        ["String"] = < name = <"String"> >
    >
    class_definitions = <
        ["THING"] = <
            name = <"THING">
            properties = <
                ["size"] = (P_BMM_SINGLE_PROPERTY) < name = <"size"> type = <"Integer"> >
            >
            constants = <
                ["Max_size"] = < name = <"Max_size"> type = <"Integer"> value = <"10"> >
            >
            functions = <
                ["is_valid"] = <
                    name = <"is_valid">
                    result = < type = <"Boolean"> >
                    post_conditions = <["Post"] = <"Result = True">>
                >
            >
            invariants = <
                ["Size_valid"] = <"size >= 0 and size <= Max_size">
                ["Self_typed"] = <"Self.is_valid">
                ["Unknown_name"] = <"Nowhere_constant = 1">
            >
        >
    >
"#;

fn assertions_model() -> Result<
    (
        openehr_lang::v1_1::bmm3::core::model::bmm_model::BmmModel,
        Vec<PBmmValidityFinding>,
    ),
    PBmmReadError,
> {
    create_bmm3_model_reporting(&read_schema(ASSERTION_SCHEMA)?)
}

#[test]
fn class_invariants_materialise_as_bmm_assertions() -> Result<(), PBmmReadError> {
    let (model, _) = assertions_model()?;
    let class = model
        .class_definitions
        .as_ref()
        .and_then(|map| map.get("THING"))
        .expect("THING is defined");
    let BmmClass::BmmSimpleClass(BmmSimpleClass::BmmSimpleClass(data)) = class else {
        panic!("THING is a plain simple class");
    };
    let invariants = data
        .invariants
        .as_ref()
        .expect("two invariants materialise");
    let tags: Vec<&str> = invariants.iter().filter_map(|a| a.tag.as_deref()).collect();
    assert_eq!(tags, vec!["Self_typed", "Size_valid"]);
    Ok(())
}

#[test]
fn a_bare_name_resolves_to_the_meta_type_its_declaration_gives() -> Result<(), PBmmReadError> {
    let (model, _) = assertions_model()?;
    let class = model
        .class_definitions
        .as_ref()
        .and_then(|map| map.get("THING"))
        .expect("THING is defined");
    let BmmClass::BmmSimpleClass(BmmSimpleClass::BmmSimpleClass(data)) = class else {
        panic!("THING is a plain simple class");
    };
    let invariants = data.invariants.as_ref().expect("invariants materialise");
    let size_valid = invariants
        .iter()
        .find(|a| a.tag.as_deref() == Some("Size_valid"))
        .expect("Size_valid materialises");
    // `size >= 0 and size <= Max_size`
    let ElExpression::ElBinaryOperator(conjunction) = &*size_valid.expression.base_expression
    else {
        panic!("the top node is the `and`");
    };
    assert_eq!(conjunction.call.name, "conjunction");
    let ElExpression::ElBinaryOperator(left) = &*conjunction.left_operand else {
        panic!("the left operand is a comparison");
    };
    assert_eq!(left.call.name, "greater_than_or_equal");
    assert!(
        matches!(&*left.left_operand, ElExpression::ElPropertyRef(p) if p.name == "size"),
        "a declared property resolves to EL_PROPERTY_REF"
    );
    assert!(
        matches!(&*left.right_operand, ElExpression::ElLiteral(_)),
        "an integer literal resolves to EL_LITERAL"
    );
    let ElExpression::ElBinaryOperator(right) = &*conjunction.right_operand else {
        panic!("the right operand is a comparison");
    };
    assert!(
        matches!(&*right.right_operand, ElExpression::ElStaticRef(s) if s.name == "Max_size"),
        "a declared constant resolves to EL_STATIC_REF"
    );
    // `Self.is_valid` — Self is the readonly variable, the scoped call the
    // declared function.
    let self_typed = invariants
        .iter()
        .find(|a| a.tag.as_deref() == Some("Self_typed"))
        .expect("Self_typed materialises");
    let ElExpression::ElFunctionCall(call) = &*self_typed.expression.base_expression else {
        panic!("the node is a scoped function call");
    };
    assert_eq!(call.name, "is_valid");
    assert!(
        call.agent.definition.is_some(),
        "the declared function resolves"
    );
    assert!(call.scoper.is_some(), "Self scopes the call");
    Ok(())
}

#[test]
fn an_unresolvable_name_is_a_collected_finding_not_a_refusal() -> Result<(), PBmmReadError> {
    let (_model, findings) = assertions_model()?;
    let rows: Vec<&PBmmValidityFinding> = findings.iter().collect();
    assert_eq!(
        rows.len(),
        1,
        "exactly one assertion does not materialise: {rows:?}"
    );
    let PBmmValidityFinding::AssertionNotMaterialised {
        class, kind, tag, ..
    } = rows[0]
    else {
        panic!("the finding is an unmaterialised assertion");
    };
    assert_eq!(class, "THING");
    assert_eq!(*kind, AssertionKind::Invariant);
    assert_eq!(tag, "Unknown_name");
    Ok(())
}

#[test]
fn a_routine_post_condition_resolves_result() -> Result<(), PBmmReadError> {
    let (model, _) = assertions_model()?;
    let class = model
        .class_definitions
        .as_ref()
        .and_then(|map| map.get("THING"))
        .expect("THING is defined");
    let BmmClass::BmmSimpleClass(BmmSimpleClass::BmmSimpleClass(data)) = class else {
        panic!("THING is a plain simple class");
    };
    let function = data
        .functions
        .as_ref()
        .and_then(|map| map.get("is_valid"))
        .expect("is_valid is declared");
    let post = function
        .post_conditions
        .as_ref()
        .expect("the post-condition materialises");
    assert_eq!(post.len(), 1);
    let ElExpression::ElBinaryOperator(equality) = &*post[0].expression.base_expression else {
        panic!("the node is `Result = True`");
    };
    assert!(
        matches!(&*equality.left_operand, ElExpression::ElWritableVariable(v) if v.name == "Result"),
        "Result resolves to EL_WRITABLE_VARIABLE over the routine's result type"
    );
    Ok(())
}
