// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: Apache-2.0

//! Whether a value of this generation can be read by the released RM
//! generation (`v1_1`), decided over typed RM (#6002).
//!
//! The `stable` profile serves only what the released generation expresses,
//! so a commit under the development generation stores that answer. The
//! answer here is the released strict reader's on this value's canonical JSON:
//! that reader refuses an undeclared attribute, a mandatory attribute left out
//! and an object of a class its model does not declare there, and the
//! generated rules (`released/generated.rs`) are those three, per class, from
//! the two generations' BMMs. A test holds the two answers equal on the
//! corpus. No openEHR specification governs runtime generation selection: our
//! own design.

mod generated;

/// A value of this generation the released reader may or may not express.
pub trait Expressible {
    /// The value's RM class (empty for a primitive).
    fn rm_type(&self) -> &'static str;
    /// Whether the released generation's strict reader reads this value.
    fn expressible(&self) -> bool;
}

/// A primitive a generic class is instantiated over carries nothing a
/// generation could fail to express.
macro_rules! primitive {
    ($($t:ty),*) => {$(
        impl Expressible for $t {
            fn rm_type(&self) -> &'static str {
                ""
            }
            fn expressible(&self) -> bool {
                true
            }
        }
    )*};
}

primitive!(String, bool, char, u8, i32, i64, f32, f64);

/// The free-form content a versioned container is instantiated over in the
/// model (`X_VERSIONED_OBJECT<Any>`) is read as it stands by either reader.
#[expect(
    clippy::disallowed_types,
    reason = "the RM's own `Any` content renders as serde_json::Value (#1694 generated-model family)"
)]
impl Expressible for serde_json::Value {
    fn rm_type(&self) -> &'static str {
        ""
    }
    fn expressible(&self) -> bool {
        true
    }
}

impl<T: Expressible + ?Sized> Expressible for Box<T> {
    fn rm_type(&self) -> &'static str {
        T::rm_type(self)
    }
    fn expressible(&self) -> bool {
        T::expressible(self)
    }
}

/// Whether `value` is of a class the released model admits where it declares
/// `declared`, and expressible itself.
fn conforms<T: Expressible + ?Sized>(value: &T, declared: &str) -> bool {
    crate::v1_1::model::is_a(value.rm_type(), declared) && value.expressible()
}

/// Whether the released RM generation's strict reader reads `value`.
#[must_use]
pub fn expressible_in_released<T: Expressible + ?Sized>(value: &T) -> bool {
    value.expressible()
}
