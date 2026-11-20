// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The query of `GET {extensions}/care_relationship`.
//!
//! It asks for the care relationships of a subject by its reference, or the
//! reverse reading by carer (draft access control package, ITS-REST
//! Resources, Care Relationships). A
//! parameter outside its form is refused `400` naming it (ITS-REST overview
//! `Requests_and_responses.md` §HTTP status codes, 400 "syntactically
//! invalid content"). No openEHR spec defines the parameters: our own design
//! over the draft package.

use fhir_search::token::Token;
use openehr_its::rest::runtime::ApiError;
use openehr_rm::draft::access_control::terminology::ForeignCode;

use ferroehr::service::demographic::care_relationship::{CarePath, CareSpan, CarerQuery};
use ferroehr::storage::access_projection::EhrSubject;

use crate::overview::error::RestError;
use crate::overview::params::query_pairs;

/// The largest page served: our own design, as the access log's.
const MAX_FETCH: usize = 1000;

/// What a `GET` of the relationship list asks.
#[derive(Debug, Clone)]
pub(super) enum CareQuery {
    /// The relationships of the subject a reference names.
    Subject(EhrSubject, CareSpan),
    /// The relationships that reach a carer.
    Carer(CarerQuery),
}

/// The parameters each form takes.
const SUBJECT: [&str; 4] = ["subject_id", "subject_namespace", "at", "all"];
const CARER: [&str; 6] = ["carer", "role", "path", "at", "offset", "fetch"];

impl CareQuery {
    /// The query the query string `query` names: the subject form when it
    /// names `subject_id`, the carer form when it names `carer`.
    ///
    /// # Errors
    /// A `400` naming the first parameter outside its form, given twice, or
    /// missing.
    pub(super) fn parse(query: Option<&str>) -> Result<Self, RestError> {
        let pairs = query.map(query_pairs).unwrap_or_default();
        let named = |name: &str| pairs.iter().any(|(key, _)| key == name);
        match (named("subject_id"), named("carer")) {
            (true, false) => {
                checked(&pairs, &SUBJECT)?;
                let (Some(id), Some(namespace)) = (
                    value(&pairs, "subject_id"),
                    value(&pairs, "subject_namespace"),
                ) else {
                    return Err(malformed(
                        "subject_id and subject_namespace name a subject together",
                    ));
                };
                Ok(Self::Subject(
                    EhrSubject::new(namespace, id),
                    span_of(&pairs)?,
                ))
            }
            (false, true) => {
                checked(&pairs, &CARER)?;
                Ok(Self::Carer(carer_of(&pairs)?))
            }
            _ => Err(malformed(
                "the care relationships are listed by subject_id and subject_namespace, or by \
                 carer, never both",
            )),
        }
    }
}

/// The span the query string `query` of a list addressed by the subject's
/// `PERSON` names, which takes `at` and `all` alone.
///
/// # Errors
/// A `400` naming the first parameter outside that form, or given twice.
pub(super) fn span(query: Option<&str>) -> Result<CareSpan, RestError> {
    let pairs = query.map(query_pairs).unwrap_or_default();
    checked(&pairs, &["at", "all"])?;
    span_of(&pairs)
}

/// Refuses a parameter of `pairs` outside `form` or given twice.
fn checked(pairs: &[(String, String)], form: &[&str]) -> Result<(), RestError> {
    let mut seen: Vec<&str> = Vec::new();
    for (name, _) in pairs {
        if !form.contains(&name.as_str()) {
            return Err(malformed(format!(
                "this form of the care relationship list takes no `{name}` parameter"
            )));
        }
        if seen.contains(&name.as_str()) {
            return Err(malformed(format!("`{name}` is given more than once")));
        }
        seen.push(name);
    }
    Ok(())
}

/// The value of the parameter `name` of `pairs`.
fn value<'p>(pairs: &'p [(String, String)], name: &str) -> Option<&'p str> {
    pairs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// The span `at` and `all` of `pairs` name.
fn span_of(pairs: &[(String, String)]) -> Result<CareSpan, RestError> {
    Ok(CareSpan {
        at: value(pairs, "at")
            .map(|day| day_of("at", day))
            .transpose()?,
        all: value(pairs, "all")
            .map(|all| boolean("all", all))
            .transpose()?
            .is_some_and(|all| all),
    })
}

/// The reverse reading `pairs` name.
fn carer_of(pairs: &[(String, String)]) -> Result<CarerQuery, RestError> {
    let carer = value(pairs, "carer")
        .and_then(|carer| uuid::Uuid::parse_str(carer).ok())
        .ok_or_else(|| malformed("`carer` is the container id of a party, a UUID"))?;
    Ok(CarerQuery {
        carer,
        role: value(pairs, "role").map(role).transpose()?,
        path: value(pairs, "path").map(path).transpose()?,
        at: value(pairs, "at")
            .map(|day| day_of("at", day))
            .transpose()?,
        offset: value(pairs, "offset")
            .map(|offset| count("offset", offset, 0, usize::MAX))
            .transpose()?
            .unwrap_or(0),
        fetch: value(pairs, "fetch")
            .map(|fetch| count("fetch", fetch, 1, MAX_FETCH))
            .transpose()?
            // NOTE: our own design (#6076), a page of 100 when `fetch` is absent.
            .unwrap_or(100),
    })
}

/// `value`, a day `YYYY-MM-DD`, as a date.
fn day_of(name: &str, value: &str) -> Result<jiff::civil::Date, RestError> {
    value.parse().map_err(|_unparsed: jiff::Error| {
        malformed(format!("`{name}` is a day YYYY-MM-DD, such as 2026-11-20"))
    })
}

/// `value`, `true` or `false`.
fn boolean(name: &str, value: &str) -> Result<bool, RestError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(malformed(format!("`{name}` is true or false"))),
    }
}

/// `value`, a count from `low` to `high`.
fn count(name: &str, value: &str, low: usize, high: usize) -> Result<usize, RestError> {
    value
        .parse::<usize>()
        .ok()
        .filter(|count| (low..=high).contains(count))
        .ok_or_else(|| malformed(format!("`{name}` is a whole number from {low} to {high}")))
}

/// `value`, a `care_role` term as the token `terminology_id|code` (FHIR R4
/// search.html §3.1.1.4.10), as the caseload members name one.
fn role(value: &str) -> Result<ForeignCode, RestError> {
    match Token::parse(value) {
        Ok(Token::SystemCode { system, code }) => Ok(ForeignCode {
            terminology: system,
            code,
        }),
        Ok(_) | Err(_) => Err(malformed(
            "`role` is a care_role term as the token terminology_id|code, such as \
             HL7_ParticipationType|RESP",
        )),
    }
}

/// `value`, the path `direct` or `team`.
fn path(value: &str) -> Result<CarePath, RestError> {
    match value {
        "direct" => Ok(CarePath::Direct),
        "team" => Ok(CarePath::Team),
        _ => Err(malformed("`path` is direct or team")),
    }
}

/// The `400` of a parameter outside its form.
fn malformed(message: impl Into<String>) -> RestError {
    RestError::from(ApiError::BadRequest(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(query: &str) -> Result<CareQuery, RestError> {
        CareQuery::parse(Some(query))
    }

    #[test]
    fn the_subject_form_takes_its_reference_and_span() {
        let Ok(CareQuery::Subject(subject, span)) =
            parsed("subject_id=p1&subject_namespace=urn%3Aex&at=2026-11-20&all=true")
        else {
            panic!("the subject form");
        };
        assert_eq!(subject, EhrSubject::new("urn:ex", "p1"));
        assert_eq!(span.at, Some(jiff::civil::date(2026, 11, 20)));
        assert!(span.all);
    }

    #[test]
    fn the_carer_form_takes_its_filters_and_page() {
        let carer = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
        let Ok(CareQuery::Carer(query)) = parsed(&format!(
            "carer={carer}&role=HL7_ParticipationType%7CRESP&path=team&offset=5&fetch=10"
        )) else {
            panic!("the carer form");
        };
        assert_eq!(query.carer.to_string(), carer);
        assert_eq!(
            query.role,
            Some(ForeignCode {
                terminology: "HL7_ParticipationType".to_owned(),
                code: "RESP".to_owned()
            })
        );
        assert_eq!(query.path, Some(CarePath::Team));
        assert_eq!((query.offset, query.fetch), (5, 10));
    }

    #[test]
    fn a_person_list_takes_at_and_all_alone() {
        let Ok(span) = span(Some("all=true")) else {
            panic!("the span form");
        };
        assert!(span.all && span.at.is_none());
        for query in ["subject_id=p1", "carer=x", "all=true&all=false", "at=today"] {
            assert!(super::span(Some(query)).is_err(), "{query:?} is refused");
        }
    }

    #[test]
    fn a_parameter_outside_its_form_is_refused() {
        let carer = "carer=3fa85f64-5717-4562-b3fc-2c963f66afa6";
        for query in [
            "",
            "subject_id=p1",
            "subject_id=p1&subject_namespace=n&carer=x",
            "subject_id=p1&subject_namespace=n&fetch=3",
            "subject_id=p1&subject_namespace=n&all=yes",
            "subject_id=p1&subject_namespace=n&at=20-11-2026",
            "subject_id=p1&subject_id=p2&subject_namespace=n",
            "carer=not-a-uuid",
            &format!("{carer}&all=true"),
            &format!("{carer}&path=household"),
            &format!("{carer}&role=RESP"),
            &format!("{carer}&fetch=0"),
            &format!("{carer}&fetch=1001"),
        ] {
            assert!(parsed(query).is_err(), "{query:?} is refused");
        }
    }
}
