// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The generic-parameter text of an emitted impl: its header, its type
//! arguments and the marker a visitor carries them in.

/// `(impl<…: Bound> , <…>)` — the impl-generics header and the type-args suffix.
pub(crate) fn generic_header(generics: &[String], bound: &str) -> (String, String) {
    if generics.is_empty() {
        (String::new(), String::new())
    } else {
        let bounded = generics
            .iter()
            .map(|g| format!("{g}: {bound}"))
            .collect::<Vec<_>>()
            .join(", ");
        (format!("<{bounded}>"), format!("<{}>", generics.join(", ")))
    }
}

/// The `impl<'de, …>` header + type-args suffix for a `Deserialize` impl.
///
/// Generic parameters are bound by `DeserializeOwned`, not `Deserialize<'de>`:
/// every emitted spec type owns its data (no borrowed fields anywhere — see
/// <https://serde.rs/lifetimes.html>), and the untagged structural fallback has
/// to deserialize a parameter from a LOCAL buffered value whose lifetime is
/// shorter than `'de`, which only the `for<'a>` bound permits.
pub(crate) fn deserialize_header(generics: &[String]) -> (String, String) {
    if generics.is_empty() {
        ("<'de>".to_owned(), String::new())
    } else {
        let bounded = generics
            .iter()
            .map(|g| format!("{g}: ::serde::de::DeserializeOwned"))
            .collect::<Vec<_>>()
            .join(", ");
        (
            format!("<'de, {bounded}>"),
            format!("<{}>", generics.join(", ")),
        )
    }
}

/// `PhantomData<(T, U)>` for a visitor that must carry the impl's generic
/// parameters (an item declared inside a function body does not inherit them).
pub(crate) fn phantom(generics: &[String]) -> (String, String, String) {
    if generics.is_empty() {
        (String::new(), String::new(), String::new())
    } else {
        let params = generics.join(", ");
        (
            format!("<{params}>"),
            format!("(::core::marker::PhantomData<({params},)>)"),
            "(::core::marker::PhantomData)".to_owned(),
        )
    }
}
