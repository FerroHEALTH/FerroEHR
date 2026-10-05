// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written surface of the BMM **v3** literal-value package — and the home
//! of its evaluation boundary.
//!
//! Spec: `LANG/docs/bmm3/master09-core-values.adoc` (§General Model, §Container
//! Literals) plus the class definitions
//! `org.openehr.lang.bmm3.bmm_literal_value.adoc`,
//! `…bmm3.bmm_primitive_value.adoc`, `…bmm3.bmm_container_value.adoc`,
//! `…bmm3.bmm_indexed_container_value.adoc`, `…bmm3.bmm_interval_value.adoc`.
//!
//! NOTE: nothing in this workspace EVALUATES a BMM literal — P_BMM carries
//! constant values as opaque strings (`P_BMM_CONSTANT`,
//! `LANG/docs/bmm_persistence/master04-syntax.adoc`) and no crate interprets
//! a BMM model at runtime — so the package's nine classes exist as the
//! complete emitted model with data accessors only: an honest maturity
//! boundary, not a model gap.
//!
//! What that boundary defers is exactly one behaviour, named here so a future
//! evaluator does not have to re-derive it:
//!
//! TODO(#1920): implement the `_value_literal_` → `_value_` deserialisation. §General
//! Model states that `_value_literal_` "is assumed to carry a serialised form of
//! the value expressed in a syntax known to the model processing environment",
//! and `…bmm3.bmm_literal_value.adoc` §Attributes describes `value` as "A native
//! representation of the value, possibly derived by deserialising
//! `_value_literal_`" — i.e. the two fields are related by a parse this crate
//! does not perform, so a literal read from a schema populates only
//! `_value_literal_`. The formalism to parse it in is already resolved by
//! [`BmmLiteralValue::syntax`] (which applies the `json` default), and the
//! stringified form the enumeration name map needs is
//! [`BmmLiteralValue::value_literal`] — the parse itself is the missing half.

use crate::v1_1::bmm3::core::literal_value::bmm_literal_value::BmmLiteralValue;
use crate::v1_1::bmm3::core::literal_value::bmm_primitive_value::BmmPrimitiveValue;

impl BmmPrimitiveValue {
    /// `BMM_LITERAL_VALUE.value_literal`: "A serial representation of the value"
    /// (`org.openehr.lang.bmm3.bmm_literal_value.adoc` §Attributes), for any
    /// primitive-literal leaf.
    #[must_use]
    pub fn value_literal(&self) -> &str {
        match self {
            Self::BmmBooleanValue(v) => v.value_literal.as_str(),
            Self::BmmIntegerValue(v) => v.value_literal.as_str(),
            Self::BmmStringValue(v) => v.value_literal.as_str(),
            Self::BmmPrimitiveValue(v) => v.value_literal.as_str(),
        }
    }
}

impl BmmLiteralValue {
    /// `BMM_LITERAL_VALUE.value_literal`: "A serial representation of the value"
    /// (`org.openehr.lang.bmm3.bmm_literal_value.adoc` §Attributes), for any
    /// literal-value leaf.
    #[must_use]
    pub fn value_literal(&self) -> &str {
        match self {
            Self::BmmContainerValue(v) => v.value_literal.as_str(),
            Self::BmmIndexedContainerValue(v) => v.value_literal.as_str(),
            Self::BmmIntervalValue(v) => v.value_literal.as_str(),
            Self::BmmPrimitiveValue(v) => v.value_literal(),
        }
    }

    /// The formalism of this literal's `_value_literal_`, with the spec's default
    /// applied: "If not set, `json` is assumed"
    /// (`org.openehr.lang.bmm3.bmm_literal_value.adoc` §Attributes).
    #[must_use]
    pub fn syntax(&self) -> &str {
        let stated = match self {
            Self::BmmContainerValue(v) => v.syntax.as_deref(),
            Self::BmmIndexedContainerValue(v) => v.syntax.as_deref(),
            Self::BmmIntervalValue(v) => v.syntax.as_deref(),
            Self::BmmPrimitiveValue(v) => match v {
                BmmPrimitiveValue::BmmBooleanValue(v) => v.syntax.as_deref(),
                BmmPrimitiveValue::BmmIntegerValue(v) => v.syntax.as_deref(),
                BmmPrimitiveValue::BmmStringValue(v) => v.syntax.as_deref(),
                BmmPrimitiveValue::BmmPrimitiveValue(v) => v.syntax.as_deref(),
            },
        };
        stated.unwrap_or("json")
    }
}
