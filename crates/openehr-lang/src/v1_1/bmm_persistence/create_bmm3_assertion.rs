// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The persisted assertion string → **v3** `BMM_ASSERTION` transform.
//!
//! P_BMM persists a class invariant and a routine pre-/post-condition as an
//! opaque expression STRING keyed by tag (`…bmm_persistence.p_bmm_class.adoc`
//! §Attributes), while v3 requires a `BMM_ASSERTION` whose `expression` is a
//! `1..1` `EL_BOOLEAN_EXPRESSION` (`LANG/docs/bmm3/master10-expressions.adoc`
//! §Usage in BMM Models; `…bmm3.bmm_assertion.adoc` §Attributes). This module is
//! the bridge: the [`crate::v1_1::el`] parser over the vendored EL grammar,
//! driving a builder that materialises the v3 `EL_*` classes.
//!
//! An assertion that does not parse, or whose names do not resolve, is NOT a
//! refusal of the schema: it is collected as a
//! [`PBmmValidityFinding::AssertionNotMaterialised`] and omitted from the class,
//! staying readable in the P_BMM graph.
//!
//! # Resolution rules
//!
//! `ElParser.g4` states its own ambiguity at `elValueGenerator` — "Can't
//! syntactically distinguish between a local variable and a property or
//! constant reference" — so a bare name's meta-type is decided by resolution,
//! against the DECLARED features of the owning class:
//!
//! * a lower-case name matching a declared property → `EL_PROPERTY_REF`;
//! * a lower-case name matching a declared function → `EL_FUNCTION_CALL` with
//!   its `EL_FUNCTION_AGENT.definition` set;
//! * any other lower-case name → `EL_FUNCTION_CALL` with no definition, the
//!   reading `elBareRef`'s `elFunctionCall` alternative gives on its own
//!   (`EL_FUNCTION_AGENT.definition` is `0..1`);
//! * an upper-case name matching a declared static → `EL_STATIC_REF`, else a
//!   class of the model → `EL_TYPE_REF`, else unresolved.
//!
//! Inherited features are not in scope here: `BMM_CLASS.properties` carries
//! only what the class declares ("Properties defined in this class",
//! `…bmm3.bmm_class.adoc` §Attributes), and the flattened view is the
//! model-level query `BMM_CLASS.flat_features`.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::v1_1::bmm_persistence::create_bmm3_model::build_named_unitary_type;
use crate::v1_1::bmm_persistence::create_model::Builder;
use crate::v1_1::bmm_persistence::p_bmm_class::PBmmClass;
use crate::v1_1::bmm_persistence::validate::AssertionKind;
use crate::v1_1::bmm_persistence::validate::PBmmValidityFinding;
use crate::v1_1::bmm3::core::entity::bmm_tuple_type::BmmTupleType;
use crate::v1_1::bmm3::core::entity::bmm_type::BmmType;
use crate::v1_1::bmm3::core::entity::bmm_unitary_type::BmmUnitaryType;
use crate::v1_1::bmm3::core::feature::bmm_function::BmmFunction;
use crate::v1_1::bmm3::core::feature::bmm_property::BmmProperty;
use crate::v1_1::bmm3::core::feature::bmm_readonly_variable::BmmReadonlyVariable;
use crate::v1_1::bmm3::core::feature::bmm_result::BmmResult;
use crate::v1_1::bmm3::core::feature::bmm_self::BmmSelf;
use crate::v1_1::bmm3::core::feature::bmm_static::BmmStatic;
use crate::v1_1::bmm3::core::feature::bmm_writable_variable::BmmWritableVariable;
use crate::v1_1::bmm3::core::literal_value::bmm_literal_value::BmmLiteralValue;
use crate::v1_1::bmm3::core::literal_value::bmm_primitive_value::BmmPrimitiveValue;
use crate::v1_1::bmm3::core::literal_value::bmm_primitive_value::BmmPrimitiveValueData;
use crate::v1_1::bmm3::expression::el_attached::ElAttached;
use crate::v1_1::bmm3::expression::el_binary_operator::ElBinaryOperator;
use crate::v1_1::bmm3::expression::el_boolean_expression::ElBooleanExpression;
use crate::v1_1::bmm3::expression::el_expression::ElExpression;
use crate::v1_1::bmm3::expression::el_function_agent::ElFunctionAgent;
use crate::v1_1::bmm3::expression::el_function_call::ElFunctionCall;
use crate::v1_1::bmm3::expression::el_literal::ElLiteral;
use crate::v1_1::bmm3::expression::el_property_ref::ElPropertyRef;
use crate::v1_1::bmm3::expression::el_readonly_variable::ElReadonlyVariable;
use crate::v1_1::bmm3::expression::el_static_ref::ElStaticRef;
use crate::v1_1::bmm3::expression::el_tuple::ElTuple;
use crate::v1_1::bmm3::expression::el_tuple_item::ElTupleItem;
use crate::v1_1::bmm3::expression::el_type_ref::ElTypeRef;
use crate::v1_1::bmm3::expression::el_unary_operator::ElUnaryOperator;
use crate::v1_1::bmm3::expression::el_value_generator::ElValueGenerator;
use crate::v1_1::bmm3::expression::el_writable_variable::ElWritableVariable;
use crate::v1_1::bmm3::statement::bmm_assertion::BmmAssertion;
use crate::v1_1::el::ElBuilder;
use crate::v1_1::el::ElError;
use crate::v1_1::el::ElOperator;
use crate::v1_1::el::parse_boolean_expression_with;

/// What a bare name may resolve against while an assertion of one class is
/// materialised.
pub(super) struct AssertionScope<'a, 'b> {
    /// The shared class index, for type and class-name resolution.
    pub(super) builder: &'a Builder<'b>,
    /// The class the assertion belongs to.
    pub(super) owner: &'a PBmmClass,
    /// The class's declared properties, as already materialised.
    pub(super) properties: Option<&'a BTreeMap<String, BmmProperty>>,
    /// The class's declared constants/singletons, as already materialised.
    pub(super) statics: Option<&'a BTreeMap<String, BmmStatic>>,
    /// The class's declared functions, as already materialised.
    pub(super) functions: Option<&'a BTreeMap<String, BmmFunction>>,
    /// The enclosing routine's result type, when the assertion is a
    /// pre-/post-condition of a function; `Result` is unresolvable without it.
    pub(super) result_type: Option<&'a BmmType>,
}

/// Materialises every persisted assertion of `source`, collecting a finding for
/// each one that does not parse or does not resolve.
///
/// `routine` names the owning function/procedure for a pre-/post-condition, and
/// is `None` for a class invariant.
pub(super) fn build_assertions(
    scope: &AssertionScope<'_, '_>,
    kind: AssertionKind,
    routine: Option<&str>,
    source: Option<&BTreeMap<String, String>>,
    findings: &mut Vec<PBmmValidityFinding>,
) -> Vec<BmmAssertion> {
    let mut out = Vec::new();
    for (tag, expression) in source.into_iter().flatten() {
        match build_assertion(scope, tag, expression) {
            Ok(assertion) => out.push(assertion),
            Err(error) => findings.push(PBmmValidityFinding::AssertionNotMaterialised {
                class: scope.owner.name().to_owned(),
                routine: routine.map(ToOwned::to_owned),
                kind,
                tag: tag.clone(),
                expression: expression.clone(),
                reason: error.to_string(),
            }),
        }
    }
    out
}

/// Parses one tagged assertion into a `BMM_ASSERTION`.
fn build_assertion(
    scope: &AssertionScope<'_, '_>,
    tag: &str,
    expression: &str,
) -> Result<BmmAssertion, ElError> {
    let mut builder = Bmm3ElBuilder {
        scope,
        visiting: BTreeSet::new(),
    };
    let parsed = parse_boolean_expression_with(expression, &mut builder)?;
    Ok(BmmAssertion {
        // `EL_BOOLEAN_EXPRESSION` is the `EL_CONSTRAINED` form whose
        // `base_expression` is the first-order expression it constrains to
        // Boolean (`org.openehr.lang.bmm3.el_boolean_expression.adoc`).
        expression: ElBooleanExpression {
            base_expression: Box::new(parsed),
        },
        tag: Some(tag.to_owned()),
    })
}

/// The [`ElBuilder`] that materialises the v3 `EL_*` expression classes.
struct Bmm3ElBuilder<'a, 'b, 'c> {
    scope: &'a AssertionScope<'b, 'c>,
    visiting: BTreeSet<String>,
}

impl Bmm3ElBuilder<'_, '_, '_> {
    /// The `BMM_TYPE` of a named class of the schema.
    fn named_type(&mut self, name: &str, at: usize) -> Result<BmmUnitaryType, ElError> {
        let context = format!("class `{}` assertion", self.scope.owner.name());
        build_named_unitary_type(
            self.scope.builder,
            &context,
            name,
            self.scope.owner,
            &mut self.visiting,
        )
        .map_err(|error| ElError::Unresolved {
            at,
            name: name.to_owned(),
            message: error.to_string(),
        })
    }

    /// A declared feature of the owning class, by name.
    fn declared_property(&self, name: &str) -> Option<&BmmProperty> {
        self.scope.properties.and_then(|map| map.get(name))
    }

    fn declared_static(&self, name: &str) -> Option<&BmmStatic> {
        self.scope.statics.and_then(|map| map.get(name))
    }

    fn declared_function(&self, name: &str) -> Option<&BmmFunction> {
        self.scope.functions.and_then(|map| map.get(name))
    }

    /// Builds the `EL_FUNCTION_CALL` an operator is equivalent to.
    ///
    /// `EL_OPERATOR.call` is `1..1` — "Function call equivalent to this
    /// operator expression, inferred by matching operator against functions
    /// defined in interface of principal operand"
    /// (`org.openehr.lang.bmm3.el_binary_operator.adoc` §Attributes) — and the
    /// EL operator tables name that function
    /// (`LANG/docs/EL/master05-expressions.adoc` §Primitive Operators). The
    /// agent's `definition` stays unset because the principal operand's
    /// interface is a typed-evaluation result, not a parse product.
    fn operator_call(operator: ElOperator) -> ElFunctionCall {
        ElFunctionCall {
            is_writable: false,
            name: operator.function.to_owned(),
            scoper: None,
            agent: Box::new(ElFunctionAgent {
                is_writable: false,
                name: operator.function.to_owned(),
                scoper: None,
                closed_args: None,
                open_args: None,
                definition: None,
            }),
        }
    }
}

/// Whether a readonly-variable reference is the `Self` variable.
fn variable_is_self(variable: &ElReadonlyVariable) -> bool {
    matches!(variable.definition, BmmReadonlyVariable::BmmSelf(_))
}

/// Lifts an expression into the `EL_VALUE_GENERATOR` union a scoper must be
/// ("Scoping expression, which must be a `EL_VALUE_GENERATOR`",
/// `org.openehr.lang.bmm3.el_feature_ref.adoc` §Attributes).
fn as_value_generator(expression: ElExpression) -> Option<ElValueGenerator> {
    match expression {
        ElExpression::ElFunctionAgent(agent) => {
            Some(ElValueGenerator::ElFunctionAgent(Box::new(agent)))
        }
        ElExpression::ElFunctionCall(call) => {
            Some(ElValueGenerator::ElFunctionCall(Box::new(call)))
        }
        ElExpression::ElProcedureAgent(agent) => {
            Some(ElValueGenerator::ElProcedureAgent(Box::new(agent)))
        }
        ElExpression::ElPropertyRef(reference) => {
            Some(ElValueGenerator::ElPropertyRef(Box::new(reference)))
        }
        ElExpression::ElReadonlyVariable(variable) => {
            Some(ElValueGenerator::ElReadonlyVariable(variable))
        }
        ElExpression::ElStaticRef(reference) => {
            Some(ElValueGenerator::ElStaticRef(Box::new(reference)))
        }
        ElExpression::ElTypeRef(reference) => Some(ElValueGenerator::ElTypeRef(reference)),
        ElExpression::ElWritableVariable(variable) => {
            Some(ElValueGenerator::ElWritableVariable(variable))
        }
        _ => None,
    }
}

impl ElBuilder for Bmm3ElBuilder<'_, '_, '_> {
    type Expr = ElExpression;

    fn literal(
        &mut self,
        literal: crate::v1_1::el::ElLiteral,
        at: usize,
    ) -> Result<ElExpression, ElError> {
        let BmmUnitaryType::BmmSimpleType(simple) = self.named_type(literal.type_name(), at)?
        else {
            return Err(ElError::Unresolved {
                at,
                name: literal.type_name().to_owned(),
                message: "a literal's type must be a BMM_SIMPLE_TYPE".to_owned(),
            });
        };
        Ok(ElExpression::ElLiteral(ElLiteral {
            // The native `value` is left unset, per the literal-evaluation
            // boundary of
            // [`crate::v1_1::bmm3::core::literal_value::bmm_literal_value_impl`];
            // `_syntax_` unset means the `json` default applies
            // (`…bmm3.bmm_literal_value.adoc` §Attributes).
            value: BmmLiteralValue::BmmPrimitiveValue(BmmPrimitiveValue::BmmPrimitiveValue(
                BmmPrimitiveValueData {
                    value_literal: literal.value_literal(),
                    value: None,
                    syntax: None,
                    r#type: simple,
                },
            )),
        }))
    }

    fn self_ref(&mut self, at: usize) -> Result<ElExpression, ElError> {
        let owner = self.scope.owner.name().to_owned();
        let r#type = BmmType::from(self.named_type(&owner, at)?);
        Ok(ElExpression::ElReadonlyVariable(ElReadonlyVariable {
            is_writable: false,
            name: "Self".to_owned(),
            definition: BmmReadonlyVariable::BmmSelf(BmmSelf {
                name: "Self".to_owned(),
                documentation: None,
                extensions: None,
                r#type,
                is_nullable: Some(false),
            }),
        }))
    }

    fn result_ref(&mut self, at: usize) -> Result<ElExpression, ElError> {
        let Some(r#type) = self.scope.result_type.cloned() else {
            return Err(ElError::Unresolved {
                at,
                name: "Result".to_owned(),
                message: "`Result` is declared only on entry to a function \
                          (LANG/docs/EL/master04-terminal_entities.adoc §Result)"
                    .to_owned(),
            });
        };
        Ok(ElExpression::ElWritableVariable(ElWritableVariable {
            is_writable: true,
            name: "Result".to_owned(),
            definition: BmmWritableVariable::BmmResult(BmmResult {
                name: "Result".to_owned(),
                documentation: None,
                extensions: None,
                r#type,
                is_nullable: None,
            }),
        }))
    }

    fn bound_variable(&mut self, name: &str, at: usize) -> Result<ElExpression, ElError> {
        Err(ElError::Unsupported {
            at,
            message: format!(
                "the data-bound variable `${name}` has no BMM meta-type \
                 (`ElParser.g4` elBoundVariableId carries an upstream TODO)"
            ),
        })
    }

    fn type_ref(&mut self, type_id: &str, at: usize) -> Result<ElExpression, ElError> {
        let r#type = BmmType::from(self.named_type(type_id, at)?);
        Ok(ElExpression::ElTypeRef(ElTypeRef {
            is_writable: false,
            name: type_id.to_owned(),
            r#type,
            is_mutable: false,
        }))
    }

    fn feature_ref(
        &mut self,
        scoper: Option<ElExpression>,
        name: &str,
        args: Option<Vec<ElExpression>>,
        at: usize,
    ) -> Result<ElExpression, ElError> {
        // `Self` denotes the owning class, so a `Self.`-scoped name resolves
        // against exactly the features an unscoped one does
        // (`LANG/docs/EL/master04-terminal_entities.adoc` §Self).
        let owner_scoped = match &scoper {
            None => true,
            Some(ElExpression::ElReadonlyVariable(variable)) => variable_is_self(variable),
            Some(_) => false,
        };
        let scoper = match scoper {
            None => None,
            Some(expression) => match as_value_generator(expression) {
                Some(generator) => Some(Box::new(generator)),
                None => {
                    return Err(ElError::Unsupported {
                        at,
                        message: format!("the scoper of `{name}` is not an EL_VALUE_GENERATOR"),
                    });
                }
            },
        };
        if name.starts_with(|c: char| c.is_ascii_uppercase()) {
            if args.is_some() {
                return Err(ElError::Parse {
                    at,
                    message: format!("`{name}` is an elConstantId and takes no arguments"),
                });
            }
            if owner_scoped && let Some(definition) = self.declared_static(name) {
                return Ok(ElExpression::ElStaticRef(ElStaticRef {
                    is_writable: false,
                    name: name.to_owned(),
                    scoper,
                    definition: definition.clone(),
                }));
            }
            if scoper.is_none()
                && self
                    .scope
                    .builder
                    .classes
                    .contains_key(&name.to_uppercase())
            {
                return self.type_ref(name, at);
            }
            return Err(ElError::Unresolved {
                at,
                name: name.to_owned(),
                message: "no declared static property and no class of that name".to_owned(),
            });
        }
        if owner_scoped
            && args.is_none()
            && let Some(definition) = self.declared_property(name)
        {
            return Ok(ElExpression::ElPropertyRef(ElPropertyRef {
                is_writable: true,
                name: name.to_owned(),
                scoper,
                definition: definition.clone(),
            }));
        }
        let definition = owner_scoped
            .then(|| self.declared_function(name).cloned())
            .flatten();
        Ok(ElExpression::ElFunctionCall(ElFunctionCall {
            is_writable: false,
            name: name.to_owned(),
            scoper,
            agent: Box::new(ElFunctionAgent {
                is_writable: false,
                name: name.to_owned(),
                scoper: None,
                closed_args: args.map(tuple_of),
                open_args: None,
                definition,
            }),
        }))
    }

    fn binary(
        &mut self,
        operator: ElOperator,
        left: ElExpression,
        right: ElExpression,
        _at: usize,
    ) -> Result<ElExpression, ElError> {
        Ok(ElExpression::ElBinaryOperator(Box::new(ElBinaryOperator {
            precedence_overridden: None,
            symbol: Some(operator.symbol.to_owned()),
            call: Self::operator_call(operator),
            left_operand: Box::new(left),
            right_operand: Box::new(right),
        })))
    }

    fn unary(
        &mut self,
        operator: ElOperator,
        operand: ElExpression,
        _at: usize,
    ) -> Result<ElExpression, ElError> {
        Ok(ElExpression::ElUnaryOperator(Box::new(ElUnaryOperator {
            precedence_overridden: None,
            symbol: Some(operator.symbol.to_owned()),
            call: Self::operator_call(operator),
            operand: Box::new(operand),
        })))
    }

    fn attached(&mut self, operand: ElExpression, at: usize) -> Result<ElExpression, ElError> {
        let Some(generator) = as_value_generator(operand) else {
            return Err(ElError::Unsupported {
                at,
                message: "the `exists` operand is not an EL_VALUE_GENERATOR".to_owned(),
            });
        };
        Ok(ElExpression::ElAttached(ElAttached { operand: generator }))
    }

    fn quantified(
        &mut self,
        universal: bool,
        variable: &str,
        _collection: ElExpression,
        _condition: ElExpression,
        at: usize,
    ) -> Result<ElExpression, ElError> {
        let keyword = if universal { "for_all" } else { "there_exists" };
        Err(ElError::Unsupported {
            at,
            message: format!(
                "`{keyword} {variable}` maps to a container function taking a Function agent \
                 (`LANG/docs/bmm3/master10-expressions.adoc` §Existential and Universal \
                 Quantifier Invariants), whose signature this reader does not infer"
            ),
        })
    }

    fn tuple(&mut self, items: Vec<ElExpression>, _at: usize) -> Result<ElExpression, ElError> {
        Ok(ElExpression::ElTuple(tuple_of(items)))
    }

    fn constraint(&mut self, raw: &str, at: usize) -> Result<ElExpression, ElError> {
        Err(ElError::Unsupported {
            at,
            message: format!(
                "the `matches {raw}` right-hand side is a cADL object matcher \
                 (`ElParser.g4` imports Cadl2Parser), which has no EL meta-type"
            ),
        })
    }
}

/// An `EL_TUPLE` over `items`.
///
/// NOTE: `EL_TUPLE.type` is "Static type inferred from literal value"
/// (`org.openehr.lang.bmm3.el_tuple.adoc` §Attributes) — an inference over
/// evaluated types, not a parse product, so the item types stay empty while the
/// items themselves are carried in full.
fn tuple_of(items: Vec<ElExpression>) -> ElTuple {
    ElTuple {
        items: Some(
            items
                .into_iter()
                .map(|item| ElTupleItem {
                    item: Some(item),
                    name: None,
                })
                .collect(),
        ),
        r#type: BmmTupleType {
            item_types: BTreeMap::new(),
        },
    }
}
