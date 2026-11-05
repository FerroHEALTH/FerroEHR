// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The subset of the filter grammar a provider supports (RFC 7644 §3.4.2.2:
//! "Providers MUST decline to filter results if the specified filter
//! operation is not recognized and return an HTTP 400 error with a "scimType"
//! error of "invalidFilter"").

use super::eval::each_leaf;
use super::{CompareOp, Filter, FilterFeature, FilterSupport, Logic, Term, Test};
use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::path::{AttrPath, PathError};
use crate::scim::protocol::urn;

impl FilterSupport {
    /// Creates the support for `eq` and `and` over `attributes`, attribute
    /// paths of the resource type whose base schema is `base_schema`.
    ///
    /// # Errors
    ///
    /// Returns [`PathError`] when an entry of `attributes` is not an attribute path.
    pub fn equality_conjunction(base_schema: &str, attributes: &[&str]) -> Result<Self, PathError> {
        Ok(Self {
            base_schema: base_schema.to_owned(),
            features: vec![FilterFeature::Operator(CompareOp::Eq)],
            attributes: attributes
                .iter()
                .map(|attribute| AttrPath::parse(attribute))
                .collect::<Result<_, _>>()?,
        })
    }

    /// Returns the support with `feature` added.
    #[must_use]
    pub fn allow(mut self, feature: FilterFeature) -> Self {
        if !self.features.contains(&feature) {
            self.features.push(feature);
        }
        self
    }

    fn allows(&self, feature: FilterFeature) -> bool {
        self.features.contains(&feature)
    }

    /// Refuses a filter that uses a feature or an attribute this support lacks.
    ///
    /// # Errors
    ///
    /// Returns a `400` with `scimType` `invalidFilter` naming the first
    /// unsupported operator ("The operator 'co' is not supported.") or
    /// attribute.
    pub fn check(&self, filter: &Filter) -> Result<(), ScimError> {
        match filter {
            Logic::Leaf(Term::Attr(expression)) => {
                self.check_attribute(&expression.path)?;
                self.check_test(&expression.test)
            }
            Logic::Leaf(Term::ValuePath(path, inner)) => {
                if !self.allows(FilterFeature::ValuePaths) {
                    return Err(attribute_unsupported(path));
                }
                self.check_attribute(path)?;
                each_leaf(inner, &mut |expression| self.check_test(&expression.test))
            }
            Logic::And(terms) => terms.iter().try_for_each(|term| self.check(term)),
            Logic::Or(terms) if self.allows(FilterFeature::Or) => {
                terms.iter().try_for_each(|term| self.check(term))
            }
            Logic::Or(_) => Err(operator_unsupported("or")),
            Logic::Not(inner) if self.allows(FilterFeature::Not) => self.check(inner),
            Logic::Not(_) => Err(operator_unsupported("not")),
        }
    }

    fn check_attribute(&self, path: &AttrPath) -> Result<(), ScimError> {
        let qualified_by_base = path
            .schema
            .as_deref()
            .is_none_or(|schema| urn::same(schema, &self.base_schema));
        let supported = qualified_by_base
            && self.attributes.iter().any(|attribute| {
                attribute.name == path.name && attribute.sub_attr == path.sub_attr
            });
        if supported {
            Ok(())
        } else {
            Err(attribute_unsupported(path))
        }
    }

    fn check_test(&self, test: &Test) -> Result<(), ScimError> {
        match test {
            Test::Present if self.allows(FilterFeature::Present) => Ok(()),
            Test::Present => Err(operator_unsupported("pr")),
            Test::Compare(op, _) if self.allows(FilterFeature::Operator(*op)) => Ok(()),
            Test::Compare(op, _) => Err(operator_unsupported(op.as_str())),
        }
    }
}

fn operator_unsupported(operator: &str) -> ScimError {
    ScimError::bad_request(
        ScimType::InvalidFilter,
        format!("The operator '{operator}' is not supported."),
    )
}

fn attribute_unsupported(path: &AttrPath) -> ScimError {
    ScimError::bad_request(
        ScimType::InvalidFilter,
        format!("The attribute '{path}' is not supported in a filter."),
    )
}
