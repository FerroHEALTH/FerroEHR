// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-81 search tokens of the audit store.
//!
//! The parameters of the `RESTful` ATNA supplement §3.81.4.1.2.2 that match an
//! element of the `AuditEvent` (`address` and the token parameters) are
//! answered from tokens taken from the rendered document when the record is
//! written, so a search reads an index and renders only the page it serves.
//! Each distinct token is stored once (`audit.audit_search_token`). The
//! tokens of the code-system parameters (`type`, `subtype`, `entity-type`,
//! `entity-role`, `source.identifier`) take few values, so a record names
//! their combination by one shared set (`audit.audit_search_set`). The
//! record keeps the ids of its other tokens and its set, negated, in
//! `search_tokens`, indexed by a GIN index of `audit.search_keys` with its
//! request and resource ids.
//!
//! An `entity.identifier` that carries the record's request id or resource id
//! is unique to few records, so it is answered from that column
//! ([`IdentifierColumn`]): the record's set holds a token saying in which
//! system the document carries the column's value, and no token of its own.
//!
//! A token is read from the document with the same nodes the profile's
//! matching reads ([`AuditElement::nodes`]), so the stored tokens match what
//! the document says.
//!
//! NOTE: no specification governs how a repository indexes its records: our own design.

pub(crate) mod condition;

use std::collections::BTreeSet;

use sqlx::PgPool;
use sqlx::postgres::PgConnection;

use crate::db::acquire::acquire;
use crate::system_log::AuditError;
use crate::system_log::event::AuditEvent;
use crate::system_log::store::{
    AuditCriterion, AuditElement, AuditSearchFilter, AuditToken, AuditTokenSystem,
};

/// The `address` parameter (`RESTful` ATNA §3.81.4.1.2.2).
pub(crate) const ADDRESS: &str = "address";

/// Every element a token parameter matches.
pub(crate) const ELEMENTS: [AuditElement; 8] = [
    AuditElement::AgentIdentifier,
    AuditElement::PatientIdentifier,
    AuditElement::EntityIdentifier,
    AuditElement::SourceIdentifier,
    AuditElement::EntityType,
    AuditElement::EntityRole,
    AuditElement::Type,
    AuditElement::Subtype,
];

/// One search token of a rendered document.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SearchToken {
    /// The ITI-81 parameter.
    pub(crate) parameter: &'static str,
    /// The system of the coding or identifier, when it names one.
    pub(crate) system: Option<String>,
    /// The code, the identifier value or the address.
    pub(crate) code: Option<String>,
}

impl AuditElement {
    /// The name of the ITI-81 parameter that matches this element.
    pub(crate) const fn parameter(self) -> &'static str {
        match self {
            Self::AgentIdentifier => "agent.identifier",
            Self::PatientIdentifier => "patient.identifier",
            Self::EntityIdentifier => "entity.identifier",
            Self::SourceIdentifier => "source.identifier",
            Self::EntityType => "entity-type",
            Self::EntityRole => "entity-role",
            Self::Type => "type",
            Self::Subtype => "subtype",
        }
    }

    /// Whether the element takes its values from code systems and the audit
    /// source, so its tokens are kept in a record's shared set.
    pub(crate) const fn shared(self) -> bool {
        matches!(
            self,
            Self::SourceIdentifier
                | Self::EntityType
                | Self::EntityRole
                | Self::Type
                | Self::Subtype
        )
    }

    /// Whether a token of the parameter `parameter` is kept in the shared set.
    pub(crate) fn shares(parameter: &str) -> bool {
        ELEMENTS
            .iter()
            .any(|element| element.parameter() == parameter && element.shared())
            || IdentifierColumn::ALL
                .iter()
                .any(|column| column.parameter() == parameter)
    }
}

/// A record column whose value the document may carry as an
/// `entity.identifier`, which a search reads from the column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdentifierColumn {
    /// `request_id`, the request id of BALP `entity:transaction`.
    RequestId,
    /// `resource_id`, the object the record names.
    ResourceId,
}

impl IdentifierColumn {
    /// Every column, in the order a node is matched against them.
    pub(crate) const ALL: [Self; 2] = [Self::RequestId, Self::ResourceId];

    /// The parameter of the token that names the system the document carries
    /// the column's value in.
    pub(crate) const fn parameter(self) -> &'static str {
        match self {
            Self::RequestId => "entity.identifier=request_id",
            Self::ResourceId => "entity.identifier=resource_id",
        }
    }

    /// The column.
    pub(crate) const fn column(self) -> &'static str {
        match self {
            Self::RequestId => "request_id",
            Self::ResourceId => "resource_id",
        }
    }

    /// The condition on the column's key in the records' token index
    /// (`audit.search_key`, #6202).
    pub(super) const fn hashed(self) -> &'static str {
        match self {
            Self::RequestId => "search_keys @> audit.search_key(1, $1)",
            Self::ResourceId => "search_keys @> audit.search_key(2, $1)",
        }
    }

    /// The value `event` stores in the column, when it stores one.
    fn value(self, event: &AuditEvent) -> Option<&str> {
        match self {
            Self::RequestId => event.request_id.as_deref(),
            Self::ResourceId => event.object_id.as_deref(),
        }
        .filter(|value| !value.is_empty())
    }
}

/// The search tokens of `document`, the rendering of `event`: each node a
/// token parameter matches, as its system and its code or value, and each
/// `agent.network.address`. An `entity.identifier` whose value is the
/// event's request id or resource id is the [`IdentifierColumn`] token of its
/// system instead.
#[cfg(test)]
#[must_use]
pub(crate) fn document_tokens(
    document: &serde_json::Value,
    event: &AuditEvent,
) -> BTreeSet<SearchToken> {
    let text = |value: Option<&serde_json::Value>| {
        value.and_then(serde_json::Value::as_str).map(str::to_owned)
    };
    let mut tokens = BTreeSet::new();
    for element in ELEMENTS {
        let (nodes, identifier) = element.nodes(document);
        let key = if identifier { "value" } else { "code" };
        for node in nodes {
            let code = text(node.get(key));
            let column = IdentifierColumn::ALL.into_iter().find(|column| {
                element == AuditElement::EntityIdentifier
                    && code.is_some()
                    && column.value(event) == code.as_deref()
            });
            tokens.insert(SearchToken {
                parameter: column.map_or(element.parameter(), IdentifierColumn::parameter),
                system: text(node.get("system")),
                code: if column.is_some() { None } else { code },
            });
        }
    }
    for address in
        crate::system_log::store::lax::lax_path(document, &["agent", "network", "address"])
    {
        tokens.insert(SearchToken {
            parameter: ADDRESS,
            system: None,
            code: Some(match address {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            }),
        });
    }
    tokens
}

/// The search tokens of `resource`, the rendering of `event`: the tokens
/// `document_tokens` reads from the resource's JSON document, read from the
/// typed resource with no document written (#6005).
#[must_use]
pub(crate) fn resource_tokens(
    resource: &fhir_types::r4::audit_event::AuditEvent,
    event: &AuditEvent,
) -> BTreeSet<SearchToken> {
    use fhir_types::r4::coding::Coding;
    use fhir_types::r4::identifier::Identifier;
    use fhir_types::r4::reference::Reference;
    fn identifier_of(reference: Option<&Reference>) -> Option<&Identifier> {
        reference.and_then(|reference| reference.identifier.as_deref())
    }
    let mut tokens = BTreeSet::new();
    let mut identified = |element: AuditElement, identifier: &Identifier| {
        let code = identifier
            .value
            .as_ref()
            .and_then(|value| value.value.clone());
        let column = IdentifierColumn::ALL.into_iter().find(|column| {
            element == AuditElement::EntityIdentifier
                && code.is_some()
                && column.value(event) == code.as_deref()
        });
        tokens.insert(SearchToken {
            parameter: column.map_or(element.parameter(), IdentifierColumn::parameter),
            system: identifier
                .system
                .as_ref()
                .and_then(|system| system.value.clone()),
            code: if column.is_some() { None } else { code },
        });
    };
    for agent in &resource.agent {
        if let Some(identifier) = identifier_of(agent.who.as_ref()) {
            identified(AuditElement::AgentIdentifier, identifier);
        }
    }
    for entity in &resource.entity {
        let patient = entity.role.as_ref().is_some_and(|role| {
            role.code.as_ref().and_then(|code| code.value.as_deref()) == Some("1")
                && role
                    .system
                    .as_ref()
                    .and_then(|system| system.value.as_deref())
                    == Some(crate::system_log::store::OBJECT_ROLE_SYSTEM)
        });
        if patient && let Some(identifier) = identifier_of(entity.what.as_ref()) {
            identified(AuditElement::PatientIdentifier, identifier);
        }
    }
    for entity in &resource.entity {
        if let Some(identifier) = identifier_of(entity.what.as_ref()) {
            identified(AuditElement::EntityIdentifier, identifier);
        }
    }
    if let Some(identifier) = identifier_of(Some(&resource.source.observer)) {
        identified(AuditElement::SourceIdentifier, identifier);
    }
    let mut coded = |element: AuditElement, coding: &Coding| {
        tokens.insert(SearchToken {
            parameter: element.parameter(),
            system: coding
                .system
                .as_ref()
                .and_then(|system| system.value.clone()),
            code: coding.code.as_ref().and_then(|code| code.value.clone()),
        });
    };
    for entity in &resource.entity {
        if let Some(coding) = &entity.r#type {
            coded(AuditElement::EntityType, coding);
        }
    }
    for entity in &resource.entity {
        if let Some(coding) = &entity.role {
            coded(AuditElement::EntityRole, coding);
        }
    }
    coded(AuditElement::Type, &resource.r#type);
    for coding in &resource.subtype {
        coded(AuditElement::Subtype, coding);
    }
    for agent in &resource.agent {
        if let Some(address) = agent
            .network
            .as_ref()
            .and_then(|network| network.address.as_ref())
            .and_then(|address| address.value.clone())
        {
            tokens.insert(SearchToken {
                parameter: ADDRESS,
                system: None,
                code: Some(address),
            });
        }
    }
    tokens
}

/// The `search_tokens` value of each record of a write, as the text of an
/// `integer[]`: one statement, `audit.search_tokens_of`, that takes the
/// writers' hold on the tokens and stores every token and shared set the
/// store does not hold yet (#6633).
const RESOLVE: &str = "SELECT t.tokens FROM audit.search_tokens_of($1::integer[], $2::text[], \
     $3::text[], $4::text[], $5::boolean[], $6::integer) t ORDER BY t.record";

/// The `search_tokens` value of each of `records` on `conn`, as the text of
/// an `integer[]`, storing every token and set the store does not hold yet.
///
/// # Errors
/// [`AuditError::TokenCount`] when the write holds more records than an
/// `integer` counts, and [`AuditError::Store`] when the statement fails.
pub(crate) async fn record_tokens(
    conn: &mut PgConnection,
    records: Vec<BTreeSet<SearchToken>>,
) -> Result<Vec<String>, AuditError> {
    let args = crate::system_log::store::batch::TokenArgs::of(records)?;
    Ok(sqlx::query_scalar(RESOLVE)
        .bind(args.records)
        .bind(args.parameters)
        .bind(args.systems)
        .bind(args.codes)
        .bind(args.shared)
        .bind(args.count)
        .fetch_all(conn)
        .await?)
}

/// What a criterion on the document resolved to: the ids of the stored tokens
/// a record may carry, and the record columns that answer it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TokenMatch {
    /// The token and negated set ids, any of which a record may carry.
    pub(crate) ids: Vec<i32>,
    /// The column matches, any of which a record may meet.
    pub(crate) columns: Vec<ColumnMatch>,
}

impl From<Vec<i32>> for TokenMatch {
    fn from(ids: Vec<i32>) -> Self {
        Self {
            ids,
            columns: Vec::new(),
        }
    }
}

impl TokenMatch {
    /// Whether no record can match.
    fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.columns.is_empty()
    }
}

/// An `entity.identifier` answered from a record column: the record carries
/// one of `sets`, which says the document carries the column's value in the
/// system the search names, and the column holds `value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ColumnMatch {
    /// The column.
    pub(crate) column: IdentifierColumn,
    /// The negated ids of the shared sets that hold a matching column token.
    pub(crate) sets: Vec<i32>,
    /// The value the column must hold; `None` for `[system]|`, which any value
    /// in the system matches.
    pub(crate) value: Option<String>,
}

/// What each criterion of `filter` resolved to, in its order, or `None` when
/// a criterion no stored token matches, which leaves no record.
///
/// # Errors
/// [`AuditError::Store`] when a lookup fails.
pub(crate) async fn resolve_criteria(
    pool: &PgPool,
    filter: &AuditSearchFilter,
) -> Result<Option<Vec<Option<TokenMatch>>>, AuditError> {
    let mut resolved = Vec::with_capacity(filter.criteria.len());
    for criterion in &filter.criteria {
        let found = criterion_ids(pool, criterion).await?;
        if found.as_ref().is_some_and(TokenMatch::is_empty) {
            // NOTE: `RESTful` ATNA §3.81.4.1.2.2 combines parameters with AND, so a parameter
            // no stored token matches leaves no record.
            return Ok(None);
        }
        resolved.push(found);
    }
    Ok(Some(resolved))
}

/// What answers `criterion` on the document; `None` for a criterion on a
/// column. An empty match means no record can match.
///
/// # Errors
/// [`AuditError::Store`] when a lookup fails.
pub(crate) async fn criterion_ids(
    pool: &PgPool,
    criterion: &AuditCriterion,
) -> Result<Option<TokenMatch>, AuditError> {
    let mut conn = acquire(pool).await?;
    match criterion {
        AuditCriterion::Address(values) => {
            let mut ids = BTreeSet::new();
            for value in values {
                // NOTE: `RESTful` ATNA §3.81.4.1.2.2, `address` holds the substring to match.
                let found: Vec<i32> = sqlx::query_scalar(
                    "SELECT id FROM audit.audit_search_token \
                     WHERE parameter = 'address' AND strpos(lower(code), lower($1)) > 0",
                )
                .bind(value)
                .fetch_all(&mut *conn)
                .await?;
                ids.extend(found);
            }
            Ok(Some(TokenMatch::from(ids.into_iter().collect::<Vec<_>>())))
        }
        AuditCriterion::Element(element, tokens) => {
            let mut ids = BTreeSet::new();
            for token in tokens {
                let found: Vec<i32> = token_lookup(element.parameter(), token)
                    .fetch_all(&mut *conn)
                    .await?;
                ids.extend(found);
            }
            let columns = if *element == AuditElement::EntityIdentifier {
                column_matches(&mut conn, tokens).await?
            } else {
                Vec::new()
            };
            if !element.shared() || ids.is_empty() {
                return Ok(Some(TokenMatch {
                    ids: ids.into_iter().collect(),
                    columns,
                }));
            }
            let sets = sets_holding(&mut conn, ids.into_iter().collect()).await?;
            Ok(Some(TokenMatch { ids: sets, columns }))
        }
        AuditCriterion::Patient(_)
        | AuditCriterion::Agent(_)
        | AuditCriterion::Entity(_)
        | AuditCriterion::Outcome(_)
        | AuditCriterion::Action(_) => Ok(None),
    }
}

/// The negated ids of the shared sets that hold any of `tokens`.
async fn sets_holding(conn: &mut PgConnection, tokens: Vec<i32>) -> Result<Vec<i32>, AuditError> {
    Ok(
        sqlx::query_scalar("SELECT -id FROM audit.audit_search_set WHERE tokens && $1 ORDER BY 1")
            .bind(tokens)
            .fetch_all(&mut *conn)
            .await?,
    )
}

/// The column matches of an `entity.identifier` criterion: for each token
/// value and column, the sets whose column token is in the system the value
/// names (FHIR R4 Search §3.1.1.4.10).
async fn column_matches(
    conn: &mut PgConnection,
    tokens: &[AuditToken],
) -> Result<Vec<ColumnMatch>, AuditError> {
    let mut matches = Vec::new();
    for token in tokens {
        for column in IdentifierColumn::ALL {
            // NOTE: a column token carries no code, so the lookup reads the NULL entries of the
            // code hash index.
            let lookup = "SELECT id FROM audit.audit_search_token \
                          WHERE hashtextextended(code, 0) IS NULL AND parameter = $1";
            let ids: Vec<i32> = match (&token.system, &token.code) {
                (AuditTokenSystem::Any, Some(_)) => {
                    sqlx::query_scalar(lookup).bind(column.parameter())
                }
                (AuditTokenSystem::Absent, Some(_)) => sqlx::query_scalar(
                    "SELECT id FROM audit.audit_search_token \
                     WHERE hashtextextended(code, 0) IS NULL AND parameter = $1 \
                     AND system IS NULL",
                )
                .bind(column.parameter()),
                (AuditTokenSystem::Named(system), _) => sqlx::query_scalar(
                    "SELECT id FROM audit.audit_search_token \
                     WHERE hashtextextended(code, 0) IS NULL AND parameter = $1 \
                     AND system = $2",
                )
                .bind(column.parameter())
                .bind(system.clone()),
                (AuditTokenSystem::Any | AuditTokenSystem::Absent, None) => continue,
            }
            .fetch_all(&mut *conn)
            .await?;
            if ids.is_empty() {
                continue;
            }
            let sets = sets_holding(conn, ids).await?;
            if !sets.is_empty() {
                matches.push(ColumnMatch {
                    column,
                    sets,
                    value: token.code.clone(),
                });
            }
        }
    }
    Ok(matches)
}

/// The lookup of the stored tokens of `parameter` a token value names: the
/// three forms of FHIR R4 Search §3.1.1.4.10.
fn token_lookup(
    parameter: &'static str,
    token: &AuditToken,
) -> sqlx::query::QueryScalar<'static, sqlx::Postgres, i32, sqlx::postgres::PgArguments> {
    let by_code = "SELECT id FROM audit.audit_search_token \
                   WHERE hashtextextended(code, 0) = hashtextextended($2, 0) AND code = $2 \
                   AND parameter = $1";
    match (&token.system, token.code.clone()) {
        (AuditTokenSystem::Any, Some(code)) => {
            sqlx::query_scalar(by_code).bind(parameter).bind(code)
        }
        (AuditTokenSystem::Absent, Some(code)) => sqlx::query_scalar(
            "SELECT id FROM audit.audit_search_token \
             WHERE hashtextextended(code, 0) = hashtextextended($2, 0) AND code = $2 \
             AND parameter = $1 AND system IS NULL",
        )
        .bind(parameter)
        .bind(code),
        (AuditTokenSystem::Named(system), Some(code)) => sqlx::query_scalar(
            "SELECT id FROM audit.audit_search_token \
             WHERE hashtextextended(code, 0) = hashtextextended($2, 0) AND code = $2 \
             AND parameter = $1 AND system = $3",
        )
        .bind(parameter)
        .bind(code)
        .bind(system.clone()),
        (AuditTokenSystem::Named(system), None) => sqlx::query_scalar(
            "SELECT id FROM audit.audit_search_token \
             WHERE system IS NOT NULL \
             AND hashtextextended(system, 0) = hashtextextended($2, 0) AND system = $2 \
             AND parameter = $1",
        )
        .bind(parameter)
        .bind(system.clone()),
        // NOTE: FHIR R4 Search §3.1.1.4.10, a token with no system names a code, so a value with
        // neither names nothing; the query grammar never yields one.
        (AuditTokenSystem::Any | AuditTokenSystem::Absent, None) => sqlx::query_scalar(
            "SELECT id FROM audit.audit_search_token WHERE false AND parameter = $1",
        )
        .bind(parameter),
    }
}

#[cfg(test)]
mod tests;
