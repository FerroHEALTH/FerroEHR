// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SCIM error response (RFC 7644 §3.12).
//!
//! A [`ScimError`] is the refusal the layer returns: an HTTP status, the
//! `scimType` keyword of Table 9 where one applies, and a human-readable
//! detail. [`ScimError::body`] is its wire form, the `Error` message whose
//! `status` is the code "expressed as a JSON string".

use std::fmt;

use http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::scim::protocol::urn;

/// A SCIM detail error keyword (RFC 7644 §3.12, Table 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScimType {
    /// The filter syntax was invalid, or the attribute and comparison are not supported.
    InvalidFilter,
    /// The filter yields more results than the provider is willing to process.
    TooMany,
    /// An attribute value is already in use or reserved.
    Uniqueness,
    /// The modification is not compatible with the attribute's mutability.
    Mutability,
    /// The request body did not conform to the request schema.
    InvalidSyntax,
    /// The PATCH `path` was invalid or malformed.
    InvalidPath,
    /// The PATCH `path` yielded no attribute or value to operate on.
    NoTarget,
    /// A required value was missing, or a value was not compatible with the attribute.
    InvalidValue,
    /// The SCIM protocol version is not supported.
    InvalidVers,
    /// The request carried sensitive information in its URI.
    Sensitive,
}

impl ScimType {
    /// Every keyword, in the order of Table 9.
    pub const ALL: [Self; 10] = [
        Self::InvalidFilter,
        Self::TooMany,
        Self::Uniqueness,
        Self::Mutability,
        Self::InvalidSyntax,
        Self::InvalidPath,
        Self::NoTarget,
        Self::InvalidValue,
        Self::InvalidVers,
        Self::Sensitive,
    ];

    /// Returns the keyword as Table 9 spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidFilter => "invalidFilter",
            Self::TooMany => "tooMany",
            Self::Uniqueness => "uniqueness",
            Self::Mutability => "mutability",
            Self::InvalidSyntax => "invalidSyntax",
            Self::InvalidPath => "invalidPath",
            Self::NoTarget => "noTarget",
            Self::InvalidValue => "invalidValue",
            Self::InvalidVers => "invalidVers",
            Self::Sensitive => "sensitive",
        }
    }
}

/// A refusal of a SCIM request: a status, an optional `scimType` and a detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScimError {
    status: StatusCode,
    scim_type: Option<ScimType>,
    detail: String,
}

impl ScimError {
    /// Creates an error with `status` and no `scimType`.
    #[must_use]
    pub fn new(status: StatusCode, detail: impl Into<String>) -> Self {
        Self {
            status,
            scim_type: None,
            detail: detail.into(),
        }
    }

    /// Creates a `400 Bad Request` with the `scimType` Table 9 defines for it.
    #[must_use]
    pub fn bad_request(scim_type: ScimType, detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            scim_type: Some(scim_type),
            detail: detail.into(),
        }
    }

    /// Creates the `409 Conflict` with `scimType` `uniqueness` that RFC 7644
    /// §3.3 requires for a value already in use.
    #[must_use]
    pub fn uniqueness(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            scim_type: Some(ScimType::Uniqueness),
            detail: detail.into(),
        }
    }

    /// Returns the HTTP status.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the `scimType`, if the error carries one.
    #[must_use]
    pub const fn scim_type(&self) -> Option<ScimType> {
        self.scim_type
    }

    /// Returns the human-readable detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// Returns the `Error` message body of this error.
    #[must_use]
    pub fn body(&self) -> ErrorBody {
        ErrorBody {
            schemas: vec![urn::ERROR.to_owned()],
            scim_type: self.scim_type,
            detail: Some(self.detail.clone()).filter(|detail| !detail.is_empty()),
            status: self.status.as_str().to_owned(),
        }
    }
}

impl fmt::Display for ScimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.scim_type {
            Some(scim_type) => write!(
                f,
                "{} {}: {}",
                self.status.as_u16(),
                scim_type.as_str(),
                self.detail
            ),
            None => write!(f, "{}: {}", self.status.as_u16(), self.detail),
        }
    }
}

impl std::error::Error for ScimError {}

/// The `Error` message of RFC 7644 §3.12 as it travels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorBody {
    /// The message schema, `urn:ietf:params:scim:api:messages:2.0:Error`.
    pub schemas: Vec<String>,
    /// The detail error keyword.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scim_type: Option<ScimType>,
    /// The human-readable message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The HTTP status code as a JSON string.
    pub status: String,
}

/// Why an [`ErrorBody`] is not a SCIM error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorBodyError {
    /// The body does not name the `Error` message schema.
    #[error("the body does not carry the schema {}", urn::ERROR)]
    NotAnError,
    /// The `status` is not the string form of an error status code.
    #[error("'{0}' is not an HTTP error status")]
    Status(String),
    /// A `scimType` accompanies a status Table 9 and §3.3 give none to.
    #[error("scimType {} does not apply to status {status}", scim_type.as_str())]
    ScimTypeStatus {
        /// The keyword.
        scim_type: ScimType,
        /// The status it was sent with.
        status: u16,
    },
}

impl TryFrom<ErrorBody> for ScimError {
    type Error = ErrorBodyError;

    fn try_from(body: ErrorBody) -> Result<Self, Self::Error> {
        if !body
            .schemas
            .iter()
            .any(|schema| urn::same(schema, urn::ERROR))
        {
            return Err(ErrorBodyError::NotAnError);
        }
        let status = body
            .status
            .parse::<u16>()
            .ok()
            .and_then(|code| StatusCode::from_u16(code).ok())
            .filter(|status| {
                status.is_client_error() || status.is_server_error() || status.is_redirection()
            })
            .ok_or_else(|| ErrorBodyError::Status(body.status.clone()))?;
        if let Some(scim_type) = body.scim_type {
            // NOTE: RFC 7644 §3.12 defines the keywords for 400, and §3.3 gives `uniqueness` to 409.
            let applies = status == StatusCode::BAD_REQUEST
                || (status == StatusCode::CONFLICT && scim_type == ScimType::Uniqueness);
            if !applies {
                return Err(ErrorBodyError::ScimTypeStatus {
                    scim_type,
                    status: status.as_u16(),
                });
            }
        }
        Ok(Self {
            status,
            scim_type: body.scim_type,
            detail: body.detail.unwrap_or_default(),
        })
    }
}
