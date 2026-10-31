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
//! `search_tokens` under a GIN index.
//!
//! A token is read from the document with the same nodes the profile's
//! matching reads ([`AuditElement::nodes`]), so the stored tokens match what
//! the document says.
//!
//! NOTE: no specification governs how a repository indexes its records: our own design.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};

use sqlx::postgres::PgConnection;
use sqlx::{PgPool, Row};

use crate::db::acquire::acquire;
use crate::system_log::AuditError;
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
    fn shares(parameter: &str) -> bool {
        ELEMENTS
            .iter()
            .any(|element| element.parameter() == parameter && element.shared())
    }
}

/// The search tokens of `document`: each node a token parameter matches, as
/// its system and its code or value, and each `agent.network.address`.
#[must_use]
pub(crate) fn document_tokens(document: &serde_json::Value) -> BTreeSet<SearchToken> {
    let text = |value: Option<&serde_json::Value>| {
        value.and_then(serde_json::Value::as_str).map(str::to_owned)
    };
    let mut tokens = BTreeSet::new();
    for element in ELEMENTS {
        let (nodes, identifier) = element.nodes(document);
        let key = if identifier { "value" } else { "code" };
        for node in nodes {
            tokens.insert(SearchToken {
                parameter: element.parameter(),
                system: text(node.get("system")),
                code: text(node.get(key)),
            });
        }
    }
    for address in crate::system_log::store::lax_path(document, &["agent", "network", "address"]) {
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

/// The token ids each criterion of `filter` resolved to, in its order, or
/// `None` when a criterion no stored token matches, which leaves no record.
///
/// # Errors
/// [`AuditError::Store`] when a lookup fails.
pub(crate) async fn resolve_criteria(
    pool: &PgPool,
    filter: &AuditSearchFilter,
) -> Result<Option<Vec<Option<Vec<i32>>>>, AuditError> {
    let mut resolved = Vec::with_capacity(filter.criteria.len());
    for criterion in &filter.criteria {
        let ids = criterion_ids(pool, criterion).await?;
        if ids.as_ref().is_some_and(Vec::is_empty) {
            // NOTE: `RESTful` ATNA §3.81.4.1.2.2 combines parameters with AND, so a parameter
            // no stored token matches leaves no record.
            return Ok(None);
        }
        resolved.push(ids);
    }
    Ok(Some(resolved))
}

/// The `search_tokens` ids that answer `criterion`, any of which a record must
/// carry; `None` for a criterion on a column. An empty list means no record
/// can match.
///
/// # Errors
/// [`AuditError::Store`] when a lookup fails.
pub(crate) async fn criterion_ids(
    pool: &PgPool,
    criterion: &AuditCriterion,
) -> Result<Option<Vec<i32>>, AuditError> {
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
            Ok(Some(ids.into_iter().collect()))
        }
        AuditCriterion::Element(element, tokens) => {
            let mut ids = BTreeSet::new();
            for token in tokens {
                let found: Vec<i32> = token_lookup(element.parameter(), token)
                    .fetch_all(&mut *conn)
                    .await?;
                ids.extend(found);
            }
            if !element.shared() || ids.is_empty() {
                return Ok(Some(ids.into_iter().collect()));
            }
            let tokens: Vec<i32> = ids.into_iter().collect();
            let sets: Vec<i32> = sqlx::query_scalar(
                "SELECT -id FROM audit.audit_search_set WHERE tokens && $1 ORDER BY 1",
            )
            .bind(tokens)
            .fetch_all(&mut *conn)
            .await?;
            Ok(Some(sets))
        }
        AuditCriterion::Patient(_)
        | AuditCriterion::Agent(_)
        | AuditCriterion::Entity(_)
        | AuditCriterion::Outcome(_)
        | AuditCriterion::Action(_) => Ok(None),
    }
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
mod tests {
    use super::*;

    /// Every element names a distinct parameter, and the shared ones are the
    /// code-system parameters and the audit source.
    #[test]
    fn every_element_names_its_own_parameter() {
        let parameters: BTreeSet<&str> = ELEMENTS.iter().map(|e| e.parameter()).collect();
        assert_eq!(parameters.len(), ELEMENTS.len());
        assert!(!parameters.contains(ADDRESS));
        let shared: Vec<&str> = ELEMENTS
            .iter()
            .filter(|e| e.shared())
            .map(|e| e.parameter())
            .collect();
        assert_eq!(
            shared,
            [
                "source.identifier",
                "entity-type",
                "entity-role",
                "type",
                "subtype"
            ]
        );
    }

    /// The tokens of a document are its nodes under each parameter, with the
    /// system kept apart from its absence.
    #[test]
    fn a_document_yields_a_token_per_node() {
        let document = serde_json::json!({
            "type": {"system": "http://terminology.hl7.org/CodeSystem/audit-event-type", "code": "rest"},
            "subtype": [{"system": "urn:openehr:its-rest:operation", "code": "composition_get"}],
            "agent": [
                {"who": {"identifier": {"value": "alice"}}, "network": {"address": "10.0.0.9"}}
            ],
            "entity": [{
                "role": {"system": "http://terminology.hl7.org/CodeSystem/object-role", "code": "1"},
                "what": {"identifier": {"value": "patient-42"}}
            }]
        });
        let tokens = document_tokens(&document);
        let token = |parameter, system: Option<&str>, code: &str| SearchToken {
            parameter,
            system: system.map(str::to_owned),
            code: Some(code.to_owned()),
        };
        for expected in [
            token(
                "type",
                Some("http://terminology.hl7.org/CodeSystem/audit-event-type"),
                "rest",
            ),
            token(
                "subtype",
                Some("urn:openehr:its-rest:operation"),
                "composition_get",
            ),
            token("agent.identifier", None, "alice"),
            token("address", None, "10.0.0.9"),
            token("patient.identifier", None, "patient-42"),
            token("entity.identifier", None, "patient-42"),
            token(
                "entity-role",
                Some("http://terminology.hl7.org/CodeSystem/object-role"),
                "1",
            ),
        ] {
            assert!(tokens.contains(&expected), "{expected:?} in {tokens:?}");
        }
        assert_eq!(tokens.len(), 7, "{tokens:?}");
    }
}
