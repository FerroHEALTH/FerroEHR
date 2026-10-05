// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Request helpers the generated `*Params` decoding does not cover: the AQL
//! binds of a query execution, the query reads of the extension routes, and the
//! `openehr-item-tag` header wrappers.
//!
//! An ITS-REST operation's parameters decode through its generated
//! `*Params::from_request` (`openehr_its::rest::generated`). A query string read
//! here decodes the same way: RFC 3986 §2.1 percent-decoding, under which `+` is
//! a literal plus, because the OAS gives every query parameter `style: form`,
//! which OAS 3.0.3 §Parameter Object defines by RFC 6570 form expansion.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 9): the wire boundary — an AQL bind is \
              a JSON criteria value, which the query request carries as one"
)]

use std::collections::BTreeMap;

use http::{HeaderMap, HeaderValue};
use serde_json::Value;

use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::ItemTag;

/// The query-string keys of the query-execution operations that are request
/// controls rather than AQL binds.
pub(crate) const QUERY_RESERVED_KEYS: &[&str] =
    &["ehr_id", "offset", "fetch", "q", "query_parameters"];

/// The pairs of `query`, each name and value percent-decoded (RFC 3986 §2.1).
///
/// Octets that do not decode to UTF-8 are replaced, never dropped, so a pair
/// keeps its place.
fn query_pairs(query: &str) -> Vec<(String, String)> {
    let decode = |text: &str| {
        String::from_utf8_lossy(&urlencoding::decode_binary(text.as_bytes())).into_owned()
    };
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(name), decode(value))
        })
        .collect()
}

/// Looks up a single percent-decoded query-string parameter by key.
pub(crate) fn query_param(query: Option<&str>, key: &str) -> Option<String> {
    query_pairs(query?)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

/// The members of the query string's named binds, outside `reserved`, each
/// read as the generated `query_parameters` decoding reads a member: JSON text
/// other than a JSON string is that value, anything else the text itself.
pub(crate) fn url_members(query: Option<&str>, reserved: &[&str]) -> BTreeMap<String, Value> {
    let Some(query) = query else {
        return BTreeMap::new();
    };
    query_pairs(query)
        .into_iter()
        .filter(|(key, _)| !reserved.contains(&key.as_str()))
        .map(|(key, raw)| {
            let value = if raw.starts_with('"') {
                Value::String(raw)
            } else {
                serde_json::from_str::<Value>(&raw).unwrap_or(Value::String(raw))
            };
            (key, value)
        })
        .collect()
}

/// The AQL binds of a query execution's `query_parameters` members (ITS-REST
/// `docs/query/Request.md` §Query parameters).
///
/// A `$` prefix is stripped: members "SHOULD NOT be prefixed with `$` sign" and
/// the server adds it. A member named `query_parameters` holding a JSON object
/// contributes its entries, and a named member wins a collision with one. An
/// array or object value binds as its JSON text, because an AQL parameter
/// substitutes a criteria value (QUERY master03 §Parameters).
///
/// # Errors
/// Returns [`ApiError::BadRequest`] when a `query_parameters` member is not a
/// JSON object.
pub(crate) fn aql_binds(
    members: Option<BTreeMap<String, Value>>,
) -> Result<BTreeMap<String, Value>, ApiError> {
    let mut members = members.unwrap_or_default();
    let mut binds = BTreeMap::new();
    // NOTE: no openEHR spec governs a literal `query_parameters=<JSON object>`
    // pair; our own extension accepts it beside the exploded form.
    if let Some(object) = members.remove("query_parameters") {
        let Value::Object(entries) = object else {
            return Err(ApiError::BadRequest(
                "the query parameter `query_parameters` is not a JSON object".to_owned(),
            ));
        };
        for (key, value) in entries {
            insert_bind(&mut binds, &key, value);
        }
    }
    for (key, value) in members {
        insert_bind(&mut binds, &key, value);
    }
    Ok(binds)
}

/// Binds `value` as `key`, the `$` stripped and a structured value as its text.
fn insert_bind(binds: &mut BTreeMap<String, Value>, key: &str, value: Value) {
    let name = key.strip_prefix('$').unwrap_or(key);
    if name.is_empty() {
        return;
    }
    let value = match value {
        Value::Array(_) | Value::Object(_) => Value::String(value.to_string()),
        scalar => scalar,
    };
    binds.insert(name.to_owned(), value);
}

// The `openehr-item-tag` / `openehr-version-item-tag` wrappers over the
// dedicated ITEM_TAG operations (overview §"openehr-item-tag and
// openehr-version-item-tag"): a `;`-separated list of entries, each a
// comma-separated `key`/`value`/`target_path` set, targeting a
// VERSIONED_OBJECT or one VERSION respectively.
// NOTE: this module owns only header parse/validate/emit; the EHR group
// validates before the content commit and writes after it, so a defective tag
// refuses the request and a tag never re-versions its content.

/// The canonical HTTP header names for the two `ITEM_TAG` wrapper headers.
pub(crate) const H_ITEM_TAG: &str = "openehr-item-tag";
pub(crate) const H_VERSION_ITEM_TAG: &str = "openehr-version-item-tag";

/// A single `openehr-item-tag` / `openehr-version-item-tag` entry: a `key`, its
/// optional `value`, and an optional `target_path`. Multiple `ITEM_TAGs` may
/// target one resource, uniquely identified by their `key`+`target_path` pair
/// (overview §"openehr-item-tag and openehr-version-item-tag").
///
/// `value` is `Option` because `ITEM_TAG.value` is `0..1` and
/// `Inv_value_valid` forbids a set-but-empty one: a header entry spelling
/// `value=""` normalizes to absent on the way in, and the echo renders no
/// `value` token on the way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemTagHeaderEntry {
    /// The tag key.
    pub(crate) key: String,
    /// The tag value, absent for a bare marker tag.
    pub(crate) value: Option<String>,
    /// Optional RM path the tag is anchored to within the target.
    pub(crate) target_path: Option<String>,
}

/// Splits a wrapper-header value into its `;`-separated entries, quote-aware.
///
/// A `target_path` is a quoted token that may legitimately contain a `;` (an AQL
/// path predicate such as `[at0001, 'a;b']`), so this scanner breaks only on a
/// `;` outside a double-quoted run, exactly as [`key_value_pairs`] treats a
/// quoted value as opaque at the `,` level.
///
/// The release gives the header no ABNF — the grammar is one worked example
/// (`Requests_and_responses.md` §openehr-item-tag and openehr-version-item-tag)
/// showing quoted values and both separators but no escaping rules. Treating a
/// quoted run as opaque is our own reading, and the only one under which the
/// section's own `target_path="/composition/start_time/value"` token stays
/// meaningful when a path contains a separator.
fn split_item_tag_entries(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut in_quotes = false;
    let mut start = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => {
                if let Some(segment) = input.get(start..idx) {
                    out.push(segment);
                }
                start = idx + 1;
            }
            _ => {}
        }
    }
    if let Some(tail) = input.get(start..) {
        out.push(tail);
    }
    out
}

/// Parses an `ITEM_TAG` wrapper header ([`H_ITEM_TAG`] or
/// [`H_VERSION_ITEM_TAG`]) into its entries, merging repeated occurrences.
///
/// Returns `None` when the header is absent and `Some(empty)` when it is present
/// but empty — the spec's "remove all `ITEM_TAGs`" signal.
///
/// # Errors
/// [`ApiError::BadRequest`] when a non-blank entry carries no `key`: the
/// released schema makes `key` the one required member of an `UPDATE_ITEM_TAG`
/// (`schemas/common/UpdateItemTag.yaml`), so the wrapper cannot admit what the
/// operation refuses. A blank segment carries no entry at all and is not an
/// error.
pub(crate) fn parse_item_tag_header(
    headers: &HeaderMap,
    name: &str,
) -> Result<Option<Vec<ItemTagHeaderEntry>>, ApiError> {
    // An undecodable value is refused, never skipped: dropping it would remove
    // a tag the client believes it set.
    let raws: Vec<String> = headers
        .get_all(name)
        .iter()
        .map(|v| {
            v.to_str().map(str::to_owned).map_err(|e| {
                tracing::debug!(header = name, error = %e, "undecodable header value → 400");
                ApiError::BadRequest(format!(
                    "header {name} carries a value that is not decodable as text"
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    if raws.is_empty() {
        return Ok(None);
    }
    let joined = raws.join(";");
    // An empty value "will effectively remove all ITEM_TAGs" for the target.
    if joined.trim().is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut out = Vec::new();
    for segment in split_item_tag_entries(&joined) {
        if segment.trim().is_empty() {
            continue;
        }
        let pairs = key_value_pairs(segment);
        let Some(key) = tag_value(&pairs, "key") else {
            return Err(ApiError::BadRequest(format!(
                "the {name} header entry {segment:?} carries no `key`; every ITEM_TAG \
                 entry must name one"
            )));
        };
        out.push(ItemTagHeaderEntry {
            key,
            // `value=""` is the absent value, never a stored empty string (RM
            // `ITEM_TAG.Inv_value_valid`: a set value may not be empty).
            value: tag_value(&pairs, "value").filter(|v| !v.is_empty()),
            target_path: tag_value(&pairs, "target_path"),
        });
    }
    Ok(Some(out))
}

/// Judges parsed wrapper-header entries against the RM `ITEM_TAG` invariants,
/// before the request's content is committed.
///
/// The invariants are evaluated through `openehr_rm`'s own predicates
/// ([`ItemTag::key_valid`] / [`ItemTag::value_valid`]), which the `Validate` impl
/// for `ItemTag` is written in terms of, so this check and the service seam that
/// writes the tags cannot disagree. They are reached as predicates rather than
/// through `Validate` because the version the tag's `target` names is not minted
/// yet, so no `ITEM_TAG` instance exists.
///
/// # Errors
/// [`ApiError::Unprocessable`] naming the offending entry and the invariant it
/// breaks (`Inv_key_valid` / `Inv_value_valid`,
/// `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.item_tag.adoc`).
pub(crate) fn validate_item_tag_entries(
    entries: &[ItemTagHeaderEntry],
    name: &str,
) -> Result<(), ApiError> {
    for entry in entries {
        if !ItemTag::key_valid(&entry.key) {
            return Err(ApiError::Unprocessable(format!(
                "the {name} header entry key {:?} breaks ITEM_TAG.Inv_key_valid \
                 (a key may not be empty or carry leading or trailing whitespace)",
                entry.key
            )));
        }
        if !ItemTag::value_valid(entry.value.as_deref()) {
            return Err(ApiError::Unprocessable(format!(
                "the {name} header entry {:?} breaks ITEM_TAG.Inv_value_valid \
                 (a value, if set, may not be empty)",
                entry.key
            )));
        }
    }
    Ok(())
}

/// Renders `ITEM_TAG` entries as a wrapper-header value (`;`-separated
/// `key="…"[,value="…"][,target_path="…"]` pairs), for echoing stored tags on a
/// response (overview §"Usage in Responses", a MAY).
///
/// Returns `None` when the list cannot be rendered as a header value — a key or
/// value carrying a byte HTTP forbids in a field value (RFC 9110 §5.5), which
/// nothing in the RM bars from an `ITEM_TAG.key`. The caller must then omit the
/// header entirely and never fall back to an empty one: an empty value is the
/// release's instruction that "providing an empty value for this header will
/// effectively remove all `ITEM_TAGs` associated with the given target"
/// (§Usage in Requests), so an echo of it would hand the client a destructive
/// form as state. A valueless tag renders without a `value` token for the same
/// reason: a `value=""` echo describes a tag violating `Inv_value_valid`.
pub(crate) fn emit_item_tag_header(entries: &[ItemTagHeaderEntry]) -> Option<HeaderValue> {
    if entries.is_empty() {
        return None;
    }
    let rendered = entries
        .iter()
        .map(|e| {
            let mut parts = vec![format!("key=\"{}\"", e.key)];
            if let Some(value) = &e.value {
                parts.push(format!("value=\"{value}\""));
            }
            if let Some(tp) = &e.target_path {
                parts.push(format!("target_path=\"{tp}\""));
            }
            parts.join(",")
        })
        .collect::<Vec<_>>()
        .join("; ");
    HeaderValue::from_str(&rendered).ok()
}

/// Projects one RM [`ItemTag`] onto the [`ItemTagHeaderEntry`]
/// [`emit_item_tag_header`] renders: the three members the header grammar
/// carries, read off the typed instance.
pub(crate) fn item_tag_to_header_entry(tag: &ItemTag) -> ItemTagHeaderEntry {
    ItemTagHeaderEntry {
        key: tag.key().to_owned(),
        value: tag.value().map(str::to_owned),
        target_path: tag.target_path().map(str::to_owned),
    }
}

/// Returns the value of a parsed `key` in a tag-pair segment.
fn tag_value(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

/// Parses a tolerant comma-separated list of `key="value"` or bare `key=value`
/// pairs.
///
/// The one scanner behind both the tag-pair segments here and the committal
/// attribute headers ([`crate::overview::committal`]): a double-quoted value is
/// read opaquely, a bare value runs to the next top-level comma, and whitespace
/// around separators and keys is trimmed. Both grammars are example-only in the
/// ITS-REST overview, hence the shared tolerant reader.
pub(crate) fn key_value_pairs(input: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = input;
    loop {
        // Skip leading separators/whitespace.
        rest = rest.trim_start_matches(|c: char| c == ',' || c.is_ascii_whitespace());
        // Read the key up to '='. A segment whose next delimiter is a comma
        // carries no '=' — not a pair; leave the comma for the skip above.
        let Some((key, after_key)) = rest.find(['=', ',']).and_then(|d| rest.split_at_checked(d))
        else {
            break;
        };
        let Some(after_eq) = after_key.strip_prefix('=') else {
            rest = after_key;
            continue;
        };
        // Read the value: quoted (opaque) or bare (to next comma).
        let (value, tail) = if let Some(quoted) = after_eq.strip_prefix('"') {
            match quoted.find('"').and_then(|q| quoted.split_at_checked(q)) {
                // Consume the closing quote; anything before the next comma is
                // then skipped as a keyless segment.
                Some((v, after_v)) => (v.to_owned(), after_v.get(1..).unwrap_or_default()),
                None => (quoted.to_owned(), ""),
            }
        } else {
            match after_eq
                .find(',')
                .and_then(|c| after_eq.split_at_checked(c))
            {
                Some((v, after_v)) => (v.trim().to_owned(), after_v),
                None => (after_eq.trim().to_owned(), ""),
            }
        };
        let key = key.trim();
        if !key.is_empty() {
            out.push((key.to_owned(), value));
        }
        rest = tail;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── AQL binds (Request.md §Query parameters) ─────────────────────────────

    /// The documented GET form: named members become AQL binds (worked example
    /// `?temperature_from=36&temperature_unit=Cel`), JSON-typed, `$` stripped,
    /// a `query_parameters` object merged with named-wins collisions.
    #[test]
    fn aql_binds_follow_the_docs_text() {
        let members = url_members(
            Some(
                "temperature_from=36&temperature_unit=Cel&$flagged=true\
                 &uid=90910cf0-66a0-4382-b1f8-c0f27e81b42d::openEHRSys.example.com::1\
                 &offset=10&fetch=5&ehr_id=abc&q=SELECT\
                 &shared=named-form&name=a+b",
            ),
            &["ehr_id", "offset", "fetch", "q"],
        );
        let mut members = members;
        members.insert(
            "query_parameters".to_owned(),
            json!({"from_object": "x", "shared": "object-form"}),
        );
        let got = aql_binds(Some(members)).expect("binds");
        assert_eq!(got["temperature_from"], json!(36));
        assert_eq!(got["temperature_unit"], json!("Cel"));
        assert_eq!(got["flagged"], json!(true), "$ prefix stripped, JSON-typed");
        assert_eq!(
            got["uid"],
            json!("90910cf0-66a0-4382-b1f8-c0f27e81b42d::openEHRSys.example.com::1"),
            "a version uid stays text"
        );
        assert_eq!(got["from_object"], json!("x"), "object-form binds survive");
        assert_eq!(
            got["shared"],
            json!("named-form"),
            "named form wins a collision"
        );
        // RFC 3986 gives `+` no meaning in a query component, and the OAS
        // `style: form` (RFC 6570) percent-encodes a space as `%20`.
        assert_eq!(got["name"], json!("a+b"), "a plus is a literal plus");
        for reserved in QUERY_RESERVED_KEYS {
            assert!(
                !got.contains_key(*reserved),
                "reserved control {reserved:?} must not bind"
            );
        }
    }

    /// Structured JSON stays text (an array or object is not an AQL criteria
    /// value), a non-object `query_parameters` is refused, and no members bind
    /// nothing.
    #[test]
    fn aql_binds_edges() {
        let got = aql_binds(Some(url_members(Some("list=%5B1%2C2%5D"), &[]))).expect("binds");
        assert_eq!(got["list"], json!("[1,2]"), "structured literals stay text");
        let refused = aql_binds(Some(
            [("query_parameters".to_owned(), json!(7))]
                .into_iter()
                .collect(),
        ));
        assert!(
            matches!(refused, Err(ApiError::BadRequest(_))),
            "{refused:?}"
        );
        assert!(aql_binds(None).expect("binds").is_empty());
    }

    /// RFC 3986 §2.1: `%20` is a space and `+` stays a plus.
    #[test]
    fn percent_and_plus_decoding() {
        let pairs = query_pairs("q=SELECT%20c&name=a+b&tz=2024-01-01T00:00:00+01:00");
        assert_eq!(
            pairs,
            vec![
                ("q".to_owned(), "SELECT c".to_owned()),
                ("name".to_owned(), "a+b".to_owned()),
                ("tz".to_owned(), "2024-01-01T00:00:00+01:00".to_owned()),
            ]
        );
    }

    // ── openehr-item-tag / openehr-version-item-tag ─────────────────────────

    fn parsed(h: &HeaderMap, name: &str) -> Option<Vec<ItemTagHeaderEntry>> {
        parse_item_tag_header(h, name).expect("a well-formed item-tag header")
    }

    fn entry(key: &str, value: Option<&str>, target_path: Option<&str>) -> ItemTagHeaderEntry {
        ItemTagHeaderEntry {
            key: key.to_owned(),
            value: value.map(str::to_owned),
            target_path: target_path.map(str::to_owned),
        }
    }

    #[test]
    fn item_tag_header_absent_is_none() {
        assert_eq!(parsed(&HeaderMap::new(), H_ITEM_TAG), None);
    }

    #[test]
    fn item_tag_header_empty_value_clears() {
        let mut h = HeaderMap::new();
        h.insert(H_ITEM_TAG, HeaderValue::from_static(""));
        // Present-but-empty ⇒ "remove all ITEM_TAGs".
        assert_eq!(parsed(&h, H_ITEM_TAG), Some(Vec::new()));
    }

    #[test]
    fn item_tag_single_pair() {
        let mut h = HeaderMap::new();
        h.insert(
            H_ITEM_TAG,
            HeaderValue::from_static("key=\"category\",value=\"final\""),
        );
        assert_eq!(
            parsed(&h, H_ITEM_TAG),
            Some(vec![entry("category", Some("final"), None)])
        );
    }

    #[test]
    fn version_item_tag_semicolon_list_with_target_path() {
        // The spec example (line 108).
        let mut h = HeaderMap::new();
        h.insert(
            H_VERSION_ITEM_TAG,
            HeaderValue::from_static(
                "key=\"reviewed\",value=\"true\"; key=\"flag\",value=\"follow-up\",target_path=\"/composition/start_time/value\"",
            ),
        );
        let entries = parsed(&h, H_VERSION_ITEM_TAG).expect("entries");
        assert_eq!(
            entries,
            vec![
                entry("reviewed", Some("true"), None),
                entry(
                    "flag",
                    Some("follow-up"),
                    Some("/composition/start_time/value")
                ),
            ]
        );
    }

    #[test]
    fn item_tag_repeated_headers_merge() {
        let mut h = HeaderMap::new();
        h.append(
            H_ITEM_TAG,
            HeaderValue::from_static("key=\"a\",value=\"1\""),
        );
        h.append(
            H_ITEM_TAG,
            HeaderValue::from_static("key=\"b\",value=\"2\""),
        );
        let entries = parsed(&h, H_ITEM_TAG).expect("entries");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "a");
        assert_eq!(entries[1].key, "b");
    }

    #[test]
    fn item_tag_emit_round_trips() {
        let entries = vec![
            entry("reviewed", Some("true"), None),
            entry(
                "flag",
                Some("follow-up"),
                Some("/composition/start_time/value"),
            ),
        ];
        let hv = emit_item_tag_header(&entries).expect("an encodable list");
        let mut h = HeaderMap::new();
        h.insert(H_VERSION_ITEM_TAG, hv);
        assert_eq!(parsed(&h, H_VERSION_ITEM_TAG), Some(entries));
    }

    #[test]
    fn a_valueless_tag_echoes_without_a_value_token() {
        // RM `ITEM_TAG.value` is 0..1 and `Inv_value_valid` forbids a
        // set-but-empty one, so `value=""` would describe a tag this server
        // never stored — and a client mirroring it back would post an invalid
        // tag. The token is omitted instead.
        let hv = emit_item_tag_header(&[entry("marker", None, None)]).expect("an encodable list");
        assert_eq!(hv.to_str().expect("ascii"), r#"key="marker""#);
        // …and it round-trips to the same valueless entry.
        let mut h = HeaderMap::new();
        h.insert(H_ITEM_TAG, hv);
        assert_eq!(
            parsed(&h, H_ITEM_TAG),
            Some(vec![entry("marker", None, None)])
        );
    }

    #[test]
    fn an_unencodable_list_yields_no_header_rather_than_an_empty_one() {
        // An EMPTY `openehr-item-tag` is the release's "remove all ITEM_TAGs"
        // instruction (overview §Usage in Requests), so it must never be the
        // fallback for a list that cannot be rendered: the caller omits the
        // header entirely. A control character in the key is the reachable
        // case: nothing in the RM forbids one (`Inv_key_valid` bars only an
        // empty or whitespace-padded key), while RFC 9110 §5.5 bars it from a
        // field value.
        assert_eq!(emit_item_tag_header(&[entry("a\nb", None, None)]), None);
        // Non-ASCII text is NOT the unencodable case — obs-text is legal in a
        // field value — so such a list still echoes.
        assert!(emit_item_tag_header(&[entry("café", Some("naïve"), None)]).is_some());
    }

    #[test]
    fn a_request_value_of_empty_string_is_the_absent_value() {
        let mut h = HeaderMap::new();
        h.insert(H_ITEM_TAG, HeaderValue::from_static("key=\"k\",value=\"\""));
        assert_eq!(parsed(&h, H_ITEM_TAG), Some(vec![entry("k", None, None)]));
    }

    #[test]
    fn a_quoted_semicolon_does_not_split_the_entry() {
        // A `target_path` is an AQL or RM path (RM `item_tag.adoc`
        // `target_path`), and an AQL predicate may carry a `;` inside a quoted
        // string. Splitting on the raw `;` shattered such an entry into
        // fragments that then parsed as garbage.
        let mut h = HeaderMap::new();
        h.insert(
            H_ITEM_TAG,
            HeaderValue::from_static(
                "key=\"flag\",target_path=\"/items[at0001, 'a;b']/value\"; key=\"other\"",
            ),
        );
        assert_eq!(
            parsed(&h, H_ITEM_TAG),
            Some(vec![
                entry("flag", None, Some("/items[at0001, 'a;b']/value")),
                entry("other", None, None),
            ])
        );
    }

    #[test]
    fn a_keyless_entry_is_refused_not_skipped() {
        // `key` is the one REQUIRED member of an UPDATE_ITEM_TAG
        // (`schemas/common/UpdateItemTag.yaml`), and the header is a wrapper
        // around that operation — so the wrapper cannot admit what the
        // operation refuses. Skipping the entry would silently drop a tag the
        // client believes it set.
        let mut h = HeaderMap::new();
        h.insert(
            H_ITEM_TAG,
            HeaderValue::from_static("key=\"a\"; value=\"orphan\""),
        );
        let refused = parse_item_tag_header(&h, H_ITEM_TAG);
        assert!(
            matches!(refused, Err(ApiError::BadRequest(_))),
            "a keyless entry must be a 400, got {refused:?}"
        );
    }

    #[test]
    fn a_blank_segment_carries_no_entry_and_is_not_an_error() {
        // A trailing `;`, or an empty repeat of the header, carries no entry at
        // all — the release's own empty-value form is meaningful, so a blank
        // segment is not a defect.
        let mut h = HeaderMap::new();
        h.append(H_ITEM_TAG, HeaderValue::from_static("key=\"a\";"));
        h.append(H_ITEM_TAG, HeaderValue::from_static(""));
        assert_eq!(parsed(&h, H_ITEM_TAG), Some(vec![entry("a", None, None)]));
    }

    #[test]
    fn the_entry_validator_enforces_the_rm_invariants() {
        // Both refusals, and the accepting twin.
        assert!(validate_item_tag_entries(&[entry("ok", Some("v"), None)], H_ITEM_TAG).is_ok());
        assert!(validate_item_tag_entries(&[entry("ok", None, None)], H_ITEM_TAG).is_ok());
        for bad in [entry(" padded ", None, None), entry("", None, None)] {
            let refused = validate_item_tag_entries(std::slice::from_ref(&bad), H_ITEM_TAG);
            assert!(
                matches!(refused, Err(ApiError::Unprocessable(_))),
                "{bad:?} breaks Inv_key_valid and must be refused, got {refused:?}"
            );
        }
        let refused = validate_item_tag_entries(&[entry("k", Some(""), None)], H_ITEM_TAG);
        assert!(
            matches!(refused, Err(ApiError::Unprocessable(_))),
            "a set-but-empty value breaks Inv_value_valid, got {refused:?}"
        );
    }
}
