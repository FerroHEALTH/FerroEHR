// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The episodes of care a COMPOSITION is placed in (draft access control,
//! Episodes of Care).
//!
//! A composition names an episode with an `ELEMENT` anywhere in its
//! `context.other_context` whose `name` is the code `episode` of the
//! `record placement element` group and whose value is the episode's
//! `DV_IDENTIFIER`. RM EHR IM §Event Context leaves `other_context` to the
//! archetype and openEHR places no data in an episode, so the element and
//! its rules are our own design (#6413).

use std::borrow::Cow;

use openehr_rm::draft::access_control::terminology::care::PlacementElement;
use openehr_rm::draft::access_control::terminology::{TERMINOLOGY_ID_ACCESS, Term as _};
use sha2::Digest as _;

use crate::error::{ServiceError, Violation};
use crate::versioning::tree::{Shape, Tree};

/// Where a placement element sits.
const PATH: &str = "EVENT_CONTEXT.other_context";

/// One episode of care as the head row keeps it: SHA-256 of its `issuer`,
/// an octet `0x00` and its `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EpisodeKey([u8; 32]);

impl EpisodeKey {
    /// The key of the episode `issuer` gives the identifier `id`.
    #[must_use]
    pub fn of(issuer: &str, id: &str) -> Self {
        let mut digest = sha2::Sha256::new();
        digest.update(issuer.as_bytes());
        digest.update([0]);
        digest.update(id.as_bytes());
        Self(digest.finalize().into())
    }

    /// The key's 32 octets.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The key whose 32 octets are `octets`; `None` for any other length.
    #[must_use]
    pub fn from_octets(octets: &[u8]) -> Option<Self> {
        octets.try_into().ok().map(Self)
    }

    /// The key in lower-case hexadecimal, as a settings class's facts hold it.
    #[must_use]
    pub fn hex(&self) -> String {
        crate::digest::hex(&self.0)
    }

    /// The key `hex` spells in hexadecimal; `None` when it spells no key.
    #[must_use]
    pub fn from_hex(hex: &str) -> Option<Self> {
        let digits = hex.as_bytes();
        if digits.len() != 64 || !digits.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let mut key = [0_u8; 32];
        for (octet, pair) in key.iter_mut().zip(digits.as_chunks::<2>().0) {
            let text = std::str::from_utf8(pair).ok()?;
            *octet = u8::from_str_radix(text, 16).ok()?;
        }
        Some(Self(key))
    }
}

/// The episodes the COMPOSITION `body` is placed in, each once and sorted;
/// empty when it is placed in none.
///
/// # Errors
/// [`ServiceError::Unprocessable`] (`422`) naming the element when an
/// `episode` element's value is no `DV_IDENTIFIER` with an `issuer` and an
/// `id`, and when a persistent composition (`openehr::431`) names an episode:
/// it is the patient-lifetime record (RM EHR IM §Persistent Compositions).
pub(crate) fn placed<'a, T: Tree<'a>>(body: T) -> Result<Vec<EpisodeKey>, ServiceError> {
    let Some(context) = body.get("context").and_then(|c| c.get("other_context")) else {
        return Ok(Vec::new());
    };
    let mut keys = Vec::new();
    let mut malformed = false;
    walk(context, &mut |element| match identifier(element) {
        Some((issuer, id)) => keys.push(EpisodeKey::of(&issuer, &id)),
        None => malformed = true,
    });
    if malformed {
        return Err(refused(
            "names an episode whose value is no DV_IDENTIFIER with an issuer and an id",
        ));
    }
    if !keys.is_empty() && persistent(body) {
        return Err(refused(
            "places a persistent COMPOSITION in an episode; a persistent composition is the \
             patient-lifetime record and is placed in none (RM EHR IM §Persistent Compositions)",
        ));
    }
    keys.sort_unstable();
    keys.dedup();
    Ok(keys)
}

/// Calls `found` with each `episode` element under `at`.
fn walk<'a, T: Tree<'a>>(at: T, found: &mut dyn FnMut(T)) {
    match at.shape() {
        Shape::Object if is_placement(at) => found(at),
        Shape::Object => at.each_member(&mut |_, value| walk(value, found)),
        Shape::Array => at.each_item(&mut |item| walk(item, found)),
        Shape::String | Shape::Other => {}
    }
}

/// Whether `at` is an `ELEMENT` named by the placement code.
fn is_placement<'a, T: Tree<'a>>(at: T) -> bool {
    let Some(code) = at.get("name").and_then(|name| name.get("defining_code")) else {
        return false;
    };
    at.get_text("_type").as_deref() == Some("ELEMENT")
        && code
            .get("terminology_id")
            .and_then(|t| t.get_text("value"))
            .as_deref()
            == Some(TERMINOLOGY_ID_ACCESS)
        && code.get_text("code_string").as_deref() == Some(PlacementElement::Episode.code())
}

/// The non-blank `issuer` and `id` of the element's `DV_IDENTIFIER` value.
fn identifier<'a, T: Tree<'a>>(element: T) -> Option<(Cow<'a, str>, Cow<'a, str>)> {
    let value = element.get("value")?;
    if value.get_text("_type").as_deref() != Some("DV_IDENTIFIER") {
        return None;
    }
    let issuer = value.get_text("issuer").filter(|s| !s.trim().is_empty())?;
    let id = value.get_text("id").filter(|s| !s.trim().is_empty())?;
    Some((issuer, id))
}

/// Whether the composition's `category` is `openehr::431|persistent|`.
fn persistent<'a, T: Tree<'a>>(body: T) -> bool {
    let Some(code) = body.get("category").and_then(|c| c.get("defining_code")) else {
        return false;
    };
    code.get("terminology_id")
        .and_then(|t| t.get_text("value"))
        .as_deref()
        == Some("openehr")
        && code.get_text("code_string").as_deref() == Some("431")
}

/// The `422` of a placement the commit refuses.
fn refused(detail: &str) -> ServiceError {
    ServiceError::content_invalid(
        Violation::new(detail).with_path(format!("{PATH} ELEMENT openehr_access::episode")),
    )
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "the tests write bodies as canonical values"
)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn element(value: &Value) -> Value {
        json!({ "_type": "ELEMENT", "archetype_node_id": "at9001",
            "name": { "_type": "DV_CODED_TEXT", "value": "episode",
                "defining_code": { "terminology_id": { "value": "openehr_access" },
                    "code_string": "episode" } },
            "value": value })
    }

    fn episode(issuer: &str, id: &str) -> Value {
        element(&json!({ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id }))
    }

    fn composition(category: &str, items: &[Value]) -> Value {
        json!({ "_type": "COMPOSITION",
            "category": { "_type": "DV_CODED_TEXT", "value": "c",
                "defining_code": { "terminology_id": { "value": "openehr" },
                    "code_string": category } },
            "context": { "_type": "EVENT_CONTEXT",
                "other_context": { "_type": "ITEM_TREE", "archetype_node_id": "at0001",
                    "items": [{ "_type": "CLUSTER", "archetype_node_id": "at0002",
                        "items": items }] } } })
    }

    #[test]
    fn each_episode_is_read_once_and_sorted() {
        let body = composition(
            "433",
            &[
                episode("H", "C-17"),
                episode("H", "D-4"),
                episode("H", "C-17"),
            ],
        );
        let mut expected = vec![EpisodeKey::of("H", "C-17"), EpisodeKey::of("H", "D-4")];
        expected.sort_unstable();
        assert_eq!(placed(&body).expect("placed"), expected);
    }

    #[test]
    fn a_composition_without_the_element_is_unplaced() {
        let body = composition("433", &[]);
        assert_eq!(placed(&body).expect("unplaced"), Vec::new());
        assert_eq!(
            placed(&json!({ "_type": "COMPOSITION" })).expect("unplaced"),
            Vec::new()
        );
    }

    #[test]
    fn a_placed_persistent_composition_and_a_malformed_episode_are_refused() {
        let persistent = composition("431", &[episode("H", "C-17")]);
        assert!(placed(&persistent).is_err());
        assert!(placed(&composition("431", &[])).is_ok());
        let blank = composition("433", &[episode("H", " ")]);
        assert!(placed(&blank).is_err());
        let text = composition(
            "433",
            &[element(&json!({ "_type": "DV_TEXT", "value": "C-17" }))],
        );
        assert!(placed(&text).is_err());
    }

    #[test]
    fn the_key_separates_issuer_and_id() {
        assert_ne!(EpisodeKey::of("Ha", "b"), EpisodeKey::of("H", "ab"));
    }

    #[test]
    fn a_key_round_trips_through_its_hex_and_octets() {
        let key = EpisodeKey::of("H", "C-17");
        assert_eq!(EpisodeKey::from_hex(&key.hex()), Some(key));
        assert_eq!(EpisodeKey::from_octets(key.as_bytes()), Some(key));
        let tail: String = key.hex().chars().skip(2).collect();
        assert_eq!(EpisodeKey::from_hex(&tail), None);
        assert_eq!(EpisodeKey::from_hex(&format!("+f{tail}")), None);
        assert_eq!(EpisodeKey::from_octets(&[0; 31]), None);
    }
}
