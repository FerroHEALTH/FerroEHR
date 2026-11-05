// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Versioning (RFC 7644 §3.14) over the entity tags of RFC 9110 §8.8.3 and
//! the preconditions of RFC 9110 §13.1.1 and §13.1.2.

use ferroehr_rest::scim::protocol::error::ScimType;
use ferroehr_rest::scim::protocol::etag::{self, EntityTag, ReadPrecondition};
use http::header::{IF_MATCH, IF_NONE_MATCH};
use http::{HeaderMap, HeaderValue, StatusCode};

use super::source::{self, Doc};

fn header_value(line: &str, name: &str) -> String {
    line.split_once(':')
        .filter(|(n, _)| n.trim() == name)
        .map(|(_, v)| v.trim().to_owned())
        .expect("a header line")
}

fn headers(name: &http::HeaderName, values: &[&str]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for value in values {
        map.append(
            name.clone(),
            HeaderValue::from_str(value).expect("a header value"),
        );
    }
    map
}

fn tag(text: &str) -> EntityTag {
    text.parse().expect("an entity tag")
}

#[test]
fn the_etag_header_equals_meta_version() {
    // RFC 7643 §3.1: `meta.version` "must be the same as the entity-tag (ETag)
    // HTTP response header".
    let header = header_value(&source::line(Doc::Rfc7644, 4009), "ETag");
    let response = source::json(Doc::Rfc7644, 4011..=4028, &[]);
    let version = response["meta"]["version"].as_str().expect("meta.version");
    assert_eq!(tag(&header), tag(version));
    assert!(tag(&header).is_weak());
    assert_eq!(tag(&header).opaque(), "e180ee84f0671b1");
    assert_eq!(tag(&header).to_string(), header);
    assert_eq!(
        EntityTag::weak("e180ee84f0671b1")
            .expect("valid")
            .to_string(),
        r#"W/"e180ee84f0671b1""#
    );
}

#[test]
fn entity_tags_follow_the_rfc_9110_grammar() {
    for good in [
        r#""xyzzy""#,
        r#"W/"xyzzy""#,
        r#""""#,
        r#"W/"8849182c::ferroehr.local::1""#,
    ] {
        tag(good);
    }
    for bad in [
        "xyzzy",
        r#"w/"x""#,
        "W/x",
        r#""a"b""#,
        r#""unterminated"#,
        "\"tab\there\"",
    ] {
        assert!(bad.parse::<EntityTag>().is_err(), "{bad}");
    }
    assert!(EntityTag::weak("has\"quote").is_err());
}

#[test]
fn comparison_follows_rfc_9110_table_3() {
    for number in [3661, 3663, 3665, 3667] {
        let cells: Vec<String> = source::line(Doc::Rfc9110, number)
            .split('|')
            .map(|c| c.trim().to_owned())
            .collect();
        let (first, second, strong, weak) = (
            tag(&cells[1]),
            tag(&cells[2]),
            cells[3] == "match",
            cells[4] == "match",
        );
        assert_eq!(first.strong_eq(&second), strong, "{cells:?}");
        assert_eq!(first.weak_eq(&second), weak, "{cells:?}");
    }
}

#[test]
fn if_match_admits_the_current_version_with_a_weak_comparison() {
    // RFC 7644 §3.14 sends a weak tag in `If-Match`, so SCIM compares weakly.
    let current = tag(r#"W/"a330bc54f0671c9""#);
    let printed = header_value(&source::line(Doc::Rfc7644, 1697), "If-Match");
    etag::if_match(&headers(&IF_MATCH, &[&printed]), &current).expect("the current version passes");
    etag::if_match(&HeaderMap::new(), &current).expect("no If-Match is accepted (§3.14: MAY)");
    etag::if_match(&headers(&IF_MATCH, &["*"]), &current)
        .expect("* matches a current representation");
    etag::if_match(
        &headers(&IF_MATCH, &[r#""old", W/"a330bc54f0671c9""#]),
        &current,
    )
    .expect("a list matches any member");
    etag::if_match(
        &headers(&IF_MATCH, &[r#""old""#, r#"W/"a330bc54f0671c9""#]),
        &current,
    )
    .expect("two fields are one list");

    for stale in [
        r#"W/"b431af54f0671a2""#,
        r#""old", "older""#,
        "not a tag",
        "*, W/\"a330bc54f0671c9\"",
    ] {
        let error = etag::if_match(&headers(&IF_MATCH, &[stale]), &current).expect_err(stale);
        assert_eq!(error.status(), StatusCode::PRECONDITION_FAILED, "{stale}");
        assert_eq!(error.scim_type(), None::<ScimType>);
    }
}

#[test]
fn if_none_match_on_a_read_is_not_modified() {
    let printed = header_value(&source::line(Doc::Rfc7644, 4049), "If-None-Match");
    let current = tag(r#"W/"e180ee84f0671b1""#);
    assert_eq!(
        etag::if_none_match_read(&headers(&IF_NONE_MATCH, &[&printed]), &current),
        ReadPrecondition::NotModified
    );
    assert_eq!(
        etag::if_none_match_read(&headers(&IF_NONE_MATCH, &["*"]), &current),
        ReadPrecondition::NotModified
    );
    assert_eq!(
        etag::if_none_match_read(&headers(&IF_NONE_MATCH, &[r#"W/"other""#]), &current),
        ReadPrecondition::Proceed
    );
    assert_eq!(
        etag::if_none_match_read(&HeaderMap::new(), &current),
        ReadPrecondition::Proceed
    );
    assert_eq!(
        etag::if_none_match_read(&headers(&IF_NONE_MATCH, &["garbage"]), &current),
        ReadPrecondition::Proceed
    );
}

#[test]
fn if_none_match_on_a_write_is_precondition_failed() {
    // RFC 9110 §13.1.2: "the 412 (Precondition Failed) status code for all
    // other request methods".
    let current = tag(r#"W/"e180ee84f0671b1""#);
    let error = etag::if_none_match_write(
        &headers(&IF_NONE_MATCH, &[r#"W/"e180ee84f0671b1""#]),
        &current,
    )
    .expect_err("matches");
    assert_eq!(error.status(), StatusCode::PRECONDITION_FAILED);
    etag::if_none_match_write(&headers(&IF_NONE_MATCH, &[r#""other""#]), &current)
        .expect("no match proceeds");
}
