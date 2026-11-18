// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The protection engine: the one object the commit path holds.
//!
//! **No openEHR spec governs this — our own design/extension.** It owns the
//! configured schemes, the root key and the store, and exposes the two
//! operations the rest of the server needs: take the protected identifiers out
//! of a body before it is stored, and put them back for a caller entitled to
//! see them.
//!
//! Built once at boot, like every other configured collaborator, so a
//! misconfigured key is a boot error rather than a first-write surprise.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): service-surface signatures over stored canonical \
              fragments and dynamic shapes"
)]

use std::collections::BTreeMap;

use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::identifier_protection::body;
use crate::identifier_protection::config::IdentifierProtectionConfig;
use crate::identifier_protection::crypto::{CryptoError, DomainKeys, KeyDomain, RootKey};
use crate::identifier_protection::store::{IdentifierStore, StoreError};
use crate::ids::VoId;
use crate::storage::identifier_repo::{self, IdentifierRepoError};
use crate::versioning::body::Body;
use crate::versioning::tree::{Shape, Tree};

/// The scheme a care subject reference is digested under (#5699).
const CARE_SUBJECT_SCHEME: &str = "ferroehr:care-subject:v1";

/// The scheme of the care projection's key check (#5699).
const CARE_KEY_CHECK_SCHEME: &str = "ferroehr:care-subject:check:v1";

/// The national-identifier protection engine.
#[derive(Debug)]
pub struct IdentifierProtection {
    schemes: Vec<String>,
    root: RootKey,
    store: IdentifierStore,
}

/// A configuration this engine cannot be built from.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The root key is missing while protection is enabled.
    #[error(
        "[privacy.identifier_protection] is configured but no `key` or `key_file` is set; \
         protection cannot run without one"
    )]
    MissingKey,
    /// The root key is not 32 bytes of hex.
    #[error(transparent)]
    Key(#[from] CryptoError),
    /// No scheme is configured, so protection would silently do nothing.
    #[error(
        "[privacy.identifier_protection] is configured but `schemes` is empty, so nothing \
         would be protected; name the schemes or remove the table"
    )]
    NoSchemes,
}

impl IdentifierProtection {
    /// Build the engine from the configured section and the demographic pool.
    ///
    /// Returns `None` when protection is off (no
    /// `[privacy.identifier_protection]` table), which is the default: the
    /// write path then stores identifiers exactly as written.
    ///
    /// # Errors
    /// [`EngineError`] when protection is on but unusable — a missing or
    /// malformed key, or an empty scheme list. Both are boot errors: a
    /// deployment that believes it protects identifiers and does not is the
    /// failure this refuses to ship.
    pub fn from_config(
        config: Option<&IdentifierProtectionConfig>,
        key_material: Option<&ferrohealth_secret::secret::Secret>,
        pool: sqlx::PgPool,
    ) -> Result<Option<Self>, EngineError> {
        let Some(config) = config else {
            return Ok(None);
        };
        if config.schemes.is_empty() {
            return Err(EngineError::NoSchemes);
        }
        let material = key_material.ok_or(EngineError::MissingKey)?;
        // The configured secret is re-wrapped rather than read as a plain
        // string: `RootKey` takes a secret so the hex never lands in an
        // ordinary `String` a `Debug` or a panic message could carry.
        let material = secrecy::SecretString::from(material.expose().to_owned());
        Ok(Some(Self {
            schemes: config.schemes.clone(),
            root: RootKey::from_hex(&material)?,
            store: IdentifierStore::new(pool),
        }))
    }

    /// The lookup digest of `value` in each configured scheme, in the
    /// configured order: what an equality search for an identifier of a
    /// protected scheme compares, without the value leaving the caller.
    #[must_use]
    pub fn lookup_digests(&self, value: &str) -> Vec<(String, Vec<u8>)> {
        let keys = DomainKeys::derive(&self.root, KeyDomain::Demographic);
        self.schemes
            .iter()
            .map(|scheme| (scheme.clone(), keys.lookup_digest(scheme, value)))
            .collect()
    }

    /// The keyed digest of the care subject reference `key` (#5699): the
    /// lookup digest under the clinical domain's subkey, in a scheme of its
    /// own, so the care projection and the EHR row match one subject without
    /// either holding the reference. A registry or national number has few
    /// enough values that an unkeyed digest of it is reversible. No openEHR
    /// spec governs this: our own design.
    #[must_use]
    pub fn care_subject_digest(&self, key: &str) -> Vec<u8> {
        DomainKeys::derive(&self.root, KeyDomain::Ehr).lookup_digest(CARE_SUBJECT_SCHEME, key)
    }

    /// The keyed digests of each of `keys`, under one derivation.
    #[must_use]
    pub fn care_subject_digests<'a>(
        &self,
        keys: impl IntoIterator<Item = &'a str>,
    ) -> Vec<Vec<u8>> {
        let derived = DomainKeys::derive(&self.root, KeyDomain::Ehr);
        keys.into_iter()
            .map(|key| derived.lookup_digest(CARE_SUBJECT_SCHEME, key))
            .collect()
    }

    /// A digest that changes with the key, which tells the care projector
    /// that the stored digests were taken under another key.
    #[must_use]
    pub fn care_key_check(&self) -> Vec<u8> {
        DomainKeys::derive(&self.root, KeyDomain::Ehr).lookup_digest(CARE_KEY_CHECK_SCHEME, "")
    }

    /// The first configured scheme the database registry does not hold.
    ///
    /// The boot check behind the rule that an unregistered scheme is a boot
    /// error rather than a first-write surprise: a deployment that names a
    /// scheme the registry refuses would otherwise fail its first party commit
    /// carrying one.
    ///
    /// # Errors
    /// [`IdentifierRepoError::UnknownScheme`] naming the first unregistered
    /// scheme, and [`IdentifierRepoError::Acquire`] or
    /// [`IdentifierRepoError::Database`] when the registry cannot be read.
    pub async fn verify_registered(&self) -> Result<(), IdentifierRepoError> {
        match identifier_repo::unregistered(self.store.pool(), &self.schemes).await? {
            Some(scheme) => Err(IdentifierRepoError::UnknownScheme { scheme }),
            None => Ok(()),
        }
    }

    /// Seal the protected identifiers of a new party version on `conn`, the
    /// party commit's own transaction, and leave references in `body`.
    ///
    /// Every version gets rows of its own: the party's live rows are retired
    /// first, then each identifier the body carries is sealed into a new live
    /// row — a plaintext value of a configured scheme, and a reference carried
    /// forward from an earlier version of this party alike, so no row is ever
    /// shared or rewritten and every version keeps the values it committed.
    /// The rows commit or roll back with the version that references them.
    /// Runs before the body is decomposed and signed, so the stored, signed and
    /// served form is one and the same — the invariant the commit path already
    /// keeps for externalized multimedia.
    ///
    /// # Errors
    /// [`StoreError::ForeignReference`] when the body references a row that is
    /// not this party's or that holds another scheme,
    /// [`IdentifierRepoError::SchemeRepeated`] when it carries two values in one
    /// scheme and [`IdentifierRepoError::ValueTaken`] when a value is live for
    /// another party, both as [`StoreError::Repo`], and
    /// [`StoreError`] when a value cannot be opened, sealed or stored. The
    /// write fails rather than proceeding: storing the body with the value
    /// still in it is precisely what this exists to prevent.
    pub(crate) async fn externalize(
        &self,
        conn: &mut PgConnection,
        party: Uuid,
        body: &mut Body,
    ) -> Result<usize, StoreError> {
        let keys = DomainKeys::derive(&self.root, KeyDomain::Demographic);
        // NOTE: a typed body is scanned through its rendering, which the
        // references are then written back into by path (#6003).
        let rendered = match &*body {
            Body::Canonical(_) => None,
            typed => {
                crate::telemetry::conversion::record("rm_to_index");
                Some(crate::storage::render::index::render(typed).map_err(StoreError::Render)?)
            }
        };
        let (mut identifiers, held) = match (&*body, &rendered) {
            (_, Some(rendered)) => self.scan(crate::versioning::tree::Indexed::root(rendered)),
            (Body::Canonical(canonical), None) => self.scan(canonical),
            (_, None) => (Vec::new(), Vec::new()),
        };
        for (pointer, row, written) in held {
            // NOTE: personal-data rule (an access event per read by a caller): this decrypts
            // only to re-seal inside the commit, so no caller reads and no event is due.
            let Some((scheme, value)) =
                IdentifierStore::open_held(&mut *conn, &keys, party, row).await?
            else {
                return Err(StoreError::ForeignReference);
            };
            if written.as_deref() != Some(scheme.as_str()) {
                return Err(StoreError::ForeignReference);
            }
            identifiers.push(body::FoundIdentifier {
                scheme,
                value,
                pointer,
            });
        }
        identifier_repo::retire_live(&mut *conn, VoId(party)).await?;
        // One row per scheme: the same value written twice shares it, and two
        // values in one scheme are refused before anything is written.
        let mut sealed: BTreeMap<String, (String, Uuid)> = BTreeMap::new();
        let mut references = Vec::with_capacity(identifiers.len());
        for identifier in identifiers {
            let row = match sealed.get(&identifier.scheme) {
                Some((value, row)) if *value == identifier.value => *row,
                Some(_) => {
                    return Err(StoreError::Repo(IdentifierRepoError::SchemeRepeated {
                        scheme: identifier.scheme,
                    }));
                }
                None => {
                    let row = IdentifierStore::seal(
                        &mut *conn,
                        &keys,
                        party,
                        &identifier.scheme,
                        &identifier.value,
                    )
                    .await?;
                    sealed.insert(identifier.scheme, (identifier.value, row));
                    row
                }
            };
            references.push((identifier.pointer, row));
        }
        match body {
            Body::Canonical(canonical) => body::substitute(canonical, &references),
            typed => {
                if let Some(rm) = typed.as_rm_mut() {
                    body::substitute_typed(rm, &references).map_err(StoreError::Substitute)?;
                }
            }
        }
        Ok(references.len())
    }

    /// The plaintext values of the configured schemes `body` carries, and the
    /// references it holds, each with the `DV_IDENTIFIER.type` written beside it.
    fn scan<'a, T: Tree<'a>>(&self, body: T) -> (Vec<body::FoundIdentifier>, Vec<Held>) {
        let held = referenced(body)
            .into_iter()
            .map(|(pointer, row)| {
                let written = body
                    .pointer_text(&format!("{pointer}/type"))
                    .map(std::borrow::Cow::into_owned);
                (pointer, row, written)
            })
            .collect();
        (body::find(body, &self.schemes), held)
    }

    /// Refuse a party body that must be stored verbatim but would hold a
    /// protected value this deployment has not sealed.
    ///
    /// The import and restore paths keep a received version byte-for-byte (RM
    /// common `master06-change_control_package.adoc` §Copying: "the
    /// `ORIGINAL_VERSION` instance is never modified"), so they cannot seal on
    /// the way in; a refusal is the only way the value stays out of the
    /// record. A reference is admitted only when `party` holds its row here,
    /// read on `conn` so a restore's own rows count.
    ///
    /// # Errors
    /// [`StoreError::Unsealed`] for a plaintext value of a protected scheme,
    /// [`StoreError::ForeignReference`] for a reference `party` does not hold
    /// (or one whose row holds another scheme), and [`StoreError`] when a
    /// referenced row cannot be read or does not authenticate under this
    /// deployment's key.
    pub async fn refuse_unsealed(
        &self,
        conn: &mut PgConnection,
        party: Uuid,
        body: &Value,
    ) -> Result<(), StoreError> {
        if let Some(plain) = body::find(body, &self.schemes).into_iter().next() {
            return Err(StoreError::Unsealed {
                scheme: plain.scheme,
            });
        }
        let keys = DomainKeys::derive(&self.root, KeyDomain::Demographic);
        for (pointer, row) in referenced(body) {
            let written = body
                .pointer(&format!("{pointer}/type"))
                .and_then(Value::as_str);
            // NOTE: personal-data rule (an access event per read by a caller): the value
            // is opened only to prove the row authenticates; nothing leaves the server.
            let held = IdentifierStore::open_held(&mut *conn, &keys, party, row).await?;
            if held.is_none_or(|(scheme, _)| written != Some(scheme.as_str())) {
                return Err(StoreError::ForeignReference);
            }
        }
        Ok(())
    }

    /// Put the plaintext values back into a stored body, and return the
    /// scheme of each value put back, in document order.
    ///
    /// The audited expansion: the caller is responsible for deciding that this
    /// reader may see the values and for recording the access.
    ///
    /// # Errors
    /// [`StoreError::ForeignReference`] for a reference whose identifier names
    /// no scheme, and [`StoreError`] when a referenced row cannot be read or
    /// does not authenticate. A reference whose row is gone expands to nothing and
    /// leaves the reference in place, which says truthfully that the value was
    /// held and is no longer.
    pub async fn expand(&self, body: &mut Value) -> Result<Vec<String>, StoreError> {
        let keys = DomainKeys::derive(&self.root, KeyDomain::Demographic);
        let references = referenced(&*body);
        let mut values = Vec::with_capacity(references.len());
        let mut schemes = Vec::with_capacity(references.len());
        for (pointer, row) in references {
            if let Some(value) = self.store.open(&keys, row).await? {
                // A reference whose identifier names no scheme was not written
                // by `externalize`, which keeps the scheme beside every row.
                let scheme = body
                    .pointer(&format!("{pointer}/type"))
                    .and_then(Value::as_str)
                    .ok_or(StoreError::ForeignReference)?
                    .to_owned();
                schemes.push(scheme);
                values.push((pointer, value));
            }
        }
        body::expand(body, &values);
        Ok(schemes)
    }
}

/// A reference a body holds: its pointer, the row it names and the
/// `DV_IDENTIFIER.type` written beside it.
type Held = (String, Uuid, Option<String>);

/// Every protected-identifier reference in a stored body, with its pointer.
fn referenced<'a, T: Tree<'a>>(body: T) -> Vec<(String, Uuid)> {
    let mut found = Vec::new();
    collect(body, &mut String::new(), &mut found);
    found
}

/// Recursive walk collecting reference-bearing `DV_IDENTIFIER` nodes.
fn collect<'a, T: Tree<'a>>(node: T, pointer: &mut String, found: &mut Vec<(String, Uuid)>) {
    match node.shape() {
        Shape::Object => {
            if node.get_text("_type").as_deref() == Some("DV_IDENTIFIER")
                && let Some(id) = node.get_text("id")
                && let Some(row) = body::referenced_row(&id)
            {
                found.push((pointer.clone(), row));
            }
            for (key, child) in node.members() {
                let mark = pointer.len();
                pointer.push('/');
                pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
                collect(child, pointer, found);
                pointer.truncate(mark);
            }
        }
        Shape::Array => {
            for (index, child) in node.items().enumerate() {
                let mark = pointer.len();
                pointer.push('/');
                pointer.push_str(&index.to_string());
                collect(child, pointer, found);
                pointer.truncate(mark);
            }
        }
        Shape::String | Shape::Other => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{EngineError, IdentifierProtection, referenced};
    use crate::identifier_protection::config::IdentifierProtectionConfig;
    use serde_json::json;

    fn pool() -> sqlx::PgPool {
        // A lazy pool: these tests never issue a statement, they only exercise
        // the construction rules.
        sqlx::PgPool::connect_lazy("postgres://localhost/ferroehr-unused")
            .expect("a lazy pool needs no server")
    }

    #[tokio::test]
    async fn protection_is_off_by_default_and_builds_nothing() {
        let engine = IdentifierProtection::from_config(None, None, pool())
            .expect("the default configuration builds");
        assert!(
            engine.is_none(),
            "off by default: the write path stores identifiers as written"
        );
    }

    #[tokio::test]
    async fn enabled_without_a_key_is_a_boot_error() {
        let config = IdentifierProtectionConfig::default();
        assert!(
            matches!(
                IdentifierProtection::from_config(Some(&config), None, pool()),
                Err(EngineError::MissingKey)
            ),
            "a deployment that believes it protects identifiers and cannot must not boot"
        );
    }

    #[tokio::test]
    async fn enabled_with_no_schemes_is_a_boot_error() {
        let config = IdentifierProtectionConfig {
            schemes: Vec::new(),
            ..IdentifierProtectionConfig::default()
        };
        let key = ferrohealth_secret::secret::Secret::new(
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        );
        assert!(
            matches!(
                IdentifierProtection::from_config(Some(&config), Some(&key), pool()),
                Err(EngineError::NoSchemes)
            ),
            "protection that would protect nothing is a configuration mistake, not a posture"
        );
    }

    #[test]
    fn references_are_found_by_pointer_in_a_stored_body() {
        let row = uuid::Uuid::from_u128(42);
        let body = json!({
            "identities": [{ "details": { "items": [
                { "value": { "_type": "DV_IDENTIFIER", "type": "nl-bsn",
                             "id": format!("urn:ferroehr:protected-identifier:{row}") } }
            ]}}]
        });
        assert_eq!(
            referenced(&body),
            vec![("/identities/0/details/items/0/value".to_owned(), row)]
        );
    }
}
