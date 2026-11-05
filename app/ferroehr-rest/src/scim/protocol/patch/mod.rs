// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The PATCH request (RFC 7644 §3.5.2): the `PatchOp` message, its `add`,
//! `remove` and `replace` operations, and their application to a resource.
//!
//! [`PatchRequest::parse`] checks the message; [`PatchRequest::apply`] runs
//! the operations in order on a copy of the resource and returns the copy
//! only when every operation succeeds, since a PATCH "SHALL be treated as
//! atomic".

mod apply;

use std::fmt;

use serde_json::Value;

use crate::scim::protocol::deviation::{self, Deviation};
use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::message;
use crate::scim::protocol::path::PatchPath;
use crate::scim::protocol::resource::Resource;
use crate::scim::protocol::urn;

/// A PATCH operation type (RFC 7644 §3.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatchOp {
    /// `add` (§3.5.2.1).
    Add,
    /// `remove` (§3.5.2.2).
    Remove,
    /// `replace` (§3.5.2.3).
    Replace,
}

impl PatchOp {
    /// Returns the operation as RFC 7644 §3.5.2 spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Replace => "replace",
        }
    }
}

/// One operation of a PATCH request.
#[derive(Clone, PartialEq)]
pub struct PatchOperation {
    /// The operation type.
    pub op: PatchOp,
    /// The target; `None` is the resource itself.
    pub path: Option<PatchPath>,
    /// The value, which `add` and `replace` require.
    pub value: Option<Value>,
}

/// A PATCH request checked against RFC 7644 §3.5.2.
#[derive(Clone, PartialEq)]
pub struct PatchRequest {
    operations: Vec<PatchOperation>,
    deviations: Vec<Deviation>,
}

/// The outcome of a PATCH request applied to a resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchOutcome {
    resource: Resource,
    changed: bool,
}

impl PatchOutcome {
    /// Returns the resource after every operation.
    #[must_use]
    pub const fn resource(&self) -> &Resource {
        &self.resource
    }

    /// Returns the resource, consuming the outcome.
    #[must_use]
    pub fn into_resource(self) -> Resource {
        self.resource
    }

    /// Returns whether the operations changed the resource; a request whose
    /// target already held each value changes nothing (§3.5.2.1).
    #[must_use]
    pub const fn changed(&self) -> bool {
        self.changed
    }
}

impl PatchRequest {
    /// Parses a PATCH request body.
    ///
    /// # Errors
    ///
    /// Returns a `400` with `invalidSyntax` when the body is not a `PatchOp`
    /// message with one or more operations each carrying exactly one valid
    /// `op`; `noTarget` for a `remove` without a `path` (§3.5.2.2);
    /// `invalidValue` for an `add` or `replace` without a `value`; and the
    /// refusals of [`PatchPath::parse`] for a malformed `path`.
    pub fn parse(body: &Value) -> Result<Self, ScimError> {
        let Value::Object(object) = body else {
            return Err(ScimError::bad_request(
                ScimType::InvalidSyntax,
                "A PATCH body is a JSON object.",
            ));
        };
        let [schemas, operations] =
            message::members(object, ["schemas", "Operations"], "PatchOp message")?;
        message::require_schema(schemas, urn::PATCH_OP, "PatchOp message")?;
        let Some(operations) = operations
            .and_then(Value::as_array)
            .filter(|operations| !operations.is_empty())
        else {
            return Err(ScimError::bad_request(
                ScimType::InvalidSyntax,
                "A PatchOp message carries 'Operations', an array of one or more operations.",
            ));
        };
        let mut deviations = Vec::new();
        let operations = operations
            .iter()
            .map(|operation| parse_operation(operation, &mut deviations))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            operations,
            deviations,
        })
    }

    /// Returns the operations, in the order they apply.
    #[must_use]
    pub fn operations(&self) -> &[PatchOperation] {
        &self.operations
    }

    /// Returns the client deviations the request needed, one per use.
    #[must_use]
    pub fn deviations(&self) -> &[Deviation] {
        &self.deviations
    }
}

fn parse_operation(
    operation: &Value,
    deviations: &mut Vec<Deviation>,
) -> Result<PatchOperation, ScimError> {
    let Value::Object(object) = operation else {
        return Err(ScimError::bad_request(
            ScimType::InvalidSyntax,
            "A PATCH operation is a JSON object.",
        ));
    };
    let [op, path, value] = message::members(object, ["op", "path", "value"], "PATCH operation")?;
    let op = match op.and_then(Value::as_str) {
        Some("add") => PatchOp::Add,
        Some("remove") => PatchOp::Remove,
        Some("replace") => PatchOp::Replace,
        Some(other) => {
            let name = deviation::operation_in_any_case(other).ok_or_else(|| {
                ScimError::bad_request(
                    ScimType::InvalidSyntax,
                    format!("'{other}' is not a PATCH operation (RFC 7644 §3.5.2)."),
                )
            })?;
            deviations.push(Deviation::OperationCase);
            match name {
                "add" => PatchOp::Add,
                "remove" => PatchOp::Remove,
                _ => PatchOp::Replace,
            }
        }
        None => {
            return Err(ScimError::bad_request(
                ScimType::InvalidSyntax,
                "A PATCH operation carries exactly one 'op' string.",
            ));
        }
    };
    let path = match path {
        None => None,
        Some(Value::String(text)) => Some(PatchPath::parse(text)?),
        Some(_) => {
            return Err(ScimError::bad_request(
                ScimType::InvalidPath,
                "A PATCH 'path' is a string.",
            ));
        }
    };
    match (op, &path, value) {
        (PatchOp::Remove, None, _) => {
            return Err(ScimError::bad_request(
                ScimType::NoTarget,
                "A remove operation carries a 'path' (RFC 7644 §3.5.2.2).",
            ));
        }
        (PatchOp::Remove, Some(_), Some(_)) => deviations.push(Deviation::RemoveValueList),
        (PatchOp::Add | PatchOp::Replace, _, None) => {
            return Err(ScimError::bad_request(
                ScimType::InvalidValue,
                format!(
                    "An {} operation carries a 'value' (RFC 7644 §3.5.2.1).",
                    op.as_str()
                ),
            ));
        }
        _ => {}
    }
    Ok(PatchOperation {
        op,
        path,
        value: value.cloned(),
    })
}

// NOTE: a PATCH value or path filter carries personal data, so Debug output shows the operation type only.
impl fmt::Debug for PatchOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PatchOperation")
            .field("op", &self.op)
            .field("has_path", &self.path.is_some())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for PatchRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PatchRequest")
            .field("operations", &self.operations)
            .field("deviations", &self.deviations)
            .finish()
    }
}
