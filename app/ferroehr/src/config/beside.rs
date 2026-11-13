// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A family crate's section read with FerroEHR's own keys beside it in one
//! table: `[auth]` holds `ferrohealth-smart`'s keys with `[auth.smart]` and
//! `[auth.scim]`, and `[server]` holds `ferrohealth-server`'s keys with
//! `public_base_url`.
//!
//! The crate reads its section with `deny_unknown_fields`, so the reader
//! takes FerroEHR's keys out of the table and hands every other key to the
//! crate's section: an unknown key is still refused with the crate's field
//! list, at its path. No openEHR spec governs configuration: our own design.

use std::fmt;
use std::marker::PhantomData;

use serde::de::value::{MapAccessDeserializer, StringDeserializer};
use serde::de::{DeserializeSeed, Error as _, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

/// FerroEHR's own keys of a table a family crate's section shares.
pub(crate) trait Own: Default {
    /// The table, as the messages name it, such as `[auth]`.
    const TABLE: &'static str;

    /// The own key `key` names, when it is one.
    fn own(key: &str) -> Option<&'static str>;

    /// Reads the value of the own key `key` from `map`.
    ///
    /// # Errors
    /// The value's own error, or a key read twice ([`read_once`]).
    fn read<'de, A: MapAccess<'de>>(
        &mut self,
        key: &'static str,
        map: &mut A,
    ) -> Result<(), A::Error>;
}

/// Reads the entries of `map` into the family section `S` and FerroEHR's
/// keys `O`.
///
/// # Errors
/// The first error of either: an unknown key with the crate's field list,
/// a value of the wrong shape, or an own key read twice.
pub(crate) fn split<'de, S: Deserialize<'de>, O: Own, A: MapAccess<'de>>(
    map: A,
) -> Result<(S, O), A::Error> {
    let mut own = O::default();
    let shared = S::deserialize(MapAccessDeserializer::new(WithoutOwn {
        inner: map,
        own: &mut own,
    }))?;
    Ok((shared, own))
}

/// Reads the value of the own key `key` into `slot`, refusing it twice.
///
/// # Errors
/// The value's own error, or `key` read before.
pub(crate) fn read_once<'de, A: MapAccess<'de>, T: Deserialize<'de>>(
    map: &mut A,
    slot: &mut Option<T>,
    key: &'static str,
) -> Result<(), A::Error> {
    if slot.is_some() {
        return Err(A::Error::duplicate_field(key));
    }
    *slot = Some(map.next_value()?);
    Ok(())
}

/// The entries of a table with FerroEHR's keys taken out into `own`.
struct WithoutOwn<'a, A, O> {
    /// The table's entries.
    inner: A,
    /// Where FerroEHR's keys go.
    own: &'a mut O,
}

impl<'de, A: MapAccess<'de>, O: Own> MapAccess<'de> for WithoutOwn<'_, A, O> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        let mut seed = Some(seed);
        loop {
            let key = KeyOrOwn {
                seed: &mut seed,
                own: PhantomData::<O>,
            };
            match self.inner.next_key_seed(key)? {
                None => return Ok(None),
                Some(Key::Shared(value)) => return Ok(Some(value)),
                Some(Key::Own(key)) => self.own.read(key, &mut self.inner)?,
            }
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        self.inner.next_value_seed(seed)
    }
}

/// A key of the table: one of the crate's, read by its seed, or one of
/// FerroEHR's.
enum Key<T> {
    /// A key the crate's section reads.
    Shared(T),
    /// A key FerroEHR reads.
    Own(&'static str),
}

/// Reads a key of the table inside the table's own key read, so a refused
/// key is reported at its path; the seed is taken by the first key that is
/// not one of FerroEHR's.
struct KeyOrOwn<'s, K, O> {
    /// The crate's key seed, until a key of the crate takes it.
    seed: &'s mut Option<K>,
    /// FerroEHR's keys of the table.
    own: PhantomData<O>,
}

impl<'de, K: DeserializeSeed<'de>, O: Own> DeserializeSeed<'de> for KeyOrOwn<'_, K, O> {
    type Value = Key<K::Value>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_identifier(self)
    }
}

impl<'de, K: DeserializeSeed<'de>, O: Own> Visitor<'de> for KeyOrOwn<'_, K, O> {
    type Value = Key<K::Value>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a key of the {} table", O::TABLE)
    }

    fn visit_str<E: serde::de::Error>(self, key: &str) -> Result<Self::Value, E> {
        if let Some(own) = O::own(key) {
            return Ok(Key::Own(own));
        }
        let seed = self
            .seed
            .take()
            .ok_or_else(|| E::custom(format!("the {} key was read twice", O::TABLE)))?;
        seed.deserialize(StringDeserializer::<E>::new(key.to_owned()))
            .map(Key::Shared)
    }
}
