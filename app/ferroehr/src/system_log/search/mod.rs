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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};

use sqlx::postgres::PgConnection;
use sqlx::{PgPool, Row};

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

/// How many times a token or set id is read again when another writer
/// stored it between this writer's insert and its read.
const READ_AGAIN: usize = 3;

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
///
/// Each element's nodes are the ones [`AuditElement::nodes`] reaches in the
/// document: `agent.who.identifier`, the `what.identifier` of an entity whose
/// role is the patient's (object-role `1`), `entity.what.identifier`,
/// `source.observer.identifier`, `entity.type`, `entity.role`, `type` and
/// `subtype`, and each `agent.network.address`.
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

/// Stores the tokens of each record of a batch that the store does not hold
/// yet and returns the `search_tokens` value of each record.
///
/// Only a token the store does not hold reaches the `INSERT`: an `ON CONFLICT`
/// row still draws its identity value (PostgreSQL 18 docs, "INSERT", the
/// `ON CONFLICT` clause), and a held token would spend one per batch.
const STORE_TOKENS: &str = "WITH input AS (\
     SELECT * FROM unnest($1::text[], $2::text[], $3::text[]) AS i(parameter, system, code)), \
     keyed AS (SELECT parameter, system, code, \
     audit.search_token_digest(parameter, system, code) AS digest FROM input), \
     inserted AS (INSERT INTO audit.audit_search_token (parameter, system, code, digest) \
     SELECT k.parameter, k.system, k.code, k.digest FROM keyed k \
     WHERE NOT EXISTS (SELECT FROM audit.audit_search_token h WHERE h.digest = k.digest) \
     ON CONFLICT (digest) DO NOTHING \
     RETURNING id, digest) \
     SELECT k.parameter, k.system, k.code, coalesce(n.id, t.id) AS id FROM keyed k \
     LEFT JOIN inserted n ON n.digest = k.digest \
     LEFT JOIN audit.audit_search_token t ON t.digest = k.digest";

/// Stores a shared set unless the store holds it and returns its id.
const STORE_SET: &str = "WITH inserted AS (\
     INSERT INTO audit.audit_search_set (tokens, digest) \
     VALUES ($1, audit.search_set_digest($1)) ON CONFLICT (tokens) DO NOTHING RETURNING id) \
     SELECT id FROM inserted UNION ALL \
     SELECT id FROM audit.audit_search_set WHERE tokens = $1 LIMIT 1";

/// The key a token is looked up by in the rows the store answers with.
type TokenKey = (String, Option<String>, Option<String>);

/// The store side of the search tokens: the shared sets this process has
/// already stored, by their token ids.
#[derive(Debug, Default)]
pub(crate) struct TokenRegistry {
    sets: Mutex<BTreeMap<Vec<i32>, i32>>,
}

/// The shared sets a write read or stored that the registry does not hold
/// yet, which [`TokenRegistry::remember`] takes once the write commits: a set
/// stored by a transaction that rolls back does not exist.
#[derive(Debug, Default)]
#[must_use = "the sets are remembered only once the write commits"]
pub(crate) struct LearnedSets(BTreeMap<Vec<i32>, i32>);

impl LearnedSets {
    /// The sets of `self` and of `other`, the parts of one write.
    pub(crate) fn merge(mut self, other: Self) -> Self {
        self.0.extend(other.0);
        self
    }
}

impl TokenRegistry {
    /// The `search_tokens` value of each of `records`, storing every token and
    /// set the store does not hold yet on `conn`, with the sets this call
    /// learned.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a statement fails, and
    /// [`AuditError::SearchToken`] when a token or set cannot be read back.
    pub(crate) async fn record_tokens(
        &self,
        conn: &mut PgConnection,
        records: &[BTreeSet<SearchToken>],
    ) -> Result<(Vec<Vec<i32>>, LearnedSets), AuditError> {
        // NOTE: our own design: the lock keeps retention from deleting a token this write names.
        sqlx::query("SELECT audit.hold_search_tokens()")
            .execute(&mut *conn)
            .await?;
        let ids = token_ids(conn, records).await?;
        let mut learned = LearnedSets::default();
        let mut values = Vec::with_capacity(records.len());
        for tokens in records {
            let mut shared = Vec::new();
            let mut own = Vec::new();
            for token in tokens {
                let key = (
                    token.parameter.to_owned(),
                    token.system.clone(),
                    token.code.clone(),
                );
                let id = *ids.get(&key).ok_or(AuditError::SearchToken)?;
                if AuditElement::shares(token.parameter) {
                    shared.push(id);
                } else {
                    own.push(id);
                }
            }
            shared.sort_unstable();
            let mut value = Vec::with_capacity(own.len() + 1);
            if !shared.is_empty() {
                let set = self.set_id(conn, shared, &mut learned).await?;
                value.push(set.checked_neg().ok_or(AuditError::SearchToken)?);
            }
            own.sort_unstable();
            value.extend(own);
            values.push(value);
        }
        Ok((values, learned))
    }

    /// Holds the sets of a committed write.
    pub(crate) fn remember(&self, learned: LearnedSets) {
        self.sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(learned.0);
    }

    /// The id of the shared set of `tokens`, stored the first time.
    async fn set_id(
        &self,
        conn: &mut PgConnection,
        tokens: Vec<i32>,
        learned: &mut LearnedSets,
    ) -> Result<i32, AuditError> {
        if let Some(id) = self
            .sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&tokens)
        {
            return Ok(*id);
        }
        if let Some(id) = learned.0.get(&tokens) {
            return Ok(*id);
        }
        for _ in 0..READ_AGAIN {
            let id: Option<i32> = sqlx::query_scalar(STORE_SET)
                .bind(&tokens)
                .fetch_optional(&mut *conn)
                .await?;
            if let Some(id) = id {
                learned.0.insert(tokens, id);
                return Ok(id);
            }
        }
        Err(AuditError::SearchToken)
    }
}

/// The id of every token of `records`, storing the ones the store does not
/// hold yet; a token another writer stored after this statement began is read
/// again.
async fn token_ids(
    conn: &mut PgConnection,
    records: &[BTreeSet<SearchToken>],
) -> Result<BTreeMap<TokenKey, i32>, AuditError> {
    let mut wanted: BTreeSet<&SearchToken> = records.iter().flatten().collect();
    let mut ids = BTreeMap::new();
    for _ in 0..READ_AGAIN {
        if wanted.is_empty() {
            return Ok(ids);
        }
        let parameters: Vec<&str> = wanted.iter().map(|token| token.parameter).collect();
        let systems: Vec<Option<&str>> =
            wanted.iter().map(|token| token.system.as_deref()).collect();
        let codes: Vec<Option<&str>> = wanted.iter().map(|token| token.code.as_deref()).collect();
        let rows = sqlx::query(STORE_TOKENS)
            .bind(parameters)
            .bind(systems)
            .bind(codes)
            .fetch_all(&mut *conn)
            .await?;
        for row in rows {
            let Some(id) = row.try_get::<Option<i32>, _>("id")? else {
                continue;
            };
            ids.insert(
                (
                    row.try_get::<String, _>("parameter")?,
                    row.try_get::<Option<String>, _>("system")?,
                    row.try_get::<Option<String>, _>("code")?,
                ),
                id,
            );
        }
        wanted.retain(|token| {
            !ids.contains_key(&(
                token.parameter.to_owned(),
                token.system.clone(),
                token.code.clone(),
            ))
        });
    }
    if wanted.is_empty() {
        Ok(ids)
    } else {
        Err(AuditError::SearchToken)
    }
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
