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

use serde_json::Value;
use uuid::Uuid;

use crate::service::demographic::identifier::body;
use crate::service::demographic::identifier::config::IdentifierProtectionConfig;
use crate::service::demographic::identifier::crypto::{
    CryptoError, DomainKeys, KeyDomain, RootKey,
};
use crate::service::demographic::identifier::store::{IdentifierStore, StoreError};

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
        "[demographic.identifier_protection] is enabled but no `key` or `key_file` is set; \
         protection cannot run without one"
    )]
    MissingKey,
    /// The root key is not 32 bytes of hex.
    #[error(transparent)]
    Key(#[from] CryptoError),
    /// No scheme is configured, so protection would silently do nothing.
    #[error(
        "[demographic.identifier_protection] is enabled but `schemes` is empty, so nothing \
         would be protected; name the schemes or turn the section off"
    )]
    NoSchemes,
}

impl IdentifierProtection {
    /// Build the engine from the configured section and the demographic pool.
    ///
    /// Returns `None` when protection is off, which is the default: the write
    /// path then stores identifiers exactly as written.
    ///
    /// # Errors
    /// [`EngineError`] when protection is on but unusable — a missing or
    /// malformed key, or an empty scheme list. Both are boot errors: a
    /// deployment that believes it protects identifiers and does not is the
    /// failure this refuses to ship.
    pub fn from_config(
        config: &IdentifierProtectionConfig,
        key_material: Option<&crate::config::secret::Secret>,
        pool: sqlx::PgPool,
    ) -> Result<Option<Self>, EngineError> {
        if !config.enabled {
            return Ok(None);
        }
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

    /// The schemes this engine protects.
    #[must_use]
    pub fn schemes(&self) -> &[String] {
        &self.schemes
    }

    /// Take every protected identifier out of `body`, leaving a reference.
    ///
    /// Runs before the body is decomposed and signed, so the stored, signed
    /// and served form is one and the same — the invariant the commit path
    /// already keeps for externalized multimedia.
    ///
    /// # Errors
    /// [`StoreError`] when a value cannot be sealed or stored. The write fails
    /// rather than proceeding: storing the body with the value still in it is
    /// precisely what this exists to prevent.
    pub async fn externalize(&self, party: Uuid, body: &mut Value) -> Result<usize, StoreError> {
        let found = body::find(body, &self.schemes);
        if found.is_empty() {
            return Ok(0);
        }
        let keys = DomainKeys::derive(&self.root, KeyDomain::Demographic);
        let mut references = Vec::with_capacity(found.len());
        for identifier in &found {
            let row = self
                .store
                .seal(&keys, party, &identifier.scheme, &identifier.value)
                .await?;
            references.push((identifier.pointer.clone(), row));
        }
        body::substitute(body, &references);
        Ok(references.len())
    }

    /// Put the plaintext values back into a stored body.
    ///
    /// The audited expansion: the caller is responsible for deciding that this
    /// reader may see the values and for recording the access.
    ///
    /// # Errors
    /// [`StoreError`] when a referenced row cannot be read or does not
    /// authenticate. A reference whose row is gone expands to nothing and
    /// leaves the reference in place, which says truthfully that the value was
    /// held and is no longer.
    pub async fn expand(&self, body: &mut Value) -> Result<usize, StoreError> {
        let keys = DomainKeys::derive(&self.root, KeyDomain::Demographic);
        let references = referenced(body);
        let mut values = Vec::with_capacity(references.len());
        for (pointer, row) in references {
            if let Some(value) = self.store.open(&keys, row).await? {
                values.push((pointer, value));
            }
        }
        body::expand(body, &values);
        Ok(values.len())
    }

    /// The opaque subject pseudonym `party` is known by on the clinical side
    /// ([`RootKey::subject_pseudonym`]).
    #[must_use]
    pub fn subject_pseudonym(&self, party: Uuid) -> Uuid {
        self.root.subject_pseudonym(party)
    }

    /// The party holding `value` in `scheme`, resolved through the keyed digest.
    ///
    /// # Errors
    /// [`StoreError`] when the resolution query fails.
    pub async fn resolve(&self, scheme: &str, value: &str) -> Result<Option<Uuid>, StoreError> {
        let keys = DomainKeys::derive(&self.root, KeyDomain::Demographic);
        self.store.resolve(&keys, scheme, value).await
    }
}

/// Every protected-identifier reference in a stored body, with its pointer.
fn referenced(body: &Value) -> Vec<(String, Uuid)> {
    let mut found = Vec::new();
    collect(body, &mut String::new(), &mut found);
    found
}

/// Recursive walk collecting reference-bearing `DV_IDENTIFIER` nodes.
fn collect(node: &Value, pointer: &mut String, found: &mut Vec<(String, Uuid)>) {
    match node {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some("DV_IDENTIFIER")
                && let Some(id) = map.get("id").and_then(Value::as_str)
                && let Some(row) = body::referenced_row(id)
            {
                found.push((pointer.clone(), row));
            }
            for (key, child) in map {
                let mark = pointer.len();
                pointer.push('/');
                pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
                collect(child, pointer, found);
                pointer.truncate(mark);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let mark = pointer.len();
                pointer.push('/');
                pointer.push_str(&index.to_string());
                collect(child, pointer, found);
                pointer.truncate(mark);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{EngineError, IdentifierProtection, referenced};
    use crate::service::demographic::identifier::config::IdentifierProtectionConfig;
    use serde_json::json;

    fn pool() -> sqlx::PgPool {
        // A lazy pool: these tests never issue a statement, they only exercise
        // the construction rules.
        sqlx::PgPool::connect_lazy("postgres://localhost/ferroehr-unused")
            .expect("a lazy pool needs no server")
    }

    #[tokio::test]
    async fn protection_is_off_by_default_and_builds_nothing() {
        let engine =
            IdentifierProtection::from_config(&IdentifierProtectionConfig::default(), None, pool())
                .expect("the default configuration builds");
        assert!(
            engine.is_none(),
            "off by default: the write path stores identifiers as written"
        );
    }

    #[tokio::test]
    async fn enabled_without_a_key_is_a_boot_error() {
        let config = IdentifierProtectionConfig {
            enabled: true,
            ..IdentifierProtectionConfig::default()
        };
        assert!(
            matches!(
                IdentifierProtection::from_config(&config, None, pool()),
                Err(EngineError::MissingKey)
            ),
            "a deployment that believes it protects identifiers and cannot must not boot"
        );
    }

    #[tokio::test]
    async fn enabled_with_no_schemes_is_a_boot_error() {
        let config = IdentifierProtectionConfig {
            enabled: true,
            schemes: Vec::new(),
            ..IdentifierProtectionConfig::default()
        };
        let key = crate::config::secret::Secret::new(
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        );
        assert!(
            matches!(
                IdentifierProtection::from_config(&config, Some(&key), pool()),
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
